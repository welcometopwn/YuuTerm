//! Terminal pop-out windows. The main window stays the only window with
//! general IPC (`main_only`). A pop-out window, labelled `term-<n>`, is bound
//! by the backend to exactly one live session and can only attach, poll, type
//! and resize that session, and hand it back. It never names a session ID.
//!
//! While a session is popped out its terminal output goes to the pop-out; the
//! main window keeps polling for status only. Screen content moves between the
//! windows as a bounded xterm serialization.
use super::{Backend, Poll, lock, main_only, meta};
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::{Emitter, Manager, State, WebviewWindow};
use yuuterm_core::Protocol;
use yuuterm_ssh::Command;

pub const PREFIX: &str = "term-";
/// Serialized screen and scrollback carried between the windows.
const SNAPSHOT_LIMIT: usize = 4 * 1024 * 1024;

/// Pop-out state of one live session.
#[derive(Default)]
pub struct Popout {
    /// Label of the pop-out window that owns the terminal output.
    pub window: Option<String>,
    pub wake: std::sync::Arc<tokio::sync::Notify>,
    /// Content for the pop-out, taken once when it attaches.
    outgoing: Option<String>,
    title: String,
    font_size: f32,
    /// Content handed back to the main window, taken once.
    returned: Option<String>,
    /// Status last sent to the pop-out, to send changes only.
    sent: Option<(bool, bool, String)>,
}

fn popout_only(window: &WebviewWindow) -> Result<(), String> {
    if window.label().starts_with(PREFIX)
        && window
            .url()
            .is_ok_and(|url| super::ipc_origin::allowed(&url))
    {
        Ok(())
    } else {
        Err("This command is restricted to a terminal pop-out window".into())
    }
}

fn valid_snapshot(snapshot: &str) -> Result<(), String> {
    if snapshot.len() > SNAPSHOT_LIMIT {
        return Err("Terminal content is too large to move".into());
    }
    Ok(())
}

/// The live session bound to this pop-out window.
fn bound(state: &Backend, window: &WebviewWindow) -> Result<u64, String> {
    popout_only(window)?;
    let label = window.label();
    lock(&state.sessions)?
        .iter()
        .find(|(_, s)| s.popout.window.as_deref() == Some(label))
        .map(|(id, _)| *id)
        .ok_or_else(|| "Connection no longer exists".into())
}

#[tauri::command]
pub async fn popout_open(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
    title: String,
    font_size: f32,
    snapshot: String,
) -> Result<(), String> {
    main_only(&window)?;
    valid_snapshot(&snapshot)?;
    let title = title.trim().to_string();
    if title.is_empty() || title.len() > 160 || title.chars().any(char::is_control) {
        return Err("Invalid window title".into());
    }
    if !font_size.is_finite() || !(8.0..=32.0).contains(&font_size) {
        return Err("Invalid font size".into());
    }
    let label = format!(
        "{PREFIX}{}",
        state.next_popout.fetch_add(1, Ordering::Relaxed)
    );
    {
        let mut sessions = lock(&state.sessions)?;
        let s = sessions.get_mut(&id).ok_or("Connection no longer exists")?;
        let snap = s.live.snapshot();
        if !matches!(
            snap.protocol,
            Protocol::Ssh | Protocol::Telnet | Protocol::Serial | Protocol::Local
        ) {
            return Err("Only terminal sessions can open in their own window".into());
        }
        if snap.finished {
            return Err("Connection has ended".into());
        }
        if s.popout.window.is_some() {
            return Err("This connection already has its own window".into());
        }
        s.popout.window = Some(label.clone());
        s.popout.outgoing = Some(snapshot);
        s.popout.title = title.clone();
        s.popout.font_size = font_size;
        s.popout.returned = None;
        s.popout.sent = None;
    }
    let builder = tauri::WebviewWindowBuilder::new(
        window.app_handle(),
        &label,
        tauri::WebviewUrl::App("index.html".into()),
    );
    // One WebView2 environment per process: use the main window's data folder.
    let builder = match yuuterm_core::config::portable_dir() {
        Some(dir) if std::env::var_os("WEBVIEW2_USER_DATA_FOLDER").is_none() => {
            builder.data_directory(dir.join("WebView2"))
        }
        _ => builder,
    };
    let built = builder
        .title(format!("{title} – YuuTerm"))
        .background_color(tauri::window::Color(0x20, 0x21, 0x25, 0xFF))
        .inner_size(960.0, 600.0)
        .min_inner_size(420.0, 260.0)
        .on_navigation(super::ipc_origin::allowed)
        .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
        .build();
    match built {
        Ok(popout) => {
            taskbar_id(&popout, Some(&label));
            Ok(())
        }
        Err(e) => {
            if let Ok(mut sessions) = lock(&state.sessions)
                && let Some(s) = sessions.get_mut(&id)
                && s.popout.window.as_deref() == Some(label.as_str())
            {
                s.popout = Popout {
                    wake: s.popout.wake.clone(),
                    ..Default::default()
                };
            }
            Err(format!("Could not open the window: {e}"))
        }
    }
}

