//! Canvas adapter. Only bookmark IDs cross the connection boundary; no IPC host.
use super::{Backend, lock, main_only};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
use tauri::{Manager, State, WebviewWindow};
use yuuterm_core::{Profile, Protocol, vnc::VncAuth};
use yuuterm_ssh::vnc::{
    self as wire, CertificateReview, Cursor, Event, Framebuffer, Handle, Input, VncTrust,
};

/// A full 2048x2048 desktop is 1024 tiles; one poll can carry a whole frame.
const MAX_TILES: u16 = 1024;

// Separate from the existing terminal/native RDP ID spaces and exact in JS.
static NEXT: AtomicU64 = AtomicU64::new(1 << 40);
type Entry = Arc<Mutex<Session>>;
pub struct DesktopVnc {
    entries: Mutex<BTreeMap<u64, Entry>>,
    prompt_gate: tokio::sync::Mutex<()>,
    trust: tokio::sync::OnceCell<VncTrust>,
    path: PathBuf,
}
impl DesktopVnc {
    pub fn has_profiles(&self, ids: &[u64]) -> Result<bool, String> {
        for entry in lock(&self.entries)?.values() {
            if ids.contains(&lock(entry)?.profile.id) {
                return Ok(true);
            }
        }
        Ok(false)
    }
    #[cfg(feature = "capture")]
    pub fn capture_ready(&self) -> bool {
        self.entries.lock().is_ok_and(|entries| {
            !entries.is_empty()
                && entries.values().all(|s| {
                    s.lock()
                        .is_ok_and(|s| s.status.connected && s.baseline.is_some())
                })
        })
    }
    pub fn new(path: PathBuf) -> Self {
        Self {
            entries: Mutex::new(BTreeMap::new()),
            prompt_gate: tokio::sync::Mutex::new(()),
            trust: tokio::sync::OnceCell::new(),
            path,
        }
    }
    fn get(&self, id: u64) -> Result<Entry, String> {
        lock(&self.entries)?
            .get(&id)
            .cloned()
            .ok_or("VNC session is closed".into())
    }
    #[cfg(test)]
    fn open(&self, profile: Profile, trust: VncTrust) -> Result<u64, String> {
        self.open_via(profile, trust, None)
    }
    fn open_via(
        &self,
        profile: Profile,
        trust: VncTrust,
        forward: Option<super::ssh_gateways::Forward>,
    ) -> Result<u64, String> {
        let mut entries = lock(&self.entries)?;
        // The renderer's double-click guard is not a backend resource bound.
        // Check and insert under the same lock, including concurrent IPC calls.
        for entry in entries.values() {
            let session = lock(entry)?;
            if session.profile.id == profile.id && !session.status.finished {
                return Err("This bookmark already has a VNC connection".into());
            }
        }
        entries.retain(|_, entry| entry.lock().is_ok_and(|s| s.profile.id != profile.id));
        let handle = start_transport(&profile, trust.clone(), forward.as_ref())?;
        let id = NEXT
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < (1 << 53) - 1).then_some(n + 1)
            })
            .map_err(|_| "VNC IDs exhausted")?;
        let status = Status {
            id,
            endpoint: format!("{}:{}", profile.host, profile.port),
            status: "Validating VNC TLS identity…".into(),
            connected: false,
            finished: false,
            width: 0,
            height: 0,
            challenge: 0,
            prompt: None,
            clipboard_enabled: profile.vnc.clipboard,
            resize_enabled: profile.vnc.resize_server,
            resize_supported: false,
            resize_status: None,
            cursor: None,
        };
        entries.insert(
            id,
            Arc::new(Mutex::new(Session {
                profile,
                trust,
                handle,
                status,
                review: None,
                latest: None,
                transfer: None,
                baseline: None,
                cursor: None,
                saved_password: None,
                forward,
            })),
        );
        Ok(id)
    }
    pub fn close_all(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            for entry in entries.values() {
                if let Ok(mut s) = entry.lock() {
                    s.disconnect();
                }
            }
            entries.clear();
        }
    }
}
#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum Prompt {
    Certificate {
        subject: String,
        issuer: String,
        valid_from: String,
        valid_until: String,
        sha256: String,
    },
    Credentials {
        username: String,
        auth: VncAuth,
    },
}
#[derive(Clone, Serialize)]
struct Status {
    id: u64,
    endpoint: String,
    status: String,
    connected: bool,
    finished: bool,
    width: u16,
    height: u16,
    challenge: u64,
    prompt: Option<Prompt>,
    clipboard_enabled: bool,
    resize_enabled: bool,
    resize_supported: bool,
    resize_status: Option<u16>,
    cursor: Option<CursorStatus>,
}
/// Server pointer shape; `serial` changes whenever the shape does.
#[derive(Clone, Serialize)]
struct CursorStatus {
    serial: u64,
    hotspot_x: u16,
    hotspot_y: u16,
    width: u16,
    height: u16,
}
struct Transfer {
    frame: Arc<Framebuffer>,
    tile: usize,
}
struct Session {
    profile: Profile,
    trust: VncTrust,
    handle: Handle,
    status: Status,
    review: Option<CertificateReview>,
    latest: Option<Arc<Framebuffer>>,
    transfer: Option<Transfer>,
    baseline: Option<Arc<Framebuffer>>,
    cursor: Option<Arc<Cursor>>,
    saved_password: Option<super::credentials::Target>,
    forward: Option<super::ssh_gateways::Forward>,
}
fn start_transport(
    profile: &Profile,
    trust: VncTrust,
    forward: Option<&super::ssh_gateways::Forward>,
) -> Result<Handle, String> {
    if !profile.gateways.is_empty() && forward.is_none() {
        return Err("SSH gateway transport ended; reconnect opens a new one".into());
    }
    match forward {
        Some(f) => wire::start_interactive_via(options(profile), trust, f.address),
        None => wire::start_interactive(options(profile), trust),
    }
    .map_err(|e| e.to_string())
}
fn options(profile: &Profile) -> wire::Options {
    wire::Options {
        host: profile.host.clone(),
        port: profile.port,
        roots: wire::RootCertStore::empty(),
        auth: match profile.vnc.auth {
            VncAuth::Plain => wire::TlsAuth::Plain,
            VncAuth::Vnc => wire::TlsAuth::Vnc,
        },
    }
}
impl Session {
    // Consume the challenge before dispatching a modal dialog. Duplicate IPC
    // cannot queue another prompt, including while the UI modal loop runs.
    fn claim_credentials(&mut self, challenge: u64) -> Result<(String, bool), String> {
        if self.status.finished
            || self.status.challenge != challenge
            || !matches!(self.status.prompt, Some(Prompt::Credentials { .. }))
        {
            return Err("VNC credential request expired".into());
        }
        let Some(Prompt::Credentials { username, auth }) = self.status.prompt.take() else {
            return Err("VNC credential request expired".into());
        };
        self.status.status = "Waiting for native VNC sign-in…".into();
        Ok((username, auth == VncAuth::Plain))
    }

