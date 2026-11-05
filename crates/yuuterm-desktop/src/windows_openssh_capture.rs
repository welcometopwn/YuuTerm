//! Opt-in external-server capture. Normal/loopback captures keep their policy.
use super::*;

/// Explicit external OpenSSH capture scopes: scope, host/user variables and the
/// generated test-run prefix (`{user}` is replaced by the profile account).
const SCOPES: [(&str, &str, &str, &str); 3] = [
    (
        "windows-openssh",
        "YUUTERM_WINDOWS_OPENSSH_HOST",
        "YUUTERM_WINDOWS_OPENSSH_USER",
        "/C:/Users/{user}/yuuterm-test/run-",
    ),
    (
        "linux-openssh",
        "YUUTERM_LINUX_OPENSSH_HOST",
        "YUUTERM_LINUX_OPENSSH_USER",
        "/home/{user}/yuuterm-test/run-",
    ),
    (
        "linux-ssh-tools",
        "YUUTERM_LINUX_OPENSSH_HOST",
        "YUUTERM_LINUX_OPENSSH_USER",
        "/home/{user}/yuuterm-test/run-",
    ),
];
fn scope() -> Option<(&'static str, &'static str, &'static str, &'static str)> {
    let scope = std::env::var("YUUTERM_CAPTURE_SCOPE").ok()?;
    SCOPES.into_iter().find(|s| s.0 == scope)
}

// Compiled in every build: the caller in backend() is always compiled and only
// reached in capture builds at runtime (a cfg-gated definition broke the
// normal release build in 500a083).
pub(super) fn external_profile(profile: &Profile) -> bool {
    if std::env::var("YUUTERM_CAPTURE_SCOPE").as_deref() == Ok("telnet-external") {
        return telnet_profile(
            profile,
            std::env::var("YUUTERM_TELNET_TEST_HOST").ok().as_deref(),
            std::env::var("YUUTERM_TELNET_TEST_USER").ok().as_deref(),
        );
    }
    let Some((_, host, user, _)) = scope() else {
        return false;
    };
    std::env::var(host).ok().as_deref() == Some(&profile.host)
        && std::env::var(user).ok().as_deref() == Some(&profile.username)
        && profile.protocol == yuuterm_core::Protocol::Ssh
        && profile.ssh.auth == yuuterm_core::SshAuth::KeyFile
        && port_admitted(
            profile,
            std::env::var("YUUTERM_LINUX_OPENSSH_PORT").ok().as_deref(),
        )
        && gateways_admitted(
            profile,
            std::env::var("YUUTERM_LINUX_GATEWAY_HOST").ok().as_deref(),
            std::env::var("YUUTERM_LINUX_GATEWAY_USER").ok().as_deref(),
        )
        && profile.id == 1
        && !profile.logging.enabled
        && !profile.ssh.x11_forward
        && profile.credential_id.is_empty()
}

/// Port 22, or exactly the runner's explicit gateway-only target port.
fn port_admitted(profile: &Profile, port: Option<&str>) -> bool {
    profile.port == port.map_or(Some(22), |p| p.parse().ok()).unwrap_or(0)
}

/// Direct, or exactly one credential-free key-file hop named by the runner.
fn gateways_admitted(profile: &Profile, host: Option<&str>, user: Option<&str>) -> bool {
    match (&profile.gateways[..], host, user) {
        ([], None, None) => true,
        ([yuuterm_core::Gateway::Manual(hop)], Some(host), Some(user)) => {
            hop.host == host
                && hop.username == user
                && hop.port == 22
                && hop.auth == yuuterm_core::SshAuth::KeyFile
                && hop.credential_id.is_empty()
        }
        _ => false,
    }
}

fn telnet_profile(profile: &Profile, host: Option<&str>, user: Option<&str>) -> bool {
    host == Some(&profile.host)
        && user == Some(&profile.username)
        && !profile.host.is_empty()
        && !profile.username.is_empty()
        && profile.protocol == yuuterm_core::Protocol::Telnet
        && profile.port == 23
        && profile.id == 1
        && !profile.logging.enabled
        && !profile.ssh.x11_forward
        && profile.credential_id.is_empty()
}