/// Gives a pop-out its own taskbar button instead of joining the YuuTerm group.
/// Without a relaunch command the entry cannot be pinned, so pinning is
/// switched off for it. `None` clears the ID before the window goes.
#[cfg(windows)]
fn taskbar_id(window: &WebviewWindow, label: Option<&str>) {
    use windows::Win32::{
        Foundation::HWND,
        Storage::EnhancedStorage::{PKEY_AppUserModel_ID, PKEY_AppUserModel_PreventPinning},
        System::Com::StructuredStorage::PROPVARIANT,
        UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow},
    };
    let Ok(hwnd) = window.hwnd() else { return };
    let hwnd = hwnd.0 as isize;
    let app = window.app_handle().config().identifier.clone();
    let id = label.map(|label| format!("{app}.{label}"));
    let apply = move || unsafe {
        let Ok(store) = SHGetPropertyStoreForWindow::<IPropertyStore>(HWND(hwnd as _)) else {
            return;
        };
        let value = id.as_deref().map(PROPVARIANT::from).unwrap_or_default();
        if id.is_some() {
            let _ = store.SetValue(&PKEY_AppUserModel_PreventPinning, &PROPVARIANT::from(true));
        }
        let _ = store.SetValue(&PKEY_AppUserModel_ID, &value);
        let _ = store.Commit();
    };
    // The shell property store needs the UI thread's COM apartment.
    if label.is_some() {
        let _ = window.run_on_main_thread(apply);
    } else {
        apply();
    }
}
#[cfg(not(windows))]
fn taskbar_id(_: &WebviewWindow, _: Option<&str>) {}

#[derive(Serialize)]
pub struct Attach {
    title: String,
    protocol: Protocol,
    snapshot: String,
    font_size: f32,
    font_family: String,
    font_color: Option<String>,
    scrollback_lines: u32,
    cursor_style: yuuterm_core::config::CursorStyle,
    steady_cursor: bool,
    theme: String,
    mocha: bool,
    highlighting: yuuterm_core::config::Highlighting,
    fixture: bool,
}

#[tauri::command]
pub fn popout_attach(window: WebviewWindow, state: State<Backend>) -> Result<Attach, String> {
    let id = bound(&state, &window)?;
    let config = lock(&state.config)?;
    let mut sessions = lock(&state.sessions)?;
    let s = sessions.get_mut(&id).ok_or("Connection no longer exists")?;
    let profile = config.prefs.profiles.iter().find(|p| p.id == s.profile_id);
    Ok(Attach {
        title: s.popout.title.clone(),
        protocol: s.live.snapshot().protocol,
        snapshot: s.popout.outgoing.take().unwrap_or_default(),
        font_size: s.popout.font_size,
        font_family: profile
            .and_then(|p| p.font_family.clone())
            .unwrap_or_else(|| config.prefs.font_family.clone()),
        font_color: profile.and_then(|p| p.font_color.clone()),
        scrollback_lines: config.prefs.scrollback_lines,
        cursor_style: config.prefs.cursor_style,
        steady_cursor: config.prefs.steady_cursor,
        theme: config.prefs.theme.clone(),
        mocha: config.prefs.mocha,
        highlighting: config.prefs.highlighting.clone(),
        // Capture builds run the window's self-check in the first pop-out only.
        fixture: cfg!(feature = "capture")
            && config.fixture
            && !FIXTURE_CHECKED.swap(true, Ordering::Relaxed),
    })
}

