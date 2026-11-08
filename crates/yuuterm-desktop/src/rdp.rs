//! Production RDP sessions hosted in the Microsoft Remote Desktop ActiveX control.
//! The endpoint always comes from the saved bookmark, never from IPC. Server
//! verification behaves like mstsc (AuthenticationLevel 2): when Windows cannot
//! verify the certificate, or it changed, the control's own prompt asks the user.
//! A remembered certificate connects without the prompt through `rdp_relay`,
//! which checks it on the authenticating connection.
//! Credentials are requested by the native Windows Security prompt (CredSSP/NLA)
//! and never pass through the WebView.
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, collections::BTreeMap, sync::OnceLock};
use windows::{
    Win32::{
        Foundation::{HMODULE, HWND, RECT},
        System::{
            Com::{
                DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPATCH_PROPERTYPUT, DISPPARAMS, IDispatch,
            },
            LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW},
            Ole::{DISPID_PROPERTYPUT, OleInitialize, OleUninitialize},
            Variant::{VARIANT, VariantClear, VariantToString},
        },
        UI::{Input::KeyboardAndMouse::SetFocus, WindowsAndMessaging::*},
    },
    core::{GUID, HRESULT, IUnknown, Interface, Result, s, w},
};
use yuuterm_core::{Profile, RdpAudio, RdpKeyboard};

#[repr(transparent)]
pub(crate) struct Variant(pub VARIANT);
impl Drop for Variant {
    fn drop(&mut self) {
        unsafe {
            let _ = VariantClear(&mut self.0);
        }
    }
}
pub(crate) fn property(control: &IDispatch, name: windows::core::PCWSTR) -> Result<Variant> {
    let mut id = 0;
    let mut result = Variant(VARIANT::default());
    unsafe {
        control.GetIDsOfNames(&GUID::zeroed(), &name, 1, 0, &mut id)?;
        control.Invoke(
            id,
            &GUID::zeroed(),
            0,
            DISPATCH_PROPERTYGET,
            &DISPPARAMS::default(),
            Some(&mut result.0),
            None,
            None,
        )?;
    }
    Ok(result)
}
pub(crate) fn put(control: &IDispatch, name: windows::core::PCWSTR, value: VARIANT) -> Result<()> {
    let mut id = 0;
    let mut named = DISPID_PROPERTYPUT;
    let mut value = Variant(value);
    let parameters = DISPPARAMS {
        rgvarg: &mut value.0,
        rgdispidNamedArgs: &mut named,
        cArgs: 1,
        cNamedArgs: 1,
    };
    unsafe {
        control.GetIDsOfNames(&GUID::zeroed(), &name, 1, 0, &mut id)?;
        control.Invoke(
            id,
            &GUID::zeroed(),
            0,
            DISPATCH_PROPERTYPUT,
            &parameters,
            None,
            None,
            None,
        )?;
    }
    Ok(())
}
/// Calls a method. `args` are in declaration order; IDispatch wants them reversed.
pub(crate) fn call(
    control: &IDispatch,
    name: windows::core::PCWSTR,
    args: Vec<VARIANT>,
) -> Result<Variant> {
    let mut id = 0;
    let mut args: Vec<Variant> = args.into_iter().rev().map(Variant).collect();
    let mut result = Variant(VARIANT::default());
    let parameters = DISPPARAMS {
        rgvarg: if args.is_empty() {
            std::ptr::null_mut()
        } else {
            // Variant is #[repr(transparent)] over VARIANT.
            args.as_mut_ptr() as *mut VARIANT
        },
        cArgs: args.len() as u32,
        ..Default::default()
    };
    unsafe {
        control.GetIDsOfNames(&GUID::zeroed(), &name, 1, 0, &mut id)?;
        control.Invoke(
            id,
            &GUID::zeroed(),
            0,
            DISPATCH_METHOD,
            &parameters,
            Some(&mut result.0),
            None,
            None,
        )?;
    }
    Ok(result)
}
pub(crate) fn method(control: &IDispatch, name: windows::core::PCWSTR) -> Result<()> {
    call(control, name, Vec::new()).map(drop)
}
pub(crate) fn sub(
    control: &IDispatch,
    name: windows::core::PCWSTR,
) -> std::result::Result<IDispatch, String> {
    property(control, name)
        .and_then(|v| IDispatch::try_from(&v.0))
        .map_err(|e| e.to_string())
}

pub(crate) struct OleApartment;
impl OleApartment {
    pub fn enter() -> std::result::Result<Self, String> {
        unsafe { OleInitialize(None) }.map_err(|e| e.to_string())?;
        Ok(Self)
    }
}
impl Drop for OleApartment {
    fn drop(&mut self) {
        unsafe {
            OleUninitialize();
        }
    }
}

