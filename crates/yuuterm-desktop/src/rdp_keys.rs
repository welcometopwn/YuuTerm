//! Windows key combinations for RDP sessions set to "on the remote computer".
//!
//! The control's own hook (KeyboardHookMode 1) let Win and Alt+Tab reach the
//! local Windows in this host. A
//! low-level hook inside YuuTerm's process was not called while a YuuTerm
//! window was in the foreground, on a dedicated thread and reinstalled every
//! second alike, while a hook in another process saw every key. So a helper
//! process (this executable with `--rdp-key-hook <pid>`) holds the hook. While
//! YuuTerm is the foreground application and the focus is inside a control host
//! marked by `register`, it takes those combinations and posts them to a
//! receiver window on YuuTerm's UI thread, which sends them into the session
//! with the control's SendKeys (rdp::forward_key). Posting key messages to the
//! control's input window was ignored for Win. The helper ends with YuuTerm.
use std::sync::{
    Once,
    atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WAIT_OBJECT_0, WPARAM},
        System::{
            LibraryLoader::GetModuleHandleW,
            Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        },
        UI::{
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE, VK_LCONTROL, VK_LMENU,
                VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
                VK_TAB,
            },
            WindowsAndMessaging::*,
        },
    },
    core::w,
};

/// Command-line switch that runs the helper instead of the application.
pub const HELPER_ARG: &str = "--rdp-key-hook";
/// Window property on control hosts whose combinations go to the remote.
const PROPERTY: windows::core::PCWSTR = w!("YuuTermRdpRemoteKeys");
static HELPER: Once = Once::new();
/// YuuTerm's process ID, in the helper.
static PARENT: AtomicU32 = AtomicU32::new(0);
/// The Windows key is down and no other key was pressed since.
static WIN_ALONE: AtomicBool = AtomicBool::new(false);
/// Modifiers (MOD_* bits) sent down with a forwarded key and not released yet.
static MODS_SENT: AtomicU32 = AtomicU32::new(0);

/// Message the helper posts to the receiver: wParam the control host, lParam
/// the forwarded key (`key_data`) with these flags.
const WM_FORWARD_KEY: u32 = WM_APP + 0x79;
const KEY_UP: u32 = 0x1_0000;
/// The Windows key pressed alone: open the Start menu.
const START: u32 = 0x2_0000;
/// Release the MOD_* modifiers instead of sending a key.
const MODS_UP: u32 = 0x4_0000;
const MOD_CTRL: u32 = 0x8_0000;
const MOD_SHIFT: u32 = 0x10_0000;
const MOD_ALT: u32 = 0x20_0000;

thread_local! {
    /// Message-only window on the UI thread that hands forwarded keys to the
    /// session's control (its COM interfaces live on this thread).
    static RECEIVER: HWND = receiver();
}

fn receiver() -> HWND {
    unsafe {
        let class = WNDCLASSW {
            lpfnWndProc: Some(receive),
            lpszClassName: w!("YuuTermRdpKeys"),
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("YuuTermRdpKeys"),
            None,
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            None,
            None,
        )
        .unwrap_or_default()
    }
}