#[tauri::command]
pub async fn popout_poll(
    window: WebviewWindow,
    state: State<'_, Backend>,
    wait_ms: Option<u64>,
) -> Result<Poll, String> {
    let id = bound(&state, &window)?;
    let wait = wait_ms.unwrap_or(0);
    if wait > 250 {
        return Err("Terminal poll wait must be 0–250 ms".into());
    }
    let wake = lock(&state.sessions)?
        .get(&id)
        .ok_or("Connection no longer exists")?
        .popout
        .wake
        .clone();
    let notified = wake.notified();
    let first = drain(&state, id)?;
    if wait == 0 || first.meta.is_some() || !first.chunks.is_empty() {
        return Ok(first);
    }
    let _ = tokio::time::timeout(std::time::Duration::from_millis(wait), notified).await;
    drain(&state, id)
}

fn drain(state: &Backend, id: u64) -> Result<Poll, String> {
    let mut sessions = lock(&state.sessions)?;
    let s = sessions.get_mut(&id).ok_or("Connection no longer exists")?;
    let snapshot = s.live.snapshot();
    let status = (
        snapshot.connected,
        snapshot.finished,
        snapshot.status.clone(),
    );
    let meta = (s.popout.sent.as_ref() != Some(&status)).then(|| {
        s.popout.sent = Some(status);
        meta(snapshot, None)
    });
    let chunks = super::take_output(s);
    Ok(Poll {
        meta,
        chunks,
        more: !s.output.is_empty(),
        logging: s.output_log.as_ref().map(|l| l.status()),
        popped: false,
    })
}

#[tauri::command]
pub fn popout_input(
    window: WebviewWindow,
    state: State<Backend>,
    bytes: Vec<u8>,
) -> Result<(), String> {
    let id = bound(&state, &window)?;
    super::send_input(&state, id, bytes)
}

#[tauri::command]
pub fn popout_resize(
    window: WebviewWindow,
    state: State<Backend>,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    let id = bound(&state, &window)?;
    lock(&state.sessions)?
        .get(&id)
        .ok_or("Connection no longer exists")?
        .live
        .send(Command::Resize { rows, cols })
        .map_err(String::from)
}

/// The pop-out hands its content back and closes; the main window takes over.
#[tauri::command]
pub fn popout_return(
    window: WebviewWindow,
    state: State<Backend>,
    snapshot: String,
) -> Result<(), String> {
    popout_only(&window)?;
    valid_snapshot(&snapshot)?;
    release(&state, window.label(), Some(snapshot));
    taskbar_id(&window, None);
    let _ = window.destroy();
    Ok(())
}

/// Main window: ask the pop-out of `id` to hand the session back.
#[tauri::command]
pub fn popout_recall(window: WebviewWindow, state: State<Backend>, id: u64) -> Result<(), String> {
    main_only(&window)?;
    let label = lock(&state.sessions)?
        .get(&id)
        .ok_or("Connection no longer exists")?
        .popout
        .window
        .clone()
        .ok_or("This connection is not in its own window")?;
    match window.app_handle().get_webview_window(&label) {
        Some(popout) => popout
            .emit_to(&label, "popout-return-request", ())
            .map_err(|e| e.to_string()),
        None => {
            release(&state, &label, None);
            Ok(())
        }
    }
}

/// Main window: the content handed back by the pop-out of `id`, once.
#[tauri::command]
pub fn popout_take_returned(
    window: WebviewWindow,
    state: State<Backend>,
    id: u64,
) -> Result<Option<String>, String> {
    main_only(&window)?;
    Ok(lock(&state.sessions)?
        .get_mut(&id)
        .and_then(|s| s.popout.returned.take()))
}

