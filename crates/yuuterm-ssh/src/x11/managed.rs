//! Bundled Cygwin/X server, started on demand for X11 sessions.
//! The packaged runtime (`x11` beside the executable, built by
//! `scripts/Get-X11Runtime.ps1`) is copied to the data folder because Cygwin needs a
//! writable root for /tmp and /var/log. XWin runs with `-nolisten tcp`, so it only
//! listens on Cygwin's AF_LOCAL emulation: 127.0.0.1 plus a secret in
//! /tmp/.X11-unix/X<n>. Each start uses a fresh MIT cookie, and a wrong cookie must be
//! refused before the server is used: XWin reads its authority file through
//! /bin/cat and silently allows every local client if that read yields nothing.
//! The server stops when the last session holding its `LocalX11` ends; a job object
//! also ends it with YuuTerm.
use super::LocalX11;
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::{Arc, Weak},
    time::Duration,
};
use zeroize::Zeroizing;

const FIRST_DISPLAY: u16 = 20;
const DISPLAYS: u16 = 20;
const LIMIT: u64 = 64 * 1024 * 1024;
/// Logged once XWin's internal window manager has connected. A client connecting
/// earlier can deadlock XWin: its first authorization check forks /bin/cat while the
/// window-manager threads are still starting.
const READY: &[u8] =
    b"winInitMultiWindowWM - xcb_connect () returned and successfully opened the display";

#[derive(Deserialize)]
struct Manifest {
    server: String,
    files: BTreeMap<String, String>,
}

struct Paths {
    runtime: PathBuf,
    root: PathBuf,
}

fn paths() -> Option<Paths> {
    let runtime = std::env::current_exe().ok()?.parent()?.join("x11");
    let root = yuuterm_core::config::path().ok()?.parent()?.join("x11");
    runtime
        .join("bin/XWin.exe")
        .is_file()
        .then_some(Paths { runtime, root })
}

fn manifest(dir: &Path) -> Result<(Vec<u8>, Manifest)> {
    let bytes = std::fs::read(dir.join("manifest.json"))?;
    ensure!(
        bytes.len() <= 1024 * 1024,
        "X runtime manifest exceeds limit"
    );
    let manifest = serde_json::from_slice(&bytes)?;
    Ok((bytes, manifest))
}

/// The bundled server's name when this installation ships one.
pub fn available() -> Option<String> {
    cfg!(windows)
        .then(paths)
        .flatten()
        .and_then(|p| manifest(&p.runtime).ok())
        .map(|(_, m)| m.server)
}

