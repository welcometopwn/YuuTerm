//! SSH gateway sign-in for a connection: resolve references, ask
//! for gateway passwords with the native prompt and remember which saved
//! password belongs to which hop.
use crate::{Backend, credentials, lock, persist_credential_id};
use windows::Win32::Foundation::HWND;
use yuuterm_core::{ManualGateway, Profile, Protocol, SshAuth};
use yuuterm_ssh::{Hop, Zeroizing};

/// A saved password used for one sign-in, and the bookmark whose next prompt
/// says it was refused.
pub(crate) struct SavedPassword {
    pub target: credentials::Target,
    pub owner: u64,
}

/// Credential-store view of a manual hop. The ID stays stable per bookmark
/// and position, is never a bookmark ID and never crosses IPC.
pub(crate) fn gateway_credential_owner(
    owner: &Profile,
    index: usize,
    hop: &ManualGateway,
) -> Profile {
    let mut p = Profile {
        id: (1 << 56) | ((index as u64) << 48) | owner.id,
        // Fixed and short: names are not part of the credential binding, and a
        // long bookmark name must not push this view past validation.
        name: format!("SSH gateway {}", index + 1),
        temporary: owner.temporary,
        protocol: Protocol::Ssh,
        host: hop.host.clone(),
        port: hop.port,
        username: hop.username.clone(),
        credential_id: hop.credential_id.clone(),
        ..Default::default()
    };
    p.ssh.auth = SshAuth::Password;
    p
}

/// The saved password of the hop that was refused (`None` = the target).
pub(crate) fn take_refused<T>(
    rejected_gateway: Option<usize>,
    target: &mut Option<T>,
    gateways: &mut [Option<T>],
) -> Option<T> {
    match rejected_gateway {
        None => target.take(),
        Some(index) => gateways.get_mut(index).and_then(Option::take),
    }
}

/// Resolve the chain and sign in to every password gateway, in order, before
/// any network I/O. No application lock is held across the modal prompts.
pub(crate) fn sign_in(
    parent: HWND,
    state: &Backend,
    owner: &Profile,
) -> Result<(Vec<Hop>, Vec<Option<SavedPassword>>), String> {
    if owner.gateways.is_empty() {
        return Ok(Default::default());
    }
    let profiles = lock(&state.config)?.prefs.profiles.clone();
    let resolved = yuuterm_core::resolve_gateways(owner, &profiles)?;
    let total = resolved.len();
    let mut hops = Vec::with_capacity(total);
    let mut saved = Vec::with_capacity(total);
    for r in resolved {
        let mut gateway = r.hop;
        let mut password = None;
        let mut used = None;
        if gateway.auth == SshAuth::Password {
            let account = match r.source_profile_id {
                Some(id) => profiles
                    .iter()
                    .find(|p| p.id == id)
                    .cloned()
                    .ok_or("Gateway session no longer exists")?,
                None => gateway_credential_owner(owner, r.index, &gateway),
            };
            let caption = format!("Gateway {} of {total} · {}", r.index + 1, gateway.host);
            let signed = credentials::resolve(parent, &account, &caption, 22)?
                .ok_or("Sign-in cancelled; nothing was sent")?;
            if let Some(identity) = &signed.new_identity {
                let persisted = match r.source_profile_id {
                    Some(id) => persist_credential_id(state, id, identity),
                    None => persist_gateway_credential_id(state, owner.id, r.index, identity),
                };
                if let Err(e) = persisted {
                    eprintln!("ssh: gateway saved-password identity not persisted: {e}");
                }
            }
            gateway.username = signed.username.clone();
            password = Some(Zeroizing::new(
                signed.secret.expose_for_authentication().to_owned(),
            ));
            used = signed.saved.map(|target| SavedPassword {
                target,
                owner: account.id,
            });
        }
        hops.push(Hop { gateway, password });
        saved.push(used);
    }
    Ok((hops, saved))
}

/// Hop identities are backend-owned: keep an old one only for the same
/// position, host, port and user; clear the rest. Returns the credential views
/// of old hop passwords that are no longer used, to forget them.
pub(crate) fn carry_gateway_credentials(old: &Profile, new: &mut Profile) -> Vec<Profile> {
    let mut kept = vec![false; old.gateways.len()];
    for (index, gateway) in new.gateways.iter_mut().enumerate() {
        let yuuterm_core::Gateway::Manual(hop) = gateway else {
            continue;
        };
        hop.credential_id.clear();
        if let Some(yuuterm_core::Gateway::Manual(before)) = old.gateways.get(index)
            && !before.credential_id.is_empty()
            && before.host.eq_ignore_ascii_case(&hop.host)
            && before.port == hop.port
            && before.username == hop.username
        {
            hop.credential_id = before.credential_id.clone();
            kept[index] = true;
        }
    }
    old.gateways
        .iter()
        .enumerate()
        .filter_map(|(index, gateway)| match gateway {
            yuuterm_core::Gateway::Manual(hop) if !hop.credential_id.is_empty() && !kept[index] => {
                Some(gateway_credential_owner(old, index, hop))
            }
            _ => None,
        })
        .collect()
}