static ATL: OnceLock<std::result::Result<usize, String>> = OnceLock::new();
// The DLL remains loaded because ATL registers window classes.
fn atl() -> std::result::Result<HMODULE, String> {
    ATL.get_or_init(|| unsafe {
        let module = LoadLibraryExW(w!("atl.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)
            .map_err(|e| e.to_string())?;
        let address =
            GetProcAddress(module, s!("AtlAxWinInit")).ok_or("System ATL lacks control hosting")?;
        let init: unsafe extern "system" fn() -> windows::core::BOOL = std::mem::transmute(address);
        if !init().as_bool() {
            return Err("ATL control-host initialization failed".into());
        }
        Ok(module.0 as usize)
    })
    .clone()
    .map(|value| HMODULE(value as *mut _))
}

/// Neither the Tauri window nor the WRY WebView host clips its siblings, so
/// Chromium paints over a native sibling that is above it in z-order. Measured
/// on a live session: the RDP desktop stayed invisible until the parent had
/// WS_CLIPCHILDREN and the WebView host WS_CLIPSIBLINGS. Idempotent.
fn clip_siblings(parent: HWND) {
    unsafe {
        let style = GetWindowLongPtrW(parent, GWL_STYLE);
        if style & WS_CLIPCHILDREN.0 as isize == 0 {
            SetWindowLongPtrW(parent, GWL_STYLE, style | WS_CLIPCHILDREN.0 as isize);
        }
        let mut child = GetWindow(parent, GW_CHILD).ok();
        while let Some(hwnd) = child {
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
            if style & WS_CLIPSIBLINGS.0 as isize == 0 {
                SetWindowLongPtrW(hwnd, GWL_STYLE, style | WS_CLIPSIBLINGS.0 as isize);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
            }
            child = GetWindow(hwnd, GW_HWNDNEXT).ok();
        }
    }
}

/// Creates the ATL host child window and returns the control's IDispatch.
pub(crate) fn create_control(
    parent: HWND,
    bounds: Bounds,
    visible: bool,
) -> std::result::Result<(HWND, IDispatch), String> {
    let module = atl()?;
    clip_siblings(parent);
    let style = WS_CHILD | WS_CLIPSIBLINGS | WS_CLIPCHILDREN | WS_TABSTOP;
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("AtlAxWin"),
            // Observed local version-12 non-scriptable control. No dynamic CLSID input.
            w!("{1DF7C823-B2D4-4B54-975A-F2AC5D7CF8B8}"),
            if visible { style | WS_VISIBLE } else { style },
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            Some(parent),
            None,
            None,
            None,
        )
    }
    .map_err(|e| e.to_string())?;
    let control = (|| -> std::result::Result<IDispatch, String> {
        let address = unsafe { GetProcAddress(module, s!("AtlAxGetControl")) }
            .ok_or("System ATL lacks control retrieval")?;
        let get: unsafe extern "system" fn(HWND, *mut *mut std::ffi::c_void) -> HRESULT =
            unsafe { std::mem::transmute(address) };
        let mut raw = std::ptr::null_mut();
        unsafe {
            get(hwnd, &mut raw).ok().map_err(|e| e.to_string())?;
        }
        if raw.is_null() {
            return Err("ATL returned no RDP control".into());
        }
        let unknown = unsafe { IUnknown::from_raw(raw) };
        unknown.cast().map_err(|e| e.to_string())
    })();
    match control {
        Ok(control) => Ok((hwnd, control)),
        Err(e) => {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            Err(e)
        }
    }
}

/// Renders one owned window (including native children and DirectX content) into
/// RGB pixels. `PrintWindow` with PW_RENDERFULLCONTENT does not depend on what is
/// currently visible on screen.
pub(crate) fn capture_rgb(
    hwnd: HWND,
    limit: i32,
) -> std::result::Result<(u32, u32, Vec<u8>), String> {
    use windows::Win32::{
        Graphics::Gdi::*,
        Storage::Xps::{PRINT_WINDOW_FLAGS, PW_CLIENTONLY, PrintWindow},
    };
    struct Bitmap {
        dc: HDC,
        bitmap: HBITMAP,
        previous: HGDIOBJ,
    }
    impl Drop for Bitmap {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.dc, self.previous);
                let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
                let _ = DeleteDC(self.dc);
            }
        }
    }
    unsafe {
        let mut rect = RECT::default();
        GetClientRect(hwnd, &mut rect).map_err(|e| e.to_string())?;
        let (width, height) = (rect.right, rect.bottom);
        if width <= 0 || height <= 0 || width > limit || height > limit {
            return Err("Capture dimensions exceed bounds".into());
        }
        let dc = CreateCompatibleDC(None);
        if dc.0.is_null() {
            return Err("Capture DC unavailable".into());
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) => bitmap,
            Err(e) => {
                let _ = DeleteDC(dc);
                return Err(e.to_string());
            }
        };
        let previous = SelectObject(dc, HGDIOBJ(bitmap.0));
        let owned = Bitmap {
            dc,
            bitmap,
            previous,
        };
        if bits.is_null() || previous.0.is_null() || previous.0 as isize == -1 {
            return Err("Capture bitmap selection failed".into());
        }
        std::ptr::write_bytes(bits as *mut u8, 0, width as usize * height as usize * 4);
        if !PrintWindow(hwnd, owned.dc, PRINT_WINDOW_FLAGS(PW_CLIENTONLY.0 | 2)).as_bool() {
            return Err("Window capture failed".into());
        }
        let bgra =
            std::slice::from_raw_parts(bits as *const u8, width as usize * height as usize * 4);
        let rgb = bgra
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[2], p[1], p[0]])
            .collect();
        Ok((width as u32, height as u32, rgb))
    }
}

/// A still JPEG of the remote desktop, shown in the WebView while a menu overlaps
/// the pane and the native window must be hidden. Never written to disk.
pub fn snapshot(id: u64) -> std::result::Result<Vec<u8>, String> {
    let hwnd = SESSIONS.with(|slot| {
        slot.borrow()
            .get(&id)
            .filter(|s| s.started)
            .map(|s| s.hwnd)
            .ok_or_else(|| "This RDP session no longer exists".to_owned())
    })?;
    let (width, height, rgb) = capture_rgb(hwnd, 8192)?;
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
        .encode(&rgb, width, height, image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())?;
    Ok(jpeg)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}
impl Bounds {
    pub fn validate(self, width: i32, height: i32) -> std::result::Result<(), String> {
        if self.x < 0
            || self.y < 0
            || self.width < 64
            || self.height < 64
            || self.width > 8192
            || self.height > 8192
            || self.x.checked_add(self.width).is_none_or(|x| x > width)
            || self.y.checked_add(self.height).is_none_or(|y| y > height)
        {
            return Err("RDP rectangle must fit inside the local client area".into());
        }
        Ok(())
    }
}