/// Copies the runtime into the writable root unless the same manifest is already
/// there. Every file is checked against its manifest hash; the manifest is written
/// last, so an interrupted copy is repeated.
fn sync_runtime(runtime: &Path, root: &Path) -> Result<()> {
    let (bytes, manifest) = manifest(runtime)?;
    if std::fs::read(root.join("manifest.json")).is_ok_and(|current| current == bytes) {
        return Ok(());
    }
    for dir in ["bin", "lib", "usr", "licenses"] {
        let path = root.join(dir);
        if path.exists() {
            std::fs::remove_dir_all(path)?;
        }
    }
    let _ = std::fs::remove_file(root.join("manifest.json"));
    for (relative, hash) in &manifest.files {
        let path = Path::new(relative);
        ensure!(
            !relative.contains('\\')
                && path.components().all(|c| matches!(c, Component::Normal(_))),
            "Invalid X runtime path"
        );
        let source = runtime.join(path);
        ensure!(
            std::fs::metadata(&source)?.len() <= LIMIT,
            "X runtime file exceeds limit"
        );
        let data = std::fs::read(&source)?;
        let actual = Sha256::digest(&data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        ensure!(
            actual == *hash,
            "Bundled X server file {relative} does not match its manifest"
        );
        let target = root.join(path);
        std::fs::create_dir_all(target.parent().context("X runtime path")?)?;
        std::fs::write(target, data)?;
    }
    std::fs::write(root.join("manifest.json"), bytes)?;
    Ok(())
}

fn authority(display: u16, cookie: &[u8; 16]) -> Vec<u8> {
    let field = |bytes: &[u8]| [&(bytes.len() as u16).to_be_bytes()[..], bytes].concat();
    // FamilyLocal; XWin compares only the cookie, not the address.
    [
        &256u16.to_be_bytes()[..],
        &field(b"yuuterm"),
        &field(display.to_string().as_bytes()),
        &field(b"MIT-MAGIC-COOKIE-1"),
        &field(cookie),
    ]
    .concat()
}

/// Parses `!<socket >PORT s XXXXXXXX-XXXXXXXX-XXXXXXXX-XXXXXXXX`, Cygwin's AF_LOCAL
/// socket file. The secret words are sent as four little-endian 32-bit integers.
fn socket_file(bytes: &[u8]) -> Result<(u16, Zeroizing<[u8; 16]>)> {
    let text = bytes.split(|b| *b == 0).next().unwrap_or_default();
    let text = std::str::from_utf8(text)?;
    let rest = text
        .strip_prefix("!<socket >")
        .context("not a Cygwin socket file")?;
    let (port, rest) = rest.split_once(' ').context("socket file port")?;
    let words = rest
        .strip_prefix("s ")
        .context("not a Cygwin stream socket")?;
    let port: u16 = port.parse()?;
    ensure!(port != 0, "socket file port");
    let mut secret = Zeroizing::new([0u8; 16]);
    let mut count = 0;
    for (index, word) in words.split('-').enumerate() {
        ensure!(
            index < 4 && word.len() == 8 && word.bytes().all(|b| b.is_ascii_hexdigit()),
            "socket file secret"
        );
        secret[index * 4..index * 4 + 4]
            .copy_from_slice(&u32::from_str_radix(word, 16)?.to_le_bytes());
        count += 1;
    }
    ensure!(count == 4, "socket file secret");
    Ok((port, secret))
}

struct Running {
    child: std::sync::Mutex<std::process::Child>,
    #[cfg(windows)]
    _job: Job,
    root: PathBuf,
    display: u16,
    port: u16,
    secret: Zeroizing<[u8; 16]>,
    cookie: Zeroizing<[u8; 16]>,
}
impl Running {
    fn alive(&self) -> bool {
        self.child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .try_wait()
            .is_ok_and(|status| status.is_none())
    }
    fn local(self: &Arc<Self>) -> LocalX11 {
        LocalX11::cygwin(self.port, *self.secret, *self.cookie, self.clone())
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        let child = self.child.get_mut().unwrap_or_else(|e| e.into_inner());
        let _ = child.kill();
        let _ = child.wait();
        let tmp = self.root.join("tmp");
        for file in [
            tmp.join(format!(".yuuterm-{}.auth", self.display)),
            tmp.join(format!(".X{}-lock", self.display)),
            tmp.join(".X11-unix").join(format!("X{}", self.display)),
        ] {
            let _ = std::fs::remove_file(file);
        }
    }
}

#[cfg(windows)]
struct Job(windows::Win32::Foundation::HANDLE);
// The handle is only closed on drop; job objects are usable from any thread.
#[cfg(windows)]
unsafe impl Send for Job {}
#[cfg(windows)]
unsafe impl Sync for Job {}
#[cfg(windows)]
impl Job {
    /// Ends the server (and its xkbcomp helpers) when YuuTerm exits or crashes.
    fn kill_on_close(child: &std::process::Child) -> Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{Foundation::HANDLE, System::JobObjects::*};
        unsafe {
            let job = Self(CreateJobObjectW(None, None)?);
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                size_of_val(&info) as u32,
            )?;
            AssignProcessToJobObject(job.0, HANDLE(child.as_raw_handle()))?;
            Ok(job)
        }
    }
}
#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

/// Returns the running bundled server, starting it first if needed.
pub async fn local() -> Result<LocalX11> {
    static SERVER: tokio::sync::Mutex<Weak<Running>> = tokio::sync::Mutex::const_new(Weak::new());
    let mut slot = SERVER.lock().await;
    if let Some(running) = slot.upgrade()
        && running.alive()
    {
        return Ok(running.local());
    }
    let paths = paths().context("YuuTerm's bundled X server is missing from this installation")?;
    let running = tokio::time::timeout(Duration::from_secs(30), start(paths))
        .await
        .context("Bundled X server start timed out")??;
    *slot = Arc::downgrade(&running);
    Ok(running.local())
}

