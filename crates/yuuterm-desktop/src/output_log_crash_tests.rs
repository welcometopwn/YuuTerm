//! Test-only subprocess: production OutputLog/LogFile, no GUI or graceful Drop.
use super::*;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
};

fn root() -> PathBuf {
    let workspace = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap();
    let target = fs::canonicalize(workspace.join("target")).unwrap();
    assert!(target.starts_with(&workspace) && target != workspace);
    let path = target.join("log-crash-tests");
    fs::create_dir_all(&path).unwrap();
    let path = fs::canonicalize(path).unwrap();
    assert!(path.starts_with(&target) && path != target);
    path
}
fn record(sequence: usize) -> Vec<u8> {
    let mut bytes = vec![b'x'; CHUNK];
    let label = format!("synthetic-log-record-{sequence:06} ");
    bytes[..label.len()].copy_from_slice(label.as_bytes());
    bytes[CHUNK - 2..].copy_from_slice(b"\r\n");
    bytes
}
fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .try_into()
        .unwrap()
}
#[derive(Deserialize, Serialize)]
struct Progress {
    sequence: usize,
    durable: usize,
    admitted_ms: u64,
}

/// Selected by the parent in a separate test process. Inert in ordinary runs;
/// the entire module is absent from normal and capture release binaries.
#[test]
fn log_crash_child() {
    let Ok(folder) = std::env::var("YUUTERM_TEST_LOG_CRASH_CHILD") else {
        return;
    };
    let folder = fs::canonicalize(folder).unwrap();
    assert!(folder.starts_with(root()) && folder != root());
    let rotating = std::env::var("YUUTERM_TEST_LOG_CRASH_ROTATE").as_deref() == Ok("1");
    let settings = yuuterm_core::LoggingSettings {
        enabled: true,
        file: folder.join("active.log").to_string_lossy().into_owned(),
        rotation: rotating.then_some(yuuterm_core::LogRotation {
            max_mib: 1,
            keep_files: 10,
        }),
    };
    let mut log = OutputLog::automatic("synthetic kill fixture".into(), settings, None, 1);
    for sequence in 1..=512 {
        let admitted_ms = epoch_ms();
        log.record(&record(sequence));
        let end = Instant::now() + Duration::from_secs(3);
        while log.status().bytes < (sequence * CHUNK) as u64 {
            assert!(
                Instant::now() < end && log.status().error.is_none(),
                "Fixture writer did not confirm output"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let status = log.status();
        assert!(status.error.is_none());
        println!(
            "YT_LOG_CRASH {}",
            serde_json::to_string(&Progress {
                sequence,
                durable: status.durable_bytes as usize / CHUNK,
                admitted_ms
            })
            .unwrap()
        );
        std::io::stdout().flush().unwrap();
        std::thread::sleep(Duration::from_millis(3));
    }
    panic!("Parent did not force-kill its fixture child");
}
struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn killed_process_preserves_checkpoints_and_records_older_than_sync_interval() {
    let root = root();
    let mut reports = Vec::new();
    for rotating in [false, true] {
        let dir = tempfile::tempdir_in(&root).unwrap();
        let mut child = KillOnDrop(
            Command::new(std::env::current_exe().unwrap())
                .creation_flags(0x08000000) // CREATE_NO_WINDOW: owned fixture only.
                .args([
                    "--exact",
                    "output_log::crash_tests::log_crash_child",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env("YUUTERM_TEST_LOG_CRASH_CHILD", dir.path())
                .env(
                    "YUUTERM_TEST_LOG_CRASH_ROTATE",
                    if rotating { "1" } else { "0" },
                )
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let out = child.0.stdout.take().unwrap();
        let (sender, receiver) = mpsc::sync_channel(8);
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                let Ok(line) = line else {
                    break;
                };
                if let Some((_, json)) = line.split_once("YT_LOG_CRASH ")
                    && let Ok(p) = serde_json::from_str::<Progress>(json)
                    && sender.send(p).is_err()
                {
                    break;
                }
            }
        });
        let mut admissions = Vec::new();
        let end = Instant::now() + Duration::from_secs(10);
        let last = loop {
            let p = receiver
                .recv_timeout(end.saturating_duration_since(Instant::now()))
                .expect("Kill fixture progress deadline");
            assert!((1..=512).contains(&p.sequence) && p.durable <= p.sequence);
            assert_eq!(p.sequence, admissions.len() + 1);
            admissions.push((p.sequence, p.admitted_ms));
            if p.durable >= if rotating { 96 } else { 16 }
                && p.sequence > p.durable
                && epoch_ms().saturating_sub(admissions[0].1)
                    >= 2 * SYNC_INTERVAL.as_millis() as u64
            {
                break p;
            }
            assert!(admissions.len() <= 512);
        };
        let kill_ms = epoch_ms();
        child.0.kill().unwrap();
        let exit = child.0.wait().unwrap();
        assert!(!exit.success());
        drop(receiver);
        reader.join().unwrap();
        let cutoff = kill_ms.saturating_sub(SYNC_INTERVAL.as_millis() as u64);
        let older = admissions
            .iter()
            .filter(|(_, at)| *at <= cutoff)
            .map(|(seq, _)| *seq)
            .max()
            .unwrap_or(0);
        assert!(
            older > 0,
            "Fixture must include records older than the interval"
        );
        let required = last.durable.max(older);
        let mut paths = Vec::new();
        let archives = dir.path().join("active.log.yuuterm-rotated");
        if archives.exists() {
            paths = fs::read_dir(archives)
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.extension().is_some_and(|e| e == "log"))
                .collect();
            paths.sort();
        }
        if rotating {
            assert!(!paths.is_empty(), "Fixture must cross a real rollover");
        }
        paths.push(dir.path().join("active.log"));
        assert!(paths.len() <= 10);
        let mut actual = Vec::new();
        for path in &paths {
            fs::File::open(path)
                .unwrap()
                .take(16 * 1024 * 1024 + 1)
                .read_to_end(&mut actual)
                .unwrap();
            assert!(actual.len() <= 16 * 1024 * 1024);
        }
        let mut expected = Vec::with_capacity(required * CHUNK);
        for n in 1..=required {
            expected.extend(record(n));
        }
        assert!(
            actual.starts_with(&expected),
            "Forced kill lost or corrupted confirmed/older output"
        );
        let (mut reopened, _) = crate::log_rotation::LogFile::open(
            &dir.path().join("active.log"),
            rotating.then_some(yuuterm_core::LogRotation {
                max_mib: 1,
                keep_files: 10,
            }),
        )
        .unwrap();
        reopened.write_all(b"explicit-reopen-marker").unwrap();
        reopened.flush().unwrap();
        drop(reopened);
        reports.push(serde_json::json!({"rotating":rotating,"forced_kill":true,"graceful_shutdown":false,"sync_interval_ms":SYNC_INTERVAL.as_millis(),"durable_prefix_records":last.durable,"records_older_than_interval":older,"latest_reported_record":last.sequence,"verified_prefix_bytes":expected.len(),"retained_files":paths.len(),"retained_bytes":actual.len(),"prefix_intact":true,"reopen_after_kill":true}));
    }
    fs::write(
        root.join("process-kill.json"),
        serde_json::to_vec_pretty(&reports).unwrap(),
    )
    .unwrap();
}

#[test]
fn idle_tail_is_synced_before_disconnect() {
    let dir = tempfile::tempdir_in(root()).unwrap();
    let settings = yuuterm_core::LoggingSettings {
        enabled: true,
        file: dir.path().join("idle.log").to_string_lossy().into_owned(),
        rotation: None,
    };
    let mut log = OutputLog::automatic("idle".into(), settings, None, 1);
    log.record(b"synthetic idle tail");
    let end = Instant::now() + Duration::from_secs(3);
    while log.status().durable_bytes < 19 {
        assert!(Instant::now() < end && log.status().error.is_none());
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(log.status().active);
    log.stop();
    while !log.status().finished {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn sync_failure_never_advances_the_durable_watermark() {
    struct FailSync(bool);
    impl Write for FailSync {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            if std::mem::take(&mut self.0) {
                Ok(())
            } else {
                Err(std::io::Error::other("synthetic sync failure"))
            }
        }
    }
    let mut log = OutputLog::spawn(FailSync(true), LIMIT).unwrap();
    log.record(b"unconfirmed");
    let end = Instant::now() + Duration::from_secs(3);
    while !log.status().finished {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(log.status().bytes, 11);
    assert_eq!(log.status().durable_bytes, 0);
    assert!(log.status().error.is_some());
}