/// Remote desktop size for a pane: within the control's 200..8192 range and an
/// even width, which dynamic resolution requires.
fn desktop_size(width: i32, height: i32) -> (i32, i32) {
    (width.clamp(200, 8192) & !1, height.clamp(200, 8192))
}
/// Desktop scale (percent) and the matching device scale class (100/140/180).
fn scale_factors(scale: u32) -> (u32, u32) {
    let desktop = scale.clamp(100, 500);
    let device = match desktop {
        ..=124 => 100,
        125..=174 => 140,
        _ => 180,
    };
    (desktop, device)
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Place,
    Hide,
    Focus,
    Status,
    Connect,
    Disconnect,
    Close,
    CtrlAltDel,
}

#[derive(Serialize)]
pub struct Status {
    id: u64,
    #[serde(skip)]
    pub profile_id: u64,
    /// A new saved-password identity the caller must persist in the bookmark.
    #[serde(skip)]
    pub new_credential_id: Option<String>,
    connected: bool,
    finished: bool,
    status: String,
    endpoint: String,
    visible: bool,
    bounds: Option<Bounds>,
    /// Remote desktop size last requested (connect or dynamic resolution).
    remote_size: (i32, i32),
    /// Whether the password came from the saved credential.
    saved_password: bool,
    /// Connected without a prompt because the certificate was remembered.
    certificate_remembered: bool,
    events: Vec<crate::rdp_events::Event>,
}

struct Session {
    hwnd: HWND,
    control: Option<IDispatch>,
    subscription: Option<crate::rdp_events::Subscription>,
    profile: Profile,
    started: bool,
    logged_on: bool,
    /// Last remote size and scale requested, to avoid redundant updates.
    size: (i32, i32, u32),
    bounds: Option<Bounds>,
    /// Remote resolution follows only once the pane stopped resizing.
    placed_at: std::time::Instant,
    /// The WebView asked to hide the control (a dialog covers the pane). The
    /// control can show its window again while connecting, so this is
    /// re-applied after Connect and on every status poll.
    hidden: bool,
    /// The saved password used for this connection, forgotten if it is refused.
    saved: Option<crate::credentials::Target>,
    /// The user cancelled the password prompt; nothing was sent.
    cancelled: bool,
    /// Shutdown already asked this session to disconnect.
    closing: bool,
    /// SHA-256 of the server certificate read before connecting.
    fingerprint: Option<String>,
    /// That certificate was remembered, so the control did not prompt.
    remembered: bool,
    /// Loopback relay that enforces the remembered certificate.
    relay: Option<crate::rdp_relay::Relay>,
    forward: Option<crate::ssh_gateways::Forward>,
    /// When the control first reported Connected, for servers that never send
    /// the Windows logon notification (measured: xrdp).
    connected_at: Option<std::time::Instant>,
    _ole: OleApartment,
}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            if self.started
                && let Some(control) = &self.control
            {
                let _ = method(control, w!("Disconnect"));
            }
            crate::rdp_keys::unregister(self.hwnd);
            // Stop callbacks before destroying the child/control on its owning STA.
            self.subscription.take();
            let _ = DestroyWindow(self.hwnd);
            self.control.take();
        }
    }
}

// Bookmarks whose saved password was just refused; the next prompt says so.

/// OnLogonError codes that refuse the credentials: 0 bad password, 1 password
/// must be updated. Negative codes are winlogon progress notifications (live:
/// -2 "continue logon" precedes every successful Windows 11 logon).
fn logon_refused(code: i32) -> bool {
    matches!(code, 0 | 1)
}

/// Disconnect reasons meaning the server refused the account or password
/// (logon failed, bad user name, password expired, password must change).
fn credentials_refused(reason: i32) -> bool {
    matches!(reason, 2055 | 2567 | 3847 | 6919)
}

#[allow(non_snake_case)]
mod abi {
    use windows_core::{HRESULT, IUnknown, IUnknown_Vtbl, interface};
    // Vtable layout read from the type library of mstscax.dll 10.0.26100:
    // IMsTscNonScriptable slot 3 put_ClearTextPassword(BSTR).
    #[interface("C1E6743A-41C1-4A74-832A-0DD06C1C7A0E")]
    pub unsafe trait IMsTscNonScriptable: IUnknown {
        fn PutClearTextPassword(&self, value: *const u16) -> HRESULT;
    }
    pub fn put_password(control: &IMsTscNonScriptable, value: *const u16) -> HRESULT {
        unsafe { control.PutClearTextPassword(value) }
    }
}
/// IMsRdpClientNonScriptable5 (4F6996D5-D7B1-412C-B0FF-063718566907) slots from
/// the same type library: 15 put_UIParentWindowHandle, 45 put_AllowCredentialSaving,
/// 63 put_AllowPromptingForCredentials, 64 get_AllowPromptingForCredentials.
const NON_SCRIPTABLE5: GUID = GUID::from_u128(0x4f6996d5_d7b1_412c_b0ff_063718566907);

