//! Explicit, capture-only admission for one real Windows-agent or password gateway.
#[cfg(any(test, feature = "capture"))]
use yuuterm_core::ManualGateway;
use yuuterm_core::{Gateway, Profile, SshAuth};

#[cfg(feature = "capture")]
pub(crate) fn hops() -> Vec<Gateway> {
    match (
        std::env::var("YUUTERM_GATEWAY_HOST"),
        std::env::var("YUUTERM_GATEWAY_USER"),
    ) {
        (Ok(host), Ok(username)) => vec![Gateway::Manual(ManualGateway {
            host,
            username,
            auth: if std::env::var("YUUTERM_GATEWAY_AUTH").as_deref() == Ok("password") {
                SshAuth::Password
            } else {
                SshAuth::WindowsAgent
            },
            identity_fingerprint: std::env::var("YUUTERM_GATEWAY_IDENTITY").unwrap_or_default(),
            ..Default::default()
        })],
        _ => Vec::new(),
    }
}

pub(crate) fn admitted(profile: &Profile) -> bool {
    admitted_hop(
        profile,
        std::env::var("YUUTERM_GATEWAY_HOST").ok().as_deref(),
        std::env::var("YUUTERM_GATEWAY_USER").ok().as_deref(),
    )
}
fn admitted_hop(profile: &Profile, host: Option<&str>, user: Option<&str>) -> bool {
    match (&profile.gateways[..], host, user) {
        ([Gateway::Manual(h)], Some(host), Some(user)) => {
            h.host == host
                && h.username == user
                && h.port == 22
                && matches!(h.auth, SshAuth::WindowsAgent | SshAuth::Password)
                && h.key_file.is_empty()
                && h.credential_id.is_empty()
                && profile.credential_id.is_empty()
                && !profile.logging.enabled
                && !profile.ssh.x11_forward
        }
        _ => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_gateway_needs_an_exact_manual_credential_free_hop() {
        let mut p = Profile {
            gateways: vec![Gateway::Manual(ManualGateway {
                host: "bastion.test".into(),
                username: "jump".into(),
                ..Default::default()
            })],
            ..Default::default()
        };
        assert!(admitted_hop(&p, Some("bastion.test"), Some("jump")));
        assert!(!admitted_hop(&p, None, None));
        assert!(!admitted_hop(&p, Some("different.test"), Some("jump")));
        for (auth, expected) in [
            (SshAuth::Password, true),
            (SshAuth::KeyFile, false),
            (SshAuth::WindowsAgent, true),
        ] {
            if let Gateway::Manual(h) = &mut p.gateways[0] {
                h.auth = auth;
            }
            assert_eq!(
                admitted_hop(&p, Some("bastion.test"), Some("jump")),
                expected
            );
        }
        p.gateways.push(p.gateways[0].clone());
        assert!(!admitted_hop(&p, Some("bastion.test"), Some("jump")));
    }
}
