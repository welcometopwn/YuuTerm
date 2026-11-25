//! Native readiness check for an already installed authenticated X server.
#[tauri::command]
pub async fn x11_probe(
    window: tauri::WebviewWindow,
) -> Result<yuuterm_ssh::x11::desktop::Probe, String> {
    super::main_only(&window)?;
    Ok(yuuterm_ssh::x11::desktop::probe().await)
}
