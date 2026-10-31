//! Development-only COM/OLE surface. Only an owned, non-authenticating loopback
//! refusal fixture can be contacted; no endpoint/credentials API is exposed.
use crate::rdp::{OleApartment, create_control, method, property, put};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, collections::BTreeMap};
use windows::{
    Win32::{
        Foundation::{HWND, POINT, RECT},
        Graphics::Gdi::ScreenToClient,
        System::{
            Com::IDispatch,
            Variant::{VARIANT, VariantToInt32, VariantToString},
        },
        UI::{
            Input::KeyboardAndMouse::{GetFocus, SetFocus},
            WindowsAndMessaging::*,
        },
    },
    core::w,
};

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}
impl Bounds {
    fn validate(self, width: i32, height: i32) -> std::result::Result<(), String> {
        if self.x < 0
            || self.y < 0
            || self.width < 64
            || self.height < 64
            || self.width > 8192
            || self.height > 8192
            || self.x.checked_add(self.width).is_none_or(|x| x > width)
            || self.y.checked_add(self.height).is_none_or(|y| y > height)
        {
            return Err("RDP probe rectangle must fit inside the local client area".into());
        }
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Place,
    Hide,
    Focus,
    Status,
    Destroy,
    TestRefusal,
    LiveConnect,
    Disconnect,
}
#[derive(Serialize)]
pub struct Status {
    id: u64,
    version: String,
    connected: i32,
    visible: bool,
    parent_matches: bool,
    focus_inside: bool,
    top_at_center: bool,
    bounds: Bounds,
    policy: Vec<PolicyValue>,
    events: Vec<crate::rdp_events::Event>,
    dropped_events: usize,
    fixture: Option<FixtureStatus>,
    key_sha256: Option<String>,
    key_pinned: bool,
}
#[derive(Clone, Default, Serialize)]
struct FixtureStatus {
    accepted: bool,
    negotiation_received: bool,
    finished: bool,
}
type Fixture = std::sync::Arc<std::sync::Mutex<FixtureStatus>>;
#[derive(Clone, Serialize)]
struct PolicyValue {
    property: &'static str,
    expected: i32,
    actual: i32,
}
// Hard-coded local policy only. Never expose arbitrary COM properties through IPC.
fn policy(control: &IDispatch) -> std::result::Result<Vec<PolicyValue>, String> {
    let advanced = IDispatch::try_from(
        &property(control, w!("AdvancedSettings9"))
            .map_err(|e| e.to_string())?
            .0,
    )
    .map_err(|e| e.to_string())?;
    let secured = IDispatch::try_from(
        &property(control, w!("SecuredSettings3"))
            .map_err(|e| e.to_string())?
            .0,
    )
    .map_err(|e| e.to_string())?;
    let mut report = Vec::new();
    for (target, name, label, expected, boolean) in [
        (
            &advanced,
            w!("AuthenticationLevel"),
            "AuthenticationLevel",
            1,
            false,
        ),
        (
            &advanced,
            w!("EnableCredSspSupport"),
            "EnableCredSspSupport",
            1,
            true,
        ),
        (
            &advanced,
            w!("EnableAutoReconnect"),
            "EnableAutoReconnect",
            0,
            true,
        ),
        (
            &advanced,
            w!("RedirectClipboard"),
            "RedirectClipboard",
            0,
            true,
        ),
        (&advanced, w!("RedirectDrives"), "RedirectDrives", 0, true),
        (
            &advanced,
            w!("RedirectPrinters"),
            "RedirectPrinters",
            0,
            true,
        ),
        (&advanced, w!("RedirectPorts"), "RedirectPorts", 0, true),
        (
            &advanced,
            w!("RedirectSmartCards"),
            "RedirectSmartCards",
            0,
            true,
        ),
        (&advanced, w!("RedirectDevices"), "RedirectDevices", 0, true),
        (
            &advanced,
            w!("RedirectPOSDevices"),
            "RedirectPOSDevices",
            0,
            true,
        ),
        (
            &advanced,
            w!("AudioCaptureRedirectionMode"),
            "AudioCaptureRedirectionMode",
            0,
            true,
        ),
        (
            &secured,
            w!("AudioRedirectionMode"),
            "AudioRedirectionMode",
            2,
            false,
        ),
        (
            &secured,
            w!("KeyboardHookMode"),
            "KeyboardHookMode",
            0,
            false,
        ),
    ] {
        let value = if boolean {
            VARIANT::from(expected != 0)
        } else {
            VARIANT::from(expected)
        };
        put(target, name, value)
            .map_err(|e| format!("RDP policy {label} could not be set: {e}"))?;
        let value = property(target, name)
            .map_err(|e| format!("RDP policy {label} could not be read: {e}"))?;
        let actual = if boolean {
            i32::from(bool::try_from(&value.0).map_err(|e| e.to_string())?)
        } else {
            i32::try_from(&value.0).map_err(|e| e.to_string())?
        };
        if actual != expected {
            return Err(format!(
                "RDP policy {label} did not retain its required value"
            ));
        }
        report.push(PolicyValue {
            property: label,
            expected,
            actual,
        });
    }
    Ok(report)
}
struct Host {
    id: u64,
    hwnd: HWND,
    control: Option<IDispatch>,
    policy: Vec<PolicyValue>,
    subscription: Option<crate::rdp_events::Subscription>,
    fixture: Option<Fixture>,
    #[cfg_attr(not(feature = "rdp-live"), allow(dead_code))]
    live: bool,
    _ole: OleApartment,
}
impl Drop for Host {
    fn drop(&mut self) {
        unsafe {
            self.subscription.take();
            let _ = DestroyWindow(self.hwnd);
            self.control.take();
        }
    }
}
// COM interfaces/HWND never cross threads. Tauri dispatches every call to its
// owning UI STA.
thread_local! { static HOSTS: RefCell<BTreeMap<u64, Host>> = const { RefCell::new(BTreeMap::new()) }; }
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn create(parent: HWND, bounds: Bounds) -> std::result::Result<Host, String> {
    let ole = OleApartment::enter()?;
    let (hwnd, control) = create_control(
        parent,
        crate::rdp::Bounds {
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
        },
        true,
    )?;
    let mut host = Host {
        id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        hwnd,
        control: None,
        policy: Vec::new(),
        subscription: None,
        fixture: None,
        live: false,
        _ole: ole,
    };
    host.policy = policy(&control)?;
    host.subscription = Some(
        crate::rdp_events::Subscription::attach(&control, true)
            .map_err(|e| format!("RDP event subscription failed: {e}"))?,
    );
    host.control = Some(control);
    Ok(host)
}

fn test_refusal(host: &mut Host) -> std::result::Result<(), String> {
    use std::{
        io::Read,
        net::TcpListener,
        time::{Duration, Instant},
    };
    if host.fixture.is_some() {
        return Err("Refusal fixture already ran on this incarnation".into());
    }
    // Keep ownership of an ephemeral IPv4 loopback listener. It never sends a
    // negotiation response, TLS certificate or authentication challenge, and
    // cannot act as an authenticated RDP server or solicit credentials.
    let listener =
        TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let control = host.control.as_ref().unwrap();
    put(control, w!("Server"), VARIANT::from("127.0.0.1")).map_err(|e| e.to_string())?;
    put(control, w!("UserName"), VARIANT::from("yuuterm-fixture")).map_err(|e| e.to_string())?;
    let advanced: IDispatch = property(control, w!("AdvancedSettings9"))
        .and_then(|v| IDispatch::try_from(&v.0))
        .map_err(|e| e.to_string())?;
    put(&advanced, w!("RDPPort"), VARIANT::from(i32::from(port))).map_err(|e| e.to_string())?;
    let state = Fixture::default();
    let worker = state.clone();
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    if let Ok(mut s) = worker.lock() {
                        s.accepted = true;
                    }
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let mut header = [0u8; 4];
                    let received = stream.read_exact(&mut header).is_ok()
                        && header[0..2] == [3, 0]
                        && (11..=4096).contains(&u16::from_be_bytes([header[2], header[3]]));
                    if let Ok(mut s) = worker.lock() {
                        s.negotiation_received = received;
                    }
                    // Closing here exercises the control's failure callback.
                    // No payload is retained, logged or replied to.
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(_) => break,
            }
        }
        if let Ok(mut s) = worker.lock() {
            s.finished = true;
        }
    });
    host.fixture = Some(state);
    method(control, w!("Connect")).map_err(|e| e.to_string())
}
#[cfg(not(feature = "rdp-live"))]
fn live_connect(_: &mut Host) -> std::result::Result<(), String> {
    Err("Live RDP is not part of this build".into())
}
#[cfg(feature = "rdp-live")]
fn live_connect(host: &mut Host) -> std::result::Result<(), String> {
    if !crate::rdp_live::enabled() {
        return Err("Live RDP test is not enabled".into());
    }
    if host.live || host.fixture.is_some() {
        return Err("This surface incarnation already connected once".into());
    }
    let target = crate::rdp_live::target()?;
    let control = host.control.as_ref().unwrap();
    let advanced: IDispatch = property(control, w!("AdvancedSettings9"))
        .and_then(|v| IDispatch::try_from(&v.0))
        .map_err(|e| e.to_string())?;
    // Strict server authentication (level 1): Windows must validate the
    // certificate chain and name before CredSSP, otherwise the control refuses
    // without offering "connect anyway". Measured on mstscax 10.0.26100: the
    // embedded control ignores mstsc's remembered CertHash, and
    // OnReceivedTSPublicKey did not fire even with NotifyTSPublicKey, so neither
    // can pin a self-signed certificate. Hosts need a trusted certificate.
    put(&advanced, w!("AuthenticationLevel"), VARIANT::from(1)).map_err(|e| e.to_string())?;
    put(&advanced, w!("NotifyTSPublicKey"), VARIANT::from(true)).map_err(|e| e.to_string())?;
    let mut rect = RECT::default();
    unsafe { GetClientRect(host.hwnd, &mut rect) }.map_err(|e| e.to_string())?;
    put(
        control,
        w!("DesktopWidth"),
        VARIANT::from(rect.right.clamp(200, 8192)),
    )
    .map_err(|e| e.to_string())?;
    put(
        control,
        w!("DesktopHeight"),
        VARIANT::from(rect.bottom.clamp(200, 8192)),
    )
    .map_err(|e| e.to_string())?;
    put(control, w!("Server"), VARIANT::from(target.host.as_str())).map_err(|e| e.to_string())?;
    put(control, w!("UserName"), VARIANT::from(target.user.as_str())).map_err(|e| e.to_string())?;
    crate::rdp_live::set_password(control, target.password)?;
    host.subscription
        .as_ref()
        .unwrap()
        .arm(crate::rdp_events::Armed { pins: target.pins });
    host.live = true;
    // Stage markers only; never the target, user or password.
    eprintln!("rdp-live: configured, calling Connect");
    let result = method(control, w!("Connect")).map_err(|e| e.to_string());
    eprintln!("rdp-live: Connect returned ok={}", result.is_ok());
    result
}
pub fn destroy() {
    HOSTS.with(|slot| {
        slot.borrow_mut().clear();
    });
}