/// A deleted bookmark's own and manual-gateway saved passwords.
pub(crate) fn forget_passwords(profile: &Profile) {
    if let Err(e) = crate::rdp::forget_password(profile) {
        eprintln!("delete: saved password not removed: {e}");
    }
    for (index, gateway) in profile.gateways.iter().enumerate() {
        if let yuuterm_core::Gateway::Manual(hop) = gateway
            && !hop.credential_id.is_empty()
            && let Err(e) =
                crate::rdp::forget_password(&gateway_credential_owner(profile, index, hop))
        {
            eprintln!("delete: gateway saved password not removed: {e}");
        }
    }
}

fn persist_gateway_credential_id(
    state: &Backend,
    profile_id: u64,
    index: usize,
    identity: &str,
) -> Result<(), String> {
    let mut c = lock(&state.config)?;
    let mut candidate = c.prefs.clone();
    let hop = candidate
        .profiles
        .iter_mut()
        .find(|p| p.id == profile_id)
        .and_then(|p| p.gateways.get_mut(index))
        .ok_or("Session changed during sign-in; reconnect to sign in")?;
    let yuuterm_core::Gateway::Manual(hop) = hop else {
        return Err("Session changed during sign-in; reconnect to sign in".into());
    };
    hop.credential_id = identity.to_owned();
    candidate.validate()?;
    if c.can_save {
        crate::config::save(&candidate)?;
    }
    c.prefs = candidate;
    Ok(())
}

/// Transport owner: never persisted or copied into a bookmark.
pub(crate) struct Forward {
    pub live: yuuterm_ssh::LiveSession,
    pub address: std::net::SocketAddr,
}

static PENDING: std::sync::Mutex<
    std::collections::BTreeMap<u64, tokio_util::sync::CancellationToken>,
> = std::sync::Mutex::new(std::collections::BTreeMap::new());
struct Admission(u64);
impl Drop for Admission {
    fn drop(&mut self) {
        if let Ok(mut pending) = PENDING.lock() {
            pending.remove(&self.0);
        }
    }
}

#[tauri::command]
pub(crate) fn gateway_cancel(window: tauri::WebviewWindow, profile_id: u64) -> Result<(), String> {
    crate::main_only(&window)?;
    if let Some(cancel) = lock(&PENDING)?.get(&profile_id) {
        cancel.cancel();
    }
    Ok(())
}

pub(crate) fn cancel_all() {
    if let Ok(pending) = PENDING.lock() {
        for cancel in pending.values() {
            cancel.cancel();
        }
    }
}

