//! Opt-in external VNC server capture (issue #21). Loopback fixtures keep their policy.
use super::*;

// Compiled in every build like windows_openssh_capture: the caller in backend()
// is always compiled and only reached in capture builds at runtime.
pub(super) fn external_profile(profile: &Profile) -> bool {
    std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref() == Ok("vnc-real")
        && std::env::var("YUUTERM_VNC_REAL_HOST").ok().as_deref() == Some(&profile.host)
        && profile.protocol == yuuterm_core::Protocol::Vnc
        && profile.id == 1
        // The qualification exercises both opt-in controls explicitly.
        && profile.vnc.clipboard
        && profile.vnc.resize_server
        && !profile.logging.enabled
        && profile.credential_id.is_empty()
}
