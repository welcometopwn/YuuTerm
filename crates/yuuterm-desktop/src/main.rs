#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tauri::{Emitter, Manager, State, WebviewWindow};
use yuuterm_core::{
    Profile,
    config::{self, Preferences},
};
use yuuterm_ssh::{Command, Engine, FileEntry, LiveSession, Snapshot, TrustPrompt};
pub mod credentials;
mod file_ui;
mod highlight_import;
mod ipc_origin;
mod log_rotation;
mod moba_icons;
mod open_with;
mod output_log;
mod project_page;
mod rdp;
mod rdp_cert;
mod rdp_events;
mod rdp_keys;
#[cfg(feature = "rdp-live")]
mod rdp_live;
#[cfg(feature = "rdp-probe")]
mod rdp_probe;
mod rdp_relay;
mod session_actions;
mod session_transfer;
mod single_instance;
mod telnet_external_capture;
mod vnc;
mod vnc_real_capture;
mod windows_openssh_capture;
mod x11_desktop;

fn rdp_live_enabled() -> bool {
    #[cfg(feature = "rdp-live")]
    return rdp_probe_enabled() && rdp_live::enabled();
    #[cfg(not(feature = "rdp-live"))]
    false
}
fn rdp_probe_enabled() -> bool {
    cfg!(feature = "rdp-probe") && std::env::var("YUUTERM_RDP_PROBE").as_deref() == Ok("1")
}

#[cfg(feature = "capture")]
mod capture;
mod gateway_capture;
mod ssh_gateways;
#[cfg(feature = "capture")]
mod startup_timing;

struct Session {
    profile_id: u64,
    file_origin: file_ui::Origin,
    live: LiveSession,
    output: tokio::sync::mpsc::Receiver<Vec<u8>>,
    wake: Arc<tokio::sync::Notify>,
    first: bool,
    last_files: Option<Arc<Vec<FileEntry>>>,
    file_busy: Arc<Mutex<Option<tokio_util::sync::CancellationToken>>>,
    output_log: Option<output_log::OutputLog>,
    /// Saved SSH password used for this connection and the bookmark that owns
    /// it (an RDP bookmark for remote tools); forgotten if refused.
    saved_password: Option<ssh_gateways::SavedPassword>,
    /// Saved passwords of the SSH gateways, by hop.
    saved_gateway_passwords: Vec<Option<ssh_gateways::SavedPassword>>,
    #[cfg(feature = "capture")]
    input_trace: Vec<Vec<u8>>,
}
struct Config {
    prefs: Preferences,
    can_save: bool,
    notice: String,
    fixture: bool,
}
struct Backend {
    vnc: vnc::DesktopVnc,
    engine: Engine,
    config: Mutex<Config>,
    sessions: Mutex<BTreeMap<u64, Session>>,
    edits: Mutex<BTreeMap<u64, file_ui::Edit>>,
    edit_polling: tokio::sync::Mutex<()>,
    next_edit: std::sync::atomic::AtomicU64,
    pending_import: Mutex<Option<session_transfer::Draft>>,
    log_close_waiting: std::sync::atomic::AtomicBool,
    #[cfg(feature = "capture")]
    startup_origin: std::time::Instant,
    #[cfg(feature = "capture")]
    startup_timings: startup_timing::Records,
}

/// Runs `then` once no RDP control is connected any more (at most 5 s), so no
/// control is destroyed mid-connection. Call on the UI thread after
/// `rdp::disconnect_all()`; waits on a worker and polls the UI thread.
fn after_rdp_disconnected(window: WebviewWindow, then: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let (sender, receiver) = std::sync::mpsc::channel();
            if window
                .run_on_main_thread(move || {
                    let _ = sender.send(rdp::any_connected());
                })
                .is_err()
                || receiver
                    .recv_timeout(std::time::Duration::from_secs(1))
                    .is_ok_and(|connected| !connected)
            {
                break;
            }
        }
        then();
    });
}

/// exit() with a live RDP control left the process in an unkillable kernel
/// wait at exit (measured with xrdp and Windows 11). Destroy the controls on
/// their UI thread first, then exit.
fn exit_without_rdp(app: &tauri::AppHandle, code: i32) {
    let (sender, receiver) = std::sync::mpsc::channel();
    if app
        .run_on_main_thread(move || {
            rdp::disconnect_all();
            rdp::close_all();
            let _ = sender.send(());
        })
        .is_ok()
    {
        let _ = receiver.recv_timeout(std::time::Duration::from_secs(5));
    }
    app.exit(code);
}

fn finish_logs_on_close(app: tauri::AppHandle) {
    // Filesystem calls can stall. Wait off the UI thread and keep the window
    // open on timeout rather than silently discarding pending log chunks.
    tauri::async_runtime::spawn_blocking(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let state = app.state::<Backend>();
            let statuses = state.sessions.lock().ok().map(|sessions| {
                sessions
                    .iter()
                    .filter_map(|(id, s)| s.output_log.as_ref().map(|log| (*id, log.status())))
                    .collect::<Vec<_>>()
            });
            if let Some(statuses) = statuses
                && statuses.iter().all(|(_, log)| log.finished)
            {
                #[cfg(feature = "capture")]
                if state.config.lock().is_ok_and(|c| c.fixture)
                    && let Ok(path) = std::env::var("YUUTERM_CAPTURE_REPORT")
                {
                    let path = std::path::PathBuf::from(path).with_file_name("log-shutdown.json");
                    if let Ok(bytes) = serde_json::to_vec_pretty(&statuses) {
                        let _ = std::fs::write(path, bytes);
                    }
                }
                exit_without_rdp(&app, 0);
                return;
            }
            if std::time::Instant::now() >= deadline {
                state
                    .log_close_waiting
                    .store(false, std::sync::atomic::Ordering::Release);
                let _ = app.emit("output-log-close-blocked", "An output log has not finished writing. YuuTerm stayed open to protect pending output; try closing again after storage responds.");
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    });
}

#[derive(Serialize)]
struct Bootstrap {
    prefs: Preferences,
    can_save: bool,
    notice: String,
    fixture: bool,
    rdp_probe: bool,
    rdp_live: bool,
}
#[derive(Serialize)]
struct SessionMeta {
    protocol: yuuterm_core::Protocol,
    endpoint: String,
    status: String,
    connected: bool,
    finished: bool,
    password_rejected: bool,
    trust: Option<TrustPrompt>,
    path: String,
    files_status: String,
    files: Option<Vec<FileEntry>>,
}
#[derive(Serialize)]
struct Poll {
    meta: Option<SessionMeta>,
    chunks: Vec<Vec<u8>>,
    more: bool,
    logging: Option<output_log::Status>,
}

#[derive(Serialize)]
struct BroadcastResult {
    id: u64,
    queued: bool,
}

#[tauri::command]
fn terminal_dispatch(
    window: WebviewWindow,
    state: State<Backend>,
    ids: Vec<u64>,
    text: String,
    append_enter: bool,
) -> Result<Vec<BroadcastResult>, String> {
    main_only(&window)?;
    let bytes = yuuterm_core::broadcast::prepare(&ids, &text, append_enter)?;
    #[cfg(feature = "capture")]
    let fixture = lock(&state.config)?.fixture;
    let mut sessions = lock(&state.sessions)?;
    // Validate the whole selection before queuing any input. Never resolve a
    // missing live ID to its bookmark's replacement connection.
    for id in &ids {
        let live = sessions
            .get(id)
            .ok_or("A selected connection no longer exists; review the targets again")?;
        let snapshot = live.live.snapshot();
        if !snapshot.connected
            || snapshot.finished
            || !matches!(
                snapshot.protocol,
                yuuterm_core::Protocol::Ssh
                    | yuuterm_core::Protocol::Telnet
                    | yuuterm_core::Protocol::Serial
                    | yuuterm_core::Protocol::Local
            )
        {
            return Err(
                "A selected terminal disconnected; nothing was queued by this dispatch".into(),
            );
        }
    }
    let mut results = Vec::with_capacity(ids.len());
    for id in ids {
        let session = sessions.get_mut(&id).unwrap();
        let queued = session.live.send(Command::Input(bytes.clone())).is_ok();
        #[cfg(feature = "capture")]
        if queued && fixture && session.input_trace.len() < 100 {
            session.input_trace.push(bytes.clone());
        }
        results.push(BroadcastResult { id, queued });
    }
    Ok(results)
}