/// Prepare graphical transport off the render thread. Password dialogs retain
/// the native main-thread flow; host trust is explicit and cancellable.
pub(crate) async fn prepare(
    window: &tauri::WebviewWindow,
    state: &Backend,
    profile: &Profile,
) -> Result<Option<Forward>, String> {
    use tauri::Manager;
    if profile.gateways.is_empty() {
        return Ok(None);
    }
    profile.validate()?;
    let cancel = tokio_util::sync::CancellationToken::new();
    {
        let mut pending = lock(&PENDING)?;
        if pending.contains_key(&profile.id) {
            return Err("This gateway is already connecting".into());
        }
        pending.insert(profile.id, cancel.clone());
    }
    let _admission = Admission(profile.id);
    let app = window.app_handle().clone();
    let parent = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
    let owner = profile.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            let state = app.state::<Backend>();
            let _ = tx.send(sign_in(
                windows::Win32::Foundation::HWND(parent as *mut _),
                &state,
                &owner,
            ));
        })
        .map_err(|e| e.to_string())?;
    let (hops, mut saved) = rx
        .await
        .map_err(|_| "Gateway sign-in dispatcher closed")??;
    if cancel.is_cancelled() {
        return Err("Gateway connection cancelled".into());
    }
    let (live, address) = state
        .engine
        .forward_via(profile.clone(), hops, std::sync::Arc::new(|| {}))
        .map_err(|e| e.to_string())?;
    let mut changes = live.subscribe_snapshot();
    let ready = async {
        loop {
            let snapshot = changes.borrow_and_update().clone();
            if snapshot.finished {
                if snapshot.password_rejected
                    && let Some(saved) =
                        take_refused(snapshot.rejected_gateway, &mut None, &mut saved)
                {
                    let _ = credentials::forget(&saved.target);
                    credentials::mark_refused(saved.owner);
                }
                return Err(snapshot.status);
            }
            if snapshot.connected {
                return Ok(());
            }
            if let Some(trust) = snapshot.trust {
                let accepted = rfd::AsyncMessageDialog::new()
                    .set_title("Verify SSH gateway host key")
                    .set_description(format!("{}\n{}\n{}\n{}\nCompare this fingerprint with a trusted source before approving.",
                        trust.role, trust.host, trust.algorithm, trust.fingerprint))
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show().await == rfd::MessageDialogResult::Yes;
                live.trust(accepted);
            }
            changes
                .changed()
                .await
                .map_err(|_| "Gateway connection closed".to_owned())?;
        }
    };
    tokio::select! {
        _ = cancel.cancelled() => Err("Gateway connection cancelled".into()),
        result = ready => { result?; Ok(Some(Forward { live, address })) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yuuterm_core::{ManualGateway, Profile, SshAuth};

    #[test]
    fn gateway_credential_owner_uses_hop_identity() {
        let owner = Profile {
            id: 7,
            name: "Target".into(),
            host: "target.example".into(),
            temporary: true,
            ..Default::default()
        };
        let hop = ManualGateway {
            host: "bastion.example".into(),
            port: 2222,
            username: "jump".into(),
            auth: SshAuth::Password,
            credential_id: "a3bb189e-8bf9-3888-9912-ace4e6543002".into(),
            ..Default::default()
        };
        let p = gateway_credential_owner(&owner, 1, &hop);
        assert_eq!(p.id, (1 << 56) | (1 << 48) | 7);
        assert_ne!(p.id, gateway_credential_owner(&owner, 0, &hop).id);
        assert_eq!(p.name, "SSH gateway 2");
        assert_eq!((p.host.as_str(), p.port), ("bastion.example", 2222));
        assert_eq!(p.username, "jump");
        assert_eq!(p.credential_id, hop.credential_id);
        assert_eq!(p.ssh.auth, SshAuth::Password);
        assert!(p.temporary, "quick-connect hops never save passwords");
        assert!(p.validate().is_ok());
        // The owner's name is not part of the credential binding; a 128-byte
        // bookmark name must not make the hop's credential view invalid.
        let long = Profile {
            name: "é".repeat(64),
            ..owner
        };
        assert!(gateway_credential_owner(&long, 3, &hop).validate().is_ok());
    }

    #[test]
    fn hop_identities_are_backend_owned_on_save() {
        let id = |n: u8| format!("a3bb189e-8bf9-3888-9912-ace4e65430{n:02}");
        let hop = |host: &str, credential: String| {
            yuuterm_core::Gateway::Manual(ManualGateway {
                host: host.into(),
                username: "jump".into(),
                credential_id: credential,
                ..Default::default()
            })
        };
        let mut old = Profile {
            id: 3,
            name: "T".into(),
            host: "t".into(),
            ..Default::default()
        };
        old.gateways = vec![hop("a", id(1)), hop("b", id(2))];
        let mut new = old.clone();
        // The WebView sends a foreign identity, keeps hop a, moves b to c.
        new.gateways = vec![hop("a", id(9)), hop("c", id(2))];
        let dropped = carry_gateway_credentials(&old, &mut new);
        let ids: Vec<_> = new
            .gateways
            .iter()
            .map(|g| match g {
                yuuterm_core::Gateway::Manual(h) => h.credential_id.clone(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(ids, [id(1), String::new()]);
        assert_eq!(dropped.len(), 1);
        assert_eq!(dropped[0].host, "b");
        let mut fresh = new.clone();
        assert_eq!(
            carry_gateway_credentials(&Profile::default(), &mut fresh).len(),
            0
        );
        assert!(
            matches!(&fresh.gateways[0], yuuterm_core::Gateway::Manual(h) if h.credential_id.is_empty())
        );
    }

    #[test]
    fn refused_gateway_password_forgets_only_that_hop() {
        let mut target = Some("target");
        let mut gateways = [Some("gw1"), Some("gw2")];
        assert_eq!(
            take_refused(Some(1), &mut target, &mut gateways),
            Some("gw2")
        );
        assert_eq!(target, Some("target"));
        assert_eq!(gateways, [Some("gw1"), None]);
        assert_eq!(
            take_refused(None, &mut target, &mut gateways),
            Some("target")
        );
        assert_eq!(target, None);
        assert_eq!(gateways[0], Some("gw1"));
        assert_eq!(take_refused(Some(9), &mut target, &mut gateways), None);
    }
}