/// Credentials are YuuTerm's job: the control may neither save them (Credential
/// Guard refuses saved RDP credentials) nor open its own prompt. A refused
/// password ends the connection instead, and YuuTerm asks again next time.
fn credential_policy(control: &IDispatch, parent: HWND) -> std::result::Result<(), String> {
    type PutPointer = unsafe extern "system" fn(*mut std::ffi::c_void, isize) -> HRESULT;
    type PutBool = unsafe extern "system" fn(*mut std::ffi::c_void, i16) -> HRESULT;
    type GetBool = unsafe extern "system" fn(*mut std::ffi::c_void, *mut i16) -> HRESULT;
    let mut raw = std::ptr::null_mut();
    unsafe { control.query(&NON_SCRIPTABLE5, &mut raw) }
        .ok()
        .map_err(|e| format!("RDP credential policy unavailable: {e}"))?;
    let unknown = unsafe { IUnknown::from_raw(raw) };
    let this = unknown.as_raw();
    unsafe {
        let vtable = *(this as *const *const usize);
        let slot = |n: usize| *vtable.add(n);
        let put_parent: PutPointer = std::mem::transmute(slot(15));
        let put_saving: PutBool = std::mem::transmute(slot(45));
        let put_prompting: PutBool = std::mem::transmute(slot(63));
        let get_prompting: GetBool = std::mem::transmute(slot(64));
        put_parent(this, parent.0 as isize)
            .ok()
            .map_err(|e| e.to_string())?;
        put_saving(this, 0).ok().map_err(|e| e.to_string())?;
        put_prompting(this, 0).ok().map_err(|e| e.to_string())?;
        let mut value = -1i16;
        get_prompting(this, &mut value)
            .ok()
            .map_err(|e| e.to_string())?;
        if value != 0 {
            return Err("RDP control kept its own credential prompt".into());
        }
    }
    Ok(())
}

/// IMsRdpClientNonScriptable (2F079C4C-87B2-4AFD-97AB-20CDB43038AE), slot 14
/// SendKeys(numKeys, pbArrayKeyUp, plKeyData) from the mstscax type library.
const NON_SCRIPTABLE: GUID = GUID::from_u128(0x2f079c4c_87b2_4afd_97ab_20cdb43038ae);

/// Sends Ctrl+Alt+Del into the session. The local Windows always handles the
/// physical combination itself, so it can only reach the remote this way.
fn send_ctrl_alt_del(control: &IDispatch) -> std::result::Result<(), String> {
    // Scan codes: Ctrl 0x1D, Alt 0x38, Del 0x53; press all, release in reverse.
    send_keys(
        control,
        &[0x1D, 0x38, 0x53, 0x53, 0x38, 0x1D],
        &[false, false, false, true, true, true],
    )
}

/// IMsRdpClientNonScriptable::SendKeys: scan codes (extended keys with 0x100,
/// the RDP KBDFLAGS_EXTENDED bit) and whether each one is a release.
fn send_keys(control: &IDispatch, keys: &[i32], up: &[bool]) -> std::result::Result<(), String> {
    type SendKeys =
        unsafe extern "system" fn(*mut std::ffi::c_void, i32, *const i16, *const i32) -> HRESULT;
    let mut raw = std::ptr::null_mut();
    unsafe { control.query(&NON_SCRIPTABLE, &mut raw) }
        .ok()
        .map_err(|e| format!("RDP key injection unavailable: {e}"))?;
    let unknown = unsafe { IUnknown::from_raw(raw) };
    let this = unknown.as_raw();
    let up: Vec<i16> = up.iter().map(|&u| if u { -1 } else { 0 }).collect();
    unsafe {
        let vtable = *(this as *const *const usize);
        let send: SendKeys = std::mem::transmute(*vtable.add(14));
        // The control answers E_FAIL while it is busy (the first call after a
        // pause nearly always, more under pointer movement); the same call a
        // few milliseconds later is accepted. Without the retry the Windows
        // key reached the local Start menu instead (#35, 6 November 2026).
        let mut attempt = 0;
        loop {
            match send(this, keys.len() as i32, up.as_ptr(), keys.as_ptr()).ok() {
                Ok(()) => return Ok(()),
                Err(e) if attempt >= 20 => return Err(e.to_string()),
                Err(_) => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(3));
                }
            }
        }
    }
}

/// A key the keyboard helper took from the local Windows (rdp_keys), for the
/// session whose control host is `host`, after the held modifier scan codes
/// `mods` are pressed. `start` opens the Start menu: the Windows key alone did
/// not open it through SendKeys, Ctrl+Esc does, while Win as an extended key
/// works in combinations such as Win+R.
pub(crate) fn forward_key(host: HWND, mods: &[i32], scan: i32, up: bool, start: bool) {
    if start {
        // Release Win (pressed in the session), then Ctrl 0x1D + Esc 0x01.
        forward(
            host,
            &[scan, 0x1D, 0x01, 0x01, 0x1D],
            &[true, false, false, true, true],
        );
    } else {
        let keys: Vec<i32> = mods.iter().copied().chain([scan]).collect();
        forward(host, &keys, &vec![up; keys.len()]);
    }
}

/// Presses or releases `keys` in the session of control host `host`.
pub(crate) fn forward_keys(host: HWND, keys: &[i32], up: bool) {
    forward(host, keys, &vec![up; keys.len()]);
}

fn forward(host: HWND, keys: &[i32], up: &[bool]) {
    if keys.is_empty() {
        return;
    }
    let _ = SESSIONS.try_with(|slot| {
        let Ok(sessions) = slot.try_borrow() else {
            return;
        };
        if let Some(control) = sessions
            .values()
            .find(|s| s.hwnd == host && s.started)
            .and_then(|s| s.control.as_ref())
            && let Err(e) = send_keys(control, keys, up)
        {
            eprintln!("rdp: key not forwarded: {e}");
        }
    });
}

/// Focuses the control window.
fn focus(session: &Session) {
    unsafe {
        let _ = SetFocus(Some(session.hwnd));
    }
}