    fn disconnect(&mut self) {
        self.handle.cancel();
        self.forward.take();
        self.review = None;
        self.status.prompt = None;
        self.status.connected = false;
        self.status.finished = true;
        self.status.status = "VNC disconnected".into();
        self.latest = None;
        self.transfer = None;
        self.baseline = None;
        self.cursor = None;
        self.status.cursor = None;
    }
    fn receive(&mut self) {
        if let Some(forward) = &self.forward {
            let snapshot = forward.live.snapshot();
            if snapshot.finished {
                self.disconnect();
                self.status.status = snapshot.status;
                return;
            }
        }
        for _ in 0..2 {
            let Ok(event) = self.handle.events.try_recv() else {
                break;
            };
            match event {
                Event::Connected { width, height, .. } => {
                    self.status.connected = true;
                    self.status.status = "VNC connected · encrypted".into();
                    self.status.width = width;
                    self.status.height = height;
                    self.status.prompt = None;
                }
                Event::Frame { framebuffer, .. } => self.latest = Some(framebuffer),
                Event::Closed {
                    reason,
                    password_rejected,
                    ..
                } => {
                    if password_rejected {
                        if let Some(target) = self.saved_password.take()
                            && let Err(error) = super::credentials::forget(&target)
                        {
                            self.status.status = error;
                            self.status.finished = true;
                            self.status.connected = false;
                            self.status.prompt = None;
                            continue;
                        }
                        super::credentials::mark_refused(self.profile.id);
                    }
                    self.status.connected = false;
                    self.status.finished = true;
                    self.forward.take();
                    self.status.status = reason;
                    self.status.prompt = None;
                }
                Event::CredentialsRequired { .. } => {
                    self.status.challenge += 1;
                    self.status.prompt = Some(Prompt::Credentials {
                        username: self.profile.username.clone(),
                        auth: self.profile.vnc.auth,
                    });
                    self.status.status = "VNC password required · TLS validated".into();
                }
                Event::TrustRequired { review, .. } => match certificate_prompt(&review) {
                    Ok(prompt) => {
                        self.status.challenge += 1;
                        self.status.prompt = Some(prompt);
                        self.status.status = "VNC certificate approval required".into();
                        self.review = Some(review);
                    }
                    Err(error) => {
                        self.status.finished = true;
                        self.status.status = error;
                    }
                },
            }
        }
    }
    fn poll(&mut self, cursor_serial: u64) -> Result<Vec<u8>, String> {
        if !self.status.finished {
            self.receive();
        }
        if let Some(frame) = &self.latest {
            self.track_cursor(frame.cursor.clone());
        }
        if self.transfer.is_none()
            && let Some(frame) = self.latest.take()
        {
            self.status.width = frame.width;
            self.status.height = frame.height;
            self.status.resize_supported = frame.resize.is_some();
            self.status.resize_status = frame.resize_status;
            self.transfer = Some(Transfer { frame, tile: 0 });
        }
        let mut tiles = Vec::with_capacity(270000);
        let mut count = 0u16;
        if let Some(t) = &mut self.transfer {
            let columns = (t.frame.width as usize).div_ceil(64);
            let rows = (t.frame.height as usize).div_ceil(64);
            while t.tile < columns * rows && count < MAX_TILES {
                let x = (t.tile % columns) * 64;
                let y = (t.tile / columns) * 64;
                t.tile += 1;
                let w = 64.min(t.frame.width as usize - x);
                let h = 64.min(t.frame.height as usize - y);
                let changed = self.baseline.as_ref().is_none_or(|old| {
                    old.width != t.frame.width
                        || old.height != t.frame.height
                        || (y..y + h).any(|row| {
                            let start = (row * t.frame.width as usize + x) * 4;
                            old.rgba[start..start + w * 4] != t.frame.rgba[start..start + w * 4]
                        })
                });
                if !changed {
                    continue;
                }
                for value in [x, y, w, h] {
                    tiles.extend_from_slice(&(value as u16).to_be_bytes());
                }
                for row in y..y + h {
                    let start = (row * t.frame.width as usize + x) * 4;
                    tiles.extend_from_slice(&t.frame.rgba[start..start + w * 4]);
                }
                count += 1;
            }
            if t.tile == columns * rows {
                self.baseline = self.transfer.take().map(|t| t.frame);
            }
        }
        let json =
            serde_json::to_vec(&self.status).map_err(|_| "VNC status serialization failed")?;
        // The renderer passes the serial it already shows; pixels only on change.
        let shape = self
            .status
            .cursor
            .as_ref()
            .filter(|c| c.serial != cursor_serial)
            .and(self.cursor.as_ref());
        let mut packet = Vec::with_capacity(7 + json.len() + tiles.len());
        packet.extend_from_slice(&(json.len() as u32).to_be_bytes());
        packet.extend(json);
        packet.extend_from_slice(&count.to_be_bytes());
        packet.extend(tiles);
        packet.push(u8::from(shape.is_some()));
        if let Some(shape) = shape {
            packet.extend_from_slice(&shape.rgba);
        }
        Ok(packet)
    }
    fn track_cursor(&mut self, cursor: Option<Arc<Cursor>>) {
        let same = match (&self.cursor, &cursor) {
            (Some(old), Some(new)) => Arc::ptr_eq(old, new),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        let serial = self.status.cursor.as_ref().map_or(0, |c| c.serial) + 1;
        self.status.cursor = cursor.as_ref().map(|c| CursorStatus {
            serial,
            hotspot_x: c.hotspot_x,
            hotspot_y: c.hotspot_y,
            width: c.width,
            height: c.height,
        });
        self.cursor = cursor;
    }
}
fn bounded(value: String) -> String {
    value.chars().take(1024).collect()
}
fn certificate_prompt(review: &CertificateReview) -> Result<Prompt, String> {
    use x509_parser::prelude::*;
    let (_, cert) = X509Certificate::from_der(&review.certificate_der)
        .map_err(|_| "VNC certificate details could not be parsed")?;
    Ok(Prompt::Certificate {
        subject: bounded(cert.subject().to_string()),
        issuer: bounded(cert.issuer().to_string()),
        valid_from: cert.validity().not_before.to_string(),
        valid_until: cert.validity().not_after.to_string(),
        sha256: review.sha256.clone(),
    })
}

#[tauri::command]
pub async fn vnc_open(
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
        .ok_or("Bookmark no longer exists")?;
    if profile.protocol != Protocol::Vnc {
        return Err("Bookmark is not VNC".into());
    }
    profile.validate().map_err(str::to_owned)?;
    let trust = state
        .vnc
        .trust
        .get_or_try_init(|| VncTrust::load(state.vnc.path.clone()))
        .await
        .map_err(|e| e.to_string())?
        .clone();
    let forward = super::ssh_gateways::prepare(&window, &state, &profile).await?;
    state.vnc.open_via(profile, trust, forward)
}
#[tauri::command]
pub async fn vnc_poll(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
    cursor: u64,
) -> Result<tauri::ipc::Response, String> {
    main_only(&window)?;
    let entry = state.vnc.get(id)?;
    tauri::async_runtime::spawn_blocking(move || {
        lock(&entry)?.poll(cursor).map(tauri::ipc::Response::new)
    })
    .await
    .map_err(|_| "VNC poll worker failed")?
}
#[tauri::command]
pub async fn vnc_decide(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
    challenge: u64,
    approve: bool,
) -> Result<(), String> {
    main_only(&window)?;
    let entry = state.vnc.get(id)?;
    let (review, trust) = {
        let mut s = lock(&entry)?;
        if s.status.finished || s.status.challenge != challenge || s.review.is_none() {
            return Err("VNC certificate review expired".into());
        }
        if !approve {
            s.disconnect();
            return Ok(());
        }
        s.status.prompt = None;
        s.status.status = "Saving VNC certificate decision…".into();
        (s.review.take().unwrap(), s.trust.clone())
    };
    let result = trust.approve(review).await.map_err(|e| e.to_string());
    let mut s = lock(&entry)?;
    if s.status.finished || s.status.challenge != challenge {
        return Err("VNC session closed during certificate approval".into());
    }
    if let Err(error) = result {
        s.disconnect();
        s.status.status = error.clone();
        return Err(error);
    }
    s.handle = start_transport(&s.profile, trust, s.forward.as_ref())?;
    s.status.status = "Revalidating approved VNC identity…".into();
    Ok(())
}
#[tauri::command]
pub async fn vnc_authenticate(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
    challenge: u64,
) -> Result<(), String> {
    main_only(&window)?;
    let parent = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
    let entry = state.vnc.get(id)?;
    let (_, account) = lock(&entry)?.claim_credentials(challenge)?;
    // Never hold a session/config mutex across a modal prompt. Serialize dialogs
    // from different panes; closing a queued session invalidates its challenge.
    let _gate = state.vnc.prompt_gate.lock().await;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let pending = entry.clone();
    let app = window.app_handle().clone();
    let dispatched = window.run_on_main_thread(move || {
        let result = (|| {
            let profile = {
                let s = lock(&pending)?;
                if s.status.finished || s.status.challenge != challenge {
                    return Err("VNC credential request expired".into());
                }
                s.profile.clone()
            };
            let backend = app.state::<Backend>();
            let current_matches = || -> Result<(), String> {
                let c = lock(&backend.config)?;
                let current = c
                    .prefs
                    .profiles
                    .iter()
                    .find(|p| p.id == profile.id)
                    .ok_or("Session no longer exists")?;
                if !super::credentials::same_destination(current, &profile)
                    || current.credential_id != profile.credential_id
                    || current.temporary != profile.temporary
                {
                    return Err("Session changed during VNC sign-in; reconnect to sign in".into());
                }
                Ok(())
            };
            current_matches()?;
            let resolved = super::credentials::resolve_for_account(
                windows::Win32::Foundation::HWND(parent as *mut _),
                &profile,
                "YuuTerm VNC sign-in",
                5900,
                account,
            )?;
            let Some(resolved) = resolved else {
                return Ok(None);
            };
            let final_result = (|| {
                let mut s = lock(&pending)?;
                if s.status.finished || s.status.challenge != challenge {
                    return Err("VNC credential request expired".into());
                }
                current_matches()?;
                if let Some(identity) = &resolved.new_identity {
                    super::persist_credential_id_checked(
                        &backend,
                        profile.id,
                        identity,
                        Some(&profile),
                    )?;
                    s.profile.credential_id = identity.clone();
                }
                Ok(())
            })();
            if let Err(error) = final_result {
                // Do not leave a newly stored orphan after cancel/edit/delete.
                if resolved.new_identity.is_some()
                    && let Some(target) = &resolved.saved
                {
                    super::credentials::forget(target)?;
                }
                return Err(error);
            }
            Ok(Some(resolved))
        })();
        let _ = sender.send(result);
    });
    let result = match dispatched {
        Ok(()) => receiver
            .await
            .unwrap_or_else(|_| Err("VNC UI dispatcher closed".into())),
        Err(_) => Err("VNC UI dispatcher closed".into()),
    };
    let mut s = lock(&entry)?;
    if s.status.finished || s.status.challenge != challenge {
        return Err("VNC credential request expired".into());
    }
    let prompted = match result {
        Ok(Some(prompted)) => prompted,
        Ok(None) => {
            s.disconnect();
            return Ok(());
        }
        Err(error) => {
            s.disconnect();
            return Err(error);
        }
    };
    let credentials = wire::Credentials {
        username: wire::Zeroizing::new(if account {
            prompted.username
        } else {
            String::new()
        }),
        password: wire::Zeroizing::new(prompted.secret.expose_for_authentication().to_owned()),
    };
    if let Err(error) = s.handle.provide_credentials(credentials) {
        s.disconnect();
        return Err(error.to_string());
    }
    s.saved_password = prompted.saved;
    s.status.prompt = None;
    s.status.status = "Authenticating VNC…".into();
    Ok(())
}
#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Command {
    Key { down: bool, keysym: u32 },
    Pointer { buttons: u8, x: u16, y: u16 },
    ReleaseAll,
    Clipboard { text: String },
    Resize { width: u16, height: u16 },
}
fn validate_control(
    profile: &Profile,
    command: &Command,
    width: u16,
    height: u16,
    resize_supported: bool,
) -> Result<(), String> {
    match command {
        Command::Clipboard { text } => {
            if !profile.vnc.clipboard {
                return Err("Enable local-to-remote clipboard in this VNC bookmark first".into());
            }
            wire::validate_clipboard(text).map_err(|e| e.to_string())?;
        }
        Command::Resize { width, height } => {
            if !profile.vnc.resize_server || !resize_supported {
                return Err("Server resize unavailable; the desktop is scaled".into());
            }
            if *width == 0 || *height == 0 || *width as usize * *height as usize > 4 * 1024 * 1024 {
                return Err("VNC resize exceeds limit".into());
            }
        }
        Command::Pointer { x, y, .. } if *x >= width || *y >= height => {
            return Err("VNC pointer outside framebuffer".into());
        }
        _ => {}
    }
    Ok(())
}

