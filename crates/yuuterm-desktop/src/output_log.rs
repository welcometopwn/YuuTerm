//! Explicit opt-in raw output capture. File I/O never runs under a session/UI lock.
use serde::Serialize;
use std::{
    io::Write,
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender},
    },
    time::{Duration, Instant},
};

const QUEUE: usize = 64;
const CHUNK: usize = 16 * 1024;
const LIMIT: u64 = 64 * 1024 * 1024;
const SYNC_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Default, Serialize)]
pub struct Status {
    pub active: bool,
    pub finished: bool,
    pub bytes: u64,
    /// Prefix confirmed by a successful sink sync, not merely write_all.
    pub durable_bytes: u64,
    pub error: Option<&'static str>,
    pub path: Option<String>,
}
pub struct OutputLog {
    sender: Option<SyncSender<Vec<u8>>>,
    state: Arc<Mutex<Status>>,
}
impl OutputLog {
    pub fn automatic(
        name: String,
        settings: yuuterm_core::LoggingSettings,
        documents_override: Option<std::path::PathBuf>,
        id: u64,
    ) -> Self {
        let limit = if settings.rotation.is_some() {
            u64::MAX
        } else {
            LIMIT
        };
        let result = Self::spawn_factory(
            move || {
                let path = if settings.file.is_empty() {
                    let documents = documents_override.map(Ok).unwrap_or_else(documents)?;
                    let folder = documents.join("YuuTerm").join(safe_session_name(&name));
                    let time = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
                    if settings.rotation.is_some() {
                        folder.join("current.log")
                    } else {
                        folder.join(format!(
                            "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}-{:03}-{id}.log",
                            time.wYear,
                            time.wMonth,
                            time.wDay,
                            time.wHour,
                            time.wMinute,
                            time.wSecond,
                            time.wMilliseconds
                        ))
                    }
                } else {
                    std::path::PathBuf::from(settings.file)
                };
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|_| "Could not create the configured log directory")?;
                }
                let (file, bytes) = crate::log_rotation::LogFile::open(&path, settings.rotation)?;
                Ok((file, bytes, Some(path)))
            },
            limit,
        );
        result.unwrap_or_else(|_| Self {
            sender: None,
            state: Arc::new(Mutex::new(Status {
                finished: true,
                error: Some("Could not start automatic output logging"),
                ..Default::default()
            })),
        })
    }
    #[cfg(test)]
    fn spawn(writer: impl Write + Send + 'static, limit: u64) -> Result<Self, String> {
        Self::spawn_factory(move || Ok((writer, 0, None)), limit)
    }
    fn spawn_factory<W: Write + Send + 'static>(
        factory: impl FnOnce() -> Result<(W, u64, Option<std::path::PathBuf>), &'static str>
        + Send
        + 'static,
        limit: u64,
    ) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(QUEUE);
        let state = Arc::new(Mutex::new(Status {
            active: true,
            ..Default::default()
        }));
        let shared = state.clone();
        std::thread::Builder::new().name("terminal-output-log".into()).spawn(move || {
            let (mut writer, mut bytes, path) = match factory() {
                Ok(value) => value,
                Err(error) => {
                    if let Ok(mut status) = shared.lock() { status.active = false; status.finished = true; status.error = Some(error); }
                    return;
                }
            };
            if let Ok(mut status) = shared.lock() {status.path = path.map(|p| p.to_string_lossy().into_owned()); status.bytes = bytes;}
            // Production LogFile::flush performs sync_all. Confirm an existing
            // append baseline before calling any bytes durable.
            if writer.flush().is_err() {
                drop(writer);
                if let Ok(mut status)=shared.lock(){status.active=false;status.finished=true;status.error=Some("Initial log sync failed; durability is unconfirmed");}
                return;
            }
            if let Ok(mut status)=shared.lock(){status.durable_bytes=bytes;}
            let mut flushed = Instant::now();
            let mut dirty = false;
            let mut error = None;
            loop {
                let wait = if dirty { SYNC_INTERVAL.saturating_sub(flushed.elapsed()) } else { SYNC_INTERVAL };
                match receiver.recv_timeout(wait) {
                    Ok(chunk) => {
                        if bytes > limit.saturating_sub(chunk.len() as u64) {
                            error = Some("Log stopped at the 64 MiB limit; remaining output was not recorded");
                            break;
                        }
                        if writer.write_all(&chunk).is_err() {
                            error = Some("Log write/rotation failed; retained files may contain only part of the output");
                            break;
                        }
                        bytes += chunk.len() as u64;
                        dirty = true;
                        if let Ok(mut status) = shared.lock() {status.bytes = bytes;}
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                // Check busy queues too; idle tails wait only the remaining interval.
                if dirty && flushed.elapsed() >= SYNC_INTERVAL {
                    if writer.flush().is_err() {
                        error = Some("Log sync failed; bytes beyond the last durable checkpoint are unconfirmed");
                        break;
                    }
                    if let Ok(mut status)=shared.lock(){status.durable_bytes=bytes;}
                    dirty=false;
                    flushed = Instant::now();
                }
            }
            if writer.flush().is_err() {error = Some("Final log sync failed; bytes beyond the last durable checkpoint are unconfirmed");}
            else if let Ok(mut status)=shared.lock(){status.durable_bytes=bytes;}
            drop(writer);
            if let Ok(mut status) = shared.lock() {
                status.active = false;
                status.finished = true;
                if error.is_some() {status.error = error;}
            }
        }).map_err(|_| "Could not start output log worker".to_owned())?;
        Ok(Self {
            sender: Some(sender),
            state,
        })
    }
    pub fn status(&self) -> Status {
        self.state.lock().map(|s| s.clone()).unwrap_or(Status {
            finished: true,
            error: Some("Output log state unavailable"),
            ..Default::default()
        })
    }
    pub fn record(&mut self, bytes: &[u8]) {
        let status = self.status();
        if !status.active {
            return;
        }
        for chunk in bytes.chunks(CHUNK) {
            if self
                .sender
                .as_ref()
                .is_none_or(|s| s.try_send(chunk.to_vec()).is_err())
            {
                if let Ok(mut status) = self.state.lock() {
                    status.active = false;
                    status.error = Some(
                        "Log queue filled or writer stopped; subsequent output was not recorded",
                    );
                }
                self.sender.take();
                break;
            }
        }
    }
    pub fn stop(&mut self) {
        if let Ok(mut status) = self.state.lock() {
            status.active = false;
        }
        self.sender.take();
    }
}

