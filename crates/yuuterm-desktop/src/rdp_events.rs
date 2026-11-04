//! Bounded, incarnation-owned COM notifications. Never retain server text or keys.
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use windows::Win32::System::{
    Com::*,
    Variant::{VARIANT, VT_BOOL, VT_BYREF},
};
use windows_core::{Error, GUID, Interface, PCWSTR, Result, implement, interface};

#[interface("336d5562-efa8-482e-8cb3-c5c0fc7a7db6")]
unsafe trait IMsTscAxEvents: IDispatch {}

#[derive(Clone, Serialize)]
pub struct Event {
    pub kind: &'static str,
    pub code: Option<i32>,
}
#[derive(Default)]
#[cfg_attr(not(feature = "rdp-probe"), allow(dead_code))]
struct Buffer {
    events: VecDeque<Event>,
    dropped: usize,
    /// SHA-256 of the last server key; a public value, never the key itself.
    key_sha256: Option<String>,
    key_pinned: bool,
}
/// Armed pin and password hand-over. COM objects stay on the owning STA, so this
/// lives beside the thread-safe buffer instead of inside it.
#[cfg(feature = "rdp-live")]
type Arming = std::rc::Rc<std::cell::RefCell<Option<Armed>>>;
#[cfg(not(feature = "rdp-live"))]
#[derive(Clone)]
struct Arming;
fn arming() -> Arming {
    #[cfg(feature = "rdp-live")]
    return Arming::default();
    #[cfg(not(feature = "rdp-live"))]
    Arming
}
/// Accepted key hashes for one live connection; consumed by the first key event.
#[cfg(feature = "rdp-live")]
pub struct Armed {
    pub pins: Vec<String>,
}
impl Buffer {
    fn push(&mut self, event: Event) {
        if self.events.len() == 64 {
            self.events.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.events.push_back(event);
    }
}
/// `close_warnings` is the developer probe's strict mode. Production sessions keep
/// the control's own server-authentication prompt so the user decides, like mstsc.
#[implement(IMsTscAxEvents)]
struct Sink(Arc<Mutex<Buffer>>, Arming, bool);
impl Sink {
    #[cfg(not(feature = "rdp-live"))]
    fn decide(&self, _: Option<&DISPPARAMS>) -> bool {
        false
    }
    #[cfg(feature = "rdp-live")]
    fn decide(&self, args: Option<&DISPPARAMS>) -> bool {
        use sha2::{Digest, Sha256};
        use windows::Win32::System::Variant::VT_BSTR;
        // Reversed order: [0] = pfContinueLogon, [1] = publicKey (binary BSTR).
        let Some(key) = args
            .filter(|p| p.cArgs == 2 && !p.rgvarg.is_null())
            .map(|p| unsafe { &*p.rgvarg.add(1) })
        else {
            return false;
        };
        let hash = unsafe {
            if key.Anonymous.Anonymous.vt != VT_BSTR {
                return false;
            }
            let bstr = &*key.Anonymous.Anonymous.Anonymous.bstrVal;
            let length = windows::Win32::Foundation::SysStringByteLen(bstr) as usize;
            if bstr.is_empty() || length > 16384 {
                return false;
            }
            let bytes = std::slice::from_raw_parts(bstr.as_ptr() as *const u8, length);
            yuuterm_ssh::hex::lower(Sha256::digest(bytes))
        };
        if let Ok(mut buffer) = self.0.lock() {
            buffer.key_sha256 = Some(hash.clone());
        }
        // One shot: any key observation disarms, matching or not.
        let pinned = self
            .1
            .borrow_mut()
            .take()
            .is_some_and(|armed| armed.pins.contains(&hash));
        if let Ok(mut buffer) = self.0.lock() {
            buffer.key_pinned = pinned;
        }
        pinned
    }
}
/// Closes the control's certificate warning. The event can fire before the dialog
/// window exists, so a short watcher keeps posting WM_CLOSE to visible dialogs
/// (#32770) of this process for five seconds. Returns the count found immediately.
fn close_warning_dialogs() -> i32 {
    use windows::Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        System::Threading::GetCurrentProcessId,
        UI::WindowsAndMessaging::{
            EnumWindows, GetClassNameW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW,
            WM_CLOSE,
        },
    };
    unsafe extern "system" fn visit(hwnd: HWND, count: LPARAM) -> windows::core::BOOL {
        let mut pid = 0;
        let mut class = [0u16; 16];
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        let length = unsafe { GetClassNameW(hwnd, &mut class) } as usize;
        if pid == unsafe { GetCurrentProcessId() }
            && unsafe { IsWindowVisible(hwnd) }.as_bool()
            && String::from_utf16_lossy(&class[..length]) == "#32770"
            && unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }.is_ok()
        {
            unsafe { *(count.0 as *mut i32) += 1 };
        }
        true.into()
    }
    let sweep = || {
        let mut count = 0i32;
        unsafe {
            let _ = EnumWindows(Some(visit), LPARAM(&mut count as *mut i32 as isize));
        }
        count
    };
    let found = sweep();
    std::thread::spawn(move || {
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            sweep();
        }
    });
    found
}
impl IMsTscAxEvents_Impl for Sink_Impl {}
#[allow(non_snake_case)]
impl IDispatch_Impl for Sink_Impl {
    fn GetTypeInfoCount(&self) -> Result<u32> {
        Ok(0)
    }
    fn GetTypeInfo(&self, _: u32, _: u32) -> Result<ITypeInfo> {
        Err(Error::from_hresult(windows_core::HRESULT(
            0x80004001u32 as i32,
        )))
    }
    fn GetIDsOfNames(
        &self,
        _: *const GUID,
        _: *const PCWSTR,
        _: u32,
        _: u32,
        _: *mut i32,
    ) -> Result<()> {
        Err(Error::from_hresult(windows_core::HRESULT(
            0x80020006u32 as i32,
        )))
    }
    fn Invoke(
        &self,
        id: i32,
        _: *const GUID,
        _: u32,
        _: DISPATCH_FLAGS,
        params: *const DISPPARAMS,
        _: *mut VARIANT,
        _: *mut EXCEPINFO,
        _: *mut u32,
    ) -> Result<()> {
        // The COM source owns these argument pointers for this synchronous call.
        // Reverse-order dispatch arguments; never parse/copy a server BSTR.
        let args = unsafe { params.as_ref() }.filter(|p| p.cArgs <= 8);
        #[cfg(feature = "rdp-live")]
        eprintln!("rdp-live: event dispid {id}");
        let first = args
            .filter(|p| p.cArgs != 0 && !p.rgvarg.is_null())
            .map(|p| unsafe { &*p.rgvarg });
        let (kind, code) = match id {
            1 => ("connecting", None),
            2 => ("connected", None),
            3 => ("login_complete", None),
            4 => ("disconnected", first.and_then(|v| i32::try_from(v).ok())),
            10 => ("fatal_error", first.and_then(|v| i32::try_from(v).ok())),
            11 => ("warning", first.and_then(|v| i32::try_from(v).ok())),
            16 => {
                // Public-key decisions default to refusal. Only an armed live test
                // whose key matches the confirmed pin continues CredSSP.
                let allow = self.decide(args);
                if let Some(value) = first {
                    unsafe {
                        if value.Anonymous.Anonymous.vt == (VT_BOOL | VT_BYREF) {
                            let out = value.Anonymous.Anonymous.Anonymous.pboolVal;
                            if !out.is_null() {
                                *out = windows::Win32::Foundation::VARIANT_BOOL(if allow {
                                    -1
                                } else {
                                    0
                                });
                            }
                        }
                    }
                }
                (
                    if allow {
                        "public_key_pinned"
                    } else {
                        "public_key_refused"
                    },
                    None,
                )
            }
            18 if self.2 => {
                // Strict probe: never let the user continue past the warning.
                // Closing the dialog cancels the connection.
                let closed = close_warning_dialogs();
                ("authentication_warning_cancelled", Some(closed))
            }
            // The control asks the user, as mstsc does (AuthenticationLevel 2).
            18 => ("authentication_warning", None),
            19 => ("authentication_warning_dismissed", None),
            22 => ("logon_error", first.and_then(|v| i32::try_from(v).ok())),
            23 => ("focus_released", None),
            _ => return Ok(()),
        };
        if let Ok(mut buffer) = self.0.lock() {
            buffer.push(Event { kind, code });
        }
        Ok(())
    }
}