/// Hands the password to the control, which otherwise has none.
pub(crate) fn set_password(control: &IDispatch, password: &str) -> std::result::Result<(), String> {
    let secure: abi::IMsTscNonScriptable = control.cast().map_err(|e| e.to_string())?;
    let bstr = windows_core::BSTR::from(password);
    let result = abi::put_password(&secure, bstr.as_ptr()).ok();
    // Best-effort: clear our BSTR copy before freeing it.
    unsafe { std::ptr::write_bytes(bstr.as_ptr() as *mut u16, 0, bstr.len()) };
    drop(bstr);
    result.map_err(|e| e.to_string())
}

// COM interfaces/HWND never cross threads. Tauri dispatches every call to the
// owning UI STA; the control performs its own network I/O off that thread.
thread_local! { static SESSIONS: RefCell<BTreeMap<u64, Session>> = const { RefCell::new(BTreeMap::new()) }; }
/// UI-thread-only guard for bookmark batch deletion.
pub fn has_profiles(ids: &[u64]) -> bool {
    SESSIONS.with(|slot| slot.borrow().values().any(|s| ids.contains(&s.profile.id)))
}
/// RDP incarnation IDs share the WebView's live-session list with SSH/TELNET/
/// SERIAL (engine, from 1) and VNC (from 2^40). Separate range, below 2^53.
/// Measured: an RDP session and its remote-tools SSH session both had ID 1, so
/// closing the tools also dropped the RDP pane's live entry.
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 41);
const MAX_SESSIONS: usize = 16;