#[tauri::command]
pub(super) fn capture_external_path(
    window: WebviewWindow,
    state: State<Backend>,
) -> Result<String, String> {
    main_only(&window)?;
    let scope = scope();
    if !cfg!(feature = "capture") || !lock(&state.config)?.fixture || scope.is_none() {
        return Err("External OpenSSH capture only".into());
    }
    let username = lock(&state.config)?.prefs.profiles[0].username.clone();
    let file = std::path::PathBuf::from(
        std::env::var("YUUTERM_CAPTURE_PROFILE").map_err(|_| "No capture profile")?,
    )
    .with_file_name("remote-path.txt");
    let path = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    // root's home is /root, not /home/root.
    let prefix = if username == "root" {
        scope.unwrap().3.replace("/home/{user}", "/root")
    } else {
        scope.unwrap().3.replace("{user}", &username)
    };
    if !path.starts_with(&prefix)
        || path[prefix.len()..].len() != 32
        || !path[prefix.len()..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Invalid scoped test path".into());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_gateway_must_match_the_environment_exactly() {
        let hop = |host: &str, user: &str| {
            yuuterm_core::Gateway::Manual(yuuterm_core::ManualGateway {
                host: host.into(),
                username: user.into(),
                auth: yuuterm_core::SshAuth::KeyFile,
                key_file: "C:/k".into(),
                ..Default::default()
            })
        };
        let mut p = Profile::default();
        assert!(gateways_admitted(&p, None, None), "direct stays admitted");
        assert!(
            !gateways_admitted(&p, Some("gw"), Some("u")),
            "expected gateway missing"
        );
        p.gateways = vec![hop("gw", "u")];
        assert!(gateways_admitted(&p, Some("gw"), Some("u")));
        assert!(!gateways_admitted(&p, None, None));
        assert!(!gateways_admitted(&p, Some("gw"), Some("other")));
        p.gateways = vec![hop("gw", "u"), hop("gw", "u")];
        assert!(
            !gateways_admitted(&p, Some("gw"), Some("u")),
            "exactly one hop"
        );
        p.gateways = vec![yuuterm_core::Gateway::Saved { profile_id: 2 }];
        assert!(!gateways_admitted(&p, Some("gw"), Some("u")));
        let mut port = Profile {
            port: 22,
            ..Default::default()
        };
        assert!(port_admitted(&port, None));
        assert!(!port_admitted(&port, Some("2222")));
        port.port = 2222;
        assert!(port_admitted(&port, Some("2222")));
        assert!(!port_admitted(&port, None));
    }

    #[test]
    fn external_telnet_requires_exact_nonempty_environment_endpoint_and_account() {
        let profile = Profile {
            id: 1,
            protocol: yuuterm_core::Protocol::Telnet,
            host: "test.invalid".into(),
            port: 23,
            username: "test-account".into(),
            ..Default::default()
        };
        assert!(telnet_profile(
            &profile,
            Some("test.invalid"),
            Some("test-account")
        ));
        for (host, user) in [
            (None, Some("test-account")),
            (Some("test.invalid"), None),
            (Some("other.invalid"), Some("test-account")),
            (Some("test.invalid"), Some("other-account")),
        ] {
            assert!(!telnet_profile(&profile, host, user));
        }
        let mut empty = profile.clone();
        empty.host.clear();
        empty.username.clear();
        assert!(!telnet_profile(&empty, Some(""), Some("")));
        for change in 0..6 {
            let mut altered = profile.clone();
            match change {
                0 => altered.protocol = yuuterm_core::Protocol::Ssh,
                1 => altered.port = 2323,
                2 => altered.id = 2,
                3 => altered.logging.enabled = true,
                4 => altered.ssh.x11_forward = true,
                _ => altered.credential_id = "unrelated-saved-credential".into(),
            }
            assert!(!telnet_profile(
                &altered,
                Some("test.invalid"),
                Some("test-account")
            ));
        }
    }
}
