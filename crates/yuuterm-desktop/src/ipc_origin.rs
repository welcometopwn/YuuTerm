//! Only the bundled document (or the explicit debug dev server) owns desktop IPC.
pub fn allowed(url: &tauri::Url) -> bool {
    allowed_in_build(url, cfg!(debug_assertions))
}

fn allowed_in_build(url: &tauri::Url, development: bool) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match (url.scheme(), url.host_str(), url.port()) {
        ("tauri", Some("localhost"), None) | ("http", Some("tauri.localhost"), None) => true,
        ("http", Some("127.0.0.1"), Some(1420)) => development,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn navigation_and_ipc_require_the_exact_local_origin() {
        for value in [
            "tauri://localhost/index.html",
            "http://tauri.localhost/",
            "http://tauri.localhost/index.html#pane",
        ] {
            assert!(allowed_in_build(&value.parse().unwrap(), false));
        }
        for value in [
            "tauri://attacker.test/",
            "tauri://localhost:123/",
            "http://tauri.localhost:1420/",
            "https://tauri.localhost/",
            "http://tauri.localhost.evil.test/",
            "http://user@tauri.localhost/",
            "file:///C:/index.html",
            "data:text/html,test",
            "http://127.0.0.1:1421/",
        ] {
            assert!(!allowed_in_build(&value.parse().unwrap(), false), "{value}");
            assert!(!allowed_in_build(&value.parse().unwrap(), true), "{value}");
        }
        let dev = "http://127.0.0.1:1420/".parse().unwrap();
        assert!(allowed_in_build(&dev, true));
        assert!(!allowed_in_build(&dev, false));
    }
}
