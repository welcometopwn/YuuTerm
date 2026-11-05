use crate::{Backend, lock, main_only, rdp};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    net::{Ipv4Addr, SocketAddrV4, UdpSocket},
    time::{Duration, Instant},
};
use tauri::{Manager, WebviewWindow};
use yuuterm_core::{
    Protocol,
    config::{self, Preferences},
    session_actions as edits, wol,
};

#[tauri::command]
pub async fn move_profiles(
    window: WebviewWindow,
    profile_ids: Vec<u64>,
    group: String,
    before_id: Option<u64>,
) -> Result<Preferences, String> {
    main_only(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Backend>();
        let mut c = lock(&state.config)?;
        let fixture = cfg!(feature = "capture") && c.fixture;
        if !c.can_save && !fixture {
            return Err("Settings saving is disabled".into());
        }
        let candidate = edits::move_to(&c.prefs, &profile_ids, &group, before_id)?;
        if !fixture {
            config::save(&candidate)?;
        }
        c.prefs = candidate.clone();
        Ok(candidate)
    })
    .await
    .map_err(|_| "Move worker stopped")?
}

#[tauri::command]
pub async fn delete_profiles(
    window: WebviewWindow,
    profile_ids: Vec<u64>,
    detach: Option<bool>,
) -> Result<Preferences, String> {
    main_only(&window)?;
    if profile_ids.is_empty() || profile_ids.len() > 1000 {
        return Err("Select 1–1000 sessions".into());
    }
    let check_ids = profile_ids.clone();
    let (send, receive) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            let _ = send.send(rdp::has_profiles(&check_ids));
        })
        .map_err(|_| "Cannot check RDP sessions")?;
    if tokio::time::timeout(Duration::from_secs(2), receive)
        .await
        .map_err(|_| "RDP check timed out")?
        .map_err(|_| "RDP check stopped")?
    {
        return Err("Close the selected RDP sessions before deleting them".into());
    }
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Backend>();
        if state.vnc.has_profiles(&profile_ids)? {
            return Err("Close the selected VNC sessions before deleting them".into());
        }
        if lock(&state.sessions)?
            .values()
            .any(|s| profile_ids.contains(&s.profile_id) && !s.live.snapshot().finished)
        {
            return Err("Disconnect the selected sessions before deleting them".into());
        }
        let mut c = lock(&state.config)?;
        let removed: Vec<_> = edits::selected(&c.prefs, &profile_ids)?
            .into_iter()
            .cloned()
            .collect();
        let fixture = cfg!(feature = "capture") && c.fixture;
        if !c.can_save && !fixture && removed.iter().any(|p| !p.temporary) {
            return Err("Settings saving is disabled".into());
        }
        let candidate = edits::remove_detaching(&c.prefs, &profile_ids, detach.unwrap_or(false))?;
        if c.can_save && !fixture {
            config::save(&candidate)?;
        }
        c.prefs = candidate.clone();
        drop(c);
        for profile in removed {
            crate::ssh_gateways::forget_passwords(&profile);
        }
        Ok(candidate)
    })
    .await
    .map_err(|_| "Delete worker stopped")?
}

#[derive(Serialize)]
pub struct WakeResult {
    pub sent: usize,
    pub skipped: usize,
    pub failed: usize,
}

#[tauri::command]
pub async fn wake_profiles(
    window: WebviewWindow,
    profile_ids: Vec<u64>,
) -> Result<WakeResult, String> {
    main_only(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Backend>();
        let c = lock(&state.config)?;
        let selected = edits::selected(&c.prefs, &profile_ids)?;
        let macs: BTreeSet<_> = selected
            .iter()
            .filter(|p| !matches!(p.protocol, Protocol::Local | Protocol::Serial))
            .filter_map(|p| wol::parse_mac(&p.mac_address).ok())
            .collect();
        let skipped = selected.len() - macs.len();
        let fixture = cfg!(feature = "capture") && c.fixture;
        drop(c);
        let destination = if fixture {
            let port = std::env::var("YUUTERM_WOL_TEST_PORT")
                .ok()
                .and_then(|s| s.parse::<u16>().ok())
                .filter(|p| *p != 0)
                .ok_or("Wake-on-LAN capture requires an isolated receiver")?;
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)
        } else {
            SocketAddrV4::new(Ipv4Addr::BROADCAST, 9)
        };
        send_packets(macs, skipped, destination)
    })
    .await
    .map_err(|_| "Wake-on-LAN worker stopped")?
}

fn send_packets(
    macs: BTreeSet<[u8; 6]>,
    skipped: usize,
    destination: SocketAddrV4,
) -> Result<WakeResult, String> {
    if macs.is_empty() {
        return Ok(WakeResult {
            sent: 0,
            skipped,
            failed: 0,
        });
    }
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .map_err(|_| "Cannot open Wake-on-LAN socket")?;
    socket
        .set_broadcast(true)
        .map_err(|_| "Cannot enable Wake-on-LAN broadcast")?;
    socket
        .set_write_timeout(Some(Duration::from_millis(250)))
        .map_err(|_| "Cannot set Wake-on-LAN timeout")?;
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut result = WakeResult {
        sent: 0,
        skipped,
        failed: 0,
    };
    for mac in macs {
        if Instant::now() < deadline && socket.send_to(&wol::magic_packet(mac), destination).is_ok()
        {
            result.sent += 1;
        } else {
            result.failed += 1;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_udp_receiver_gets_the_magic_packet_without_any_connection() {
        let receiver = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let destination = match receiver.local_addr().unwrap() {
            std::net::SocketAddr::V4(v) => v,
            _ => unreachable!(),
        };
        let mac = wol::parse_mac("02:11:22:33:44:55").unwrap();
        let result = send_packets(BTreeSet::from([mac]), 1, destination).unwrap();
        assert_eq!((result.sent, result.skipped, result.failed), (1, 1, 0));
        let mut bytes = [0; 103];
        let count = receiver.recv(&mut bytes).unwrap();
        assert_eq!(&bytes[..count], &wol::magic_packet(mac));
    }
}