pub struct Subscription {
    point: IConnectionPoint,
    cookie: u32,
    buffer: Arc<Mutex<Buffer>>,
    #[cfg_attr(not(feature = "rdp-live"), allow(dead_code))]
    arming: Arming,
}
impl Subscription {
    pub fn attach(control: &IDispatch, close_warnings: bool) -> Result<Self> {
        let container: IConnectionPointContainer = control.cast()?;
        let point = unsafe { container.FindConnectionPoint(&IMsTscAxEvents::IID)? };
        let buffer = Arc::new(Mutex::new(Buffer::default()));
        let arming = arming();
        let sink: IMsTscAxEvents = Sink(buffer.clone(), arming.clone(), close_warnings).into();
        let unknown: windows_core::IUnknown = sink.cast()?;
        let cookie = unsafe { point.Advise(&unknown)? };
        Ok(Self {
            point,
            cookie,
            buffer,
            arming,
        })
    }
    pub fn snapshot(&self) -> (Vec<Event>, usize) {
        self.buffer
            .lock()
            .map(|b| (b.events.iter().cloned().collect(), b.dropped))
            .unwrap_or_default()
    }
    #[cfg_attr(not(feature = "rdp-probe"), allow(dead_code))]
    pub fn key(&self) -> (Option<String>, bool) {
        self.buffer
            .lock()
            .map(|b| (b.key_sha256.clone(), b.key_pinned))
            .unwrap_or_default()
    }
    #[cfg(feature = "rdp-live")]
    pub fn arm(&self, armed: Armed) {
        *self.arming.borrow_mut() = Some(armed);
    }
}
impl Drop for Subscription {
    fn drop(&mut self) {
        // Stop callbacks before destroying the child/control on its owning STA.
        unsafe {
            let _ = self.point.Unadvise(self.cookie);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_dispatch_sink_exposes_event_iid_and_refuses_public_key_callback() {
        let buffer = Arc::new(Mutex::new(Buffer::default()));
        let sink: IMsTscAxEvents = Sink(buffer.clone(), arming(), true).into();
        let dispatch: IDispatch = sink.cast().unwrap();
        assert!(dispatch.cast::<IMsTscAxEvents>().is_ok());
        let mut decision = windows::Win32::Foundation::VARIANT_BOOL(-1);
        let mut values = [
            VARIANT::default(),
            VARIANT::from("synthetic-key-not-retained"),
        ];
        // [out] comes first in reversed IDispatch argument order.
        unsafe {
            (*values[0].Anonymous.Anonymous).vt = VT_BOOL | VT_BYREF;
            (*values[0].Anonymous.Anonymous).Anonymous.pboolVal = &mut decision;
        }
        let args = DISPPARAMS {
            rgvarg: values.as_mut_ptr(),
            cArgs: 2,
            ..Default::default()
        };
        unsafe {
            dispatch
                .Invoke(
                    16,
                    &GUID::zeroed(),
                    0,
                    DISPATCH_METHOD,
                    &args,
                    None,
                    None,
                    None,
                )
                .unwrap();
        }
        assert_eq!(decision.0, 0);
        let events = &buffer.lock().unwrap().events;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "public_key_refused");
        assert!(events[0].code.is_none());
    }
    #[test]
    fn event_history_is_bounded_and_keeps_latest_error() {
        let mut buffer = Buffer::default();
        for code in 0..100 {
            buffer.push(Event {
                kind: "warning",
                code: Some(code),
            });
        }
        assert_eq!(buffer.events.len(), 64);
        assert_eq!(buffer.dropped, 36);
        assert_eq!(buffer.events.front().unwrap().code, Some(36));
        assert_eq!(buffer.events.back().unwrap().code, Some(99));
    }
}