#[tauri::command]
pub async fn vnc_input(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
    commands: Vec<Command>,
) -> Result<(), String> {
    main_only(&window)?;
    if commands.len() > 64 {
        return Err("VNC input batch exceeds limit".into());
    }
    let entry = state.vnc.get(id)?;
    let mut s = lock(&entry)?;
    if !s.status.connected || s.status.finished {
        return Err("VNC session is not connected".into());
    }
    for command in &commands {
        validate_control(
            &s.profile,
            command,
            s.status.width,
            s.status.height,
            s.status.resize_supported,
        )?;
    }
    for command in commands {
        let input = match command {
            Command::Key { down, keysym } => Input::Key { down, keysym },
            Command::Pointer { buttons, x, y } => Input::Pointer { buttons, x, y },
            Command::ReleaseAll => Input::ReleaseAll,
            Command::Clipboard { text } => Input::Clipboard {
                text: wire::Zeroizing::new(text),
            },
            Command::Resize { width, height } => Input::Resize { width, height },
        };
        if let Err(error) = s.handle.try_input(s.handle.id, input) {
            s.disconnect();
            s.status.status = "VNC input stalled; disconnected to release keys".into();
            return Err(error.to_string());
        }
    }
    Ok(())
}
#[tauri::command]
pub async fn vnc_close(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
    remove: bool,
) -> Result<(), String> {
    main_only(&window)?;
    let entry = state.vnc.get(id)?;
    lock(&entry)?.disconnect();
    if remove {
        lock(&state.vnc.entries)?.remove(&id);
    }
    Ok(())
}