unsafe extern "system" fn receive(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_FORWARD_KEY {
        let host = HWND(wparam.0 as *mut _);
        let data = lparam.0 as u32;
        // Left Ctrl 0x1D, Shift 0x2A, Alt 0x38.
        let mods: Vec<i32> = [(MOD_CTRL, 0x1D), (MOD_SHIFT, 0x2A), (MOD_ALT, 0x38)]
            .into_iter()
            .filter(|(bit, _)| data & bit != 0)
            .map(|(_, scan)| scan)
            .collect();
        if data & MODS_UP != 0 {
            crate::rdp::forward_keys(host, &mods, true);
        } else {
            crate::rdp::forward_key(
                host,
                &mods,
                (data & 0xFFFF) as i32,
                data & KEY_UP != 0,
                data & START != 0,
            );
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// Routes the combinations of this control host to the remote computer. The
/// property holds the receiver window the helper posts the keys to.
pub fn register(host: HWND) {
    let receiver = RECEIVER.with(|r| *r);
    if receiver.is_invalid() {
        eprintln!("rdp: keyboard receiver unavailable; Windows key combinations stay local");
        return;
    }
    unsafe {
        let _ = SetPropW(host, PROPERTY, Some(HANDLE(receiver.0)));
    }
    HELPER.call_once(|| {
        let started = std::env::current_exe().and_then(|exe| {
            std::process::Command::new(exe)
                .arg(HELPER_ARG)
                .arg(std::process::id().to_string())
                .spawn()
        });
        if let Err(e) = started {
            eprintln!(
                "rdp: keyboard helper unavailable ({e}); Windows key combinations stay local"
            );
        }
    });
}

pub fn unregister(host: HWND) {
    unsafe {
        let _ = RemovePropW(host, PROPERTY);
    }
}

/// The helper: hooks the keyboard until YuuTerm's process `parent` ends.
pub fn run_helper(parent: &str) {
    let Ok(parent) = parent.parse::<u32>() else {
        return;
    };
    PARENT.store(parent, Ordering::Relaxed);
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_SYNCHRONIZE, false, parent) else {
            return;
        };
        PARENT_HANDLE.store(process.0 as usize, Ordering::Relaxed);
        rehook();
        if HOOK.load(Ordering::Relaxed) != 0 {
            let _ = SetTimer(None, 0, 500, Some(parent_check));
            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let _ = CloseHandle(process);
    }
}

static PARENT_HANDLE: AtomicUsize = AtomicUsize::new(0);
/// The helper's installed hook.
static HOOK: AtomicUsize = AtomicUsize::new(0);

/// Installs the hook again so it is the newest. The RDP control installs its
/// own hook when it connects or gets the focus; Windows calls the newest hook
/// first, and the control's did not pass the keys on (27 October 2026).
fn rehook() {
    unsafe {
        let Ok(new) = GetModuleHandleW(None).and_then(|module| {
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), Some(module.into()), 0)
        }) else {
            return;
        };
        let old = HOOK.swap(new.0 as usize, Ordering::Relaxed);
        if old != 0 {
            let _ = UnhookWindowsHookEx(HHOOK(old as *mut _));
        }
    }
}

/// Every 500 ms: ends with YuuTerm, and stays the newest hook while YuuTerm is
/// the foreground application.
unsafe extern "system" fn parent_check(_: HWND, _: u32, _: usize, _: u32) {
    let process = HANDLE(PARENT_HANDLE.load(Ordering::Relaxed) as *mut _);
    if unsafe { WaitForSingleObject(process, 0) } == WAIT_OBJECT_0 {
        unsafe { PostQuitMessage(0) };
        return;
    }
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid)) };
    if pid == PARENT.load(Ordering::Relaxed) {
        rehook();
    }
}

/// The marked control host holding the focus and its receiver window, while
/// YuuTerm is the foreground application.
fn target() -> Option<(HWND, HWND)> {
    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.is_invalid() {
            return None;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(foreground, Some(&mut pid));
        if pid != PARENT.load(Ordering::Relaxed) {
            return None;
        }
        let mut info = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        // The control's input window (IHWindowClass) runs on its own thread;
        // thread 0 is the foreground input queue.
        GetGUIThreadInfo(0, &mut info).ok()?;
        let mut window = info.hwndFocus;
        while !window.is_invalid() {
            let receiver = GetPropW(window, PROPERTY);
            if !receiver.is_invalid() {
                return Some((window, HWND(receiver.0)));
            }
            window = GetParent(window).unwrap_or_default();
        }
        None
    }
}

/// Whether this key belongs to a combination the local Windows would take.
fn taken(vk: VIRTUAL_KEY, alt: bool, ctrl: bool) -> bool {
    matches!(vk, VK_LWIN | VK_RWIN)
        || (alt && matches!(vk, VK_TAB | VK_ESCAPE))
        || (ctrl && vk == VK_ESCAPE)
}

/// Forwarded key: scan code, 0x100 for extended keys, KEY_UP for a release.
fn key_data(scan: u32, extended: bool, up: bool) -> u32 {
    let mut data = scan & 0xFF;
    if extended {
        data |= 0x100;
    }
    if up {
        data |= KEY_UP;
    }
    data
}

