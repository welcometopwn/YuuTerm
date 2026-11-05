//! Local terminal: a fixed Windows shell in a pseudo console (ConPTY).
//!
//! No polling: one thread blocks on the console output pipe, commands arrive
//! through the session channel, and a second blocking wait watches the shell
//! process. Dropping the session terminates the shell and closes the console
//! on a helper thread (ClosePseudoConsole may wait for output to drain).
use crate::{Command, Snapshot, Wake, terminal::Terminal};
use anyhow::{Context, Result};
use std::{
    ffi::c_void,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use windows::{
    Win32::{
        Foundation::{CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, WAIT_OBJECT_0},
        Storage::FileSystem::{ReadFile, WriteFile},
        System::{
            Console::{COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole},
            Pipes::CreatePipe,
            Threading::{
                CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
                EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetExitCodeProcess, INFINITE,
                InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
                STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
            },
        },
    },
    core::{PCWSTR, PWSTR},
};
use yuuterm_core::{LocalShell, Profile};

/// An owned Win32 handle that may move between threads.
struct Owned(HANDLE);
unsafe impl Send for Owned {}
unsafe impl Sync for Owned {}
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

struct Console(HPCON);
unsafe impl Send for Console {}
unsafe impl Sync for Console {}

struct Pty {
    console: Console,
    process: Owned,
    input: Mutex<Owned>,
}

impl Pty {
    fn write(&self, mut bytes: &[u8]) -> Result<()> {
        let input = self.input.lock().unwrap_or_else(|e| e.into_inner());
        while !bytes.is_empty() {
            let mut written = 0;
            unsafe { WriteFile(input.0, Some(bytes), Some(&mut written), None) }
                .context("Writing to the local shell failed")?;
            anyhow::ensure!(written > 0, "The local shell stopped reading input");
            bytes = &bytes[written as usize..];
        }
        Ok(())
    }
    fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        unsafe { ResizePseudoConsole(self.console.0, size(rows, cols)) }
            .context("Resizing the local console failed")
    }
    fn terminate(&self) {
        let _ = unsafe { TerminateProcess(self.process.0, 1) };
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.terminate();
        let console = Console(self.console.0);
        // Never block the async runtime on the console's final flush.
        std::thread::spawn(move || unsafe { ClosePseudoConsole(console.0) });
    }
}

fn size(rows: u16, cols: u16) -> COORD {
    COORD {
        X: cols.clamp(2, 1000) as i16,
        Y: rows.clamp(1, 1000) as i16,
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Absolute path of a fixed system shell; never a bookmark-supplied command.
pub fn shell_path(shell: LocalShell) -> Result<PathBuf> {
    let system = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32");
    let path = match shell {
        LocalShell::Cmd => system.join("cmd.exe"),
        LocalShell::WindowsPowerShell => system.join(r"WindowsPowerShell\v1.0\powershell.exe"),
        LocalShell::PowerShell => {
            let installed = std::env::var_os("ProgramFiles")
                .map(|p| PathBuf::from(p).join(r"PowerShell\7\pwsh.exe"));
            installed
                .filter(|p| p.is_file())
                .or_else(|| {
                    std::env::split_paths(&std::env::var_os("PATH")?)
                        .map(|dir| dir.join("pwsh.exe"))
                        .find(|p| p.is_absolute() && p.exists())
                })
                .context("PowerShell 7 is not installed; choose Windows PowerShell instead")?
        }
    };
    anyhow::ensure!(path.exists(), "{} is not available", shell.label());
    Ok(path)
}

fn start_directory(directory: &str) -> PathBuf {
    let wanted = Path::new(directory);
    if !directory.is_empty() && wanted.is_dir() {
        return wanted.to_path_buf();
    }
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from(r"C:\"))
}

/// Starts the shell; returns the console and the read end of its output.
fn spawn(shell: LocalShell, directory: &str, rows: u16, cols: u16) -> Result<(Pty, Owned)> {
    let program = shell_path(shell)?;
    let cwd = start_directory(directory);
    let (mut in_read, mut in_write) = (HANDLE::default(), HANDLE::default());
    let (mut out_read, mut out_write) = (HANDLE::default(), HANDLE::default());
    unsafe { CreatePipe(&mut in_read, &mut in_write, None, 0) }.context("Console pipe failed")?;
    let (in_read, in_write) = (Owned(in_read), Owned(in_write));
    unsafe { CreatePipe(&mut out_read, &mut out_write, None, 0) }.context("Console pipe failed")?;
    let (out_read, out_write) = (Owned(out_read), Owned(out_write));
    let console = Console(
        unsafe { CreatePseudoConsole(size(rows, cols), in_read.0, out_write.0, 0) }
            .context("Creating the pseudo console failed")?,
    );
    // From here the Pty owns the console; a failed start closes it again.
    let mut pty = Pty {
        console,
        process: Owned(HANDLE::default()),
        input: Mutex::new(in_write),
    };
    let mut bytes = 0usize;
    let _ = unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut bytes) };
    let mut list = vec![0u8; bytes];
    let attributes = LPPROC_THREAD_ATTRIBUTE_LIST(list.as_mut_ptr().cast());
    unsafe { InitializeProcThreadAttributeList(Some(attributes), 1, None, &mut bytes) }
        .context("Console attributes failed")?;
    struct List(LPPROC_THREAD_ATTRIBUTE_LIST);
    impl Drop for List {
        fn drop(&mut self) {
            unsafe { DeleteProcThreadAttributeList(self.0) };
        }
    }
    let attributes = List(attributes);
    unsafe {
        UpdateProcThreadAttribute(
            attributes.0,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
            Some(pty.console.0.0 as *const c_void),
            size_of::<HPCON>(),
            None,
            None,
        )
    }
    .context("Console attribute failed")?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    // No inherited standard handles: the console is the shell's only I/O.
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.lpAttributeList = attributes.0;
    let application = wide(&program.to_string_lossy());
    let mut command_line = wide(&match shell {
        LocalShell::Cmd => format!("\"{}\"", program.display()),
        _ => format!("\"{}\" -NoLogo", program.display()),
    });
    let directory = wide(&cwd.to_string_lossy());
    let mut info = PROCESS_INFORMATION::default();
    unsafe {
        CreateProcessW(
            PCWSTR(application.as_ptr()),
            Some(PWSTR(command_line.as_mut_ptr())),
            None,
            None,
            false,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            None,
            PCWSTR(directory.as_ptr()),
            &startup.StartupInfo,
            &mut info,
        )
    }
    .with_context(|| format!("Starting {} failed", shell.label()))?;
    drop(Owned(info.hThread));
    pty.process = Owned(info.hProcess);
    // The console holds its own ends; ours would keep the pipes open forever.
    drop((in_read, out_write));
    drop(attributes);
    Ok((pty, out_read))
}

fn duplicate(handle: &Owned) -> Result<Owned> {
    let mut copy = HANDLE::default();
    unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            handle.0,
            GetCurrentProcess(),
            &mut copy,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
    }
    .context("Watching the local shell failed")?;
    Ok(Owned(copy))
}