fn main_only(window: &WebviewWindow) -> Result<(), String> {
    if window.label() == "main" && window.url().is_ok_and(|url| ipc_origin::allowed(&url)) {
        Ok(())
    } else {
        Err("This command is restricted to the main local window".into())
    }
}
fn lock<T>(mutex: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, String> {
    mutex
        .lock()
        .map_err(|_| "Application state unavailable".into())
}

#[tauri::command]
async fn serial_ports(window: WebviewWindow) -> Result<Vec<String>, String> {
    main_only(&window)?;
    tauri::async_runtime::spawn_blocking(|| yuuterm_ssh::serial_ports().map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
fn bootstrap(window: WebviewWindow, state: State<Backend>) -> Result<Bootstrap, String> {
    main_only(&window)?;
    let c = lock(&state.config)?;
    Ok(Bootstrap {
        prefs: c.prefs.clone(),
        can_save: c.can_save,
        notice: c.notice.clone(),
        fixture: c.fixture,
        rdp_probe: rdp_probe_enabled(),
        rdp_live: rdp_live_enabled(),
    })
}
#[tauri::command]
async fn rdp_probe_surface(
    window: WebviewWindow,
    action: String,
    bounds: Option<[i32; 4]>,
    id: Option<u64>,
) -> Result<serde_json::Value, String> {
    main_only(&window)?;
    if !rdp_probe_enabled() {
        return Err("RDP composition probe is not enabled".into());
    }
    #[cfg(feature = "rdp-probe")]
    {
        if action.len() > 16 {
            return Err("Invalid probe action".into());
        }
        let action: rdp_probe::Action = serde_json::from_value(serde_json::Value::String(action))
            .map_err(|_| "Invalid probe action")?;
        let parent = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
        let bounds = bounds.map(|[x, y, width, height]| rdp_probe::Bounds {
            x,
            y,
            width,
            height,
        });
        let (sender, receiver) = tokio::sync::oneshot::channel();
        window
            .run_on_main_thread(move || {
                let result = rdp_probe::perform(parent, action, bounds, id)
                    .and_then(|value| serde_json::to_value(value).map_err(|e| e.to_string()));
                let _ = sender.send(result);
            })
            .map_err(|e| e.to_string())?;
        receiver
            .await
            .map_err(|_| "Probe UI dispatcher closed".to_owned())?
    }
    #[cfg(not(feature = "rdp-probe"))]
    {
        let _ = (action, bounds, id);
        Err("RDP composition probe is not part of this build".into())
    }
}

/// Opens a native RDP control for a saved bookmark. Only the bookmark ID crosses
/// IPC; host, port, user and channel settings come from the stored profile.
#[tauri::command]
async fn rdp_open(
    window: WebviewWindow,
    state: State<'_, Backend>,
    profile_id: u64,
) -> Result<u64, String> {
    main_only(&window)?;
    let profile = lock(&state.config)?
        .prefs
        .profiles
        .iter()
        .find(|p| p.id == profile_id)
        .cloned()
        .ok_or("Session no longer exists")?;
    if profile.protocol != yuuterm_core::Protocol::Rdp {
        return Err("Not an RDP bookmark".into());
    }
    let forward = ssh_gateways::prepare(&window, &state, &profile).await?;
    let parent = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            let _ = sender.send(rdp::open_via(parent, profile, forward));
        })
        .map_err(|e| e.to_string())?;
    receiver
        .await
        .map_err(|_| "RDP UI dispatcher closed".to_owned())?
}

/// Stores a bookmark's new saved-password identity (local metadata only).
fn persist_credential_id(state: &Backend, profile_id: u64, identity: &str) -> Result<(), String> {
    persist_credential_id_checked(state, profile_id, identity, None)
}

fn persist_credential_id_checked(
    state: &Backend,
    profile_id: u64,
    identity: &str,
    expected: Option<&Profile>,
) -> Result<(), String> {
    let mut c = lock(&state.config)?;
    let mut candidate = c.prefs.clone();
    let profile = candidate
        .profiles
        .iter_mut()
        .find(|p| p.id == profile_id)
        .ok_or("Session no longer exists")?;
    if let Some(expected) = expected
        && (!credentials::same_destination(profile, expected)
            || profile.credential_id != expected.credential_id
            || profile.temporary != expected.temporary)
    {
        return Err("Session changed during sign-in; reconnect to sign in".into());
    }
    profile.credential_id = identity.to_owned();
    candidate.validate()?;
    if c.can_save {
        config::save(&candidate)?;
    }
    c.prefs = candidate;
    Ok(())
}

/// Whether an RDP bookmark's host accepts TCP on the SSH port (OpenSSH for the
/// remote-tools panel). Only the bookmark ID crosses IPC; runs on a worker.
#[tauri::command]
async fn rdp_ssh_available(
    window: WebviewWindow,
    state: State<'_, Backend>,
    profile_id: u64,
) -> Result<bool, String> {
    main_only(&window)?;
    let host = lock(&state.config)?
        .prefs
        .profiles
        .iter()
        .find(|p| p.id == profile_id && p.protocol == yuuterm_core::Protocol::Rdp)
        .map(|p| p.host.clone())
        .ok_or("RDP session no longer exists")?;
    tauri::async_runtime::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        let addresses = (host.as_str(), 22)
            .to_socket_addrs()
            .map_err(|e| e.to_string())?;
        Ok(addresses.take(4).any(|address| {
            std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_millis(1500))
                .is_ok()
        }))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// One Connect (certificate read plus control Connect) per RDP session at a
/// time, so repeated requests cannot pile up blocking reads.
struct ConnectAdmission(u64);
static CONNECTING: Mutex<Vec<u64>> = Mutex::new(Vec::new());
impl ConnectAdmission {
    fn enter(id: u64) -> Result<Self, String> {
        let mut busy = CONNECTING.lock().map_err(|_| "RDP state unavailable")?;
        if busy.contains(&id) {
            return Err("This RDP session is already connecting".into());
        }
        busy.push(id);
        Ok(Self(id))
    }
}
impl Drop for ConnectAdmission {
    fn drop(&mut self) {
        if let Ok(mut busy) = CONNECTING.lock() {
            busy.retain(|id| *id != self.0);
        }
    }
}

fn rdp_endpoint(state: &Backend, profile_id: u64) -> Result<(String, u16), String> {
    lock(&state.config)?
        .prefs
        .profiles
        .iter()
        .find(|p| p.id == profile_id && p.protocol == yuuterm_core::Protocol::Rdp)
        .map(|p| (p.host.clone(), p.port))
        .ok_or_else(|| "RDP session no longer exists".to_owned())
}

/// Whether a certificate is remembered for an RDP bookmark ("Don't ask me
/// again" in Windows, or an older YuuTerm pin). No network access.
#[tauri::command]
async fn rdp_certificate_remembered(
    window: WebviewWindow,
    state: State<'_, Backend>,
    profile_id: u64,
) -> Result<bool, String> {
    main_only(&window)?;
    let (host, port) = rdp_endpoint(&state, profile_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        Ok(rdp_cert::any_remembered(
            &rdp_cert::store_path()?,
            &host,
            port,
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Forgets the remembered certificate of an RDP bookmark's host and port, in
/// YuuTerm and in Windows, so the next connection asks again.
#[tauri::command]
async fn rdp_forget_certificate(
    window: WebviewWindow,
    state: State<'_, Backend>,
    profile_id: u64,
) -> Result<(), String> {
    main_only(&window)?;
    let (host, port) = rdp_endpoint(&state, profile_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        rdp_cert::forget_everywhere(&rdp_cert::store_path()?, &host, port)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn forget_saved_password(
    window: WebviewWindow,
    state: State<'_, Backend>,
    profile_id: u64,
) -> Result<Preferences, String> {
    main_only(&window)?;
    let profile = lock(&state.config)?
        .prefs
        .profiles
        .iter()
        .find(|p| p.id == profile_id)
        .cloned()
        .ok_or("Session no longer exists")?;
    let previous = profile.clone();
    tauri::async_runtime::spawn_blocking(move || rdp::forget_password(&profile))
        .await
        .map_err(|e| e.to_string())??;
    let mut c = lock(&state.config)?;
    let mut candidate = c.prefs.clone();
    if let Some(p) = candidate.profiles.iter_mut().find(|p| p.id == profile_id)
        && credentials::same_destination(p, &previous)
        && p.credential_id == previous.credential_id
    {
        p.credential_id.clear();
    }
    if c.can_save {
        config::save(&candidate)?;
    }
    c.prefs = candidate.clone();
    Ok(candidate)
}

#[tauri::command]
async fn rdp_session(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
    action: String,
    bounds: Option<[i32; 4]>,
    scale: Option<u32>,
) -> Result<serde_json::Value, String> {
    main_only(&window)?;
    if action.len() > 16 {
        return Err("Invalid RDP action".into());
    }
    let action: rdp::Action = serde_json::from_value(serde_json::Value::String(action))
        .map_err(|_| "Invalid RDP action")?;
    let bounds = bounds.map(|[x, y, width, height]| rdp::Bounds {
        x,
        y,
        width,
        height,
    });
    let parent = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
    // Before connecting, read the server certificate on a worker (no
    // credentials) and compare it with a remembered one (rdp_cert).
    let mut certificate = None;
    let _connecting = if matches!(action, rdp::Action::Connect) {
        Some(ConnectAdmission::enter(id)?)
    } else {
        None
    };
    if matches!(action, rdp::Action::Connect) {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        window
            .run_on_main_thread(move || {
                let _ = sender.send(rdp::endpoint(id).zip(rdp::transport_endpoint(id)));
            })
            .map_err(|e| e.to_string())?;
        if let Ok(Some(((host, port), (connect_host, connect_port)))) = receiver.await {
            let gateway = (host.as_str(), port) != (connect_host.as_str(), connect_port);
            let original_host = host.clone();
            let probe = tauri::async_runtime::spawn_blocking(move || {
                let fingerprint = rdp_cert::fingerprint(
                    &connect_host,
                    connect_port,
                    std::time::Duration::from_secs(4),
                )?;
                let remembered = rdp_cert::store_path()
                    .is_ok_and(|path| rdp_cert::trusted(&path, &host, port, &fingerprint));
                Ok::<_, String>(rdp::Certificate {
                    fingerprint,
                    enforce: remembered,
                    remembered,
                })
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|result| result);
            let probe_error = probe.as_ref().err().cloned();
            certificate = probe.ok();
            if gateway {
                let review = certificate.as_mut().ok_or_else(|| {
                    format!(
                        "Cannot verify the RDP certificate through the SSH gateway: {}",
                        probe_error.unwrap_or_default()
                    )
                })?;
                if !review.remembered {
                    let once = "Connect once";
                    let remember = "Remember and connect";
                    let choice = rfd::AsyncMessageDialog::new()
                        .set_title("Verify RDP server certificate")
                        .set_description(format!("{original_host}:{port}\nSHA-256: {}\nCompare this fingerprint with a trusted source before connecting.", review.fingerprint))
                        .set_buttons(rfd::MessageButtons::YesNoCancelCustom(once.into(), remember.into(), "Cancel".into()))
                        .show().await;
                    match choice {
                        rfd::MessageDialogResult::Custom(label) if label == once => {}
                        rfd::MessageDialogResult::Custom(label) if label == remember => {
                            let fingerprint = review.fingerprint.clone();
                            tauri::async_runtime::spawn_blocking(move || {
                                rdp_cert::remember(
                                    &rdp_cert::store_path()?,
                                    &original_host,
                                    port,
                                    &fingerprint,
                                )
                            })
                            .await
                            .map_err(|e| e.to_string())??;
                            review.remembered = true;
                        }
                        _ => {
                            return Err("RDP certificate was not approved; nothing was sent".into());
                        }
                    }
                }
                // Every gateway RDP connection is bound to the original target
                // certificate on the authenticating socket, including trust-once.
                review.enforce = true;
            }
        }
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            let _ = sender.send(rdp::perform(parent, id, action, bounds, scale, certificate));
        })
        .map_err(|e| e.to_string())?;
    let status = receiver
        .await
        .map_err(|_| "RDP UI dispatcher closed".to_owned())??;
    if let Some(status) = &status
        && let Some(identity) = &status.new_credential_id
    {
        // The password is already stored; keep the connection even if this fails.
        if let Err(e) = persist_credential_id(&state, status.profile_id, identity) {
            eprintln!("rdp: saved-password identity not persisted: {e}");
        }
    }
    serde_json::to_value(status).map_err(|e| e.to_string())
}

/// Raw JPEG bytes (an ArrayBuffer in the WebView) of one RDP session.
#[tauri::command]
async fn rdp_snapshot(window: WebviewWindow, id: u64) -> Result<tauri::ipc::Response, String> {
    main_only(&window)?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            let _ = sender.send(rdp::snapshot(id));
        })
        .map_err(|e| e.to_string())?;
    let jpeg = receiver
        .await
        .map_err(|_| "RDP UI dispatcher closed".to_owned())??;
    Ok(tauri::ipc::Response::new(jpeg))
}

#[tauri::command]
async fn rdp_probe_report(window: WebviewWindow, report: serde_json::Value) -> Result<(), String> {
    main_only(&window)?;
    if !rdp_probe_enabled() {
        return Err("RDP composition probe is not enabled".into());
    }
    let data = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    if data.len() > 16384 {
        return Err("Probe report too large".into());
    }
    let path =
        std::env::var("YUUTERM_RDP_PROBE_REPORT").map_err(|_| "Probe report path missing")?;
    std::fs::write(path, data).map_err(|e| e.to_string())?;
    #[cfg(feature = "rdp-probe")]
    if let Ok(path) = std::env::var("YUUTERM_RDP_PROBE_CAPTURE") {
        let parent = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        window
            .run_on_main_thread(move || {
                let _ = sender.send(rdp_probe::capture(parent, &path));
            })
            .map_err(|e| e.to_string())?;
        receiver
            .await
            .map_err(|_| "Probe capture dispatcher closed")??;
    }
    window
        .app_handle()
        .exit(if report["passed"] == true { 0 } else { 2 });
    Ok(())
}
#[tauri::command]
fn session_nano(
    window: WebviewWindow,
    state: State<Backend>,
    id: u64,
    path: String,
) -> Result<(), String> {
    main_only(&window)?;
    let bytes = yuuterm_ssh::files::nano_command(&path).map_err(|e| e.to_string())?;
    #[cfg(feature = "capture")]
    let fixture = lock(&state.config)?.fixture;
    let mut sessions = lock(&state.sessions)?;
    let session = sessions
        .get_mut(&id)
        .ok_or("Original connection no longer exists")?;
    let meta = session.live.snapshot();
    if meta.protocol != yuuterm_core::Protocol::Ssh || !meta.connected || meta.finished {
        return Err("An active SSH session is required".into());
    }
    session.live.send(Command::Input(bytes.clone()))?;
    #[cfg(feature = "capture")]
    if fixture && session.input_trace.len() < 100 {
        session.input_trace.push(bytes);
    }
    Ok(())
}
#[tauri::command]
async fn session_statistics(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
) -> Result<yuuterm_ssh::statistics::Sample, String> {
    main_only(&window)?;
    let (reply, received) = tokio::sync::oneshot::channel();
    {
        let sessions = lock(&state.sessions)?;
        let session = sessions.get(&id).ok_or("Connection no longer exists")?;
        let snapshot = session.live.snapshot();
        if snapshot.protocol != yuuterm_core::Protocol::Ssh
            || !snapshot.connected
            || snapshot.finished
        {
            return Err("Statistics require an active SSH session".into());
        }
        session.live.send(Command::Statistics(reply))?;
    }
    tokio::time::timeout(std::time::Duration::from_secs(8), received)
        .await
        .map_err(|_| "Statistics timed out".to_string())?
        .map_err(|_| "Statistics connection closed".to_string())?
}
#[tauri::command]
async fn session_processes(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
) -> Result<Vec<yuuterm_ssh::statistics::Process>, String> {
    main_only(&window)?;
    let (reply, received) = tokio::sync::oneshot::channel();
    {
        let sessions = lock(&state.sessions)?;
        let session = sessions.get(&id).ok_or("Connection no longer exists")?;
        let snapshot = session.live.snapshot();
        if snapshot.protocol != yuuterm_core::Protocol::Ssh
            || !snapshot.connected
            || snapshot.finished
        {
            return Err("Statistics require an active SSH session".into());
        }
        session.live.send(Command::Processes(reply))?;
    }
    tokio::time::timeout(std::time::Duration::from_secs(8), received)
        .await
        .map_err(|_| "Statistics timed out".to_string())?
        .map_err(|_| "Statistics connection closed".to_string())?
}
/// Sends one tools request to an active SSH session.
fn ssh_tools_send(state: &Backend, id: u64, command: Command) -> Result<(), String> {
    let sessions = lock(&state.sessions)?;
    let session = sessions.get(&id).ok_or("Connection no longer exists")?;
    let snapshot = session.live.snapshot();
    if snapshot.protocol != yuuterm_core::Protocol::Ssh || !snapshot.connected || snapshot.finished
    {
        return Err("Remote tools require an active SSH session".into());
    }
    session.live.send(command).map_err(String::from)
}
#[tauri::command]
async fn ssh_tools_probe(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
) -> Result<yuuterm_ssh::tools::Probe, String> {
    main_only(&window)?;
    let (reply, received) = tokio::sync::oneshot::channel();
    ssh_tools_send(&state, id, Command::ToolsProbe(reply))?;
    tokio::time::timeout(std::time::Duration::from_secs(10), received)
        .await
        .map_err(|_| "Tools probe timed out".to_string())?
        .map_err(|_| "Tools connection closed".to_string())?
}
#[tauri::command]
async fn ssh_folder_sizes(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
    path: String,
) -> Result<yuuterm_ssh::tools::FolderSizes, String> {
    main_only(&window)?;
    let (reply, received) = tokio::sync::oneshot::channel();
    ssh_tools_send(&state, id, Command::FolderSizes { path, reply })?;
    tokio::time::timeout(std::time::Duration::from_secs(130), received)
        .await
        .map_err(|_| "Folder sizes timed out".to_string())?
        .map_err(|_| "Tools connection closed".to_string())?
}
#[tauri::command]
fn ssh_folder_sizes_cancel(
    window: WebviewWindow,
    state: State<Backend>,
    id: u64,
) -> Result<(), String> {
    main_only(&window)?;
    ssh_tools_send(&state, id, Command::CancelFolderSizes)
}
/// Deletes a session bookmark and its saved password. The UI closes its tab
/// first; a still-running connection is refused.
#[tauri::command]
fn delete_profile(
    window: WebviewWindow,
    state: State<Backend>,
    profile_id: u64,
    detach: Option<bool>,
) -> Result<Preferences, String> {
    main_only(&window)?;
    if lock(&state.sessions)?
        .values()
        .any(|s| s.profile_id == profile_id && !s.live.snapshot().finished)
    {
        return Err("Disconnect the session before deleting it".into());
    }
    let mut c = lock(&state.config)?;
    let capture_only = cfg!(feature = "capture") && c.fixture;
    let profile = c
        .prefs
        .profiles
        .iter()
        .find(|p| p.id == profile_id)
        .cloned()
        .ok_or("Session no longer exists")?;
    if !c.can_save && !capture_only && !profile.temporary {
        return Err("Settings saving is disabled".into());
    }
    let candidate = yuuterm_core::session_actions::remove_detaching(
        &c.prefs,
        &[profile_id],
        detach.unwrap_or(false),
    )?;
    if !capture_only && c.can_save {
        config::save(&candidate)?;
    }
    c.prefs = candidate.clone();
    drop(c);
    // The bookmark is gone; its saved passwords must not outlive it.
    ssh_gateways::forget_passwords(&profile);
    Ok(candidate)
}
#[tauri::command]
fn duplicate_profile(
    window: WebviewWindow,
    state: State<Backend>,
    profile_id: u64,
) -> Result<Preferences, String> {
    main_only(&window)?;
    let mut c = lock(&state.config)?;
    // Capture fixtures are isolated in-memory profiles, never the user's store.
    let capture_only = cfg!(feature = "capture") && c.fixture;
    if !c.can_save && !capture_only {
        return Err("Settings saving is disabled".into());
    }
    let candidate = c.prefs.with_duplicate(profile_id)?;
    if !capture_only {
        config::save(&candidate)?;
    }
    c.prefs = candidate.clone();
    Ok(candidate)
}
/// The validated settings after saving `profile`, and the credential views of
/// saved passwords that the new settings no longer use (forget them only after
/// the save succeeded). Saved-password identities stay backend-owned.
fn plan_profile_save(
    prefs: &Preferences,
    mut profile: Profile,
) -> Result<(Preferences, Vec<Profile>), String> {
    profile.validate()?;
    let mut candidate = prefs.clone();
    let mut forget = Vec::new();
    if profile.id == 0 {
        profile.credential_id.clear();
        ssh_gateways::carry_gateway_credentials(&Profile::default(), &mut profile);
        profile.id = candidate
            .profiles
            .iter()
            .map(|p| p.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or("Session ID limit reached")?;
        candidate.profiles.push(profile);
    } else {
        let old = candidate
            .profiles
            .iter_mut()
            .find(|p| p.id == profile.id)
            .ok_or("Session no longer exists")?;
        // The saved-password identity is backend-owned local metadata.
        profile.credential_id = old.credential_id.clone();
        if !credentials::same_destination(old, &profile) {
            // A changed destination or account never reuses the old password.
            forget.push(old.clone());
            profile.credential_id.clear();
        }
        forget.extend(ssh_gateways::carry_gateway_credentials(old, &mut profile));
        *old = profile;
    }
    candidate.validate()?;
    Ok((candidate, forget))
}
#[tauri::command]
fn save_profile(
    window: WebviewWindow,
    state: State<Backend>,
    profile: Profile,
) -> Result<Preferences, String> {
    main_only(&window)?;
    let mut c = lock(&state.config)?;
    let capture_only = cfg!(feature = "capture") && c.fixture;
    // Quick-connect sessions live in memory only, so they work without saving.
    if !c.can_save && !capture_only && !profile.temporary {
        return Err("Settings saving is disabled; the original file is preserved".into());
    }
    let (candidate, forget) = plan_profile_save(&c.prefs, profile)?;
    if !capture_only && c.can_save {
        config::save(&candidate)?;
    }
    c.prefs = candidate.clone();
    drop(c);
    // Only after the new settings are in place: a refused save keeps every
    // saved password (target and gateway hops).
    for old in forget {
        let _ = rdp::forget_password(&old);
    }
    Ok(candidate)
}
#[tauri::command]
fn change_group(
    window: WebviewWindow,
    state: State<Backend>,
    group: String,
    new_name: Option<String>,
) -> Result<Preferences, String> {
    main_only(&window)?;
    let mut c = lock(&state.config)?;
    let capture_only = cfg!(feature = "capture") && c.fixture;
    if !c.can_save && !capture_only {
        return Err("Settings saving is disabled".into());
    }
    let candidate = c.prefs.with_group_change(&group, new_name.as_deref())?;
    if !capture_only {
        config::save(&candidate)?;
    }
    c.prefs = candidate.clone();
    Ok(candidate)
}
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Preserve named IPC fields used by existing clients.
fn save_appearance(
    window: WebviewWindow,
    state: State<Backend>,
    mocha: bool,
    theme: Option<String>,
    font_size: f32,
    font_family: String,
    scrollback_lines: Option<u32>,
    highlighting: config::Highlighting,
    statistics: config::StatisticsSettings,
) -> Result<Preferences, String> {
    main_only(&window)?;
    let mut c = lock(&state.config)?;
    let capture_only = cfg!(feature = "capture") && c.fixture;
    if !c.can_save && !capture_only {
        return Err("Settings saving is disabled".into());
    }
    let mut candidate = c.prefs.clone();
    candidate.mocha = mocha;
    if let Some(theme) = theme {
        candidate.theme = theme;
    }
    candidate.font_size = font_size;
    candidate.font_family = font_family;
    if let Some(lines) = scrollback_lines {
        candidate.scrollback_lines = lines;
    }
    candidate.highlighting = highlighting;
    candidate.statistics = statistics;
    candidate.validate()?;
    if !capture_only {
        config::save(&candidate)?;
    }
    c.prefs = candidate.clone();
    Ok(candidate)
}
#[tauri::command]
fn session_start(
    window: WebviewWindow,
    state: State<Backend>,
    profile_id: u64,
) -> Result<serde_json::Value, String> {
    main_only(&window)?;
    let profile = lock(&state.config)?
        .prefs
        .profiles
        .iter()
        .find(|p| p.id == profile_id)
        .cloned()
        .ok_or("Session no longer exists")?;
    start_session(&window, &state, profile_id, profile, None)
}

/// Internal session key of an RDP bookmark's remote-tools SSH session. Below
/// 2^53 so the WebView can hold it as a number; never a bookmark ID.
const RDP_TOOLS_KEY: u64 = 1 << 50;

/// Remote tools beside an RDP session: an SSH session to the same host on port
/// 22 with password sign-in, using the RDP bookmark's saved password and account
/// (or the native prompt). Only the RDP bookmark ID crosses IPC.
#[tauri::command]
fn rdp_tools_start(
    window: WebviewWindow,
    state: State<Backend>,
    profile_id: u64,
) -> Result<serde_json::Value, String> {
    main_only(&window)?;
    let rdp = lock(&state.config)?
        .prefs
        .profiles
        .iter()
        .find(|p| p.id == profile_id && p.protocol == yuuterm_core::Protocol::Rdp)
        .cloned()
        .ok_or("RDP session no longer exists")?;
    let key = RDP_TOOLS_KEY | profile_id;
    let ssh = Profile {
        id: key,
        name: format!("{} tools", rdp.name),
        protocol: yuuterm_core::Protocol::Ssh,
        host: rdp.host.clone(),
        port: 22,
        username: rdp.username.clone(),
        ssh: yuuterm_core::SshSettings {
            auth: yuuterm_core::SshAuth::Password,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut started = start_session(&window, &state, key, ssh, Some(rdp))?;
    started["profileId"] = key.into();
    Ok(started)
}

/// Starts an SSH/TELNET/SERIAL session under `key`. Password sign-in uses the
/// saved password of `owner` (the bookmark the password belongs to; the session
/// profile itself when None) or the native prompt.
fn start_session(
    window: &WebviewWindow,
    state: &Backend,
    profile_id: u64,
    mut profile: Profile,
    owner: Option<Profile>,
) -> Result<serde_json::Value, String> {
    if lock(&state.sessions)?
        .values()
        .any(|s| s.profile_id == profile_id && !s.live.snapshot().finished)
    {
        return Err("This bookmark already has a connection".into());
    }
    // Password sign-in: saved password or the native Windows prompt. The prompt
    // is modal and runs other IPC inside it, so no application lock is held.
    // SSH gateways sign in first, in connection order, before any network I/O.
    let (hops, saved_gateway_passwords) = if matches!(
        profile.protocol,
        yuuterm_core::Protocol::Ssh | yuuterm_core::Protocol::Telnet
    ) {
        ssh_gateways::sign_in(window.hwnd().map_err(|e| e.to_string())?, state, &profile)?
    } else {
        Default::default()
    };
    let mut password = None;
    let mut saved_password = None;
    let owner = owner.unwrap_or_else(|| profile.clone());
    if profile.protocol == yuuterm_core::Protocol::Ssh
        && profile.ssh.auth == yuuterm_core::SshAuth::Password
    {
        let parent = window.hwnd().map_err(|e| e.to_string())?;
        let caption = format!("{} · SSH", profile.name);
        let default_port = owner.protocol.default_port();
        let resolved = credentials::resolve(parent, &owner, &caption, default_port)?
            .ok_or("Sign-in cancelled; nothing was sent")?;
        if let Some(identity) = &resolved.new_identity
            && let Err(e) = persist_credential_id(state, owner.id, identity)
        {
            eprintln!("ssh: saved-password identity not persisted: {e}");
        }
        // Sign in as the account entered in (or saved from) the prompt, like
        // RDP; the bookmark's user name is only the prefill.
        profile.username = resolved.username.clone();
        password = Some(zeroize::Zeroizing::new(
            resolved.secret.expose_for_authentication().to_owned(),
        ));
        saved_password = resolved.saved.map(|target| ssh_gateways::SavedPassword {
            target,
            owner: owner.id,
        });
    }
    let mut sessions = lock(&state.sessions)?;
    if sessions
        .values()
        .any(|s| s.profile_id == profile_id && !s.live.snapshot().finished)
    {
        return Err("This bookmark already has a connection".into());
    }
    sessions.retain(|_, s| s.profile_id != profile_id);
    let file_origin = file_ui::Origin::from_profile(&profile);
    let logging = profile.logging.clone();
    let telnet_line_editing =
        profile.protocol == yuuterm_core::Protocol::Telnet && profile.telnet.local_line_editing;
    let log_name = profile.name.clone();
    let documents_override = if cfg!(feature = "capture") && lock(&state.config)?.fixture {
        std::env::var_os("YUUTERM_CAPTURE_PROFILE")
            .map(std::path::PathBuf::from)
            .and_then(|p| p.parent().map(|parent| parent.join("Documents")))
    } else {
        None
    };
    let wake = Arc::new(tokio::sync::Notify::new());
    let signal = wake.clone();
    let (live, output) = state
        .engine
        .connect_streaming_via(
            profile,
            hops,
            Arc::new(move || signal.notify_one()),
            password,
        )
        .map_err(|e| e.to_string())?;
    let id = live.id;
    let protocol = live.snapshot().protocol;
    #[cfg(feature = "capture")]
    if std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref() == Ok("performance")
        && lock(&state.config)?.fixture
    {
        startup_timing::observe(
            state.startup_origin,
            profile_id,
            live.subscribe_snapshot(),
            state.startup_timings.clone(),
        );
    }
    sessions.insert(
        id,
        Session {
            profile_id,
            file_origin,
            live,
            output,
            wake,
            first: true,
            last_files: None,
            file_busy: Arc::new(Mutex::new(None)),
            output_log: logging.enabled.then(|| {
                output_log::OutputLog::automatic(log_name, logging, documents_override, id)
            }),
            saved_password,
            saved_gateway_passwords,
            #[cfg(feature = "capture")]
            input_trace: Vec::new(),
        },
    );
    Ok(
        serde_json::json!({"id": id, "protocol": protocol, "telnetLineEditing": telnet_line_editing}),
    )
}
#[tauri::command]
async fn session_poll(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
    wait_ms: Option<u64>,
) -> Result<Poll, String> {
    main_only(&window)?;
    let wait = wait_ms.unwrap_or(0);
    if wait > 250 {
        return Err("Terminal poll wait must be 0–250 ms".into());
    }
    let wake = lock(&state.sessions)?
        .get(&id)
        .ok_or("Connection no longer exists")?
        .wake
        .clone();
    // Register before inspecting state/output. notify_one retains a permit if
    // data arrives before this future is polled; no empty-check/wait race.
    let notified = wake.notified();
    let first = drain_poll(&state, id)?;
    if wait == 0 || first.meta.is_some() || !first.chunks.is_empty() {
        return Ok(first);
    }
    // Never hold a session/config mutex while waiting. The deadline also bounds
    // shutdown and status latency if an auxiliary worker misses a notification.
    let _ = tokio::time::timeout(std::time::Duration::from_millis(wait), notified).await;
    drain_poll(&state, id)
}
fn drain_poll(state: &Backend, id: u64) -> Result<Poll, String> {
    let mut sessions = lock(&state.sessions)?;
    let s = sessions.get_mut(&id).ok_or("Connection no longer exists")?;
    let meta = if s.first || s.live.has_changes() {
        s.first = false;
        let snapshot = s.live.changed_snapshot();
        let files_changed = s
            .last_files
            .as_ref()
            .is_none_or(|f| !Arc::ptr_eq(f, &snapshot.files));
        let files = files_changed.then(|| snapshot.files.as_ref().clone());
        s.last_files = Some(snapshot.files.clone());
        Some(meta(snapshot, files))
    } else {
        None
    };
    let mut chunks = Vec::new();
    for _ in 0..2 {
        match s.output.try_recv() {
            Ok(bytes) => {
                if let Some(log) = &mut s.output_log {
                    log.record(&bytes);
                }
                chunks.push(bytes);
            }
            Err(_) => break,
        }
    }
    if s.live.snapshot().finished
        && s.output.is_empty()
        && let Some(log) = &mut s.output_log
    {
        log.stop();
    }
    let snapshot = s.live.snapshot();
    if snapshot.password_rejected
        && let Some(saved) = ssh_gateways::take_refused(
            snapshot.rejected_gateway,
            &mut s.saved_password,
            &mut s.saved_gateway_passwords,
        )
    {
        // Never retry a refused password: forget it; the next prompt says why.
        let _ = credentials::forget(&saved.target);
        credentials::mark_refused(saved.owner);
    }
    Ok(Poll {
        meta,
        chunks,
        more: !s.output.is_empty(),
        logging: s.output_log.as_ref().map(|l| l.status()),
    })
}
fn meta(s: Snapshot, files: Option<Vec<FileEntry>>) -> SessionMeta {
    SessionMeta {
        protocol: s.protocol,
        endpoint: s.endpoint,
        status: s.status,
        connected: s.connected,
        finished: s.finished,
        password_rejected: s.password_rejected,
        trust: s.trust,
        path: s.path,
        files_status: s.files_status,
        files,
    }
}
#[tauri::command]
fn session_input(
    window: WebviewWindow,
    state: State<Backend>,
    id: u64,
    bytes: Vec<u8>,
) -> Result<(), String> {
    main_only(&window)?;
    let mut sessions = lock(&state.sessions)?;
    let s = sessions.get_mut(&id).ok_or("Connection no longer exists")?;
    if !s.live.snapshot().connected {
        return Err("Connection is not active; input discarded".into());
    }
    s.live
        .send(Command::Input(bytes.clone()))
        .map_err(String::from)?;
    #[cfg(feature = "capture")]
    if lock(&state.config)?.fixture && s.input_trace.len() < 100 {
        s.input_trace.push(bytes);
    }
    Ok(())
}
#[tauri::command]
fn capture_trace(
    window: WebviewWindow,
    state: State<Backend>,
    id: u64,
) -> Result<Vec<Vec<u8>>, String> {
    main_only(&window)?;
    #[cfg(feature = "capture")]
    {
        if !lock(&state.config)?.fixture {
            return Err("Fixture only".into());
        }
        Ok(lock(&state.sessions)?
            .get(&id)
            .ok_or("Missing fixture connection")?
            .input_trace
            .clone())
    }
    #[cfg(not(feature = "capture"))]
    {
        let _ = (state, id);
        Err("Fixture only".into())
    }
}
#[tauri::command]
fn capture_scope(window: WebviewWindow, state: State<Backend>) -> Result<String, String> {
    main_only(&window)?;
    if !cfg!(feature = "capture") || !lock(&state.config)?.fixture {
        return Err("Fixture only".into());
    }
    let scope = std::env::var("YUUTERM_CAPTURE_SCOPE").unwrap_or_else(|_| "all".into());
    if !matches!(
        scope.as_str(),
        "all"
            | "session"
            | "files"
            | "file-navigation"
            | "editor"
            | "recovery"
            | "restart-prepare"
            | "restart-restore"
            | "filemenus"
            | "history"
            | "search-flood"
            | "highlight-wrap"
            | "highlight-import"
            | "log-rotation"
            | "serial-import"
            | "bookmark-groups"
            | "session-actions"
            | "telnet-line"
            | "telnet-refused"
            | "telnet-external"
            | "logging"
            | "transfer"
            | "performance"
            | "pollwait"
            | "icons"
            | "about"
            | "qol"
            | "vnc"
            | "rdp"
            | "gateway-rdp"
            | "rdp-hold"
            | "rdp-refused"
            | "rdp-power"
            | "rdp-network"
            | "quick-connect"
            | "local-terminal"
            | "local-readme"
            | "local-hold"
            | "local-flood"
            | "ssh-password"
            | "ssh-two"
            | "windows-openssh"
            | "linux-openssh"
            | "linux-ssh-tools"
            | "vnc-real"
            | "vnc-saved"
            | "zsh-input"
            | "clipboard"
            | "layout-prepare"
            | "layout-restore"
    ) {
        return Err("Unknown fixture scope".into());
    }
    Ok(scope)
}
#[tauri::command]
fn capture_report(
    window: WebviewWindow,
    state: State<Backend>,
    checks: Vec<String>,
    passed: bool,
) -> Result<(), String> {
    main_only(&window)?;
    #[cfg(feature = "capture")]
    {
        if !lock(&state.config)?.fixture
            || checks.len() > 128
            || checks.iter().any(|c| c.len() > 400)
        {
            return Err("Fixture only".into());
        }
        let path =
            std::env::var("YUUTERM_CAPTURE_REPORT").map_err(|_| "Capture report output missing")?;
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&serde_json::json!({"passed":passed,"checks":checks}))
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let rdp = matches!(
            std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref(),
            Ok("rdp"
                | "gateway-rdp"
                | "rdp-refused"
                | "rdp-power"
                | "rdp-network"
                | "quick-connect"
                | "local-terminal"
                | "local-readme"
                | "ssh-password"
                | "ssh-two")
        );
        if !passed || rdp {
            // The RDP fixture has no WebView screenshot step; finish here after
            // its sessions disconnected.
            // Leave like a user does: close the window (RDP disconnect-first, then
            // control teardown while the window still exists). Every capture run
            // that called exit() directly with a live control hung in a kernel
            // wait at process exit, with xrdp and with Windows 11.
            CAPTURE_EXIT.store(
                if passed { 0 } else { 2 },
                std::sync::atomic::Ordering::Relaxed,
            );
            let _ = window.close();
        }
        Ok(())
    }
    #[cfg(not(feature = "capture"))]
    {
        let _ = (state, checks, passed);
        Err("Fixture only".into())
    }
}

#[tauri::command]
fn capture_metrics(
    window: WebviewWindow,
    state: State<Backend>,
    report: serde_json::Value,
) -> Result<(), String> {
    main_only(&window)?;
    if !cfg!(feature = "capture")
        || !lock(&state.config)?.fixture
        || std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref() != Ok("performance")
    {
        return Err("Performance fixture only".into());
    }
    if !report.is_object() || !matches!(report["phase"].as_str(), Some("idle" | "complete")) {
        return Err("Invalid performance report phase".into());
    }
    #[cfg(feature = "capture")]
    let report = {
        let mut report = report;
        report["backend_elapsed_ms"] =
            serde_json::json!(state.startup_origin.elapsed().as_secs_f64() * 1000.0);
        report["backend_startup"] = serde_json::to_value(
            lock(&state.startup_timings)?
                .values()
                .cloned()
                .collect::<Vec<_>>(),
        )
        .map_err(|_| "Cannot encode startup timings")?;
        report
    };
    let data = serde_json::to_vec_pretty(&report).map_err(|_| "Invalid performance report")?;
    if data.len() > 16384 {
        return Err("Performance report too large".into());
    }
    let path =
        std::env::var("YUUTERM_CAPTURE_REPORT").map_err(|_| "Performance report path missing")?;
    std::fs::write(path, data).map_err(|_| "Cannot write performance report")?;
    if report["phase"] == "complete" {
        window
            .app_handle()
            .exit(if report["passed"] == true { 0 } else { 2 });
    }
    Ok(())
}
#[tauri::command]
fn session_resize(
    window: WebviewWindow,
    state: State<Backend>,
    id: u64,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    main_only(&window)?;
    lock(&state.sessions)?
        .get(&id)
        .ok_or("Connection no longer exists")?
        .live
        .send(Command::Resize { rows, cols })
        .map_err(String::from)
}
#[tauri::command]
async fn session_browse(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
    path: String,
) -> Result<String, String> {
    main_only(&window)?;
    let files = {
        let sessions = lock(&state.sessions)?;
        let s = sessions.get(&id).ok_or("Connection no longer exists")?;
        if !s.live.snapshot().connected {
            return Err("Connection is not active".into());
        }
        s.live.files()
    };
    files.browse(path).await
}
#[tauri::command]
fn session_trust(
    window: WebviewWindow,
    state: State<Backend>,
    id: u64,
    accept: bool,
) -> Result<(), String> {
    main_only(&window)?;
    lock(&state.sessions)?
        .get(&id)
        .ok_or("Connection no longer exists")?
        .live
        .trust(accept);
    Ok(())
}
#[tauri::command]
fn session_disconnect(window: WebviewWindow, state: State<Backend>, id: u64) -> Result<(), String> {
    main_only(&window)?;
    lock(&state.sessions)?
        .get(&id)
        .ok_or("Connection no longer exists")?
        .live
        .disconnect();
    Ok(())
}
#[tauri::command]
fn session_close(window: WebviewWindow, state: State<Backend>, id: u64) -> Result<(), String> {
    main_only(&window)?;
    lock(&state.sessions)?.remove(&id);
    Ok(())
}
#[tauri::command]
fn capture_bookmark_protocol(
    window: WebviewWindow,
    state: State<Backend>,
    profile_id: u64,
    protocol: yuuterm_core::Protocol,
) -> Result<Preferences, String> {
    main_only(&window)?;
    #[cfg(feature = "capture")]
    {
        let mut config = lock(&state.config)?;
        if !config.fixture
            || !matches!(
                protocol,
                yuuterm_core::Protocol::Ssh | yuuterm_core::Protocol::Telnet
            )
        {
            return Err("Isolated fixture only".into());
        }
        let profile = config
            .prefs
            .profiles
            .iter_mut()
            .find(|p| p.id == profile_id)
            .ok_or("Fixture bookmark missing")?;
        profile.protocol = protocol;
        // In-memory fixture metadata only; never persist real user settings.
        Ok(config.prefs.clone())
    }
    #[cfg(not(feature = "capture"))]
    {
        let _ = (state, profile_id, protocol);
        Err("Fixture only".into())
    }
}
#[tauri::command]
fn capture_ready(
    window: WebviewWindow,
    state: State<Backend>,
    terminal_ready: bool,
) -> Result<(), String> {
    main_only(&window)?;
    #[cfg(feature = "capture")]
    {
        if !lock(&state.config)?.fixture || !terminal_ready {
            return Err("Capture requires the isolated fixture".into());
        }
        let telnet = lock(&state.config)?
            .prefs
            .profiles
            .first()
            .is_some_and(|p| p.protocol == yuuterm_core::Protocol::Telnet);
        let vnc_ready = state.vnc.capture_ready();
        if !vnc_ready
            && !lock(&state.sessions)?.values().any(|s| {
                let v = s.live.snapshot();
                (v.connected && (telnet || v.files.len() >= 2))
                    || (telnet
                        && v.finished
                        && std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref()
                            == Ok("telnet-refused"))
            })
        {
            return Err("Fixture is not ready".into());
        }
        capture::save(window)
    }
    #[cfg(not(feature = "capture"))]
    {
        let _ = (state, terminal_ready);
        Err("Capture is not part of this build".into())
    }
}

/// Capture builds only: a WebView still in the middle of the quick-connect
/// fixture, which keeps running afterwards.
#[tauri::command]
fn capture_still(window: WebviewWindow, state: State<Backend>, name: String) -> Result<(), String> {
    main_only(&window)?;
    #[cfg(feature = "capture")]
    {
        if !lock(&state.config)?.fixture
            || !matches!(
                std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref(),
                Ok("quick-connect"
                    | "gateway-rdp"
                    | "local-terminal"
                    | "local-readme"
                    | "qol"
                    | "files"
                    | "vnc-saved"
                    | "linux-ssh-tools")
            )
            || name.is_empty()
            || name.len() > 40
            || !name.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
        {
            return Err("Fixture only".into());
        }
        // Named stills go beside the run's capture path.
        let base = std::env::var("YUUTERM_CAPTURE").map_err(|_| "Capture output path missing")?;
        let path = std::path::Path::new(&base).with_file_name(format!("{name}.png"));
        capture::save_to(window, path.to_string_lossy().into_owned(), false)
    }
    #[cfg(not(feature = "capture"))]
    {
        let _ = (state, name);
        Err("Capture is not part of this build".into())
    }
}

/// Capture builds only: simulates a native file drop of one fixture file at a
/// physical position (the same record/emit path as WindowEvent::DragDrop).
#[tauri::command]
fn capture_simulate_drop(
    window: WebviewWindow,
    state: State<Backend>,
    x: f64,
    y: f64,
) -> Result<String, String> {
    main_only(&window)?;
    #[cfg(feature = "capture")]
    {
        if !lock(&state.config)?.fixture {
            return Err("Fixture only".into());
        }
        let dir = std::path::PathBuf::from(
            std::env::var_os("LOCALAPPDATA").ok_or("Fixture profile missing")?,
        );
        let name = format!("yt-drop-{}.txt", std::process::id());
        let file = dir.join(&name);
        std::fs::write(
            &file,
            b"dropped by the YuuTerm fixture
",
        )
        .map_err(|e| e.to_string())?;
        let token = file_ui::record_drop(vec![file]).ok_or("Drop not recorded")?;
        window
            .emit(
                "files-dropped",
                serde_json::json!({"token": token, "x": x, "y": y, "count": 1}),
            )
            .map_err(|e| e.to_string())?;
        Ok(name)
    }
    #[cfg(not(feature = "capture"))]
    {
        let _ = (state, x, y);
        Err("Fixture only".into())
    }
}

/// Exit code a capture run reports after its window closed.
#[cfg(feature = "capture")]
static CAPTURE_EXIT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Capture builds only: the RDP fixture bookmark, kept to remove its saved
/// password when the window closes.
#[cfg(feature = "capture")]
static RDP_FIXTURE: Mutex<Vec<Profile>> = Mutex::new(Vec::new());

/// Capture builds only: one RDP bookmark for the disposable test target that the
/// test script names (scripts/Test-RdpProduction.ps1). Its password becomes the
/// bookmark's saved password under a fresh identity, so connects exercise the
/// production saved-credential path without a prompt. Never in shipping builds.
#[cfg(feature = "capture")]
fn rdp_fixture() -> Result<Config, Box<dyn std::error::Error>> {
    let var = |name: &str| std::env::var(name).map_err(|_| format!("{name} missing"));
    if std::env::var_os("LOCALAPPDATA")
        .is_none_or(|v| !v.to_string_lossy().contains("rdp-production-"))
    {
        return Err("The RDP fixture requires the isolated test profile folder".into());
    }
    let mut profile = Profile {
        id: 1,
        name: "RDP fixture".into(),
        protocol: yuuterm_core::Protocol::Rdp,
        host: var("YUUTERM_RDP_FIXTURE_HOST")?,
        port: var("YUUTERM_RDP_FIXTURE_PORT")?.parse()?,
        username: var("YUUTERM_RDP_FIXTURE_USER")?,
        ..Default::default()
    };
    profile.gateways = gateway_capture::hops();
    // Optional audio mode for audio-redirection checks.
    if let Ok(audio) = std::env::var("YUUTERM_RDP_FIXTURE_AUDIO") {
        profile.rdp.audio = match audio.as_str() {
            "Local" => yuuterm_core::RdpAudio::Local,
            "Remote" => yuuterm_core::RdpAudio::Remote,
            _ => yuuterm_core::RdpAudio::Off,
        };
    }
    // "ssh-password": the same host and account as an SSH bookmark with
    // password sign-in (Windows OpenSSH on the test host).
    // "ssh-two": two such bookmarks, both connected at once.
    let two = std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref() == Ok("ssh-two");
    if two || std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref() == Ok("ssh-password") {
        profile.name = "SSH password fixture".into();
        profile.protocol = yuuterm_core::Protocol::Ssh;
        profile.port = 22;
        profile.ssh.auth = yuuterm_core::SshAuth::Password;
    }
    profile.validate()?;
    if std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref() == Ok("gateway-rdp") {
        return Ok(Config {
            prefs: Preferences {
                profiles: vec![profile],
                ..Preferences::fresh()
            },
            can_save: false,
            notice: "RDP gateway qualification · Capture only".into(),
            fixture: true,
        });
    }
    let mut password = zeroize::Zeroizing::new(var("YUUTERM_RDP_FIXTURE_PASSWORD")?);
    // "rdp-refused" saves a wrong password to observe the refusal path.
    if std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref() == Ok("rdp-refused") {
        password.push_str("-wrong");
    }
    let identity = credentials::StorageIdentity::fresh()?;
    let target = credentials::Target::for_profile(&profile, &identity)?;
    credentials::store(
        &target,
        &profile.username,
        &credentials::Secret::new(password.as_bytes().to_vec())?,
    )?;
    profile.credential_id = identity.as_local_metadata().to_owned();
    let mut profiles = vec![profile.clone()];
    if two {
        let mut second = profile.clone();
        second.id = 2;
        second.name = "SSH password fixture 2".into();
        let target = credentials::Target::for_profile(&second, &identity)?;
        credentials::store(
            &target,
            &second.username,
            &credentials::Secret::new(password.as_bytes().to_vec())?,
        )?;
        profiles.push(second);
    }
    *lock(&RDP_FIXTURE)? = profiles.clone();
    Ok(Config {
        prefs: Preferences {
            profiles,
            ..Preferences::fresh()
        },
        can_save: false,
        notice: "RDP test target fixture · Capture only".into(),
        fixture: true,
    })
}

/// Capture builds only: one Windows PowerShell bookmark starting in C:\Windows.
#[cfg(feature = "capture")]
fn local_fixture() -> Result<Config, Box<dyn std::error::Error>> {
    if std::env::var_os("LOCALAPPDATA")
        .is_none_or(|v| !v.to_string_lossy().contains("local-terminal-"))
    {
        return Err("The local terminal fixture requires an isolated profile folder".into());
    }
    let profile = Profile {
        id: 1,
        name: "Local terminal".into(),
        protocol: yuuterm_core::Protocol::Local,
        port: 0,
        local: yuuterm_core::LocalSettings {
            shell: if std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref() == Ok("local-readme") {
                yuuterm_core::LocalShell::Cmd
            } else {
                yuuterm_core::LocalShell::WindowsPowerShell
            },
            directory: r"C:\Windows".into(),
        },
        ..Default::default()
    };
    profile.validate()?;
    let readme = std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref() == Ok("local-readme");
    let profiles = if readme {
        let directory = std::env::current_dir()?.to_string_lossy().into_owned();
        ["Project", "Release checks"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| Profile {
                id: index as u64 + 1,
                name: name.into(),
                group: "Local workspace".into(),
                local: yuuterm_core::LocalSettings {
                    shell: yuuterm_core::LocalShell::Cmd,
                    directory: directory.clone(),
                },
                ..profile.clone()
            })
            .collect()
    } else {
        vec![profile]
    };
    Ok(Config {
        prefs: Preferences {
            profiles,
            theme: if readme {
                "catppuccin-mocha".into()
            } else {
                String::new()
            },
            ..Preferences::fresh()
        },
        can_save: false,
        notice: "Local terminal fixture · Capture only".into(),
        fixture: true,
    })
}

fn backend() -> Result<Backend, Box<dyn std::error::Error>> {
    let (prefs, can_save, notice) = match config::load() {
        Ok(p) => (p, true, "Ready".into()),
        Err(e) => (
            Preferences::fresh(),
            false,
            format!("Settings not loaded: {e} — original preserved"),
        ),
    };
    let mut config = Config {
        prefs,
        can_save,
        notice,
        fixture: false,
    };
    if rdp_probe_enabled() {
        config.prefs = Preferences::fresh();
        config.can_save = false;
        config.notice = "Disconnected native RDP composition probe".into();
    }
    let mut trust = config::path()?.with_file_name("host-keys.json");
    if cfg!(feature = "capture")
        && (std::env::var_os("YUUTERM_CAPTURE_PROFILE").is_some()
            || std::env::var_os("YUUTERM_CAPTURE_PROFILE_JSON").is_some())
    {
        use std::io::Read;
        let path = std::env::var("YUUTERM_CAPTURE_PROFILE").ok();
        let mut bytes = Vec::new();
        if let Some(path) = &path {
            std::fs::File::open(path)?
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
        } else {
            bytes = std::env::var("YUUTERM_CAPTURE_PROFILE_JSON")?.into_bytes();
        }
        if bytes.len() > 1024 * 1024 {
            return Err("Capture profile oversized".into());
        }
        let profile: Profile = serde_json::from_slice(&bytes)?;
        let windows_fixture = windows_openssh_capture::external_profile(&profile);
        let vnc_real = vnc_real_capture::external_profile(&profile);
        let zsh_fixture = std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref() == Ok("zsh-input")
            && std::env::var("YUUTERM_ZSH_HOST").ok().as_deref() == Some(&profile.host)
            && std::env::var("YUUTERM_ZSH_USER").ok().as_deref() == Some(&profile.username)
            && profile.protocol == yuuterm_core::Protocol::Ssh
            && profile.ssh.auth == yuuterm_core::SshAuth::WindowsAgent
            && profile.id == 1
            && !profile.logging.enabled
            && !profile.ssh.x11_forward
            && profile.credential_id.is_empty();
        let gateway_fixture = gateway_capture::admitted(&profile) && (windows_fixture || vnc_real);
        if !capture_gateways_allowed(
            (windows_fixture && profile.protocol == yuuterm_core::Protocol::Ssh) || gateway_fixture,
            &profile,
        ) {
            return Err("SSH gateways need the explicit external capture admission".into());
        }
        if !windows_fixture
            && !zsh_fixture
            && !vnc_real
            && (profile.host != "127.0.0.1"
                || profile.username != "fixture"
                || profile.id != 1
                || !matches!(
                    profile.protocol,
                    yuuterm_core::Protocol::Ssh
                        | yuuterm_core::Protocol::Telnet
                        | yuuterm_core::Protocol::Vnc
                ))
        {
            return Err(
                "Only an explicitly configured protocol capture fixture is accepted".into(),
            );
        }
        if let Some(path) = &path {
            trust = std::path::Path::new(path).with_file_name("host-keys.json");
        }
        let count = if windows_fixture || zsh_fixture || vnc_real {
            1
        } else if std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref() == Ok("performance") {
            let count = std::env::var("YUUTERM_CAPTURE_COUNT")?.parse::<u64>()?;
            if ![1, 10, 20].contains(&count) {
                return Err("Performance count must be 1, 10 or 20".into());
            }
            count
        } else {
            4
        };
        config = Config {
            prefs: Preferences {
                highlighting: config::Highlighting {
                    enabled: true,
                    ..Default::default()
                },
                profiles: (1..=count)
                    .map(|id| {
                        let mut p = profile.clone();
                        p.id = id;
                        p.name = format!(
                            "{} fixture {id}",
                            if p.protocol == yuuterm_core::Protocol::Vnc {
                                "VNC"
                            } else if p.protocol == yuuterm_core::Protocol::Telnet {
                                "TELNET"
                            } else {
                                "SSH"
                            }
                        );
                        p
                    })
                    .collect(),
                ..Preferences::fresh()
            },
            can_save: false,
            notice: if std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref() == Ok("telnet-external") {
                "External TELNET server qualification · Capture only"
            } else if windows_fixture {
                "External Windows OpenSSH qualification · Capture only"
            } else if vnc_real {
                "External VNC server qualification · Capture only"
            } else {
                "Local protocol fixture · Capture only"
            }
            .into(),
            fixture: true,
        };
    }
    #[cfg(feature = "capture")]
    if std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref() == Ok("vnc-saved") {
        // Only generated loopback bookmarks; never enumerate existing entries.
        for profile in &mut config.prefs.profiles {
            if !config.fixture
                || profile.protocol != yuuterm_core::Protocol::Vnc
                || profile.host != "127.0.0.1"
                || profile.username != "fixture"
                || !profile.credential_id.is_empty()
                || profile.temporary
            {
                return Err("Saved VNC capture requires fresh loopback fixtures".into());
            }
            let identity = credentials::StorageIdentity::fresh()?;
            let target = credentials::Target::for_profile(profile, &identity)?;
            credentials::store(
                &target,
                if profile.vnc.auth == yuuterm_core::vnc::VncAuth::Plain {
                    "fixture"
                } else {
                    ""
                },
                &credentials::Secret::new(b"password".to_vec())?,
            )?;
            profile.credential_id = identity.as_local_metadata().into();
            lock(&RDP_FIXTURE)?.push(profile.clone());
        }
    }
    #[cfg(feature = "capture")]
    // "rdp-hold" connects the same fixture and leaves the session to an
    // external script (scripts/Test-RdpClipboard.ps1), which closes the window.
    if matches!(
        std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref(),
        Ok("rdp"
            | "gateway-rdp"
            | "rdp-hold"
            | "rdp-refused"
            | "rdp-power"
            | "rdp-network"
            | "quick-connect"
            | "ssh-password"
            | "ssh-two")
    ) {
        config = rdp_fixture()?;
    }
    #[cfg(feature = "capture")]
    if matches!(
        std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref(),
        Ok("local-terminal" | "local-readme" | "local-hold" | "local-flood")
    ) {
        config = local_fixture()?;
    }
    Ok(Backend {
        vnc: vnc::DesktopVnc::new(trust.with_file_name("vnc-pins.json")),
        engine: Engine::new(trust)?,
        config: Mutex::new(config),
        sessions: Mutex::new(BTreeMap::new()),
        edits: Mutex::new(BTreeMap::new()),
        edit_polling: tokio::sync::Mutex::new(()),
        next_edit: std::sync::atomic::AtomicU64::new(1),
        pending_import: Mutex::new(None),
        log_close_waiting: std::sync::atomic::AtomicBool::new(false),
        #[cfg(feature = "capture")]
        startup_origin: std::time::Instant::now(),
        #[cfg(feature = "capture")]
        startup_timings: Arc::new(Mutex::new(BTreeMap::new())),
    })
}

/// Marks every build not made by GitHub Actions, so local executables are
/// never mistaken for published releases in the taskbar.
fn window_title() -> &'static str {
    match option_env!("GITHUB_ACTIONS") {
        Some("true") => "YuuTerm",
        _ => "YuuTerm (Dev)",
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 3 && args[1] == rdp_keys::HELPER_ARG {
        rdp_keys::run_helper(&args[2]);
        return;
    }
    let mut lease = match config::path().and_then(|path| single_instance::enter(&path)) {
        Ok(Some(lease)) => lease,
        Ok(None) => return,
        Err(error) => {
            single_instance::report(error);
            std::process::exit(1);
        }
    };
    tauri::Builder::default()
        .setup(move |app| {
            app.manage(backend()?);
            let builder = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            );
            // Portable mode keeps the WebView profile beside the executable too.
            let builder = match config::portable_dir() {
                Some(dir) if std::env::var_os("WEBVIEW2_USER_DATA_FOLDER").is_none() => {
                    builder.data_directory(dir.join("WebView2"))
                }
                _ => builder,
            };
            let window = builder
                .title(window_title())
                .decorations(false)
                .shadow(true)
                .inner_size(1280.0, 820.0)
                .min_inner_size(900.0, 600.0)
                .on_navigation(ipc_origin::allowed)
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .build()?;
            single_instance::publish(&mut lease, &window)?;
            app.manage(lease);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, position }) =
                event
                && let Some(token) = file_ui::record_drop(paths.clone())
            {
                let _ = window.emit(
                    "files-dropped",
                    serde_json::json!({
                        "token": token,
                        "x": position.x,
                        "y": position.y,
                        "count": paths.len(),
                    }),
                );
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && let Some(webview) = window.app_handle().get_webview_window(window.label())
                && rdp::disconnect_all()
            {
                // Close again once the remote sessions ended; the second request
                // finds nothing to disconnect and continues normally.
                api.prevent_close();
                let again = webview.clone();
                after_rdp_disconnected(webview, move || {
                    let _ = again.close();
                });
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && let Some(state) = window.try_state::<Backend>()
                && let Ok(mut sessions) = state.sessions.lock()
                && sessions.values().any(|s| s.output_log.is_some())
            {
                api.prevent_close();
                if !state
                    .log_close_waiting
                    .swap(true, std::sync::atomic::Ordering::AcqRel)
                {
                    for s in sessions.values_mut() {
                        if let Some(log) = &mut s.output_log {
                            log.stop();
                        }
                    }
                    finish_logs_on_close(window.app_handle().clone());
                }
            }
            if matches!(event, tauri::WindowEvent::Destroyed) {
                #[cfg(feature = "rdp-probe")]
                rdp_probe::destroy();
                rdp::close_all();
                #[cfg(feature = "capture")]
                for profile in RDP_FIXTURE
                    .lock()
                    .map(|mut p| std::mem::take(&mut *p))
                    .unwrap_or_default()
                {
                    let _ = rdp::forget_password(&profile);
                }
                if let Some(state) = window.try_state::<Backend>() {
                    ssh_gateways::cancel_all();
                    state.vnc.close_all();
                }
                #[cfg(feature = "capture")]
                {
                    let code = CAPTURE_EXIT.load(std::sync::atomic::Ordering::Relaxed);
                    if code != 0 {
                        window.app_handle().exit(code);
                    }
                }
            }
            if matches!(event, tauri::WindowEvent::Destroyed)
                && let Some(state) = window.try_state::<Backend>()
                && let Ok(mut sessions) = state.sessions.lock()
            {
                sessions.clear();
            }
        })
        .invoke_handler(tauri::generate_handler![
            project_page::open_project_page,
            change_group,
            bootstrap,
            vnc::vnc_open,
            vnc::vnc_poll,
            vnc::vnc_decide,
            vnc::vnc_authenticate,
            vnc::vnc_input,
            vnc::vnc_close,
            session_transfer::profiles_export,
            session_transfer::profiles_import_preview,
            highlight_import::highlight_import_preview,
            session_transfer::profiles_import_cancel,
            session_transfer::profiles_import_apply,
            serial_ports,
            x11_desktop::x11_probe,
            rdp_probe_surface,
            rdp_probe_report,
            rdp_open,
            ssh_gateways::gateway_cancel,
            rdp_session,
            rdp_snapshot,
            forget_saved_password,
            rdp_ssh_available,
            rdp_certificate_remembered,
            rdp_forget_certificate,
            rdp_tools_start,
            save_profile,
            duplicate_profile,
            delete_profile,
            session_actions::delete_profiles,
            session_actions::move_profiles,
            session_actions::wake_profiles,
            save_appearance,
            session_start,
            session_poll,
            session_input,
            session_statistics,
            session_nano,
            session_processes,
            ssh_tools_probe,
            ssh_folder_sizes,
            ssh_folder_sizes_cancel,
            terminal_dispatch,
            session_resize,
            session_browse,
            session_trust,
            session_disconnect,
            session_close,
            capture_bookmark_protocol,
            file_ui::file_action,
            file_ui::file_transfer,
            file_ui::file_upload_dropped,
            capture_simulate_drop,
            windows_openssh_capture::capture_external_path,
            telnet_external_capture::capture_telnet_password,
            file_ui::files_download,
            file_ui::file_cancel,
            file_ui::edit_open,
            file_ui::edit_list,
            file_ui::edit_recoverable,
            file_ui::recovery::edit_restore_list,
            file_ui::recovery::edit_restore_decide,
            file_ui::recovery::capture_restore_probe,
            file_ui::edit_resume,
            file_ui::edit_reopen,
            file_ui::edit_upload,
            file_ui::edit_changes,
            file_ui::edit_remote_changes,
            file_ui::edit_remote_decide,
            file_ui::capture_edit_probe,
            file_ui::capture_editor_bridge,
            file_ui::edit_decline,
            file_ui::edit_stop,
            capture_ready,
            capture_trace,
            capture_scope,
            capture_report,
            capture_still,
            capture_metrics
        ])
        .run(tauri::generate_context!())
        .expect("YuuTerm desktop runtime failed");
}

/// Capture profiles may name SSH gateways only through the env-matched
/// external admission; loopback and zsh fixtures stay direct.
fn capture_gateways_allowed(external: bool, profile: &Profile) -> bool {
    external || profile.gateways.is_empty()
}

#[cfg(test)]
mod tests {
    use super::ConnectAdmission;
    #[test]
    fn refused_save_forgets_no_password() {
        let id = "a3bb189e-8bf9-3888-9912-ace4e6543002";
        let mut prefs = yuuterm_core::config::Preferences::fresh();
        let mut a = yuuterm_core::Profile {
            id: 1,
            name: "A".into(),
            host: "a.test".into(),
            ..Default::default()
        };
        a.gateways = vec![yuuterm_core::Gateway::Saved { profile_id: 2 }];
        let b = yuuterm_core::Profile {
            id: 2,
            name: "B".into(),
            host: "b.test".into(),
            credential_id: id.into(),
            ..Default::default()
        };
        prefs.profiles = vec![a, b.clone()];
        // B is A's gateway: switching it to TELNET is refused and forgets nothing.
        let mut telnet = b.clone();
        telnet.protocol = yuuterm_core::Protocol::Telnet;
        telnet.port = 23;
        assert!(super::plan_profile_save(&prefs, telnet).is_err());
        // A real destination change saves and only then lists the old password.
        let mut moved = b.clone();
        moved.host = "moved.test".into();
        let (saved, forget) = super::plan_profile_save(&prefs, moved).unwrap();
        assert_eq!(saved.profiles[1].host, "moved.test");
        assert!(saved.profiles[1].credential_id.is_empty());
        assert_eq!(forget.len(), 1);
        assert_eq!(forget[0].credential_id, id);
        assert_eq!(forget[0].host, "b.test");
    }
    #[test]
    fn capture_gateways_only_with_the_explicit_external_admission() {
        let mut p = yuuterm_core::Profile {
            host: "127.0.0.1".into(),
            username: "fixture".into(),
            ..Default::default()
        };
        assert!(super::capture_gateways_allowed(false, &p));
        p.gateways = vec![yuuterm_core::Gateway::Manual(yuuterm_core::ManualGateway {
            host: "198.51.100.7".into(),
            username: "u".into(),
            ..Default::default()
        })];
        assert!(!super::capture_gateways_allowed(false, &p));
        assert!(super::capture_gateways_allowed(true, &p));
    }
    #[test]
    fn one_connect_per_rdp_session_at_a_time() {
        let first = ConnectAdmission::enter(u64::MAX - 1).unwrap();
        assert!(ConnectAdmission::enter(u64::MAX - 1).is_err());
        let other = ConnectAdmission::enter(u64::MAX - 2).unwrap();
        drop(first);
        let again = ConnectAdmission::enter(u64::MAX - 1).unwrap();
        drop((other, again));
    }
}
