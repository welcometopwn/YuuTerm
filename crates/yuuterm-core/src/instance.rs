//! One settings writer per data directory, independent of executable location.
//! The persistent file is never renamed/deleted: ownership is the OS handle,
//! released on normal exit or a crash. It contains only bounded window metadata.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

pub struct Lease {
    file: File,
}

fn lock_path(settings: &Path) -> io::Result<PathBuf> {
    let directory = settings
        .parent()
        .ok_or_else(|| io::Error::other("Settings directory is missing"))?;
    fs::create_dir_all(directory)?;
    Ok(directory.join("instance.lock"))
}

/// `None` means another frontend owns this directory. Errors must fail closed.
pub fn acquire(settings: &Path) -> io::Result<Option<Lease>> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Permit bounded metadata readers, but no other writer or deletion.
        options.share_mode(1); // FILE_SHARE_READ
    }
    let file = match options.open(lock_path(settings)?) {
        #[cfg(windows)]
        Err(error) if error.raw_os_error() == Some(32) => return Ok(None),
        result => result?,
    };
    #[cfg(not(windows))]
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
        Err(std::fs::TryLockError::Error(error)) => return Err(error),
    }
    file.set_len(0)?;
    Ok(Some(Lease { file }))
}

impl Lease {
    pub fn publish(&mut self, record: &[u8; 32]) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(record)?;
        self.file.flush()
    }
}

pub fn read_record(settings: &Path) -> io::Result<[u8; 32]> {
    let mut file = File::open(lock_path(settings)?)?;
    let mut record = [0; 32];
    file.read_exact(&mut record)?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_metadata_aliases_and_release() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!(
                "yuuterm-instance-{}-{:?}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        let settings = root.join("one/settings.json");
        let mut first = acquire(&settings).unwrap().unwrap();
        assert!(acquire(&settings).unwrap().is_none());
        assert!(
            acquire(&root.join("one/another-settings-name.json"))
                .unwrap()
                .is_none()
        );
        #[cfg(windows)]
        {
            assert!(fs::remove_file(root.join("one/instance.lock")).is_err());
            assert!(
                fs::rename(root.join("one/instance.lock"), root.join("one/moved.lock")).is_err()
            );
        }
        assert!(
            acquire(&root.join("one/./settings.json"))
                .unwrap()
                .is_none()
        );
        let other = acquire(&root.join("two/settings.json")).unwrap().unwrap();
        assert!(read_record(&settings).is_err());
        first.publish(&[42; 32]).unwrap();
        assert_eq!(read_record(&settings).unwrap(), [42; 32]);
        drop(first);
        let next = acquire(&settings).unwrap().unwrap();
        assert!(
            read_record(&settings).is_err(),
            "stale window metadata is cleared"
        );
        drop((next, other));
        fs::create_dir_all(root.join("invalid/instance.lock")).unwrap();
        assert!(
            acquire(&root.join("invalid/settings.json")).is_err(),
            "inspection errors must fail closed"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