fn documents() -> Result<std::path::PathBuf, &'static str> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_Documents, KF_FLAG_DEFAULT, SHGetKnownFolderPath},
    };
    unsafe {
        let value = SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None)
            .map_err(|_| "Windows Documents folder is unavailable; configure a log file path")?;
        let path = value.to_string();
        CoTaskMemFree(Some(value.0.cast()));
        path.map(std::path::PathBuf::from)
            .map_err(|_| "Windows Documents path is invalid")
    }
}
fn safe_session_name(name: &str) -> String {
    let safe = name
        .chars()
        .take(80)
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect::<String>()
        .trim_matches([' ', '.'])
        .to_owned();
    let base = safe.split('.').next().unwrap_or("").to_ascii_uppercase();
    if safe.is_empty() {
        "session".into()
    } else if is_device_name(&base) {
        format!("_{safe}")
    } else {
        safe
    }
}
fn is_device_name(base: &str) -> bool {
    matches!(base, "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            base.strip_prefix(prefix).is_some_and(|suffix| {
                suffix.chars().count() == 1 && "123456789¹²³".contains(suffix)
            })
        })
}
pub(crate) fn append_target(path: &std::path::Path) -> Result<(std::fs::File, u64), &'static str> {
    use std::os::windows::fs::OpenOptionsExt;
    // FILE_SHARE_READ permits reading/tailing but refuses overlapping writers.
    let deadline = Instant::now() + Duration::from_secs(3);
    let file = loop {
        match std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .share_mode(1)
            .open(path)
        {
            Ok(file) => break file,
            // A reconnect may beat the old writer's final drain/handle close.
            Err(error) if error.raw_os_error() == Some(32) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(_) => {
                return Err(
                    "Cannot open the configured log file; check its path, permissions and other writers",
                );
            }
        }
    };
    let length = file
        .metadata()
        .map_err(|_| "Cannot inspect the output log")?
        .len();
    if length >= LIMIT {
        return Err(
            "Log file has reached the 64 MiB limit; choose another file in session settings",
        );
    }
    Ok((file, length))
}

