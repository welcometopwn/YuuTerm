//! Native file selection and explicit external-edit staging. No frontend disk API.
use super::{Backend, lock, main_only};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, atomic::Ordering},
};
use tauri::{State, WebviewWindow};
use tokio_util::sync::CancellationToken;
use yuuterm_ssh::files::{FileAction, FileClient, FileResult, validate_name};
pub(crate) mod recovery;

pub(super) struct Edit {
    origin: Origin,
    live_id: u64,
    remote: String,
    local: PathBuf,
    baseline: Option<String>,
    acknowledged: String,
    stamp: Stamp,
    candidate: Option<Stamp>,
    pending: Option<String>,
    remote_ack: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Origin {
    profile_id: u64,
    host: String,
    port: u16,
    username: String,
}
impl Origin {
    pub(super) fn from_profile(p: &yuuterm_core::Profile) -> Self {
        Self {
            profile_id: p.id,
            host: p.host.clone(),
            port: p.port,
            username: p.username.clone(),
        }
    }
}
fn recovery_origin(state: &Backend, id: u64) -> Result<Origin, String> {
    let sessions = lock(&state.sessions)?;
    let session = sessions.get(&id).ok_or("Connection no longer exists")?;
    let meta = session.live.snapshot();
    if !meta.connected || meta.finished || meta.protocol != yuuterm_core::Protocol::Ssh {
        return Err("Connect SSH before recovering edits".into());
    }
    Ok(session.file_origin.clone())
}
fn active_edit(state: &Backend, id: u64) -> Result<bool, String> {
    Ok(lock(&state.sessions)?.get(&id).is_some_and(|s| {
        let m = s.live.snapshot();
        m.connected && !m.finished
    }))
}

#[tauri::command]
pub(super) fn edit_recoverable(
    window: WebviewWindow,
    state: State<Backend>,
    id: u64,
) -> Result<Vec<EditView>, String> {
    main_only(&window)?;
    let origin = recovery_origin(&state, id)?;
    let snapshot = lock(&state.edits)?
        .iter()
        .filter(|(_, e)| e.origin == origin && e.live_id != id)
        .map(|(key, e)| EditView {
            id: *key,
            live_id: e.live_id,
            remote: e.remote.clone(),
            local: e.local.to_string_lossy().into(),
        })
        .collect::<Vec<_>>();
    snapshot
        .into_iter()
        .filter_map(|v| match active_edit(&state, v.live_id) {
            Ok(false) => Some(Ok(v)),
            Ok(true) => None,
            Err(e) => Some(Err(e)),
        })
        .collect()
}

#[tauri::command]
pub(super) async fn edit_resume(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
    edit_id: u64,
) -> Result<EditView, String> {
    main_only(&window)?;
    let _poll = state.edit_polling.lock().await;
    let origin = recovery_origin(&state, id)?;
    let (old, path, local, baseline) = {
        let edits = lock(&state.edits)?;
        let e = edits.get(&edit_id).ok_or("Local edit no longer exists")?;
        if e.origin != origin || e.live_id == id {
            return Err("This edit belongs to a different connection target".into());
        }
        (
            e.live_id,
            e.remote.clone(),
            e.local.clone(),
            e.baseline
                .clone()
                .ok_or("No verified baseline; local copy retained")?,
        )
    };
    if active_edit(&state, old)? {
        return Err("Original connection is still active".into());
    }
    if lock(&state.edits)?
        .iter()
        .any(|(key, e)| *key != edit_id && e.live_id == id && e.remote == path)
    {
        return Err(
            "This file is already open on the new connection; both local copies retained".into(),
        );
    }
    let (client, lease) = begin(&state, id)?;
    let verified = client
        .execute(
            FileAction::Verify { path: path.clone() },
            lease.cancel.clone(),
        )
        .await?;
    if verified.fingerprint.as_deref() != Some(&baseline) {
        return Err("Conflict: server file changed since the previous connection. Local copy retained; open the server version separately to compare.".into());
    }
    let observed = stamp(&local).await?;
    let revision = yuuterm_ssh::files::local_revision(&local)
        .await
        .map_err(|e| e.to_string())?;
    if stamp(&local).await? != observed {
        return Err("Local file changed during recovery; review and try again".into());
    }
    if lease.cancel.is_cancelled() || !active_edit(&state, id)? {
        return Err("Connection closed during recovery; local copy retained".into());
    }
    let mut edits = lock(&state.edits)?;
    let e = edits
        .get_mut(&edit_id)
        .ok_or("Local edit no longer exists")?;
    if e.live_id != old || e.baseline.as_deref() != Some(&baseline) {
        return Err("Edit changed during recovery; review again".into());
    }
    let acknowledged = baseline
        .rsplit(':')
        .next()
        .ok_or("Missing content baseline")?
        .to_owned();
    e.live_id = id;
    e.remote_ack = metadata_part(&baseline).to_owned();
    e.acknowledged = acknowledged;
    e.pending = (revision != e.acknowledged).then_some(revision);
    e.stamp = observed;
    e.candidate = None;
    Ok(EditView {
        id: edit_id,
        live_id: id,
        remote: path,
        local: local.to_string_lossy().into(),
    })
}

#[tauri::command]
pub(super) async fn edit_reopen(
    window: WebviewWindow,
    state: State<'_, Backend>,
    edit_id: u64,
) -> Result<(), String> {
    main_only(&window)?;
    let local = lock(&state.edits)?
        .get(&edit_id)
        .ok_or("Local edit no longer exists")?
        .local
        .clone();
    yuuterm_ssh::files::local_revision(&local)
        .await
        .map_err(|e| e.to_string())?;
    if cfg!(feature = "capture") && lock(&state.config)?.fixture {
        return Ok(());
    }
    let editor = [
        std::env::var_os("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("Programs/Microsoft VS Code/Code.exe")),
        std::env::var_os("ProgramFiles")
            .map(|p| PathBuf::from(p).join("Microsoft VS Code/Code.exe")),
    ]
    .into_iter()
    .flatten()
    .find(|p| p.is_file())
    .ok_or("VS Code was not found; copy the local path to open it in your editor")?;
    std::process::Command::new(editor)
        .arg(local)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}
#[derive(Clone, Serialize)]
pub(super) struct EditView {
    id: u64,
    live_id: u64,
    remote: String,
    local: String,
}
#[tauri::command]
pub(super) fn edit_list(
    window: WebviewWindow,
    state: State<Backend>,
    id: u64,
) -> Result<Vec<EditView>, String> {
    main_only(&window)?;
    Ok(lock(&state.edits)?
        .iter()
        .filter(|(_, e)| e.live_id == id)
        .map(|(key, e)| EditView {
            id: *key,
            live_id: e.live_id,
            remote: e.remote.clone(),
            local: e.local.to_string_lossy().into(),
        })
        .collect())
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(super) enum Action {
    Create {
        path: String,
        name: String,
    },
    Mkdir {
        path: String,
        name: String,
    },
    Properties {
        path: String,
    },
    Resolve {
        path: String,
    },
    Rename {
        path: String,
        name: String,
    },
    Delete {
        path: String,
    },
    Permissions {
        path: String,
        mode: u32,
        #[serde(default)]
        recursive: bool,
    },
}
struct Lease {
    busy: Arc<Mutex<Option<CancellationToken>>>,
    cancel: CancellationToken,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut busy) = self.busy.lock() {
            *busy = None;
        }
    }
}
fn begin(state: &Backend, id: u64) -> Result<(FileClient, Lease), String> {
    let sessions = lock(&state.sessions)?;
    let session = sessions
        .get(&id)
        .ok_or("Original connection no longer exists")?;
    if !session.live.snapshot().connected || session.live.snapshot().finished {
        return Err("Original connection is not active".into());
    }
    let mut busy = lock(&session.file_busy)?;
    if busy.is_some() {
        return Err("A file operation is already running for this connection".into());
    }
    let cancel = CancellationToken::new();
    *busy = Some(cancel.clone());
    Ok((
        session.live.files(),
        Lease {
            busy: session.file_busy.clone(),
            cancel,
        },
    ))
}
#[tauri::command]
pub(super) fn file_cancel(
    window: WebviewWindow,
    state: State<Backend>,
    id: u64,
) -> Result<(), String> {
    main_only(&window)?;
    let sessions = lock(&state.sessions)?;
    let session = sessions
        .get(&id)
        .ok_or("Original connection no longer exists")?;
    if let Some(cancel) = lock(&session.file_busy)?.as_ref() {
        cancel.cancel();
    }
    Ok(())
}
#[tauri::command]
pub(super) async fn file_action(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
    action: Action,
) -> Result<FileResult, String> {
    main_only(&window)?;
    let (client, lease) = begin(&state, id)?;
    let action = match action {
        Action::Mkdir { path, name } => {
            validate_name(&name).map_err(|e| e.to_string())?;
            if !path.starts_with('/') {
                return Err("Choose an absolute remote directory".into());
            }
            FileAction::Mkdir {
                path: format!("{}/{name}", path.trim_end_matches('/')),
            }
        }
        Action::Create { path, name } => {
            validate_name(&name).map_err(|e| e.to_string())?;
            if !path.starts_with('/') {
                return Err("Choose an absolute remote directory".into());
            }
            FileAction::Create {
                path: format!("{}/{name}", path.trim_end_matches('/')),
            }
        }
        Action::Resolve { path } => FileAction::Resolve { path },
        Action::Properties { path } => FileAction::Properties { path },
        Action::Rename { path, name } => FileAction::Rename { path, name },
        Action::Delete { path } => FileAction::Delete { path },
        Action::Permissions {
            path,
            mode,
            recursive,
        } => FileAction::Permissions {
            path,
            mode,
            recursive,
        },
    };
    client.execute(action, lease.cancel.clone()).await
}
#[derive(Serialize)]
pub(super) struct TransferResult {
    cancelled: bool,
    bytes: u64,
    local: Option<String>,
    remote: String,
}
#[derive(Serialize)]
pub(super) struct BatchResult {
    completed: usize,
    problem: Option<String>,
    cancelled: bool,
    /// Symlinks and special files left out of downloaded directories.
    skipped: u64,
}
#[tauri::command]
pub(super) async fn files_download(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
    paths: Vec<String>,
) -> Result<BatchResult, String> {
    main_only(&window)?;
    if paths.is_empty() || paths.len() > 100 {
        return Err("Select 1–100 files or directories".into());
    }
    let mut names = std::collections::HashSet::new();
    for path in &paths {
        if !path.starts_with('/') || path.len() > 4096 || path.contains('\0') {
            return Err("Invalid absolute remote path".into());
        }
        if !names.insert(safe_name(path).to_lowercase()) {
            return Err("Selected filenames collide on Windows; download them separately".into());
        }
    }
    let (client, lease) = begin(&state, id)?;
    let fixture = cfg!(feature = "capture") && lock(&state.config)?.fixture;
    let folder = if fixture {
        Some(
            PathBuf::from(
                std::env::var("YUUTERM_CAPTURE_PROFILE").map_err(|_| "Fixture path missing")?,
            )
            .with_file_name("downloads"),
        )
    } else {
        rfd::AsyncFileDialog::new()
            .set_parent(&window)
            .set_title("Choose a folder — downloads use a new YuuTerm subfolder")
            .pick_folder()
            .await
            .map(|f| f.path().to_owned())
    };
    let Some(folder) = folder else {
        return Ok(BatchResult {
            completed: 0,
            problem: None,
            cancelled: true,
            skipped: 0,
        });
    };
    if lease.cancel.is_cancelled() {
        return Err("Download cancelled".into());
    }
    if fixture {
        tokio::fs::create_dir_all(&folder)
            .await
            .map_err(|e| e.to_string())?;
    }
    // A new private batch directory prevents overwrites and destination name races.
    let directory = tempfile::Builder::new()
        .prefix("YuuTerm-download-")
        .tempdir_in(&folder)
        .map_err(|e| e.to_string())?
        .keep();
    let mut result = BatchResult {
        completed: 0,
        problem: None,
        cancelled: false,
        skipped: 0,
    };
    for path in paths {
        let local = directory.join(safe_name(&path));
        match client
            .execute(
                FileAction::DownloadTree { path, local },
                lease.cancel.clone(),
            )
            .await
        {
            Ok(done) => result.skipped += done.skipped,
            Err(e) => {
                result.problem = Some(format!(
                    "Stopped: {e}. Completed files remain in the chosen folder's YuuTerm subfolder."
                ));
                break;
            }
        }
        result.completed += 1;
    }
    Ok(result)
}
#[tauri::command]
pub(super) async fn file_transfer(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
    path: String,
    upload: bool,
) -> Result<TransferResult, String> {
    main_only(&window)?;
    let (client, lease) = begin(&state, id)?;
    let dialog = rfd::AsyncFileDialog::new().set_parent(&window);
    let picked = if upload {
        dialog
            .set_title("Upload a file — existing remote names are never overwritten")
            .pick_file()
            .await
    } else {
        dialog
            .set_title("Download remote file")
            .set_file_name(safe_name(&path))
            .save_file()
            .await
    };
    let Some(picked) = picked else {
        return Ok(TransferResult {
            cancelled: true,
            bytes: 0,
            local: None,
            remote: path,
        });
    };
    let local = picked.path().to_owned();
    let remote = if upload {
        let name = local
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("Filename is not Unicode")?;
        validate_name(name).map_err(|e| e.to_string())?;
        format!("{}/{}", path.trim_end_matches('/'), name)
    } else {
        path
    };
    let action = if upload {
        FileAction::Upload {
            path: remote.clone(),
            local: local.clone(),
        }
    } else {
        FileAction::Download {
            path: remote.clone(),
            local: local.clone(),
            edit: false,
        }
    };
    let result = client.execute(action, lease.cancel.clone()).await?;
    Ok(TransferResult {
        cancelled: false,
        bytes: result.bytes,
        local: Some(local.to_string_lossy().into()),
        remote,
    })
}

/// Local files dropped on the window. Paths come only from the native drop
/// event; the WebView receives a short-lived token and the drop position, never
/// a way to name arbitrary local paths.
pub(super) struct Dropped {
    token: u64,
    at: std::time::Instant,
    paths: Vec<PathBuf>,
}
pub(super) static DROPPED: Mutex<Option<Dropped>> = Mutex::new(None);
static DROP_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Records a native file drop and returns its token (at most 64 paths).
pub(super) fn record_drop(paths: Vec<PathBuf>) -> Option<u64> {
    if paths.is_empty() || paths.len() > 64 {
        return None;
    }
    let token = DROP_TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    *DROPPED.lock().ok()? = Some(Dropped {
        token,
        at: std::time::Instant::now(),
        paths,
    });
    Some(token)
}

#[derive(Serialize)]
pub(super) struct DropResult {
    uploaded: usize,
    bytes: u64,
    skipped: Vec<String>,
}

/// Uploads the files of one native drop into the remote directory `path`.
/// The token is single-use and expires after 30 s. Directories and names the
/// remote side cannot take are skipped and reported; existing remote names
/// are never overwritten (same rule as the upload picker).
#[tauri::command]
pub(super) async fn file_upload_dropped(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
    path: String,
    token: u64,
) -> Result<DropResult, String> {
    main_only(&window)?;
    let paths = {
        let mut dropped = lock(&DROPPED)?;
        match dropped.take() {
            Some(d) if d.token == token && d.at.elapsed() < std::time::Duration::from_secs(30) => {
                d.paths
            }
            other => {
                *dropped = other;
                return Err("This drop expired; drop the files again".into());
            }
        }
    };
    let (client, lease) = begin(&state, id)?;
    let mut result = DropResult {
        uploaded: 0,
        bytes: 0,
        skipped: Vec::new(),
    };
    for local in paths {
        let name = local
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        if !local.is_file() || name.is_empty() || validate_name(&name).is_err() {
            result.skipped.push(if name.is_empty() {
                local.to_string_lossy().into()
            } else {
                name
            });
            continue;
        }
        let remote = format!("{}/{}", path.trim_end_matches('/'), name);
        match client
            .execute(
                FileAction::Upload {
                    path: remote,
                    local: local.clone(),
                },
                lease.cancel.clone(),
            )
            .await
        {
            Ok(done) => {
                result.uploaded += 1;
                result.bytes += done.bytes;
            }
            Err(_) => result.skipped.push(name),
        }
    }
    Ok(result)
}

fn safe_name(path: &str) -> String {
    let name = yuuterm_ssh::files::windows_name(path.rsplit('/').next().unwrap_or("download"));
    // Keep the staged bytes distinct from their recovery sidecar, including
    // case-folded Windows aliases (for example the Unicode long s).
    if name.to_uppercase() == "RECOVERY.JSON" {
        "edited-recovery.json".into()
    } else {
        name
    }
}
#[tauri::command]
pub(super) async fn edit_open(
    window: WebviewWindow,
    state: State<'_, Backend>,
    id: u64,
    path: String,
    choose: bool,
) -> Result<Option<EditView>, String> {
    main_only(&window)?;
    let (client, lease) = begin(&state, id)?;
    let target = client
        .execute(FileAction::Resolve { path }, lease.cancel.clone())
        .await?
        .resolved
        .ok_or("No file target returned")?;
    if target.directory {
        return Err("Choose a file, not a directory".into());
    }
    let path = target.path;
    let capture_only = cfg!(feature = "capture") && lock(&state.config)?.fixture;
    let editor = if capture_only || choose {
        Some(PathBuf::new())
    } else {
        let mut candidates = Vec::new();
        if let Some(p) = std::env::var_os("LOCALAPPDATA") {
            candidates.push(PathBuf::from(p).join("Programs/Microsoft VS Code/Code.exe"));
        }
        if let Some(p) = std::env::var_os("ProgramFiles") {
            candidates.push(PathBuf::from(p).join("Microsoft VS Code/Code.exe"));
        }
        let candidate = candidates.into_iter().find(|p| p.is_file());
        match candidate {
            Some(p) => Some(p),
            None => rfd::AsyncFileDialog::new()
                .set_parent(&window)
                .set_title("Locate your text editor executable")
                .add_filter("Applications", &["exe"])
                .pick_file()
                .await
                .map(|f| f.path().to_owned()),
        }
    };
    let Some(editor) = editor else {
        return Ok(None);
    };
    if lease.cancel.is_cancelled() {
        return Err("Opening the editor was cancelled".into());
    }
    // Reopening a staged edit must never overwrite an unsaved local version.
    let existing = {
        let edits = lock(&state.edits)?;
        edits
            .iter()
            .find(|(_, e)| e.live_id == id && e.remote == path)
            .map(|(key, e)| (*key, e.local.clone()))
    };
    let (key, local) = if let Some(existing) = existing {
        existing
    } else {
        let root = if capture_only {
            PathBuf::from(
                std::env::var("YUUTERM_CAPTURE_PROFILE").map_err(|_| "Fixture profile missing")?,
            )
            .with_file_name("staging")
        } else {
            yuuterm_core::config::path()
                .map_err(|e| e.to_string())?
                .with_file_name("staging")
        };
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(|e| e.to_string())?;
        let directory = tempfile::Builder::new()
            .prefix("edit-")
            .tempdir_in(root)
            .map_err(|e| e.to_string())?
            .keep();
        let local = directory.join(safe_name(&path));
        let result = client
            .execute(
                FileAction::Download {
                    path: path.clone(),
                    local: local.clone(),
                    edit: true,
                },
                lease.cancel.clone(),
            )
            .await?;
        let key = state.next_edit.fetch_add(1, Ordering::Relaxed);
        if lock(&state.edits)?.len() >= 32 {
            return Err("At most 32 local edits can be watched; staged copy retained".into());
        }
        let acknowledged = yuuterm_ssh::files::local_revision(&local)
            .await
            .map_err(|e| e.to_string())?;
        let stamp = stamp(&local).await?;
        let edit = Edit {
            origin: recovery_origin(&state, id)?,
            remote_ack: metadata_part(result.fingerprint.as_deref().unwrap_or_default()).to_owned(),
            acknowledged,
            stamp,
            candidate: None,
            pending: None,
            live_id: id,
            remote: path.clone(),
            local: local.clone(),
            baseline: result.fingerprint,
        };
        recovery::persist(&edit).await?;
        {
            let mut edits = lock(&state.edits)?;
            if edits.len() >= 32 {
                return Err("At most 32 local edits can be watched; local copy retained".into());
            }
            edits.insert(key, edit);
        }
        (key, local)
    };
    if lease.cancel.is_cancelled() {
        return Err("Editor launch cancelled; staged file retained".into());
    }
    if !capture_only {
        if choose {
            super::open_with::show(&window, local.clone()).await?;
        } else {
            std::process::Command::new(&editor)
                .arg(&local)
                .spawn()
                .map_err(|e| {
                    format!(
                        "Editor failed to start: {e}; local copy retained at {}",
                        local.display()
                    )
                })?;
        }
    }
    Ok(Some(EditView {
        id: key,
        live_id: id,
        remote: path,
        local: local.to_string_lossy().into(),
    }))
}
#[tauri::command]
pub(super) async fn edit_upload(
    window: WebviewWindow,
    state: State<'_, Backend>,
    edit_id: u64,
    revision: Option<String>,
) -> Result<FileResult, String> {
    main_only(&window)?;
    if revision
        .as_ref()
        .is_some_and(|r| r.len() != 64 || !r.as_bytes().iter().all(u8::is_ascii_hexdigit))
    {
        return Err("Invalid saved revision".into());
    }
    let (id, path, local, expected) = {
        let edits = lock(&state.edits)?;
        let edit = edits.get(&edit_id).ok_or("Staged edit no longer exists")?;
        (
            edit.live_id,
            edit.remote.clone(),
            edit.local.clone(),
            edit.baseline
                .clone()
                .ok_or("No verified edit baseline is available; download/review again")?,
        )
    };
    let (client, lease) = begin(&state, id)?;
    let result = client
        .execute(
            FileAction::Replace {
                path,
                local,
                expected,
                expected_local: revision,
            },
            lease.cancel.clone(),
        )
        .await?;
    if let Some(edit) = lock(&state.edits)?.get_mut(&edit_id) {
        edit.baseline = result.fingerprint.clone();
        edit.remote_ack =
            metadata_part(result.fingerprint.as_deref().unwrap_or_default()).to_owned();
        if let Some(hash) = result
            .fingerprint
            .as_ref()
            .and_then(|v| v.rsplit(':').next())
        {
            edit.acknowledged = hash.to_owned();
            if edit.pending.as_deref() == Some(hash) {
                edit.pending = None;
            }
        }
    }
    recovery::persist_id(&state, edit_id).await?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_origin_requires_same_bookmark_host_port_and_user() {
        let p = yuuterm_core::Profile {
            id: 1,
            host: "server".into(),
            username: "alice".into(),
            ..Default::default()
        };
        let origin = Origin::from_profile(&p);
        let mut different = p.clone();
        different.id = 2;
        assert!(Origin::from_profile(&different) != origin);
        let mut other = p.clone();
        other.host = "other".into();
        assert!(Origin::from_profile(&other) != origin);
        let mut other = p.clone();
        other.port = 2222;
        assert!(Origin::from_profile(&other) != origin);
        let mut other = p.clone();
        other.username = "bob".into();
        assert!(Origin::from_profile(&other) != origin);
        let mut renamed = p;
        renamed.name = "Renamed".into();
        assert!(Origin::from_profile(&renamed) == origin);
    }
    #[test]
    fn remote_names_cannot_escape_windows_staging() {
        for name in [
            "/tmp/recovery.json",
            "/tmp/RECOVERY.JSON",
            "/tmp/recovery.jſon",
        ] {
            assert_eq!(safe_name(name), "edited-recovery.json");
        }
        for name in ["../../CON", "/tmp/a\\b:bad?.txt", "/tmp/..", "/tmp/NUL.txt"] {
            let clean = safe_name(name);
            assert!(!clean.contains(['/', '\\', ':', '?']));
            assert!(!clean.ends_with('.'));
            assert!(!clean.is_empty());
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: std::time::SystemTime,
}
fn metadata_part(fingerprint: &str) -> &str {
    fingerprint
        .rsplit_once(':')
        .map_or(fingerprint, |(metadata, _)| metadata)
}

#[derive(Serialize)]
pub(super) struct RemoteChange {
    edit: EditView,
    revision: String,
    local_revision: String,
}

#[tauri::command]
pub(super) async fn edit_remote_changes(
    window: WebviewWindow,
    state: State<'_, Backend>,
) -> Result<Vec<RemoteChange>, String> {
    main_only(&window)?;
    let _poll = state
        .edit_polling
        .try_lock()
        .map_err(|_| "Edit scan already running")?;
    let snapshot = lock(&state.edits)?
        .iter()
        .filter(|(_, e)| e.candidate.is_none() && e.pending.is_none())
        .map(|(key, e)| {
            (
                *key,
                e.live_id,
                e.remote.clone(),
                e.local.clone(),
                e.remote_ack.clone(),
            )
        })
        .collect::<Vec<_>>();
    let mut changes = Vec::new();
    let mut remaining = yuuterm_ssh::files::MAX_EDIT;
    for (key, id, path, local, ack) in snapshot {
        // Foreground transfers take priority; closed/busy sessions are retried later.
        let Ok((client, lease)) = begin(&state, id) else {
            continue;
        };
        let result = client
            .execute(
                FileAction::Revision { path: path.clone() },
                lease.cancel.clone(),
            )
            .await;
        let Ok(result) = result else { continue };
        let Some(revision) = result.fingerprint.filter(|v| *v != ack) else {
            continue;
        };
        let Ok(stamp) = stamp(&local).await else {
            continue;
        };
        if stamp.len > remaining {
            continue;
        }
        remaining -= stamp.len;
        let Ok(local_revision) = yuuterm_ssh::files::local_revision(&local).await else {
            continue;
        };
        changes.push(RemoteChange {
            edit: EditView {
                id: key,
                live_id: id,
                remote: path,
                local: local.to_string_lossy().into(),
            },
            revision,
            local_revision,
        });
    }
    Ok(changes)
}

#[tauri::command]
pub(super) async fn edit_remote_decide(
    window: WebviewWindow,
    state: State<'_, Backend>,
    edit_id: u64,
    revision: String,
    local_revision: String,
    reload: bool,
) -> Result<(), String> {
    main_only(&window)?;
    let _poll = state.edit_polling.lock().await;
    if revision.len() > 256
        || local_revision.len() != 64
        || !local_revision.as_bytes().iter().all(u8::is_ascii_hexdigit)
    {
        return Err("Invalid file revision".into());
    }
    let (id, path, local) = {
        let edits = lock(&state.edits)?;
        let e = edits.get(&edit_id).ok_or("Local edit no longer exists")?;
        (e.live_id, e.remote.clone(), e.local.clone())
    };
    let (client, lease) = begin(&state, id)?;
    if !reload {
        if let Some(e) = lock(&state.edits)?.get_mut(&edit_id) {
            e.remote_ack = revision;
        }
        return Ok(());
    }
    let check = client
        .execute(
            FileAction::Revision { path: path.clone() },
            lease.cancel.clone(),
        )
        .await?;
    if check.fingerprint.as_deref() != Some(&revision) {
        return Err("Server file changed again; dismiss and review the latest version".into());
    }
    if yuuterm_ssh::files::local_revision(&local)
        .await
        .map_err(|e| e.to_string())?
        != local_revision
    {
        return Err("Local file changed after this prompt. Both versions retained; dismiss and review again.".into());
    }
    let dir = tempfile::tempdir_in(local.parent().ok_or("No staging directory")?)
        .map_err(|e| e.to_string())?;
    let staged = dir.path().join("server-version");
    let result = client
        .execute(
            FileAction::Download {
                path: path.clone(),
                local: staged.clone(),
                edit: true,
            },
            lease.cancel.clone(),
        )
        .await?;
    let after = client
        .execute(FileAction::Revision { path }, lease.cancel.clone())
        .await?;
    if after.fingerprint.as_deref() != Some(&revision)
        || metadata_part(result.fingerprint.as_deref().unwrap_or_default()) != revision
    {
        return Err("Server file changed during download; local file retained".into());
    }
    // Preserve the existing disk version before publication. Unsaved editor buffers
    // are unknown to this external-editor integration and require user review.
    let backup = tempfile::Builder::new()
        .prefix(".local-backup-")
        .tempfile_in(local.parent().ok_or("No staging directory")?)
        .map_err(|e| e.to_string())?;
    let (file, backup) = backup.keep().map_err(|e| e.to_string())?;
    let mut dest = tokio::fs::File::from_std(file);
    let mut source = tokio::fs::File::open(&local)
        .await
        .map_err(|e| e.to_string())?;
    let copied = tokio::io::copy(
        &mut tokio::io::AsyncReadExt::take(&mut source, yuuterm_ssh::files::MAX_EDIT + 1),
        &mut dest,
    )
    .await
    .map_err(|e| e.to_string())?;
    dest.sync_all().await.map_err(|e| e.to_string())?;
    drop(dest);
    drop(source);
    if copied > yuuterm_ssh::files::MAX_EDIT
        || yuuterm_ssh::files::local_revision(&backup)
            .await
            .map_err(|e| e.to_string())?
            != local_revision
        || yuuterm_ssh::files::local_revision(&local)
            .await
            .map_err(|e| e.to_string())?
            != local_revision
    {
        return Err("Local file changed during reload; recovery copy retained".into());
    }
    if lease.cancel.is_cancelled() {
        return Err("Reload cancelled; local copy retained".into());
    }
    tokio::fs::rename(&staged, &local)
        .await
        .map_err(|e| e.to_string())?;
    let hash = result
        .fingerprint
        .as_deref()
        .and_then(|f| f.rsplit(':').next())
        .ok_or("Missing server revision")?
        .to_owned();
    let stamp = stamp(&local).await?;
    if let Some(e) = lock(&state.edits)?.get_mut(&edit_id) {
        e.baseline = result.fingerprint;
        e.remote_ack = revision;
        e.acknowledged = hash;
        e.stamp = stamp;
        e.candidate = None;
        e.pending = None;
    }
    recovery::persist_id(&state, edit_id).await?;
    Ok(())
}
async fn stamp(path: &std::path::Path) -> Result<Stamp, String> {
    let m = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| e.to_string())?;
    if !m.is_file() || m.len() > yuuterm_ssh::files::MAX_EDIT {
        return Err("Local edit exceeds the regular-file/64 MiB limit".into());
    }
    Ok(Stamp {
        len: m.len(),
        modified: m.modified().map_err(|e| e.to_string())?,
    })
}
#[derive(Clone, Serialize)]
pub(super) struct EditChange {
    edit: EditView,
    revision: String,
}

#[tauri::command]
pub(super) async fn edit_changes(
    window: WebviewWindow,
    state: State<'_, Backend>,
) -> Result<Vec<EditChange>, String> {
    main_only(&window)?;
    let _poll = state
        .edit_polling
        .try_lock()
        .map_err(|_| "Edit scan already running")?;
    let active = lock(&state.sessions)?
        .iter()
        .filter_map(|(id, s)| {
            let m = s.live.snapshot();
            (m.connected && !m.finished).then_some(*id)
        })
        .collect::<std::collections::BTreeSet<_>>();
    let snapshot = lock(&state.edits)?
        .iter()
        .filter(|(_, e)| active.contains(&e.live_id))
        .map(|(id, e)| (*id, e.local.clone(), e.stamp.clone(), e.candidate.clone()))
        .collect::<Vec<_>>();
    let mut remaining = yuuterm_ssh::files::MAX_EDIT;
    for (id, local, previous, candidate) in snapshot {
        let Ok(current) = stamp(&local).await else {
            continue;
        };
        if current == previous {
            continue;
        }
        if candidate.as_ref() != Some(&current) {
            if let Some(e) = lock(&state.edits)?.get_mut(&id) {
                e.candidate = Some(current);
            }
            continue;
        }
        if current.len > remaining {
            continue;
        }
        remaining -= current.len;
        let Ok(hash) = yuuterm_ssh::files::local_revision(&local).await else {
            continue;
        };
        if stamp(&local).await? != current {
            continue;
        }
        if let Some(e) = lock(&state.edits)?.get_mut(&id) {
            if e.candidate.as_ref() != Some(&current) {
                continue;
            }
            e.stamp = current;
            e.candidate = None;
            e.pending = (hash != e.acknowledged).then_some(hash);
        }
    }
    Ok(lock(&state.edits)?
        .iter()
        .filter_map(|(id, e)| {
            if !active.contains(&e.live_id) {
                return None;
            }
            e.pending.as_ref().map(|revision| EditChange {
                revision: revision.clone(),
                edit: EditView {
                    id: *id,
                    live_id: e.live_id,
                    remote: e.remote.clone(),
                    local: e.local.to_string_lossy().into(),
                },
            })
        })
        .collect())
}

#[tauri::command]
pub(super) fn edit_decline(
    window: WebviewWindow,
    state: State<Backend>,
    edit_id: u64,
    revision: String,
) -> Result<(), String> {
    main_only(&window)?;
    let mut edits = lock(&state.edits)?;
    let edit = edits
        .get_mut(&edit_id)
        .ok_or("Local edit no longer exists")?;
    if edit.pending.as_ref() != Some(&revision) {
        return Err("Saved version changed; waiting for the latest revision".into());
    }
    edit.acknowledged = revision;
    edit.pending = None;
    Ok(())
}

#[tauri::command]
pub(super) async fn capture_edit_probe(
    window: WebviewWindow,
    state: State<'_, Backend>,
    edit_id: u64,
    operation: String,
    text: String,
) -> Result<String, String> {
    main_only(&window)?;
    if !cfg!(feature = "capture") || !lock(&state.config)?.fixture || text.len() > 65536 {
        return Err("Fixture only".into());
    }
    let (id, path, local) = {
        let e = lock(&state.edits)?;
        let e = e.get(&edit_id).ok_or("No fixture edit")?;
        (e.live_id, e.remote.clone(), e.local.clone())
    };
    if operation == "save" {
        tokio::fs::write(local, text)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(String::new());
    }
    if operation == "atomic-save" {
        let temp =
            tempfile::tempdir_in(local.parent().ok_or("No parent")?).map_err(|e| e.to_string())?;
        let replacement = temp.path().join("replacement");
        tokio::fs::write(&replacement, text)
            .await
            .map_err(|e| e.to_string())?;
        tokio::fs::rename(replacement, local)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(String::new());
    }
    if operation == "local-read" {
        return tokio::fs::read_to_string(local)
            .await
            .map_err(|e| e.to_string());
    }
    if operation == "backups" {
        let mut entries = tokio::fs::read_dir(local.parent().ok_or("No parent")?)
            .await
            .map_err(|e| e.to_string())?;
        let mut values = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(".local-backup-")
            {
                values.push(
                    tokio::fs::read_to_string(entry.path())
                        .await
                        .map_err(|e| e.to_string())?,
                );
            }
        }
        return serde_json::to_string(&values).map_err(|e| e.to_string());
    }
    let (client, lease) = begin(&state, id)?;
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let file = dir.path().join("probe.txt");
    if operation == "remote-change" {
        let baseline = client
            .execute(
                FileAction::Download {
                    path: path.clone(),
                    local: file.clone(),
                    edit: true,
                },
                lease.cancel.clone(),
            )
            .await?
            .fingerprint
            .ok_or("No remote baseline")?;
        tokio::fs::write(&file, text)
            .await
            .map_err(|e| e.to_string())?;
        client
            .execute(
                FileAction::Replace {
                    path,
                    local: file,
                    expected: baseline,
                    expected_local: None,
                },
                lease.cancel.clone(),
            )
            .await?;
        return Ok(String::new());
    }
    if operation != "read" {
        return Err("Unknown fixture operation".into());
    }
    client
        .execute(
            FileAction::Download {
                path,
                local: file.clone(),
                edit: true,
            },
            lease.cancel.clone(),
        )
        .await?;
    let content = tokio::fs::read(file).await.map_err(|e| e.to_string())?;
    if content.len() > 65536 {
        return Err("Fixture probe exceeded limit".into());
    }
    String::from_utf8(content).map_err(|e| e.to_string())
}

#[tauri::command]
pub(super) async fn capture_editor_bridge(
    window: WebviewWindow,
    state: State<'_, Backend>,
    operation: String,
    edit_id: Option<u64>,
) -> Result<String, String> {
    main_only(&window)?;
    if !cfg!(feature = "capture") || !lock(&state.config)?.fixture {
        return Err("Fixture only".into());
    }
    let root = PathBuf::from(
        std::env::var("YUUTERM_CAPTURE_PROFILE").map_err(|_| "Fixture profile missing")?,
    )
    .parent()
    .ok_or("No fixture parent")?
    .to_owned();
    if operation == "request" {
        let local = lock(&state.edits)?
            .get(&edit_id.ok_or("No edit ID")?)
            .ok_or("No fixture edit")?
            .local
            .clone();
        tokio::fs::write(
            root.join("editor-request.json"),
            serde_json::to_vec(&serde_json::json!({"local":local})).map_err(|e| e.to_string())?,
        )
        .await
        .map_err(|e| e.to_string())?;
        return Ok("requested".into());
    }
    if operation == "state" {
        let path = root.join("editor-state.json");
        let Ok(meta) = tokio::fs::symlink_metadata(&path).await else {
            return Ok("{}".into());
        };
        if !meta.is_file() || meta.len() > 16384 {
            return Err("Invalid editor test response".into());
        }
        return tokio::fs::read_to_string(path)
            .await
            .map_err(|e| e.to_string());
    }
    if !matches!(operation.as_str(), "uploaded" | "finish") {
        return Err("Unknown editor test command".into());
    }
    tokio::fs::write(
        root.join("editor-control.json"),
        serde_json::to_vec(&serde_json::json!({"command":operation})).map_err(|e| e.to_string())?,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(String::new())
}

#[tauri::command]
pub(super) fn edit_stop(
    window: WebviewWindow,
    state: State<Backend>,
    edit_id: u64,
) -> Result<(), String> {
    main_only(&window)?;
    lock(&state.edits)?
        .remove(&edit_id)
        .ok_or("Local edit no longer exists")?;
    // Stop monitoring only; recovery files and local edits are deliberately retained.
    Ok(())
}
