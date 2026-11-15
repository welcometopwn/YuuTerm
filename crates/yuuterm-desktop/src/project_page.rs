//! Opens the public project page in the default browser. The URL is fixed here;
//! the UI cannot pass its own, so this never becomes a generic URL launcher.
use windows::{
    Win32::{
        System::Com::{
            COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
        },
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    },
    core::{HSTRING, w},
};

pub const URL: &str = "https://github.com/welcometopwn/yuuterm";
const RELEASES_URL: &str = "https://github.com/welcometopwn/yuuterm/releases/latest";

#[tauri::command]
pub async fn open_project_page() -> Result<(), String> {
    open(URL).await
}

#[tauri::command]
pub async fn open_release_page() -> Result<(), String> {
    open(RELEASES_URL).await
}

async fn open(url: &'static str) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || unsafe {
        // ShellExecute may use COM; Microsoft recommends an STA without OLE1 DDE.
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).is_ok();
        let result = ShellExecuteW(
            None,
            w!("open"),
            &HSTRING::from(url),
            None,
            None,
            SW_SHOWNORMAL,
        );
        if com {
            CoUninitialize();
        }
        // Values above 32 mean success.
        if result.0 as usize > 32 {
            Ok(())
        } else {
            Err(format!("Could not open {url} in the default browser"))
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
