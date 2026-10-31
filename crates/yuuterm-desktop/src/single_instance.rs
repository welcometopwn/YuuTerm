//! Acquire before any settings, trust store, backend or WebView is opened.
use std::{
    io,
    path::Path,
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{CloseHandle, FILETIME, HWND},
    System::Threading::{
        AttachThreadInput, GetCurrentThreadId, GetProcessTimes, OpenProcess,
        PROCESS_QUERY_LIMITED_INFORMATION,
    },
    UI::WindowsAndMessaging::{
        AllowSetForegroundWindow, GetForegroundWindow, GetWindowThreadProcessId, IsIconic,
        MB_ICONERROR, MB_OK, MSG, MessageBoxW, PM_NOREMOVE, PeekMessageW, SW_RESTORE,
        SetForegroundWindow, ShowWindowAsync,
    },
};
use yuuterm_core::instance::{self, Lease};

fn creation(pid: u32) -> Option<u64> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        let result = GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user);
        let _ = CloseHandle(process);
        result.ok()?;
        Some((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }
}

pub fn publish(
    lease: &mut Lease,
    window: &tauri::WebviewWindow,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut record = [0; 32];
    record[..8].copy_from_slice(b"YuuTerm1");
    record[8..12].copy_from_slice(&std::process::id().to_le_bytes());
    record[16..24].copy_from_slice(&(window.hwnd()?.0 as usize as u64).to_le_bytes());
    record[24..].copy_from_slice(
        &creation(std::process::id())
            .ok_or("Cannot identify owning process")?
            .to_le_bytes(),
    );
    lease.publish(&record)?;
    Ok(())
}

fn activate(record: [u8; 32]) -> bool {
    if &record[..8] != b"YuuTerm1" {
        return false;
    }
    let pid = u32::from_le_bytes(record[8..12].try_into().unwrap());
    let created = u64::from_le_bytes(record[24..].try_into().unwrap());
    let hwnd = HWND(u64::from_le_bytes(record[16..24].try_into().unwrap()) as usize as *mut _);
    unsafe {
        let mut window_pid = 0;
        let owner_thread = GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
        // A persisted HWND/PID can be reused after a crash. Check process birth too.
        if window_pid != pid || creation(pid) != Some(created) {
            return false;
        }
        let _ = AllowSetForegroundWindow(pid);
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindowAsync(hwnd, SW_RESTORE);
        }
        if !SetForegroundWindow(hwnd).as_bool() {
            // A duplicate launched by a background shell may not own foreground
            // permission. Temporarily join that input queue, without injecting
            // keystrokes or changing the system foreground-lock timeout.
            let foreground = GetWindowThreadProcessId(GetForegroundWindow(), None);
            let current = GetCurrentThreadId();
            // Secondary startup has not created a Tauri event loop/message queue.
            let _ = PeekMessageW(&mut MSG::default(), None, 0, 0, PM_NOREMOVE);
            let attached_owner =
                owner_thread != current && AttachThreadInput(current, owner_thread, true).as_bool();
            if foreground != 0
                && foreground != current
                && AttachThreadInput(current, foreground, true).as_bool()
            {
                let _ = SetForegroundWindow(hwnd);
                let _ = AttachThreadInput(current, foreground, false);
            }
            if attached_owner {
                let _ = AttachThreadInput(current, owner_thread, false);
            }
        }
    }
    true
}

/// Secondary startup waits at most eight seconds for the primary's window.
/// Launch arguments are deliberately not forwarded (no implicit connections).
pub fn enter(settings: &Path) -> io::Result<Option<Lease>> {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(lease) = instance::acquire(settings)? {
            return Ok(Some(lease));
        }
        if let Ok(record) = instance::read_record(settings)
            && activate(record)
        {
            return Ok(None);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::other(
                "This data folder is already in use, but its window is not available. Wait for startup/exit or close the older YuuTerm instance. A second settings writer was blocked.",
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn report(error: impl std::fmt::Display) {
    let message: Vec<u16> = format!("YuuTerm could not start: {error}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    unsafe {
        MessageBoxW(
            None,
            windows::core::PCWSTR(message.as_ptr()),
            windows::core::w!("YuuTerm"),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_and_stale_records_never_activate() {
        assert!(!activate([0; 32]));
        let mut record = [0; 32];
        record[..8].copy_from_slice(b"YuuTerm1");
        record[8..12].copy_from_slice(&std::process::id().to_le_bytes());
        record[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(!activate(record));
    }
}