#[cfg(test)]
mod control_tests {
    use super::*;
    #[test]
    fn duplicate_ipc_opens_cannot_accumulate_workers_for_one_bookmark() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/security-review-tests");
            std::fs::create_dir_all(&root).unwrap();
            let dir = tempfile::tempdir_in(root).unwrap();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let profile = Profile {
                id: 1,
                host: "127.0.0.1".into(),
                port: listener.local_addr().unwrap().port(),
                protocol: Protocol::Vnc,
                ..Default::default()
            };
            let desktop = Arc::new(DesktopVnc::new(dir.path().join("pins.json")));
            let trust = VncTrust::load(desktop.path.clone()).await.unwrap();
            let jobs = (0..16)
                .map(|_| {
                    let desktop = desktop.clone();
                    let profile = profile.clone();
                    let trust = trust.clone();
                    let runtime = tokio::runtime::Handle::current();
                    std::thread::spawn(move || {
                        let _guard = runtime.enter();
                        desktop.open(profile, trust)
                    })
                })
                .collect::<Vec<_>>();
            let results = jobs
                .into_iter()
                .map(|j| j.join().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
            let first = *results.iter().find_map(|r| r.as_ref().ok()).unwrap();
            assert_eq!(lock(&desktop.entries).unwrap().len(), 1);
            lock(&desktop.get(first).unwrap()).unwrap().disconnect();
            let next = desktop.open(profile, trust).unwrap();
            assert_ne!(first, next);
            assert!(desktop.get(first).is_err());
            assert_eq!(lock(&desktop.entries).unwrap().len(), 1);
            {
                let entry = desktop.get(next).unwrap();
                let mut s = lock(&entry).unwrap();
                assert!(
                    s.claim_credentials(0).is_err(),
                    "No prompt before TLS event"
                );
                s.status.prompt = Some(Prompt::Certificate {
                    subject: "fixture".into(),
                    issuer: "fixture".into(),
                    valid_from: "fixture".into(),
                    valid_until: "fixture".into(),
                    sha256: "fixture".into(),
                });
                assert!(s.claim_credentials(0).is_err());
                assert!(matches!(s.status.prompt, Some(Prompt::Certificate { .. })));
                for (auth, account) in [(VncAuth::Plain, true), (VncAuth::Vnc, false)] {
                    s.status.challenge += 1;
                    let challenge = s.status.challenge;
                    s.status.prompt = Some(Prompt::Credentials {
                        username: "fixture".into(),
                        auth,
                    });
                    assert!(s.claim_credentials(challenge - 1).is_err());
                    assert_eq!(
                        s.claim_credentials(challenge).unwrap(),
                        ("fixture".into(), account)
                    );
                    assert!(
                        s.claim_credentials(challenge).is_err(),
                        "No duplicate modal prompt"
                    );
                }
                s.disconnect();
                let challenge = s.status.challenge;
                assert!(s.claim_credentials(challenge).is_err());
            }
            desktop.close_all();
        });
    }
    #[test]
    fn clipboard_and_resize_require_explicit_opt_in_and_validation() {
        let mut profile = Profile::default();
        let clipboard = Command::Clipboard {
            text: "fixture".into(),
        };
        assert!(validate_control(&profile, &clipboard, 640, 400, true).is_err());
        let resize = Command::Resize {
            width: 800,
            height: 600,
        };
        assert!(validate_control(&profile, &resize, 640, 400, true).is_err());
        profile.vnc.clipboard = true;
        profile.vnc.resize_server = true;
        assert!(validate_control(&profile, &clipboard, 640, 400, true).is_ok());
        assert!(validate_control(&profile, &resize, 640, 400, true).is_ok());
        assert!(validate_control(&profile, &resize, 640, 400, false).is_err());
        assert!(
            validate_control(
                &profile,
                &Command::Clipboard { text: "€".into() },
                640,
                400,
                true
            )
            .is_err()
        );
    }
}
