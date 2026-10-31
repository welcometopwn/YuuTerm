//! Native readiness check for an already installed authenticated X server.
#[tauri::command]
pub async fn x11_probe(
    window: tauri::WebviewWindow,
) -> Result<yuuterm_ssh::x11::desktop::Probe, String> {
    if window.label() != "main" {
        return Err("X11 discovery is available in the main window only".into());
    }
    Ok(yuuterm_ssh::x11::desktop::probe().await)
}