/// Captures only the owned probe window, including its native child surface.
/// WebView2 CapturePreview alone would omit native sibling windows.
pub fn capture(parent: usize, path: &str) -> std::result::Result<(), String> {
    let (width, height, rgb) = crate::rdp::capture_rgb(HWND(parent as *mut _), 4096)?;
    image::RgbImage::from_raw(width, height, rgb)
        .ok_or("Invalid probe image")?
        .save(path)
        .map_err(|e| e.to_string())
}
pub fn perform(
    parent: usize,
    action: Action,
    bounds: Option<Bounds>,
    expected_id: Option<u64>,
) -> std::result::Result<Option<Status>, String> {
    let parent = HWND(parent as *mut _);
    HOSTS.with(|slot| {
        let mut hosts = slot.borrow_mut();
        if expected_id.is_some_and(|id| !hosts.contains_key(&id)) {
            return Err("Original RDP probe surface no longer exists".into());
        }
        if expected_id.is_none() && !matches!(action, Action::Place) {
            return Err("A native surface incarnation is required".into());
        }
        if matches!(action, Action::Destroy) {
            hosts.remove(&expected_id.unwrap());
            return Ok(None);
        }
        let mut selected = expected_id;
        if matches!(action, Action::Place) {
            let bounds = bounds.ok_or("Missing probe bounds")?;
            let mut client = RECT::default();
            unsafe {
                GetClientRect(parent, &mut client).map_err(|e| e.to_string())?;
            }
            bounds.validate(client.right, client.bottom)?;
            if selected.is_none() {
                if hosts.len() >= 4 {
                    return Err("The disconnected probe allows at most four native surfaces".into());
                }
                let host = create(parent, bounds)?;
                selected = Some(host.id);
                hosts.insert(host.id, host);
            }
            let host = &hosts[&selected.unwrap()];
            unsafe {
                SetWindowPos(
                    host.hwnd,
                    Some(HWND_TOP),
                    bounds.x,
                    bounds.y,
                    bounds.width,
                    bounds.height,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )
                .map_err(|e| e.to_string())?;
            }
        }
        let host = hosts.get_mut(&selected.unwrap()).unwrap();
        if matches!(action, Action::TestRefusal) {
            test_refusal(host)?;
        }
        if matches!(action, Action::LiveConnect) {
            live_connect(host)?;
        }
        if matches!(action, Action::Disconnect) {
            method(host.control.as_ref().unwrap(), w!("Disconnect")).map_err(|e| e.to_string())?;
        }
        unsafe {
            match action {
                Action::Hide => {
                    let _ = ShowWindow(host.hwnd, SW_HIDE);
                }
                Action::Focus => {
                    let _ = SetFocus(Some(host.hwnd));
                }
                _ => {}
            }
            let mut rect = RECT::default();
            GetWindowRect(host.hwnd, &mut rect).map_err(|e| e.to_string())?;
            let mut point = POINT {
                x: rect.left,
                y: rect.top,
            };
            if !ScreenToClient(parent, &mut point).as_bool() {
                return Err("Probe coordinate conversion failed".into());
            }
            let control = host.control.as_ref().unwrap();
            let value = property(control, w!("Version")).map_err(|e| e.to_string())?;
            let mut version = [0u16; 128];
            VariantToString(&value.0, &mut version).map_err(|e| e.to_string())?;
            let connected = VariantToInt32(
                &property(control, w!("Connected"))
                    .map_err(|e| e.to_string())?
                    .0,
            )
            .map_err(|e| e.to_string())?;
            let focused = GetFocus();
            let hit = WindowFromPoint(POINT {
                x: rect.left + (rect.right - rect.left) / 2,
                y: rect.top + (rect.bottom - rect.top) / 2,
            });
            let (events, dropped_events) = host.subscription.as_ref().unwrap().snapshot();
            let (key_sha256, key_pinned) = host.subscription.as_ref().unwrap().key();
            Ok(Some(Status {
                id: host.id,
                version: String::from_utf16_lossy(
                    &version[..version
                        .iter()
                        .position(|v| *v == 0)
                        .unwrap_or(version.len())],
                ),
                connected,
                visible: IsWindowVisible(host.hwnd).as_bool(),
                parent_matches: GetParent(host.hwnd).ok() == Some(parent),
                focus_inside: focused == host.hwnd || IsChild(host.hwnd, focused).as_bool(),
                top_at_center: hit == host.hwnd || IsChild(host.hwnd, hit).as_bool(),
                bounds: Bounds {
                    x: point.x,
                    y: point.y,
                    width: rect.right - rect.left,
                    height: rect.bottom - rect.top,
                },
                policy: host.policy.clone(),
                events,
                dropped_events,
                fixture: host
                    .fixture
                    .as_ref()
                    .and_then(|f| f.lock().ok().map(|s| s.clone())),
                key_sha256,
                key_pinned,
            }))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rectangle_refuses_overflow_and_parent_escape() {
        for b in [
            Bounds {
                x: -1,
                y: 0,
                width: 100,
                height: 100,
            },
            Bounds {
                x: 1,
                y: 0,
                width: i32::MAX,
                height: 100,
            },
            Bounds {
                x: 950,
                y: 0,
                width: 100,
                height: 100,
            },
        ] {
            assert!(b.validate(1000, 800).is_err());
        }
        assert!(
            Bounds {
                x: 200,
                y: 100,
                width: 700,
                height: 600
            }
            .validate(1000, 800)
            .is_ok()
        );
    }
}
