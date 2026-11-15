use serde::Serialize;
use std::ffi::c_void;
use windows::{
    Win32::Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
        WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen,
        WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData,
        WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    },
    core::{HSTRING, PCWSTR, w},
};

const HOST: &str = "api.github.com";
const PATH: &str = "/repos/welcometopwn/yuuterm/releases/latest";
const MAX_BODY: usize = 512 * 1024;
const TIMEOUT_MS: i32 = 10_000;

#[derive(Serialize)]
pub struct UpdateStatus {
    current: String,
    latest: String,
    available: bool,
}

struct Handle(*mut c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            let _ = unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

fn version(text: &str) -> Option<[u64; 3]> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let mut out = [0; 3];
    for slot in &mut out {
        *slot = parts.next()?.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

fn tag_name(body: &[u8]) -> Result<String, String> {
    let json: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| "The update server sent an invalid answer")?;
    json.get("tag_name")
        .and_then(|t| t.as_str())
        .filter(|t| version(t).is_some())
        .map(str::to_owned)
        .ok_or_else(|| "The update server sent no valid version".into())
}

fn fetch() -> Result<Vec<u8>, String> {
    let fail = |step: &str| format!("Update check failed ({step})");
    unsafe {
        let session = Handle(WinHttpOpen(
            w!("YuuTerm update check"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ));
        if session.0.is_null() {
            return Err(fail("start"));
        }
        let _ = WinHttpSetTimeouts(session.0, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS);
        let connection = Handle(WinHttpConnect(session.0, &HSTRING::from(HOST), 443, 0));
        if connection.0.is_null() {
            return Err(fail("connect"));
        }
        let request = Handle(WinHttpOpenRequest(
            connection.0,
            w!("GET"),
            &HSTRING::from(PATH),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            return Err(fail("request"));
        }
        let headers: Vec<u16> = "Accept: application/vnd.github+json"
            .encode_utf16()
            .collect();
        WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0)
            .map_err(|_| "No connection to the update server")?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())
            .map_err(|_| "No answer from the update server")?;
        let mut status = 0u32;
        let mut size = size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut size,
            std::ptr::null_mut(),
        )
        .map_err(|_| fail("status"))?;
        match status {
            200 => {}
            404 => return Err("No public release exists yet".into()),
            403 | 429 => return Err("The update server is rate limiting; try again later".into()),
            other => return Err(format!("The update server answered {other}")),
        }
        let mut body = Vec::new();
        loop {
            let mut available = 0u32;
            WinHttpQueryDataAvailable(request.0, &mut available).map_err(|_| fail("read"))?;
            if available == 0 {
                return Ok(body);
            }
            if body.len() + available as usize > MAX_BODY {
                return Err("The update server answer is too large".into());
            }
            let start = body.len();
            body.resize(start + available as usize, 0);
            let mut read = 0u32;
            WinHttpReadData(
                request.0,
                body[start..].as_mut_ptr().cast(),
                available,
                &mut read,
            )
            .map_err(|_| fail("read"))?;
            body.truncate(start + read as usize);
        }
    }
}

fn compare(current: &str, tag: &str) -> Result<UpdateStatus, String> {
    let latest = version(tag).ok_or("The update server sent no valid version")?;
    let running = version(current).ok_or("This build has no valid version")?;
    Ok(UpdateStatus {
        current: current.into(),
        latest: tag.trim_start_matches('v').into(),
        available: latest > running,
    })
}

#[tauri::command]
pub async fn check_for_update(window: tauri::WebviewWindow) -> Result<UpdateStatus, String> {
    super::main_only(&window)?;
    tauri::async_runtime::spawn_blocking(|| {
        compare(env!("CARGO_PKG_VERSION"), &tag_name(&fetch()?)?)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically_and_reject_noise() {
        assert!(compare("0.3.1", "v0.3.2").unwrap().available);
        assert!(compare("0.3.9", "v0.3.10").unwrap().available);
        assert!(!compare("0.3.1", "v0.3.1").unwrap().available);
        assert!(!compare("0.4.0", "v0.3.9").unwrap().available);
        assert_eq!(compare("0.3.1", "v0.3.2").unwrap().latest, "0.3.2");
        for bad in ["", "latest", "v1.2", "1.2.3.4", "v1.2.x", "1.2.3-rc1"] {
            assert!(version(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn only_a_valid_tag_is_accepted_from_the_server() {
        assert_eq!(
            tag_name(br#"{"tag_name":"v1.2.3","x":1}"#).unwrap(),
            "v1.2.3"
        );
        assert!(tag_name(br#"{"tag_name":"nightly"}"#).is_err());
        assert!(tag_name(br#"{"name":"v1.2.3"}"#).is_err());
        assert!(tag_name(b"<html>").is_err());
    }

    #[test]
    #[ignore]
    fn live_request_reaches_the_release_api() {
        let result = fetch().and_then(|body| tag_name(&body));
        eprintln!("live result: {result:?}");
        assert!(result.is_ok() || result.as_ref().unwrap_err().contains("No public release"));
    }
}