/// The MOD_* bit of a modifier key, if it is one.
fn modifier(vk: VIRTUAL_KEY) -> u32 {
    match vk {
        VK_LCONTROL | VK_RCONTROL | VK_CONTROL => MOD_CTRL,
        VK_LSHIFT | VK_RSHIFT | VK_SHIFT => MOD_SHIFT,
        VK_LMENU | VK_RMENU | VK_MENU => MOD_ALT,
        _ => 0,
    }
}

/// Modifiers held while a key is pressed.
fn held(alt: bool) -> u32 {
    let down = |vk: VIRTUAL_KEY| unsafe { GetAsyncKeyState(vk.0 as i32) } < 0;
    let mut mods = 0;
    if down(VK_CONTROL) {
        mods |= MOD_CTRL;
    }
    if down(VK_SHIFT) {
        mods |= MOD_SHIFT;
    }
    if alt {
        mods |= MOD_ALT;
    }
    mods
}

fn post(receiver: HWND, host: HWND, data: u32) -> bool {
    unsafe {
        PostMessageW(
            Some(receiver),
            WM_FORWARD_KEY,
            WPARAM(host.0 as usize),
            LPARAM(data as isize),
        )
    }
    .is_ok()
}

unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32
        && let Some((host, receiver)) = target()
    {
        let key = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let vk = VIRTUAL_KEY(key.vkCode as u16);
        let alt = key.flags.0 & LLKHF_ALTDOWN.0 != 0;
        let ctrl = unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) } < 0;
        let up = key.flags.0 & LLKHF_UP.0 != 0;
        let win = matches!(vk, VK_LWIN | VK_RWIN);
        if win && !up {
            WIN_ALONE.store(true, Ordering::Relaxed);
        } else if !win && !up {
            WIN_ALONE.store(false, Ordering::Relaxed);
        }
        // The held modifiers reach the session through the normal input path,
        // which did not keep them in order with SendKeys: Ctrl+Shift+Esc and
        // Alt+Tab did nothing (28 October 2026). So they are sent along with
        // the key, and released again when the physical key is released.
        let released = modifier(vk) & MODS_SENT.load(Ordering::Relaxed);
        if up && released != 0 {
            MODS_SENT.fetch_and(!released, Ordering::Relaxed);
            post(receiver, host, MODS_UP | released);
        }
        if taken(vk, alt, ctrl) {
            let extended = key.flags.0 & LLKHF_EXTENDED.0 != 0;
            let mut data = key_data(key.scanCode, extended, up);
            if win && up && WIN_ALONE.swap(false, Ordering::Relaxed) {
                data |= START;
            }
            if !up && !win {
                let mods = held(alt);
                MODS_SENT.fetch_or(mods, Ordering::Relaxed);
                data |= mods;
            }
            if post(receiver, host, data) {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_system_combinations_are_taken() {
        assert!(taken(VK_LWIN, false, false));
        assert!(taken(VK_RWIN, false, false));
        assert!(taken(VK_TAB, true, false));
        assert!(taken(VK_ESCAPE, true, false));
        assert!(taken(VK_ESCAPE, false, true));
        assert!(!taken(VK_TAB, false, false));
        assert!(!taken(VK_ESCAPE, false, false));
        assert!(!taken(VIRTUAL_KEY(b'R' as u16), false, false));
    }
    #[test]
    fn forwarded_keys_carry_scan_code_and_flags() {
        // Left Windows key: scan 0x5B, extended.
        assert_eq!(key_data(0x5B, true, false), 0x15B);
        assert_eq!(key_data(0x5B, true, true), 0x1_015B);
        // Tab.
        assert_eq!(key_data(0x0F, false, false), 0x0F);
    }
    #[test]
    fn modifier_keys_map_to_their_bits() {
        assert_eq!(modifier(VK_LCONTROL), MOD_CTRL);
        assert_eq!(modifier(VK_RSHIFT), MOD_SHIFT);
        assert_eq!(modifier(VK_LMENU), MOD_ALT);
        assert_eq!(modifier(VK_TAB), 0);
        // Flags stay clear of scan code and extended bit.
        assert_eq!(
            (MOD_CTRL | MOD_SHIFT | MOD_ALT | MODS_UP | START | KEY_UP) & 0xFFFF,
            0
        );
    }
}