/// Unbinds the pop-out `label` and tells the main window to show the session
/// again. Without content (window closed abnormally) the main terminal starts
/// empty and continues with new output.
pub fn release(state: &Backend, label: &str, snapshot: Option<String>) {
    let released = lock(&state.sessions).ok().and_then(|mut sessions| {
        sessions
            .iter_mut()
            .find(|(_, s)| s.popout.window.as_deref() == Some(label))
            .map(|(id, s)| {
                s.popout.window = None;
                s.popout.outgoing = None;
                s.popout.sent = None;
                s.popout.returned = snapshot;
                // Output that arrived meanwhile belongs to the main window now.
                s.wake.notify_one();
                *id
            })
    });
    if let (Some(id), Some(app)) = (released, state.app.get()) {
        let _ = app.emit_to("main", "popout-returned", id);
    }
}

/// Closes the pop-out of a session that is being removed.
pub fn close_for(app: &tauri::AppHandle, label: Option<String>) {
    if let Some(label) = label
        && let Some(window) = app.get_webview_window(&label)
    {
        taskbar_id(&window, None);
        let _ = window.destroy();
    }
}

/// Closes every pop-out (the main window is closing).
pub fn close_all(app: &tauri::AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with(PREFIX) {
            taskbar_id(&window, None);
            let _ = window.destroy();
        }
    }
}

static FIXTURE_CHECKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Capture builds: the pop-out window's own checks, read by the main fixture.
static FIXTURE_REPORT: std::sync::Mutex<Option<serde_json::Value>> = std::sync::Mutex::new(None);

#[tauri::command]
pub fn capture_popout_report(
    window: WebviewWindow,
    state: State<Backend>,
    checks: Vec<String>,
    passed: bool,
) -> Result<(), String> {
    bound(&state, &window)?;
    if !cfg!(feature = "capture") || !lock(&state.config)?.fixture {
        return Err("Fixture only".into());
    }
    if checks.len() > 20 || checks.iter().any(|c| c.len() > 400) {
        return Err("Report too large".into());
    }
    *lock(&FIXTURE_REPORT)? = Some(serde_json::json!({ "checks": checks, "passed": passed }));
    Ok(())
}

#[tauri::command]
pub fn capture_popout_result(
    window: WebviewWindow,
    state: State<Backend>,
) -> Result<Option<serde_json::Value>, String> {
    main_only(&window)?;
    if !cfg!(feature = "capture") || !lock(&state.config)?.fixture {
        return Err("Fixture only".into());
    }
    Ok(lock(&FIXTURE_REPORT)?.take())
}

/// Capture builds: the taskbar IDs the open pop-out windows carry.
#[tauri::command]
pub fn capture_popout_taskbar_ids(
    window: WebviewWindow,
    state: State<Backend>,
) -> Result<Vec<String>, String> {
    main_only(&window)?;
    if !cfg!(feature = "capture") || !lock(&state.config)?.fixture {
        return Err("Fixture only".into());
    }
    let mut ids = Vec::new();
    #[cfg(windows)]
    for (label, popout) in window.app_handle().webview_windows() {
        use windows::Win32::{
            Foundation::HWND,
            Storage::EnhancedStorage::PKEY_AppUserModel_ID,
            UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow},
        };
        if !label.starts_with(PREFIX) {
            continue;
        }
        let hwnd = popout.hwnd().map_err(|e| e.to_string())?;
        let value = unsafe {
            SHGetPropertyStoreForWindow::<IPropertyStore>(HWND(hwnd.0 as _))
                .and_then(|store| store.GetValue(&PKEY_AppUserModel_ID))
        }
        .map_err(|e| e.to_string())?;
        ids.push(value.to_string());
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    #[test]
    fn snapshots_are_bounded() {
        assert!(super::valid_snapshot(&"x".repeat(super::SNAPSHOT_LIMIT)).is_ok());
        assert!(super::valid_snapshot(&"x".repeat(super::SNAPSHOT_LIMIT + 1)).is_err());
    }
}