fn apply_policy(control: &IDispatch, profile: &Profile) -> std::result::Result<(), String> {
    let advanced = sub(control, w!("AdvancedSettings9"))?;
    let secured = sub(control, w!("SecuredSettings3"))?;
    let audio = match profile.rdp.audio {
        RdpAudio::Local => 0,
        RdpAudio::Remote => 1,
        RdpAudio::Off => 2,
    };
    let int = VARIANT::from;
    let flag = |v: bool| VARIANT::from(v);
    for (target, name, label, value) in [
        // Warn and let the user decide in the control's prompt, like mstsc.
        (
            &advanced,
            w!("AuthenticationLevel"),
            "AuthenticationLevel",
            int(2),
        ),
        (
            &advanced,
            w!("EnableCredSspSupport"),
            "EnableCredSspSupport",
            flag(true),
        ),
        // No automatic reconnect: a new connection is always an explicit action.
        (
            &advanced,
            w!("EnableAutoReconnect"),
            "EnableAutoReconnect",
            flag(false),
        ),
        (
            &advanced,
            w!("RedirectClipboard"),
            "RedirectClipboard",
            flag(profile.rdp.clipboard),
        ),
        (
            &advanced,
            w!("RedirectDrives"),
            "RedirectDrives",
            flag(false),
        ),
        (
            &advanced,
            w!("RedirectPrinters"),
            "RedirectPrinters",
            flag(false),
        ),
        (&advanced, w!("RedirectPorts"), "RedirectPorts", flag(false)),
        (
            &advanced,
            w!("RedirectSmartCards"),
            "RedirectSmartCards",
            flag(false),
        ),
        (
            &advanced,
            w!("RedirectDevices"),
            "RedirectDevices",
            flag(false),
        ),
        (
            &advanced,
            w!("RedirectPOSDevices"),
            "RedirectPOSDevices",
            flag(false),
        ),
        (
            &advanced,
            w!("AudioCaptureRedirectionMode"),
            "AudioCaptureRedirectionMode",
            flag(false),
        ),
        (
            &advanced,
            w!("RDPPort"),
            "RDPPort",
            int(i32::from(profile.port)),
        ),
        (
            &secured,
            w!("AudioRedirectionMode"),
            "AudioRedirectionMode",
            int(audio),
        ),
        (
            &secured,
            w!("KeyboardHookMode"),
            "KeyboardHookMode",
            // Always 0: the control's own hook let Win reach the local Windows
            // in this host and hid the keys from later hooks (owner trace,
            // 27 October 2026). "Remote" is routed by rdp_keys instead.
            int(0),
        ),
    ] {
        put(target, name, value)
            .map_err(|e| format!("RDP setting {label} could not be set: {e}"))?;
    }
    put(control, w!("Server"), VARIANT::from(profile.host.as_str())).map_err(|e| e.to_string())?;
    if !profile.username.is_empty() {
        put(
            control,
            w!("UserName"),
            VARIANT::from(profile.username.as_str()),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) fn open_via(
    parent: usize,
    profile: Profile,
    forward: Option<crate::ssh_gateways::Forward>,
) -> std::result::Result<u64, String> {
    if profile.protocol != yuuterm_core::Protocol::Rdp {
        return Err("Not an RDP bookmark".into());
    }
    profile.validate()?;
    if !profile.gateways.is_empty() && forward.is_none() {
        return Err("SSH gateway transport is required for this bookmark".into());
    }
    let parent = HWND(parent as *mut _);
    SESSIONS.with(|slot| {
        let mut sessions = slot.borrow_mut();
        if sessions.len() >= MAX_SESSIONS {
            return Err(format!("At most {MAX_SESSIONS} RDP sessions can be open"));
        }
        let ole = OleApartment::enter()?;
        let (hwnd, control) = create_control(
            parent,
            Bounds {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            },
            false,
        )?;
        let mut session = Session {
            hwnd,
            control: None,
            subscription: None,
            profile,
            started: false,
            logged_on: false,
            size: (0, 0, 0),
            bounds: None,
            placed_at: std::time::Instant::now(),
            hidden: true,
            saved: None,
            cancelled: false,
            closing: false,
            fingerprint: None,
            remembered: false,
            relay: None,
            forward,
            connected_at: None,
            _ole: ole,
        };
        apply_policy(&control, &session.profile)?;
        if session.profile.rdp.keyboard == RdpKeyboard::Remote {
            crate::rdp_keys::register(hwnd);
        }
        session.subscription = Some(
            crate::rdp_events::Subscription::attach(&control, false)
                .map_err(|e| format!("RDP event subscription failed: {e}"))?,
        );
        session.control = Some(control);
        let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        sessions.insert(id, session);
        Ok(id)
    })
}

/// Host and port of a session (for the certificate read before connecting).
pub fn endpoint(id: u64) -> Option<(String, u16)> {
    SESSIONS.with(|slot| {
        slot.borrow()
            .get(&id)
            .map(|s| (s.profile.host.clone(), s.profile.port))
    })
}

pub fn transport_endpoint(id: u64) -> Option<(String, u16)> {
    SESSIONS.with(|slot| {
        slot.borrow().get(&id).map(|s| match &s.forward {
            Some(f) => (f.address.ip().to_string(), f.address.port()),
            None => (s.profile.host.clone(), s.profile.port),
        })
    })
}

pub fn close_all() {
    SESSIONS.with(|slot| slot.borrow_mut().clear());
}

/// Starts disconnecting every session that is connecting or connected and has
/// not been asked yet. Destroying a control mid-connection left the process
/// stuck in the kernel on exit (measured twice), so shutdown waits for this.
/// Returns true when at least one disconnect was started now.
pub fn disconnect_all() -> bool {
    SESSIONS.with(|slot| {
        let mut started = false;
        for session in slot.borrow_mut().values_mut() {
            let Some(control) = session.control.as_ref() else {
                continue;
            };
            if session.closing || !session.started || connection_state(control) == 0 {
                continue;
            }
            session.closing = true;
            started |= method(control, w!("Disconnect")).is_ok();
        }
        started
    })
}

/// Whether any control still reports connecting or connected.
pub fn any_connected() -> bool {
    SESSIONS.with(|slot| {
        slot.borrow()
            .values()
            .filter_map(|s| s.control.as_ref())
            .any(|c| connection_state(c) != 0)
    })
}

fn connection_state(control: &IDispatch) -> i32 {
    property(control, w!("Connected"))
        .ok()
        .and_then(|v| i32::try_from(&v.0).ok())
        .unwrap_or(0)
}

pub(crate) struct Certificate {
    pub fingerprint: String,
    pub enforce: bool,
    pub remembered: bool,
}

fn connect(
    parent: HWND,
    id: u64,
    scale: u32,
    certificate: Option<Certificate>,
) -> std::result::Result<Option<String>, String> {
    let profile = SESSIONS.with(|slot| {
        let sessions = slot.borrow();
        let session = sessions
            .get(&id)
            .ok_or("This RDP session no longer exists")?;
        if !session.profile.gateways.is_empty() && session.forward.is_none() {
            return Err("SSH gateway transport ended; reconnect opens a new one");
        }
        if session.started || session.cancelled {
            return Err("This RDP session already connected once; reconnect opens a new one");
        }
        session
            .bounds
            .ok_or("Place the RDP session before connecting")?;
        Ok(session.profile.clone())
    })?;
    let credentials = crate::credentials::resolve(
        parent,
        &profile,
        &format!("{} · Remote desktop", profile.name),
        3389,
    )?;
    SESSIONS.with(|slot| {
        let mut sessions = slot.borrow_mut();
        // The session may have been closed while the prompt was open.
        let session = sessions
            .get_mut(&id)
            .ok_or("This RDP session no longer exists")?;
        let Some(credentials) = credentials else {
            session.cancelled = true;
            return Ok(None);
        };
        let control = session.control.as_ref().unwrap();
        credential_policy(control, parent)?;
        // A remembered, unchanged certificate connects without the prompt
        // (level 0) through the relay, which refuses any other certificate on
        // this very connection; anything else asks as mstsc does (level 2).
        let remembered = certificate.as_ref().is_some_and(|c| c.remembered);
        let enforced = certificate.as_ref().is_some_and(|c| c.enforce);
        let advanced = sub(control, w!("AdvancedSettings9"))?;
        if let Some(certificate) = certificate.as_ref().filter(|c| c.enforce) {
            let relay = crate::rdp_relay::Relay::start(
                session.forward.as_ref().map_or_else(
                    || session.profile.host.clone(),
                    |f| f.address.ip().to_string(),
                ),
                session
                    .forward
                    .as_ref()
                    .map_or(session.profile.port, |f| f.address.port()),
                certificate.fingerprint.clone(),
            )?;
            put(control, w!("Server"), VARIANT::from("127.0.0.1")).map_err(|e| e.to_string())?;
            put(
                &advanced,
                w!("RDPPort"),
                VARIANT::from(i32::from(relay.port())),
            )
            .map_err(|e| e.to_string())?;
            session.relay = Some(relay);
        } else if let Some(forward) = &session.forward {
            put(control, w!("Server"), VARIANT::from("127.0.0.1")).map_err(|e| e.to_string())?;
            put(
                &advanced,
                w!("RDPPort"),
                VARIANT::from(i32::from(forward.address.port())),
            )
            .map_err(|e| e.to_string())?;
        }
        put(
            &advanced,
            w!("AuthenticationLevel"),
            VARIANT::from(if enforced { 0 } else { 2 }),
        )
        .map_err(|e| e.to_string())?;
        session.fingerprint = certificate.map(|c| c.fingerprint);
        session.remembered = remembered;
        put(
            control,
            w!("UserName"),
            VARIANT::from(credentials.username.as_str()),
        )
        .map_err(|e| e.to_string())?;
        set_password(control, credentials.secret.expose_for_authentication())?;
        let bounds = session
            .bounds
            .ok_or("Place the RDP session before connecting")?;
        let (width, height) = desktop_size(bounds.width, bounds.height);
        put(control, w!("DesktopWidth"), VARIANT::from(width)).map_err(|e| e.to_string())?;
        put(control, w!("DesktopHeight"), VARIANT::from(height)).map_err(|e| e.to_string())?;
        session.size = (width, height, scale);
        session.saved = credentials.saved;
        if let Some(identity) = &credentials.new_identity {
            session.profile.credential_id = identity.clone();
        }
        session.started = true;
        method(control, w!("Connect")).map_err(|e| e.to_string())?;
        Ok(credentials.new_identity)
    })
}

/// Forgets a bookmark's saved password (Windows Credential Manager entry).
pub fn forget_password(profile: &Profile) -> std::result::Result<(), String> {
    if let Some(target) = crate::credentials::Target::saved_for(profile)? {
        crate::credentials::forget(&target)?;
    }
    Ok(())
}

/// Asks the server to match the pane after login (dynamic resolution). Servers
/// without the display-control channel keep their size; errors are not fatal.
fn update_resolution(session: &mut Session, scale: u32) {
    let Some(bounds) = session.bounds else {
        return;
    };
    let (width, height) = desktop_size(bounds.width, bounds.height);
    if !session.logged_on || session.size == (width, height, scale) {
        return;
    }
    let (desktop, device) = scale_factors(scale);
    // Physical size in millimetres at the effective DPI.
    let mm = |px: i32| (f64::from(px) * 25.4 / (96.0 * f64::from(desktop) / 100.0)) as u32;
    let control = session.control.as_ref().unwrap();
    let args = [
        width as u32,
        height as u32,
        mm(width),
        mm(height),
        0,
        desktop,
        device,
    ]
    .into_iter()
    .map(VARIANT::from)
    .collect();
    if call(control, w!("UpdateSessionDisplaySettings"), args).is_ok() {
        session.size = (width, height, scale);
    }
}

/// Plain text for the ordinary disconnect reasons. GetErrorDescription answers
/// the local disconnect (1) with "An internal error has occurred", measured live.
fn ordinary_disconnect(reason: i32, extended: i32) -> Option<&'static str> {
    // ExtendedDisconnectReason values from the control's documentation.
    Some(match (reason, extended) {
        (1, _) => "Disconnected",
        (2 | 3, 3) => "The remote session ended after being idle",
        (2 | 3, 5) => "Another connection took over this remote session",
        (2 | 3, 2 | 12) => "Signed out of the remote computer",
        (2, _) => "The remote session was disconnected on the remote computer",
        (3, _) => "The remote computer ended the session",
        _ => return None,
    })
}

fn describe(control: &IDispatch, reason: i32) -> String {
    let extended = property(control, w!("ExtendedDisconnectReason"))
        .ok()
        .and_then(|v| i32::try_from(&v.0).ok())
        .unwrap_or(0);
    if let Some(text) = ordinary_disconnect(reason, extended) {
        return text.into();
    }
    let text = call(
        control,
        w!("GetErrorDescription"),
        vec![VARIANT::from(reason as u32), VARIANT::from(extended as u32)],
    )
    .ok()
    .and_then(|v| {
        let mut buffer = [0u16; 512];
        unsafe { VariantToString(&v.0, &mut buffer) }.ok()?;
        let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..end]))
    })
    .map(|t| t.trim().to_owned())
    .filter(|t| !t.is_empty());
    match text {
        Some(text) => format!("{text} (code {reason})"),
        None => format!("Disconnected (code {reason})"),
    }
}

