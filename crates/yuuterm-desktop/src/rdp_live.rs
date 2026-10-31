//! Developer-only authenticated RDP test against one explicitly configured host.
//! Target and password come only from a host-side file named by the environment,
//! never from IPC. The test script verifies the server certificate against the
//! user-confirmed pin right before launch and passes the matching public-key
//! hashes; the live session must present that key in OnReceivedTSPublicKey, or
//! CredSSP is aborted before credentials are delegated (see `rdp_events`).
use windows::Win32::System::Com::IDispatch;
use zeroize::Zeroizing;

pub struct Target {
    pub host: String,
    pub user: String,
    pub password: Zeroizing<String>,
    /// Lowercase hex SHA-256 values accepted for the OnReceivedTSPublicKey bytes.
    pub pins: Vec<String>,
}

pub fn enabled() -> bool {
    std::env::var("YUUTERM_RDP_LIVE").as_deref() == Ok("1")
}

pub fn target() -> Result<Target, String> {
    let path = std::env::var("YUUTERM_RDP_LIVE_ENV").map_err(|_| "Live RDP target file missing")?;
    let text = Zeroizing::new(std::fs::read_to_string(path).map_err(|e| e.to_string())?);
    if text.len() > 4096 {
        return Err("Live RDP target file is too large".into());
    }
    let value = |key: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim_end_matches('\r'))
    };
    // A negative test may address the same server by another name (no remembered
    // certificate for it), which must end in a cancelled warning.
    let alias = std::env::var("YUUTERM_RDP_LIVE_SERVER").ok();
    let host = alias
        .as_deref()
        .filter(|a| !a.is_empty())
        .or(value("RDP_TEST_HOST"))
        .unwrap_or_default();
    let user = value("RDP_TEST_USER").unwrap_or_default();
    let password = Zeroizing::new(value("RDP_TEST_PASSWORD").unwrap_or_default().to_owned());
    if host.is_empty()
        || host.len() > 253
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".-:".contains(c))
    {
        return Err("RDP_TEST_HOST must be a hostname or IP address".into());
    }
    if user.is_empty() || user.len() > 256 || user.chars().any(char::is_control) {
        return Err("RDP_TEST_USER is missing or invalid".into());
    }
    if password.is_empty() || password.len() > 256 || password.chars().any(char::is_control) {
        return Err("RDP_TEST_PASSWORD is missing or invalid".into());
    }
    let pins: Vec<String> = std::env::var("YUUTERM_RDP_LIVE_PIN")
        .unwrap_or_default()
        .split(',')
        .filter(|p| !p.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    if pins.is_empty()
        || pins.len() > 4
        || pins
            .iter()
            .any(|p| p.len() != 64 || !p.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("YUUTERM_RDP_LIVE_PIN needs 1–4 SHA-256 hex values".into());
    }
    Ok(Target {
        host: host.to_owned(),
        user: user.to_owned(),
        password,
        pins,
    })
}

/// Hands the password to the control (which otherwise opens a Windows prompt).
pub fn set_password(control: &IDispatch, password: Zeroizing<String>) -> Result<(), String> {
    crate::rdp::set_password(control, &password)
}