async fn start(paths: Paths) -> Result<Arc<Running>> {
    let root = paths.root.clone();
    tokio::task::spawn_blocking(move || -> Result<()> {
        sync_runtime(&paths.runtime, &paths.root)?;
        std::fs::create_dir_all(paths.root.join("tmp/.X11-unix"))?;
        std::fs::create_dir_all(paths.root.join("var/log/xwin"))?;
        Ok(())
    })
    .await??;
    for display in FIRST_DISPLAY..FIRST_DISPLAY + DISPLAYS {
        let Some(running) = spawn(&root, display).await? else {
            continue;
        };
        let local = running.local();
        tokio::time::timeout(Duration::from_secs(5), super::desktop::verify(&local))
            .await
            .context("Bundled X server authentication check timed out")?
            .context("Bundled X server failed its authentication check and was stopped")?;
        return Ok(running);
    }
    bail!("No free display for the bundled X server")
}

/// Starts XWin on one display. `None` means the display is taken (XWin exits).
async fn spawn(root: &Path, display: u16) -> Result<Option<Arc<Running>>> {
    let tmp = root.join("tmp");
    let socket = tmp.join(".X11-unix").join(format!("X{display}"));
    let log = root.join(format!("var/log/xwin/XWin.{display}.log"));
    let cookie = Zeroizing::new(rand::random::<[u8; 16]>());
    {
        let (tmp, socket, log, cookie) = (tmp.clone(), socket.clone(), log.clone(), cookie.clone());
        tokio::task::spawn_blocking(move || -> Result<()> {
            // Leftovers of a crashed run in YuuTerm's own root; no XWin of ours is running.
            let _ = std::fs::remove_file(&socket);
            let _ = std::fs::remove_file(&log);
            let _ = std::fs::remove_file(tmp.join(format!(".X{display}-lock")));
            std::fs::write(
                tmp.join(format!(".yuuterm-{display}.auth")),
                Zeroizing::new(authority(display, &cookie)),
            )?;
            Ok(())
        })
        .await??;
    }
    let mut command = std::process::Command::new(root.join("bin/XWin.exe"));
    command
        .arg(format!(":{display}"))
        .args(["-multiwindow", "-nolisten", "tcp", "-noclipboard", "-nowgl"])
        .args(["-notrayicon", "-silent-dup-error", "-auth"])
        .arg(format!("/tmp/.yuuterm-{display}.auth"))
        .current_dir(root.join("bin"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
    let mut child = command
        .spawn()
        .context("Bundled X server could not start")?;
    #[cfg(windows)]
    let job = match Job::kill_on_close(&child) {
        Ok(job) => job,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    let mut running = Running {
        child: std::sync::Mutex::new(child),
        #[cfg(windows)]
        _job: job,
        root: root.to_path_buf(),
        display,
        port: 0,
        secret: Zeroizing::new([0; 16]),
        cookie,
    };
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        if !running.alive() {
            return Ok(None);
        }
        if let Ok(bytes) = std::fs::read(&socket)
            && let Ok((port, secret)) = socket_file(&bytes[..bytes.len().min(128)])
            && std::fs::metadata(&log).is_ok_and(|m| m.len() <= 1024 * 1024)
            && std::fs::read(&log).is_ok_and(|text| text.windows(READY.len()).any(|w| w == READY))
        {
            running.port = port;
            running.secret = secret;
            return Ok(Some(Arc::new(running)));
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "Bundled X server did not become ready"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn socket_file_is_exact_and_little_endian() {
        let (port, secret) =
            socket_file(b"!<socket >50675 s 01020304-0A0B0C0D-00000000-FFFFFFFF\0").unwrap();
        assert_eq!(port, 50675);
        assert_eq!(
            *secret,
            [4, 3, 2, 1, 13, 12, 11, 10, 0, 0, 0, 0, 255, 255, 255, 255]
        );
        for invalid in [
            &b"!<socket >0 s 01020304-0A0B0C0D-00000000-FFFFFFFF"[..],
            b"!<socket >50675 d 01020304-0A0B0C0D-00000000-FFFFFFFF",
            b"!<socket >50675 s 01020304-0A0B0C0D-00000000",
            b"!<socket >50675 s 01020304-0A0B0C0D-00000000-FFFFFFFF-00000000",
            b"!<socket >50675 s 0102030-0A0B0C0D-00000000-FFFFFFFFF",
            b"!<socket >70000 s 01020304-0A0B0C0D-00000000-FFFFFFFF",
            b"<socket >50675 s 01020304-0A0B0C0D-00000000-FFFFFFFF",
        ] {
            assert!(socket_file(invalid).is_err());
        }
    }
    #[test]
    fn authority_holds_one_exact_cookie_entry() {
        let bytes = authority(21, &[9; 16]);
        assert_eq!(&bytes[..2], &[1, 0]);
        assert_eq!(bytes.len(), 2 + 9 + 4 + 20 + 18);
        assert_eq!(&bytes[bytes.len() - 16..], &[9; 16]);
    }
    /// Native check against the real runtime from `scripts/Get-X11Runtime.ps1`:
    /// set YUUTERM_X11_RUNTIME to target/x11-runtime/x11.
    #[tokio::test]
    async fn bundled_server_is_authenticated_loopback_only_and_stops() {
        let Some(runtime) = std::env::var_os("YUUTERM_X11_RUNTIME") else {
            eprintln!("YUUTERM_X11_RUNTIME not set; skipped");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("x11");
        let running = start(Paths {
            runtime: runtime.into(),
            root: root.clone(),
        })
        .await
        .unwrap();
        // start() already required a wrong cookie to be refused.
        let local = running.local();
        assert!(super::super::desktop::verify(&local).await.is_ok());
        // Without the AF_LOCAL secret the server drops the connection.
        let mut plain = tokio::net::TcpStream::connect(("127.0.0.1", running.port))
            .await
            .unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let _ = plain.write_all(&[0x6c; 48]).await;
        let mut byte = [0; 1];
        assert!(!matches!(plain.read(&mut byte).await, Ok(1..)));
        let socket = root
            .join("tmp/.X11-unix")
            .join(format!("X{}", running.display));
        assert!(socket.exists());
        let port = running.port;
        drop(local);
        drop(running);
        assert!(!socket.exists());
        assert!(
            tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_err()
        );
    }
    #[test]
    fn runtime_copy_verifies_hashes_and_rejects_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let (runtime, root) = (dir.path().join("runtime"), dir.path().join("root"));
        std::fs::create_dir_all(runtime.join("bin")).unwrap();
        std::fs::write(runtime.join("bin/a.dll"), b"abc").unwrap();
        let write = |hash: &str, name: &str| {
            std::fs::write(
                runtime.join("manifest.json"),
                format!(r#"{{"server":"fixture","files":{{"{name}":"{hash}"}}}}"#),
            )
            .unwrap()
        };
        let good = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        write(good, "bin/a.dll");
        sync_runtime(&runtime, &root).unwrap();
        assert_eq!(std::fs::read(root.join("bin/a.dll")).unwrap(), b"abc");
        // Unchanged manifest: no copy; the root's own tmp survives a runtime update.
        std::fs::create_dir_all(root.join("tmp")).unwrap();
        std::fs::write(root.join("bin/a.dll"), b"local").unwrap();
        sync_runtime(&runtime, &root).unwrap();
        assert_eq!(std::fs::read(root.join("bin/a.dll")).unwrap(), b"local");
        write(&good.replace('b', "c"), "bin/a.dll");
        assert!(sync_runtime(&runtime, &root).is_err());
        assert!(!root.join("manifest.json").exists());
        assert!(root.join("tmp").is_dir());
        write(good, "../a.dll");
        assert!(sync_runtime(&runtime, &root).is_err());
    }
}
