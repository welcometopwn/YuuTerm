//! Windows application chooser for an already staged local file.
use std::path::PathBuf;
use tauri::WebviewWindow;
use windows::{
    Win32::{
        Foundation::HWND,
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        UI::Shell::{OAIF_EXEC, OPENASINFO, SHOpenWithDialog},
    },
    core::{HSTRING, PCWSTR},
};
pub async fn show(window: &WebviewWindow, path: PathBuf) -> Result<(), String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .run_on_main_thread(move || {
            // The shell chooser needs an STA and a message loop, supplied by the
            // native UI thread. Balance our successful COM initialization only.
            let result = (|| unsafe {
                CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
                let file = HSTRING::from(path.as_os_str());
                let result = SHOpenWithDialog(
                    Some(HWND(hwnd as *mut _)),
                    &OPENASINFO {
                        pcszFile: PCWSTR(file.as_ptr()),
                        pcszClass: PCWSTR::null(),
                        oaifInFlags: OAIF_EXEC,
                    },
                );
                CoUninitialize();
                result
            })();
            let result = match result {
                Err(e) if e.code().0 as u32 == 0x800704c7 => Ok(()),
                other => {
                    other.map_err(|e| format!("Windows Open with failed: {e}; local copy retained"))
                }
            };
            let _ = tx.send(result);
        })
        .map_err(|e| e.to_string())?;
    rx.await
        .map_err(|_| "Windows application chooser closed unexpectedly".to_owned())?
}