fn status(id: u64, session: &mut Session) -> std::result::Result<Status, String> {
    let control = session.control.as_ref().unwrap();
    let state = i32::try_from(
        &property(control, w!("Connected"))
            .map_err(|e| e.to_string())?
            .0,
    )
    .map_err(|e| e.to_string())?;
    let (events, _) = session.subscription.as_ref().unwrap().snapshot();
    session.logged_on |= events.iter().any(|e| e.kind == "login_complete");
    if state == 1 {
        let since = *session
            .connected_at
            .get_or_insert_with(std::time::Instant::now);
        // Treat a connection stable for 3 s as logged on for resolution updates.
        session.logged_on |= since.elapsed() >= std::time::Duration::from_secs(3);
    } else {
        session.connected_at = None;
    }
    let ended = events
        .iter()
        .rev()
        .find(|e| matches!(e.kind, "disconnected" | "fatal_error"));
    let finished = session.cancelled || (session.started && state == 0 && ended.is_some());
    let refused = !session.logged_on
        && (events
            .iter()
            .any(|e| e.kind == "logon_error" && e.code.is_some_and(logon_refused))
            || ended.is_some_and(|e| {
                e.kind == "disconnected" && e.code.is_some_and(credentials_refused)
            }));
    if refused {
        // Never retry a refused password: forget it; the next prompt says why.
        if let Some(target) = session.saved.take() {
            let _ = crate::credentials::forget(&target);
        }
        crate::credentials::mark_refused(session.profile.id);
    }
    let relay_refusal = session.relay.as_ref().and_then(|r| r.refusal());
    let status = match ended.filter(|_| finished) {
        _ if session.cancelled => "Sign-in cancelled".into(),
        Some(_) if refused => "The user name or password was not accepted".into(),
        Some(_) if relay_refusal.is_some() => format!(
            "Stopped before sign-in: {}",
            relay_refusal.unwrap_or_default()
        ),
        Some(end) => match (end.kind, end.code) {
            ("disconnected", Some(code)) => describe(control, code),
            (_, Some(code)) => format!("The RDP client stopped with error {code}"),
            _ => "Disconnected".into(),
        },
        None if state == 1 => "Connected".into(),
        None if session.started => "Connecting…".into(),
        None => "Ready to connect".into(),
    };
    if finished {
        session.forward.take();
        session.relay.take();
    }
    let p = &session.profile;
    let user = if p.username.is_empty() {
        String::new()
    } else {
        format!("{}@", p.username)
    };
    let port = if p.port == 3389 {
        String::new()
    } else {
        format!(":{}", p.port)
    };
    Ok(Status {
        id,
        profile_id: session.profile.id,
        new_credential_id: None,
        connected: state == 1,
        finished,
        status,
        endpoint: format!("{user}{}{port}", p.host),
        visible: unsafe { IsWindowVisible(session.hwnd) }.as_bool(),
        bounds: session.bounds,
        remote_size: (session.size.0, session.size.1),
        saved_password: session.saved.is_some(),
        certificate_remembered: session.remembered,
        events,
    })
}

