use crate::{Backend, lock, main_only};
use std::io::Read;
use tauri::{State, WebviewWindow};
use yuuterm_core::highlight_import::{self as parser, Preview};

#[tauri::command]
pub async fn highlight_import_preview(
    window: WebviewWindow,
    state: State<'_, Backend>,
) -> Result<Option<Preview>, String> {
    main_only(&window)?;
    let fixture = lock(&state.config)?.fixture;
    tauri::async_runtime::spawn_blocking(move || {
        let path = if cfg!(feature = "capture") && fixture {
            Some(
                std::path::PathBuf::from(
                    std::env::var("YUUTERM_CAPTURE_PROFILE").map_err(|_| "Fixture path missing")?,
                )
                .with_file_name("highlight-synthetic.msyn"),
            )
        } else {
            rfd::FileDialog::new()
                .set_title("Import MobaXterm syntax export")
                .add_filter("MobaXterm syntax", &["msyn", "ini"])
                .pick_file()
        };
        let Some(path) = path else { return Ok(None) };
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| "Cannot open syntax file")?
            .take(parser::MAX_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read syntax file")?;
        parser::parse(&bytes).map(Some)
    })
    .await
    .map_err(|_| "Syntax import worker stopped")?
}
