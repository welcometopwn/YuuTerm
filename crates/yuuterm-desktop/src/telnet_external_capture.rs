//! Capture-only bridge to the application's native one-shot credential prompt.
//! TELNET has no protocol authentication: the fixture calls this only after the
//! real server's password prompt. No saved credential or WebView secret is used.
use super::*;

#[tauri::command]
pub(super) fn capture_telnet_password(
    window: WebviewWindow,
    state: State<Backend>,
    id: u64,
) -> Result<(), String> {
    main_only(&window)?;
    #[cfg(feature = "capture")]
    {
        let profile = {
            let config = lock(&state.config)?;
            let profile = config.prefs.profiles.first().ok_or("No capture profile")?;
            if !config.fixture
                || std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref() != Ok("telnet-external")
                || !windows_openssh_capture::external_profile(profile)
            {
                return Err("Explicit external TELNET capture only".into());
            }
            profile.clone()
        };
        let validate = |sessions: &BTreeMap<u64, Session>| -> Result<(), String> {
            let s = sessions.get(&id).ok_or("Capture connection ended")?;
            if s.profile_id != profile.id
                || s.live.snapshot().protocol != yuuterm_core::Protocol::Telnet
                || !s.live.snapshot().connected
                || s.output_log.is_some()
            {
                return Err("Unlogged live TELNET capture required".into());
            }
            Ok(())
        };
        validate(&*lock(&state.sessions)?)?;
        // CredUI pumps messages: hold no locks across the modal prompt. Check
        // this exact incarnation again afterwards; never redirect to a new one.
        let prompted = credentials::prompt_once(
            window.hwnd().map_err(|e| e.to_string())?,
            "YuuTerm TELNET test sign-in",
            &format!("Sign in to {}:23 · Plaintext TELNET", profile.host),
            &profile.username,
            false,
        )?
        .ok_or("TELNET sign-in cancelled")?;
        let sessions = lock(&state.sessions)?;
        validate(&sessions)?;
        let mut bytes = prompted
            .secret
            .expose_for_authentication()
            .as_bytes()
            .to_vec();
        bytes.push(b'\r');
        // Deliberately bypass session_input's capture trace. The normal bounded
        // terminal input queue and TELNET codec still carry the password.
        sessions[&id]
            .live
            .send(Command::Input(bytes))
            .map_err(String::from)
    }
    #[cfg(not(feature = "capture"))]
    {
        let _ = (state, id);
        Err("Capture is not part of this build".into())
    }
}