pub fn perform(
    parent: usize,
    id: u64,
    action: Action,
    bounds: Option<Bounds>,
    scale: Option<u32>,
    certificate: Option<Certificate>,
) -> std::result::Result<Option<Status>, String> {
    let parent = HWND(parent as *mut _);
    let scale = scale.unwrap_or(100).clamp(100, 500);
    let mut new_credential_id = None;
    if matches!(action, Action::Connect) {
        new_credential_id = connect(parent, id, scale, certificate)?;
    }
    SESSIONS.with(|slot| {
        let mut sessions = slot.borrow_mut();
        if matches!(action, Action::Close) {
            sessions.remove(&id);
            return Ok(None);
        }
        let session = sessions
            .get_mut(&id)
            .ok_or("This RDP session no longer exists")?;
        match action {
            Action::Place => {
                let bounds = bounds.ok_or("Missing RDP bounds")?;
                let mut client = RECT::default();
                unsafe { GetClientRect(parent, &mut client) }.map_err(|e| e.to_string())?;
                bounds.validate(client.right, client.bottom)?;
                unsafe {
                    SetWindowPos(
                        session.hwnd,
                        Some(HWND_TOP),
                        bounds.x,
                        bounds.y,
                        bounds.width,
                        bounds.height,
                        SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    )
                }
                .map_err(|e| e.to_string())?;
                session.hidden = false;
                if session.bounds != Some(bounds) {
                    session.bounds = Some(bounds);
                    session.placed_at = std::time::Instant::now();
                }
            }
            Action::Hide => {
                session.hidden = true;
                unsafe {
                    let _ = ShowWindow(session.hwnd, SW_HIDE);
                }
            }
            Action::Focus => focus(session),
            Action::CtrlAltDel => {
                if !session.started {
                    return Err("The remote desktop is not connected".into());
                }
                // The button click moved the focus into the WebView; the
                // control drops injected keys while it is not focused.
                focus(session);
                send_ctrl_alt_del(session.control.as_ref().unwrap())?;
            }
            Action::Disconnect => {
                session.forward.take();
                session.relay.take();
                if !session.started {
                    session.cancelled = true;
                }
                if session.started {
                    method(session.control.as_ref().unwrap(), w!("Disconnect"))
                        .map_err(|e| e.to_string())?;
                }
            }
            Action::Status | Action::Close | Action::Connect => {}
        }
        if session.hidden && unsafe { IsWindowVisible(session.hwnd) }.as_bool() {
            unsafe {
                let _ = ShowWindow(session.hwnd, SW_HIDE);
            }
        }
        let mut result = status(id, session)?;
        result.new_credential_id = new_credential_id;
        // Polled status applies the pane size after login once it settled.
        if result.connected
            && matches!(action, Action::Status)
            && session.placed_at.elapsed() >= std::time::Duration::from_millis(400)
        {
            update_resolution(session, scale);
        }
        Ok(Some(result))
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
    #[test]
    fn ordinary_disconnects_get_plain_text_and_errors_fall_through() {
        assert_eq!(ordinary_disconnect(1, 0), Some("Disconnected"));
        assert_eq!(
            ordinary_disconnect(3, 12),
            Some("Signed out of the remote computer")
        );
        assert_eq!(
            ordinary_disconnect(2, 5),
            Some("Another connection took over this remote session")
        );
        assert_eq!(ordinary_disconnect(263, 0), None);
        assert_eq!(ordinary_disconnect(2308, 0), None);
    }
    #[test]
    fn only_account_refusals_forget_a_saved_password() {
        for code in [2055, 2567, 3847, 6919] {
            assert!(credentials_refused(code));
        }
        // Local/remote disconnects, network and certificate failures keep it.
        for code in [1, 2, 3, 263, 516, 1028, 2308] {
            assert!(!credentials_refused(code));
        }
        assert!(logon_refused(0) && logon_refused(1));
        for code in [-7, -6, -5, -3, -2, -1, 2, 3] {
            assert!(!logon_refused(code), "{code}");
        }
    }
    #[test]
    fn desktop_size_is_bounded_and_even() {
        assert_eq!(desktop_size(1001, 701), (1000, 701));
        assert_eq!(desktop_size(64, 64), (200, 200));
        assert_eq!(desktop_size(20000, 9000), (8192, 8192));
    }
    #[test]
    fn scale_factors_follow_windows_device_classes() {
        assert_eq!(scale_factors(50), (100, 100));
        assert_eq!(scale_factors(125), (125, 140));
        assert_eq!(scale_factors(150), (150, 140));
        assert_eq!(scale_factors(200), (200, 180));
        assert_eq!(scale_factors(900), (500, 180));
    }
}