#[cfg(test)]
#[path = "output_log_crash_tests.rs"]
mod crash_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn storage_failure_stops_recording_and_is_not_reported_as_saved() {
        struct Failed;
        impl Write for Failed {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("synthetic storage failure"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut log = OutputLog::spawn(Failed, LIMIT).unwrap();
        log.record(b"never confirmed");
        let status = finish(&mut log);
        assert!(!status.active);
        assert_eq!(status.bytes, 0);
        assert!(status.error.is_some());
    }
    fn finish(log: &mut OutputLog) -> Status {
        log.stop();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !log.status().finished && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let status = log.status();
        assert!(status.finished);
        status
    }
    #[test]
    fn automatic_custom_file_appends_raw_output_and_releases_writer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.log");
        let settings = yuuterm_core::LoggingSettings {
            enabled: true,
            file: path.to_string_lossy().into_owned(),
            rotation: None,
        };
        let mut log = OutputLog::automatic("test".into(), settings.clone(), None, 1);
        log.record(b"\x1b[31mred\x1b[0m\r\n\0\xff");
        let status = finish(&mut log);
        assert!(status.error.is_none());
        let mut next = OutputLog::automatic("test".into(), settings, None, 2);
        next.record(b"tail");
        let status = finish(&mut next);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"\x1b[31mred\x1b[0m\r\n\0\xfftail"
        );
        assert_eq!(status.bytes, std::fs::metadata(&path).unwrap().len());
        assert!(status.error.is_none());
        let (held, _) = append_target(&path).unwrap();
        assert!(append_target(&path).is_err());
        drop(held);
        assert!(append_target(&path).is_ok());
    }
    #[test]
    fn default_logs_use_sanitized_session_directory_and_new_files() {
        let dir = tempfile::tempdir().unwrap();
        let settings = yuuterm_core::LoggingSettings {
            enabled: true,
            file: String::new(),
            rotation: None,
        };
        let mut first = OutputLog::automatic(
            "CON".into(),
            settings.clone(),
            Some(dir.path().to_owned()),
            1,
        );
        first.record(b"initial banner");
        let status = finish(&mut first);
        assert!(status.error.is_none());
        let first_path = std::path::PathBuf::from(status.path.unwrap());
        assert_eq!(
            first_path.parent().unwrap(),
            dir.path().join("YuuTerm/_CON")
        );
        assert_eq!(std::fs::read(&first_path).unwrap(), b"initial banner");
        let mut second =
            OutputLog::automatic("CON".into(), settings, Some(dir.path().to_owned()), 2);
        second.record(b"reconnect");
        let status = finish(&mut second);
        assert!(status.error.is_none());
        assert_ne!(status.path.unwrap(), first_path.to_string_lossy());
        for name in [
            "..",
            "../escape",
            "NUL.txt",
            "COM¹",
            "folder\\nested",
            "hello\nworld",
        ] {
            let safe = safe_session_name(name);
            assert!(!safe.is_empty() && safe != ".." && !safe.contains(['\\', '/', '\n']));
            assert!(!is_device_name(
                &safe.split('.').next().unwrap().to_ascii_uppercase()
            ));
        }
        assert!(documents().unwrap().is_absolute());
    }
    #[test]
    fn rotated_default_path_is_stable_and_worker_reports_retention_failures() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/log-rotation-tests");
        std::fs::create_dir_all(&root).unwrap();
        let dir = tempfile::tempdir_in(root).unwrap();
        let settings = yuuterm_core::LoggingSettings {
            enabled: true,
            file: String::new(),
            rotation: Some(yuuterm_core::LogRotation {
                max_mib: 1,
                keep_files: 2,
            }),
        };
        let mut log =
            OutputLog::automatic("test".into(), settings.clone(), Some(dir.path().into()), 1);
        log.record(b"first");
        let first = finish(&mut log);
        assert!(first.error.is_none());
        let path = std::path::PathBuf::from(first.path.unwrap());
        assert_eq!(path, dir.path().join("YuuTerm/test/current.log"));
        let mut log =
            OutputLog::automatic("test".into(), settings.clone(), Some(dir.path().into()), 2);
        log.record(b"second");
        let second = finish(&mut log);
        assert!(second.error.is_none());
        assert_eq!(second.bytes, 11);
        assert_eq!(std::fs::read(&path).unwrap(), b"firstsecond");
        std::fs::write(
            path.with_file_name("current.log.yuuterm-rotated")
                .join("unknown.txt"),
            b"keep",
        )
        .unwrap();
        let mut log = OutputLog::automatic("test".into(), settings, Some(dir.path().into()), 3);
        log.record(b"not recorded");
        let failed = finish(&mut log);
        assert!(!failed.active && failed.error.is_some());
        assert_eq!(std::fs::read(path).unwrap(), b"firstsecond");
    }
    #[test]
    fn size_limit_stops_without_overwriting_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("limited.log");
        let file = std::fs::File::create(&path).unwrap();
        let mut log = OutputLog::spawn(file, 4).unwrap();
        log.record(b"1234");
        log.record(b"56");
        let status = finish(&mut log);
        assert_eq!(std::fs::read(path).unwrap(), b"1234");
        assert_eq!(status.bytes, 4);
        assert!(status.error.is_some());
    }
    #[test]
    fn stalled_writer_does_not_block_input_and_queue_failure_is_visible() {
        struct Stalled(Arc<(Mutex<bool>, std::sync::Condvar)>);
        impl Write for Stalled {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                let (lock, cv) = &*self.0;
                let guard = lock.lock().unwrap();
                drop(cv.wait_while(guard, |released| !*released).unwrap());
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let mut log = OutputLog::spawn(Stalled(gate.clone()), LIMIT).unwrap();
        for _ in 0..QUEUE + 2 {
            log.record(b"bounded");
        }
        assert!(log.status().error.is_some());
        assert!(!log.status().active);
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        assert!(finish(&mut log).error.is_some());
    }
}
