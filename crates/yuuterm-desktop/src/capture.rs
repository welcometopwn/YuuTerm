//! Native WebView2 capture, available only in isolated fixture builds.
use tauri::{Manager, WebviewWindow};
use webview2_com::{
    CapturePreviewCompletedHandler,
    Microsoft::Web::WebView2::Win32::COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
};
use windows::{
    Win32::{
        Storage::FileSystem::FILE_ATTRIBUTE_NORMAL,
        System::Com::{IStream, STGC_DEFAULT, STGM_CREATE, STGM_WRITE},
        UI::Shell::SHCreateStreamOnFileEx,
    },
    core::HSTRING,
};

pub fn save(window: WebviewWindow) -> Result<(), String> {
    save_then(window, true)
}

/// Saves the WebView image; `close` ends the fixture run once it is written.
pub fn save_then(window: WebviewWindow, close: bool) -> Result<(), String> {
    let path = std::env::var("YUUTERM_CAPTURE").map_err(|_| "Capture output path missing")?;
    save_to(window, path, close)
}

/// Saves the WebView image to `path`.
pub fn save_to(window: WebviewWindow, path: String, close: bool) -> Result<(), String> {
    let app = window.app_handle().clone();
    window
        .with_webview(move |webview| {
            let result = (|| -> windows::core::Result<()> {
                unsafe {
                    let stream = SHCreateStreamOnFileEx(
                        &HSTRING::from(path),
                        (STGM_CREATE | STGM_WRITE).0,
                        FILE_ATTRIBUTE_NORMAL.0,
                        true,
                        None::<&IStream>,
                    )?;
                    let saved = stream.clone();
                    let completion_app = app.clone();
                    let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
                        let result = result.and_then(|_| saved.Commit(STGC_DEFAULT));
                        if result.is_ok() && !close {
                            // Keep running: the fixture continues after the still.
                        } else if result.is_ok() {
                            if let Some(window) = completion_app.get_webview_window("main") {
                                if window.close().is_err() {
                                    completion_app.exit(1);
                                }
                            } else {
                                completion_app.exit(1);
                            }
                        } else {
                            completion_app.exit(1);
                        }
                        result
                    }));
                    webview.controller().CoreWebView2()?.CapturePreview(
                        COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                        &stream,
                        &handler,
                    )?;
                    Ok(())
                }
            })();
            if result.is_err() {
                app.exit(1);
            }
        })
        .map_err(|e| e.to_string())
}