/// Blocks until the console's output pipe closes. Before cancellation, output
/// goes to the UI; afterwards it is drained and discarded so the console can close.
fn read_output(
    pipe: Owned,
    state: watch::Sender<Snapshot>,
    wake: Wake,
    output: Option<mpsc::Sender<Vec<u8>>>,
    cancel: CancellationToken,
) {
    let mut terminal = Terminal::new();
    let mut buffer = vec![0u8; 16384];
    loop {
        let mut read = 0;
        if unsafe { ReadFile(pipe.0, Some(&mut buffer), Some(&mut read), None) }.is_err()
            || read == 0
        {
            return;
        }
        if cancel.is_cancelled() {
            continue;
        }
        let bytes = &buffer[..read as usize];
        if let Some(output) = &output {
            let mut chunk = bytes.to_vec();
            // Bounded channel: back-pressure the console while the UI catches up.
            loop {
                match output.try_send(chunk) {
                    Ok(()) => break,
                    Err(mpsc::error::TrySendError::Full(c)) if !cancel.is_cancelled() => {
                        chunk = c;
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        } else {
            terminal.process(bytes);
            state.send_modify(|s| s.terminal = terminal.view());
        }
        wake();
    }
}

pub async fn run(
    profile: Profile,
    mut commands: mpsc::Receiver<Command>,
    state: watch::Sender<Snapshot>,
    wake: Wake,
    output: Option<mpsc::Sender<Vec<u8>>>,
    cancel: CancellationToken,
) -> Result<()> {
    let shell = profile.local.shell;
    let directory = profile.local.directory.clone();
    let (pty, pipe) = tokio::task::spawn_blocking(move || spawn(shell, &directory, 24, 80))
        .await
        .context("Local shell start failed")??;
    let pty = Arc::new(pty);
    // A separate handle for the waiter, so it never keeps the console alive.
    let waiter = duplicate(&pty.process)?;
    {
        let (state, wake, cancel) = (state.clone(), wake.clone(), cancel.clone());
        std::thread::Builder::new()
            .name("yuuterm-local-output".into())
            .spawn(move || read_output(pipe, state, wake, output, cancel))
            .context("Local output reader failed to start")?;
    }
    state.send_modify(|s| {
        if !s.finished {
            s.connected = true;
            s.status = format!("LOCAL · {}", shell.label());
        }
    });
    wake();
    let exited = tokio::task::spawn_blocking(move || {
        let waiter = waiter; // the whole owner moves, not its raw handle
        if unsafe { WaitForSingleObject(waiter.0, INFINITE) } != WAIT_OBJECT_0 {
            return None;
        }
        let mut code = 0u32;
        unsafe { GetExitCodeProcess(waiter.0, &mut code) }.ok()?;
        Some(code)
    });
    tokio::pin!(exited);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            code = &mut exited => {
                return match code.ok().flatten() {
                    Some(0) | None => Ok(()),
                    Some(code) => anyhow::bail!("{} exited with code {code}", shell.label()),
                };
            }
            command = commands.recv() => match command {
                Some(Command::Input(bytes)) => {
                    let pty = pty.clone();
                    tokio::task::spawn_blocking(move || pty.write(&bytes))
                        .await
                        .context("Local input failed")??;
                }
                Some(Command::Resize { rows, cols }) => pty.resize(rows, cols)?,
                Some(_) => {}
                None => return Ok(()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> watch::Sender<Snapshot> {
        watch::channel(Snapshot {
            #[cfg(feature = "startup-probe")]
            startup: Default::default(),
            protocol: yuuterm_core::Protocol::Local,
            endpoint: "Command Prompt".into(),
            status: String::new(),
            connected: false,
            finished: false,
            password_rejected: false,
            rejected_gateway: None,
            trust: None,
            terminal: Terminal::new().view(),
            path: String::new(),
            files: Arc::new(Vec::new()),
            files_status: "Unavailable".into(),
        })
        .0
    }

    #[test]
    fn shells_are_fixed_system_paths() {
        let cmd = shell_path(LocalShell::Cmd).unwrap();
        assert!(cmd.is_absolute() && cmd.ends_with("cmd.exe"));
        let ps = shell_path(LocalShell::WindowsPowerShell).unwrap();
        assert!(ps.ends_with(r"WindowsPowerShell\v1.0\powershell.exe"));
        let fallback = start_directory(r"Z:\does\not\exist");
        assert!(
            fallback.is_dir(),
            "a missing folder falls back to the user folder"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cmd_echoes_through_the_pseudo_console_and_stops_on_cancel() {
        let profile = Profile {
            protocol: yuuterm_core::Protocol::Local,
            local: yuuterm_core::LocalSettings {
                shell: LocalShell::Cmd,
                directory: String::new(),
            },
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel(8);
        let (out_tx, mut out_rx) = mpsc::channel(64);
        let state = state();
        let cancel = CancellationToken::new();
        let wake: Wake = Arc::new(|| {});
        let task = tokio::spawn(run(profile, rx, state, wake, Some(out_tx), cancel.clone()));
        tx.send(Command::Resize {
            rows: 30,
            cols: 100,
        })
        .await
        .unwrap();
        tx.send(Command::Input(
            b"echo yuuterm-local-%COMPUTERNAME:~0,0%ok\r".to_vec(),
        ))
        .await
        .unwrap();
        let mut seen = String::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while !seen.contains("yuuterm-local-ok") {
            let chunk = tokio::time::timeout_at(deadline, out_rx.recv())
                .await
                .expect("shell output within 20 s")
                .expect("output open");
            seen.push_str(&String::from_utf8_lossy(&chunk));
        }
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("cancel ends the session")
            .unwrap()
            .unwrap();
    }

    /// PowerShell 7 is optional on Windows; when it resolves, it must start.
    #[tokio::test(flavor = "multi_thread")]
    async fn powershell_7_starts_when_installed() {
        let Ok(path) = shell_path(LocalShell::PowerShell) else {
            eprintln!("PowerShell 7 not installed; skipped");
            return;
        };
        eprintln!("PowerShell 7 at {}", path.display());
        let (pty, _pipe) = spawn(LocalShell::PowerShell, "", 24, 80).unwrap();
        pty.write(b"exit 5\r").unwrap();
        let waiter = duplicate(&pty.process).unwrap();
        let code = tokio::task::spawn_blocking(move || {
            let waiter = waiter;
            unsafe { WaitForSingleObject(waiter.0, 20_000) };
            let mut code = 0;
            unsafe { GetExitCodeProcess(waiter.0, &mut code) }.unwrap();
            code
        })
        .await
        .unwrap();
        assert_eq!(code, 5);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exit_ends_the_session_with_the_shell_exit_code() {
        let profile = Profile {
            protocol: yuuterm_core::Protocol::Local,
            local: yuuterm_core::LocalSettings {
                shell: LocalShell::Cmd,
                directory: String::new(),
            },
            ..Default::default()
        };
        let (tx, rx) = mpsc::channel(8);
        let (out_tx, mut out_rx) = mpsc::channel(64);
        tokio::spawn(async move { while out_rx.recv().await.is_some() {} });
        let state = state();
        let wake: Wake = Arc::new(|| {});
        let task = tokio::spawn(run(
            profile,
            rx,
            state,
            wake,
            Some(out_tx),
            CancellationToken::new(),
        ));
        tx.send(Command::Input(b"exit 3\r".to_vec())).await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(20), task)
            .await
            .expect("exit ends the session")
            .unwrap();
        assert!(result.unwrap_err().to_string().contains("code 3"));
    }
}
