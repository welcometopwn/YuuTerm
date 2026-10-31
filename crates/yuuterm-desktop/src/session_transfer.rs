use crate::{Backend, lock, main_only};
use serde::Serialize;
use std::{
    io::{Read, Write},
    path::PathBuf,
};
use tauri::{Manager, State, WebviewWindow};
use yuuterm_core::{
    Profile,
    config::{self, Preferences},
    session_transfer as transfer,
};

pub struct Draft {
    token: u64,
    profiles: Vec<Profile>,
}
#[derive(Serialize)]
pub struct Preview {
    token: u64,
    profiles: Vec<Profile>,
    format: &'static str,
    skipped: Vec<yuuterm_core::mobaxterm::Skipped>,
    duplicates: usize,
}
fn fixture_file(fixture: bool, importing: bool) -> Result<Option<PathBuf>, String> {
    if cfg!(feature = "capture") && fixture {
        let base = std::env::var("YUUTERM_CAPTURE_PROFILE").map_err(|_| "Fixture path missing")?;
        return Ok(Some(PathBuf::from(base).with_file_name(
            if importing && std::env::var_os("YUUTERM_CAPTURE_MOBA_IMPORT").is_some() {
                "moba-import.ini"
            } else {
                "session-export.json"
            },
        )));
    }
    Ok(None)
}

#[tauri::command]
pub async fn profiles_export(
    window: WebviewWindow,
    state: State<'_, Backend>,
) -> Result<bool, String> {
    main_only(&window)?;
    let (prefs, fixture) = {
        let c = lock(&state.config)?;
        (c.prefs.clone(), c.fixture)
    };
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = transfer::export(&prefs)?;
        let path = fixture_file(fixture, false)?.or_else(|| {
            rfd::FileDialog::new()
                .set_title("Export YuuTerm sessions")
                .add_filter("YuuTerm sessions", &["json"])
                .set_file_name("YuuTerm-sessions.json")
                .save_file()
        });
        let Some(path) = path else { return Ok(false) };
        let parent = path.parent().ok_or("Choose an absolute export filename")?;
        let mut temp =
            tempfile::NamedTempFile::new_in(parent).map_err(|_| "Cannot create export file")?;
        temp.write_all(&bytes)
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|_| "Cannot write export file")?;
        temp.persist(&path)
            .map_err(|_| "Cannot replace export destination; previous file preserved")?;
        Ok(true)
    })
    .await
    .map_err(|_| "Export worker stopped")?
}

#[tauri::command]
pub async fn profiles_import_preview(
    window: WebviewWindow,
    state: State<'_, Backend>,
) -> Result<Option<Preview>, String> {
    main_only(&window)?;
    let fixture = {
        let c = lock(&state.config)?;
        if !c.can_save && !(cfg!(feature = "capture") && c.fixture) {
            return Err("Settings saving is disabled".into());
        }
        c.fixture
    };
    // Invalidate an earlier review even if this picker is cancelled or invalid.
    *lock(&state.pending_import)? = None;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let path = fixture_file(fixture, true)?.or_else(|| {
            rfd::FileDialog::new()
                .set_title("Import sessions")
                .add_filter(
                    "YuuTerm or MobaXterm sessions",
                    &["json", "mxtsessions", "ini"],
                )
                .pick_file()
        });
        let Some(path) = path else { return Ok(None) };
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| "Cannot open import file")?
            .take(transfer::MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read import file")?;
        let imported = transfer::parse_import_with_icons(&bytes, crate::moba_icons::installed())?;
        let profiles = imported.profiles;
        let state = app.state::<Backend>();
        let c = lock(&state.config)?;
        let preview = if profiles.is_empty() {
            Vec::new()
        } else {
            transfer::merge(&c.prefs, &profiles)?.profiles[c.prefs.profiles.len()..].to_vec()
        };
        let token = state
            .next_edit
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let duplicates = profiles.len() - preview.len();
        *lock(&state.pending_import)? = Some(Draft { token, profiles });
        Ok(Some(Preview {
            token,
            profiles: preview,
            format: imported.format,
            skipped: imported.skipped,
            duplicates,
        }))
    })
    .await
    .map_err(|_| "Import worker stopped")?
}

#[tauri::command]
pub fn profiles_import_cancel(
    window: WebviewWindow,
    state: State<Backend>,
    token: u64,
) -> Result<(), String> {
    main_only(&window)?;
    let mut pending = lock(&state.pending_import)?;
    if pending.as_ref().is_some_and(|p| p.token == token) {
        *pending = None;
    }
    Ok(())
}

#[tauri::command]
pub async fn profiles_import_apply(
    window: WebviewWindow,
    token: u64,
) -> Result<Preferences, String> {
    main_only(&window)?;
    let app = window.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<Backend>();
        let mut c = lock(&state.config)?;
        let capture_only = cfg!(feature = "capture") && c.fixture;
        if !c.can_save && !capture_only {
            return Err("Settings saving is disabled".into());
        }
        let mut pending = lock(&state.pending_import)?;
        let draft = pending
            .as_ref()
            .filter(|p| p.token == token)
            .ok_or("Import preview expired; choose the file again")?;
        let candidate = transfer::merge(&c.prefs, &draft.profiles)?;
        if !capture_only {
            config::save(&candidate)?;
        }
        c.prefs = candidate.clone();
        *pending = None;
        Ok(candidate)
    })
    .await
    .map_err(|_| "Import worker stopped")?
}
