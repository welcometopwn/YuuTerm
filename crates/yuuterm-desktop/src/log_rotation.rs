//! Worker-owned rotation. Never truncate/replace an archive, or prune unknown files.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use yuuterm_core::LogRotation;

pub struct LogFile {
    file: Option<File>,
    path: PathBuf,
    bytes: u64,
    archive: Option<Archive>,
    // Held across close/rename/reopen, including nonrotating writers.
    _guard: File,
}
struct Archive {
    path: PathBuf,
    policy: LogRotation,
    files: Vec<(u64, PathBuf)>,
    next: u64,
}
fn error(message: &'static str) -> io::Error {
    io::Error::other(message)
}
fn regular(path: &Path) -> io::Result<fs::Metadata> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_file() || m.file_attributes() & 0x400 != 0 {
        return Err(error("Log metadata must be a regular non-reparse file"));
    }
    Ok(m)
}
fn sibling(path: &Path, suffix: &str) -> io::Result<PathBuf> {
    let mut name = path
        .file_name()
        .ok_or_else(|| error("Missing log filename"))?
        .to_os_string();
    name.push(suffix);
    Ok(path.with_file_name(name))
}
fn guard(path: &Path) -> io::Result<File> {
    let path = sibling(path, ".yuuterm-lock")?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                if regular(&path)?.len() != 0 {
                    return Err(error("Unexpected log lock contents"));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(&path)
        {
            Ok(file) => return Ok(file),
            Err(e) if e.raw_os_error() == Some(32) && Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20))
            }
            Err(e) => return Err(e),
        }
    }
}
impl Archive {
    fn open(path: &Path, policy: LogRotation) -> io::Result<Self> {
        if !(1..=64).contains(&policy.max_mib) || !(2..=10).contains(&policy.keep_files) {
            return Err(error("Invalid log rotation bounds"));
        }
        let folder = sibling(path, ".yuuterm-rotated")?;
        let marker = folder.join(".owner");
        let expected = format!(
            "YuuTerm rotated logs v1\n{}\n",
            path.file_name()
                .ok_or_else(|| error("Missing log filename"))?
                .to_string_lossy()
        );
        match fs::create_dir(&folder) {
            Ok(()) => {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&marker)?;
                file.write_all(expected.as_bytes())?;
                file.flush()?;
                file.sync_all()?;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let meta = fs::symlink_metadata(&folder)?;
        if !meta.is_dir() || meta.file_attributes() & 0x400 != 0 {
            return Err(error(
                "Archive directory is a reparse point or not a directory",
            ));
        }
        if regular(&marker)?.len() > 2048 {
            return Err(error("Invalid log archive ownership marker"));
        }
        let mut contents = String::new();
        File::open(&marker)?
            .take(2049)
            .read_to_string(&mut contents)?;
        if contents != expected {
            return Err(error("Log archive directory is not owned by this logger"));
        }
        let mut files = Vec::new();
        for (n, entry) in fs::read_dir(&folder)?.enumerate() {
            if n >= 32 {
                return Err(error("Archive directory exceeds scan bound"));
            }
            let entry = entry?;
            if entry.file_name() == ".owner" {
                continue;
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| error("Unknown file in log archive directory"))?;
            let number = name
                .strip_suffix(".log")
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|n| format!("{n:020}.log") == name);
            let Some(number) = number else {
                return Err(error("Unknown file in log archive directory"));
            };
            regular(&entry.path())?;
            files.push((number, entry.path()));
        }
        files.sort_by_key(|p| p.0);
        let next = files
            .last()
            .map_or(Some(1), |p| p.0.checked_add(1))
            .ok_or_else(|| error("Log archive sequence exhausted"))?;
        Ok(Self {
            path: folder,
            policy,
            files,
            next,
        })
    }
    fn prune(&mut self) -> io::Result<()> {
        while self.files.len() >= usize::from(self.policy.keep_files) {
            regular(&self.files[0].1)?;
            fs::remove_file(&self.files[0].1)?;
            self.files.remove(0);
        }
        Ok(())
    }
    fn move_active(&mut self, path: &Path) -> io::Result<()> {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            Win32::Storage::FileSystem::{MOVE_FILE_FLAGS, MoveFileExW},
            core::PCWSTR,
        };
        let destination = self.path.join(format!("{:020}.log", self.next));
        let source = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let target = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        // Flags=0 explicitly refuses an existing destination (fs::rename may replace it).
        unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(target.as_ptr()),
                MOVE_FILE_FLAGS(0),
            )
        }
        .map_err(io::Error::other)?;
        self.files.push((self.next, destination));
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| error("Log archive sequence exhausted"))?;
        self.prune()
    }
}
impl LogFile {
    pub fn open(path: &Path, policy: Option<LogRotation>) -> Result<(Self, u64), &'static str> {
        let guard = guard(path).map_err(
            |_| "Cannot lock the log path; another writer or unexpected lock file exists",
        )?;
        let mut archive = policy
            .map(|p| Archive::open(path, p))
            .transpose()
            .map_err(|_| "Cannot use the rotation archive; check ownership, unknown files, bounds and permissions")?;
        match fs::symlink_metadata(path) {
            Ok(_) => {
                regular(path).map_err(|_| "Log path must be a regular non-reparse file")?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot inspect the log path"),
        }
        let (file, bytes) = if let Some(policy) = policy {
            let file = OpenOptions::new()
                .append(true)
                .create(true)
                .share_mode(1)
                .open(path)
                .map_err(
                    |_| "Cannot append to the rotating log; check path, permissions and readers",
                )?;
            let length = file
                .metadata()
                .map_err(|_| "Cannot inspect rotating log")?
                .len();
            if length > u64::from(policy.max_mib) * 1024 * 1024 {
                return Err(
                    "Existing active log exceeds the selected rotation size; choose a different path or move it manually",
                );
            }
            (file, length)
        } else {
            super::output_log::append_target(path)?
        };
        if let Some(archive) = &mut archive {
            archive
                .prune()
                .map_err(|_| "Could not prune owned log archives")?;
        }
        Ok((
            Self {
                file: Some(file),
                path: path.into(),
                bytes,
                archive,
                _guard: guard,
            },
            bytes,
        ))
    }
}
impl Write for LogFile {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if let Some(archive) = &mut self.archive {
            let limit = u64::from(archive.policy.max_mib) * 1024 * 1024;
            if data.len() as u64 > limit {
                return Err(error("Log chunk exceeds rotation size"));
            }
            if self.bytes > limit.saturating_sub(data.len() as u64) {
                self.flush()?;
                self.file.take();
                self.archive.as_mut().unwrap().move_active(&self.path)?;
                self.file = Some(
                    OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .share_mode(1)
                        .open(&self.path)?,
                );
                self.bytes = 0;
            }
        }
        let n = self
            .file
            .as_mut()
            .ok_or_else(|| error("Log rotation stopped"))?
            .write(data)?;
        self.bytes += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| error("Log rotation stopped"))?;
        file.flush()?;
        file.sync_all()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn directory() -> tempfile::TempDir {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/log-rotation-tests");
        fs::create_dir_all(&root).unwrap();
        tempfile::tempdir_in(root).unwrap()
    }
    fn policy(keep_files: u8) -> Option<LogRotation> {
        Some(LogRotation {
            max_mib: 1,
            keep_files,
        })
    }
    fn contents(path: &Path) -> Vec<Vec<u8>> {
        let mut entries = fs::read_dir(sibling(path, ".yuuterm-rotated").unwrap())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "log"))
            .collect::<Vec<_>>();
        entries.sort();
        entries.push(path.into());
        entries.iter().map(|p| fs::read(p).unwrap()).collect()
    }
    #[test]
    fn exact_boundary_raw_bytes_retention_and_reopen() {
        let dir = directory();
        let path = dir.path().join("active.log");
        let (mut writer, _) = LogFile::open(&path, policy(3)).unwrap();
        for byte in [0xff, 0, 0x1b, b'd'] {
            writer.write_all(&vec![byte; 1024 * 1024]).unwrap();
        }
        writer.flush().unwrap();
        drop(writer);
        let files = contents(&path);
        assert_eq!(files.len(), 3);
        assert_eq!(
            files,
            [
                vec![0; 1024 * 1024],
                vec![0x1b; 1024 * 1024],
                vec![b'd'; 1024 * 1024]
            ]
        );
        let (mut writer, bytes) = LogFile::open(&path, policy(2)).unwrap();
        assert_eq!(bytes, 1024 * 1024);
        assert_eq!(contents(&path).len(), 2);
        writer.write_all(b"\r\n\0\xfftail").unwrap();
        drop(writer);
        assert_eq!(
            contents(&path),
            [vec![b'd'; 1024 * 1024], b"\r\n\0\xfftail".to_vec()]
        );
    }
    #[test]
    fn held_guard_refuses_other_modes_across_rollover() {
        let dir = directory();
        let path = dir.path().join("active.log");
        let (mut writer, _) = LogFile::open(&path, policy(2)).unwrap();
        writer.write_all(&vec![b'a'; 1024 * 1024]).unwrap();
        writer.write_all(b"new active").unwrap();
        assert!(LogFile::open(&path, None).is_err());
        drop(writer);
        let (mut writer, _) = LogFile::open(&path, None).unwrap();
        writer.write_all(b" appended").unwrap();
        drop(writer);
        assert_eq!(fs::read(&path).unwrap(), b"new active appended");
    }
    #[test]
    fn unknown_entries_and_unowned_archives_are_preserved() {
        let dir = directory();
        let path = dir.path().join("active.log");
        let (mut writer, _) = LogFile::open(&path, policy(3)).unwrap();
        for b in 1..=3 {
            writer.write_all(&vec![b; 1024 * 1024]).unwrap();
        }
        drop(writer);
        let folder = sibling(&path, ".yuuterm-rotated").unwrap();
        fs::write(folder.join("user.txt"), b"unrelated").unwrap();
        assert!(LogFile::open(&path, policy(2)).is_err());
        assert_eq!(contents(&path).len(), 3); // no partial pruning before validation
        assert_eq!(fs::read(folder.join("user.txt")).unwrap(), b"unrelated");
        fs::remove_file(folder.join("user.txt")).unwrap();
        fs::write(folder.join(".owner"), b"not this logger").unwrap();
        assert!(LogFile::open(&path, policy(2)).is_err());
        assert_eq!(contents(&path).len(), 3);
    }
    #[test]
    fn archive_collision_and_reader_sharing_failure_never_replace_or_truncate() {
        let dir = directory();
        let path = dir.path().join("active.log");
        let (mut writer, _) = LogFile::open(&path, policy(2)).unwrap();
        writer.write_all(&vec![b'a'; 1024 * 1024]).unwrap();
        let collision = sibling(&path, ".yuuterm-rotated")
            .unwrap()
            .join("00000000000000000001.log");
        fs::write(&collision, b"unexpected").unwrap();
        assert!(writer.write_all(b"next").is_err());
        drop(writer);
        assert_eq!(fs::read(&collision).unwrap(), b"unexpected");
        assert_eq!(fs::metadata(&path).unwrap().len(), 1024 * 1024);
        fs::remove_file(collision).unwrap();
        let (mut writer, _) = LogFile::open(&path, policy(2)).unwrap();
        let reader = OpenOptions::new()
            .read(true)
            .share_mode(3)
            .open(&path)
            .unwrap();
        assert!(writer.write_all(b"next").is_err());
        drop(writer);
        drop(reader);
        assert_eq!(fs::metadata(&path).unwrap().len(), 1024 * 1024);
    }
    #[test]
    fn bounds_and_nonregular_metadata_fail_closed() {
        let dir = directory();
        let path = dir.path().join("active.log");
        assert!(
            LogFile::open(
                &path,
                Some(LogRotation {
                    max_mib: 0,
                    keep_files: 2
                })
            )
            .is_err()
        );
        assert!(!path.exists());
        let lock = sibling(&path, ".yuuterm-lock").unwrap();
        fs::write(&lock, b"unexpected").unwrap();
        assert!(LogFile::open(&path, policy(2)).is_err());
        fs::remove_file(lock).unwrap();
        let (writer, _) = LogFile::open(&path, policy(2)).unwrap();
        drop(writer);
        let folder = sibling(&path, ".yuuterm-rotated").unwrap();
        fs::create_dir(folder.join("00000000000000000001.log")).unwrap();
        assert!(LogFile::open(&path, policy(2)).is_err());
        fs::remove_dir(folder.join("00000000000000000001.log")).unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(64 * 1024 * 1024)
            .unwrap();
        assert!(LogFile::open(&path, policy(2)).is_err());
    }
    #[test]
    fn full_maximum_size_log_can_reopen_and_roll_without_truncation() {
        let dir = directory();
        let path = dir.path().join("active.log");
        File::create(&path)
            .unwrap()
            .set_len(64 * 1024 * 1024)
            .unwrap();
        let (mut writer, bytes) = LogFile::open(
            &path,
            Some(LogRotation {
                max_mib: 64,
                keep_files: 2,
            }),
        )
        .unwrap();
        assert_eq!(bytes, 64 * 1024 * 1024);
        writer.write_all(b"after64").unwrap();
        drop(writer);
        assert_eq!(fs::read(&path).unwrap(), b"after64");
        assert_eq!(
            fs::metadata(
                sibling(&path, ".yuuterm-rotated")
                    .unwrap()
                    .join("00000000000000000001.log")
            )
            .unwrap()
            .len(),
            64 * 1024 * 1024
        );
    }
}
