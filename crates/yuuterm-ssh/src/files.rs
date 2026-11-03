//! Bounded SFTP work, independent of the terminal input/output queue.
use crate::{Snapshot, Wake};
use anyhow::{Context, Result, bail, ensure};
use russh_sftp::{
    client::{RawSftpSession, error::Error as SftpError},
    protocol::{FileAttributes, OpenFlags, Packet, StatusCode},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot, watch},
};
use tokio_util::sync::CancellationToken;

pub const MAX_TRANSFER: u64 = 1024 * 1024 * 1024;
pub const MAX_EDIT: u64 = 64 * 1024 * 1024;
/// Bounds for a directory download: entries visited, nesting depth.
pub const MAX_TREE_ENTRIES: usize = 10_000;
pub const MAX_TREE_DEPTH: usize = 32;
/// Aggregate filename bytes retained for one recursive directory listing.
pub const MAX_TREE_NAME_BYTES: usize = 1024 * 1024;
const CHUNK: usize = 32768;

#[derive(Debug)]
pub enum FileAction {
    Create {
        path: String,
    },
    Mkdir {
        path: String,
    },
    Verify {
        path: String,
    },
    Revision {
        path: String,
    },
    Resolve {
        path: String,
    },
    Properties {
        path: String,
    },
    Rename {
        path: String,
        name: String,
    },
    Delete {
        path: String,
    },
    Permissions {
        path: String,
        mode: u32,
        /// Like `chmod -R`: the same mode for everything inside a directory.
        /// Symlinks and special files are skipped, never followed.
        recursive: bool,
    },
    Download {
        path: String,
        local: PathBuf,
        edit: bool,
    },
    Upload {
        path: String,
        local: PathBuf,
    },
    /// A regular file, or a directory copied recursively into `local`.
    /// Symlinks and special files inside the tree are skipped, never followed.
    DownloadTree {
        path: String,
        local: PathBuf,
    },
    Replace {
        path: String,
        local: PathBuf,
        expected: String,
        expected_local: Option<String>,
    },
}
impl FileAction {
    pub fn path(&self) -> &str {
        match self {
            Self::Create { path }
            | Self::Mkdir { path }
            | Self::Verify { path }
            | Self::Revision { path }
            | Self::Resolve { path }
            | Self::Properties { path }
            | Self::Rename { path, .. }
            | Self::Delete { path }
            | Self::Permissions { path, .. }
            | Self::Download { path, .. }
            | Self::Upload { path, .. }
            | Self::DownloadTree { path, .. }
            | Self::Replace { path, .. } => path,
        }
    }
    pub fn changes_remote(&self) -> bool {
        matches!(
            self,
            Self::Create { .. }
                | Self::Mkdir { .. }
                | Self::Rename { .. }
                | Self::Delete { .. }
                | Self::Permissions { .. }
                | Self::Upload { .. }
                | Self::Replace { .. }
        )
    }
    fn validate(&self) -> Result<()> {
        let path = self.path();
        ensure!(
            !path.is_empty() && path.len() <= 4096 && !path.contains('\0'),
            "Invalid remote path"
        );
        ensure!(
            path != "/"
                && path != "."
                && path != ".."
                && !path.ends_with("/.")
                && !path.ends_with("/.."),
            "Choose a concrete file or directory"
        );
        if let Self::Rename { name, .. } = self {
            validate_name(name)?;
        }
        if let Self::Permissions { mode, .. } = self {
            ensure!(*mode <= 0o7777, "Permissions must be between 0000 and 7777");
        }
        Ok(())
    }
}
pub fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 255
            && !matches!(name, "." | "..")
            && !name.contains(['/', '\\', '\0']),
        "Enter a single filename (at most 255 bytes)"
    );
    Ok(())
}

/// Drive-absolute paths returned by Windows OpenSSH (not relative `C:file`).
pub fn windows_drive_path(path: &str) -> bool {
    let path = path.strip_prefix('/').unwrap_or(path).as_bytes();
    path.len() >= 3
        && path[0].is_ascii_alphabetic()
        && path[1] == b':'
        && matches!(path[2], b'/' | b'\\')
}

#[derive(Clone, Debug, Serialize)]
pub struct Properties {
    /// Windows OpenSSH drive paths expose a read-only approximation, not NTFS ACLs.
    pub windows_permissions: bool,
    pub size: Option<u64>,
    pub permissions: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub mtime: Option<u32>,
    pub directory: bool,
    pub symlink: bool,
}
impl From<FileAttributes> for Properties {
    fn from(a: FileAttributes) -> Self {
        Self {
            windows_permissions: false,
            size: a.size,
            permissions: a.permissions,
            uid: a.uid,
            gid: a.gid,
            mtime: a.mtime,
            directory: a.is_dir(),
            symlink: a.is_symlink(),
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Resolved {
    pub path: String,
    pub directory: bool,
}

pub async fn resolve(sftp: &RawSftpSession, path: &str) -> Result<Resolved> {
    let names = sftp.realpath(path).await?.files;
    ensure!(
        names.len() == 1,
        "Server returned an ambiguous resolved path"
    );
    let path = names[0].filename.clone();
    ensure!(
        path.starts_with('/') && path.len() <= 4096 && !path.contains('\0'),
        "Invalid resolved path"
    );
    let attrs = sftp.lstat(&path).await?.attrs;
    ensure!(
        !attrs.is_symlink() && (attrs.is_dir() || attrs.is_regular()),
        "Link target is unavailable, unresolved or not a regular file/directory"
    );
    Ok(Resolved {
        path,
        directory: attrs.is_dir(),
    })
}

pub fn nano_command(path: &str) -> Result<Vec<u8>> {
    ensure!(
        !windows_drive_path(path),
        "Open in nano requires a POSIX shell; Windows OpenSSH paths are unsupported"
    );
    ensure!(
        path.starts_with('/') && path.len() <= 4096 && !path.chars().any(char::is_control),
        "Invalid terminal editor path"
    );
    Ok(format!("nano -- '{}'\r", path.replace('\'', "'\"'\"'")).into_bytes())
}

#[derive(Debug, Default, Serialize)]
pub struct FileResult {
    pub resolved: Option<Resolved>,
    pub properties: Option<Properties>,
    pub bytes: u64,
    pub fingerprint: Option<String>,
    /// Symlinks and special files left out of a directory download or
    /// recursive permission change.
    pub skipped: u64,
    /// Entries whose permissions were changed.
    pub changed: u64,
}
#[derive(Debug)]
pub(crate) enum Request {
    Browse(String),
    BrowseWait {
        path: String,
        cancel: CancellationToken,
        reply: oneshot::Sender<std::result::Result<String, String>>,
    },
    Action {
        action: FileAction,
        cancel: CancellationToken,
        reply: oneshot::Sender<Result<FileResult, String>>,
    },
}
#[derive(Clone)]
pub struct FileClient {
    tx: mpsc::Sender<Request>,
    session: CancellationToken,
}
impl FileClient {
    /// Complete only when the canonical directory/listing has been published.
    pub async fn browse(&self, path: String) -> std::result::Result<String, String> {
        if path.len() > 4096 || path.contains('\0') {
            return Err("Invalid directory path".into());
        }
        let cancel = self.session.child_token();
        let _guard = cancel.clone().drop_guard();
        let (reply, received) = oneshot::channel();
        tokio::select! {
            biased;
            _=cancel.cancelled()=>Err("Original connection closed".into()),
            result=tokio::time::timeout(Duration::from_secs(25),async {
                self.tx.send(Request::BrowseWait {path,cancel:cancel.clone(),reply}).await.map_err(|_| "SFTP is unavailable".to_owned())?;
                received.await.map_err(|_| "SFTP directory request stopped".to_owned())?
            })=>result.unwrap_or_else(|_|Err("Directory request timed out".into())),
        }
    }
    pub(crate) fn new(tx: mpsc::Sender<Request>, session: CancellationToken) -> Self {
        Self { tx, session }
    }
    pub async fn execute(
        &self,
        action: FileAction,
        cancel: CancellationToken,
    ) -> Result<FileResult, String> {
        action.validate().map_err(|e| e.to_string())?;
        if self.session.is_cancelled() || cancel.is_cancelled() {
            return Err("Original connection or operation was cancelled".into());
        }
        let (reply, rx) = oneshot::channel();
        self.tx
            .try_send(Request::Action {
                action,
                cancel: cancel.clone(),
                reply,
            })
            .map_err(|_| "SFTP is busy or unavailable")?;
        tokio::select! {
            biased;
            _=self.session.cancelled()=>Err("Original connection closed; remote result may be uncertain. Refresh before retrying.".into()),
            _=cancel.cancelled()=>Err("Cancelled; partial files may remain. Refresh/reconnect before retrying.".into()),
            result=rx=>result.unwrap_or_else(|_|Err("SFTP closed; remote result may be uncertain. Refresh before retrying.".into())),
        }
    }
}

pub(crate) async fn execute(
    sftp: &RawSftpSession,
    action: &FileAction,
    posix_rename: bool,
    state: &watch::Sender<Snapshot>,
    wake: &Wake,
) -> Result<FileResult> {
    action.validate()?;
    let path = action.path();
    match action {
        FileAction::Mkdir { .. } => {
            sftp.mkdir(
                path,
                FileAttributes {
                    permissions: Some(0o700),
                    ..Default::default()
                },
            )
            .await?;
            Ok(FileResult::default())
        }
        FileAction::Create { .. } => {
            let handle = sftp
                .open(
                    path,
                    OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE,
                    FileAttributes {
                        permissions: Some(0o600),
                        ..Default::default()
                    },
                )
                .await?
                .handle;
            sftp.close(handle).await?;
            Ok(FileResult::default())
        }
        FileAction::Verify { .. } => Ok(FileResult {
            fingerprint: Some(remote_hash(sftp, path).await?.0),
            ..Default::default()
        }),
        FileAction::Revision { .. } => {
            let attrs = regular(sftp, path, MAX_EDIT).await?;
            Ok(FileResult {
                fingerprint: Some(metadata_revision(&attrs)),
                ..Default::default()
            })
        }
        FileAction::Resolve { .. } => Ok(FileResult {
            resolved: Some(resolve(sftp, path).await?),
            ..Default::default()
        }),
        FileAction::Properties { .. } => {
            let mut properties: Properties = sftp.lstat(path).await?.attrs.into();
            properties.windows_permissions = windows_drive_path(path);
            Ok(FileResult {
                properties: Some(properties),
                ..Default::default()
            })
        }
        FileAction::Rename { name, .. } => {
            let parent = path
                .rsplit_once('/')
                .map_or(".", |(p, _)| if p.is_empty() { "/" } else { p });
            let target = format!("{}/{name}", parent.trim_end_matches('/'));
            ensure_missing(sftp, &target).await?;
            sftp.rename(path, target).await?;
            Ok(FileResult::default())
        }
        FileAction::Delete { .. } => {
            let attrs = sftp.lstat(path).await?.attrs;
            if attrs.is_dir() {
                sftp.rmdir(path).await?;
            } else {
                sftp.remove(path).await?;
            }
            Ok(FileResult::default())
        }
        FileAction::Permissions {
            mode, recursive, ..
        } => {
            let attrs = sftp.lstat(path).await?.attrs;
            ensure!(
                attrs.permissions.is_some() && !attrs.is_symlink(),
                "Permissions on symlinks or unknown file types are not changed"
            );
            if *recursive && attrs.is_dir() {
                ensure!(
                    !windows_drive_path(path),
                    "Recursive permissions are not available for Windows drive paths"
                );
                return chmod_tree(sftp, path, *mode).await;
            }
            set_mode(sftp, path, *mode).await?;
            Ok(FileResult {
                changed: 1,
                ..Default::default()
            })
        }
        FileAction::Download { local, edit, .. } => {
            let target = resolve(sftp, path).await?;
            ensure!(!target.directory, "Choose a regular file, not a directory");
            download(
                sftp,
                &target.path,
                local,
                if *edit { MAX_EDIT } else { MAX_TRANSFER },
                state,
                wake,
            )
            .await
        }
        FileAction::DownloadTree { local, .. } => {
            download_tree(sftp, path, local, state, wake).await
        }
        FileAction::Upload { local, .. } => {
            ensure_missing(sftp, path).await?;
            upload(sftp, path, local, None, posix_rename, state, wake).await
        }
        FileAction::Replace {
            local,
            expected,
            expected_local,
            ..
        } => {
            upload(
                sftp,
                path,
                local,
                Some((expected, expected_local.as_deref())),
                posix_rename,
                state,
                wake,
            )
            .await
        }
    }
}

#[cfg(test)]
mod browse_tests {
    use super::*;
    #[tokio::test]
    async fn cancelled_browse_calls_cancel_the_queued_request_and_ignore_late_replies() {
        let (tx, mut rx) = mpsc::channel(1);
        let session = CancellationToken::new();
        let client = FileClient::new(tx, session.clone());
        let task = tokio::spawn(async move { client.browse("/fixture".into()).await });
        let Request::BrowseWait { cancel, reply, .. } = rx.recv().await.unwrap() else {
            panic!("Wrong request")
        };
        session.cancel();
        assert!(task.await.unwrap().is_err());
        assert!(cancel.is_cancelled());
        assert!(reply.send(Ok("/late".into())).is_err());
    }
    #[tokio::test]
    async fn dropped_browse_future_cancels_its_pending_request() {
        let (tx, mut rx) = mpsc::channel(1);
        let session = CancellationToken::new();
        let client = FileClient::new(tx, session.clone());
        let task = tokio::spawn(async move { client.browse("/fixture".into()).await });
        let Request::BrowseWait { cancel, .. } = rx.recv().await.unwrap() else {
            panic!("Wrong request")
        };
        task.abort();
        let _ = task.await;
        assert!(cancel.is_cancelled());
        assert!(!session.is_cancelled());
    }
}
async fn ensure_missing(sftp: &RawSftpSession, path: &str) -> Result<()> {
    match sftp.lstat(path).await {
        Err(SftpError::Status(s)) if s.status_code == StatusCode::NoSuchFile => Ok(()),
        Ok(_) => bail!("Destination already exists; no file was overwritten"),
        Err(e) => Err(e.into()),
    }
}
async fn regular(sftp: &RawSftpSession, path: &str, limit: u64) -> Result<FileAttributes> {
    let attrs = sftp.lstat(path).await?.attrs;
    ensure!(
        attrs.is_regular() && !attrs.is_symlink(),
        "Choose a regular file; symlinks and special files are not transferred"
    );
    ensure!(
        attrs.size.is_some_and(|s| s <= limit),
        "File exceeds the {} MiB preview limit or has unknown size",
        limit / 1024 / 1024
    );
    Ok(attrs)
}
fn fingerprint(attrs: &FileAttributes, hash: Sha256) -> String {
    format!(
        "{}:{}",
        metadata_revision(attrs),
        crate::hex::lower(hash.finalize())
    )
}
pub fn metadata_revision(attrs: &FileAttributes) -> String {
    format!(
        "{:?}:{:?}:{:?}:{:?}:{:?}",
        attrs.size, attrs.permissions, attrs.uid, attrs.gid, attrs.mtime
    )
}
fn progress(
    state: &watch::Sender<Snapshot>,
    wake: &Wake,
    verb: &str,
    bytes: u64,
    last: &mut Instant,
) {
    if last.elapsed() >= Duration::from_millis(150) {
        state.send_modify(|s| s.files_status = format!("{verb} · {bytes} bytes"));
        wake();
        *last = Instant::now();
    }
}

async fn remote_hash(sftp: &RawSftpSession, path: &str) -> Result<(String, FileAttributes)> {
    let attrs = regular(sftp, path, MAX_EDIT).await?;
    let handle = sftp
        .open(path, OpenFlags::READ, FileAttributes::default())
        .await?
        .handle;
    let result = async {
        let mut hash = Sha256::new();
        let mut offset = 0;
        loop {
            match sftp.read(&handle, offset, CHUNK as u32).await {
                Ok(data) => {
                    ensure!(
                        !data.data.is_empty() && data.data.len() <= CHUNK,
                        "Invalid SFTP data length"
                    );
                    offset += data.data.len() as u64;
                    ensure!(offset <= MAX_EDIT, "Edit file grew beyond preview limit");
                    hash.update(data.data);
                }
                Err(SftpError::Status(s)) if s.status_code == StatusCode::Eof => break,
                Err(e) => return Err(e.into()),
            }
        }
        ensure!(
            Some(offset) == attrs.size,
            "Remote file changed during verification"
        );
        Ok((fingerprint(&attrs, hash), attrs))
    }
    .await;
    let closed = sftp.close(handle).await;
    let result = result?;
    closed?;
    Ok(result)
}

async fn download(
    sftp: &RawSftpSession,
    path: &str,
    local: &Path,
    limit: u64,
    state: &watch::Sender<Snapshot>,
    wake: &Wake,
) -> Result<FileResult> {
    let attrs = regular(sftp, path, limit).await?;
    let parent = local
        .parent()
        .context("Destination has no parent directory")?;
    let partial = parent.join(format!(".yuuterm-{:032x}.part", rand::random::<u128>()));
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .await?;
    let handle = sftp
        .open(path, OpenFlags::READ, FileAttributes::default())
        .await?
        .handle;
    let result = async {
        let mut hash = Sha256::new();
        let mut offset = 0;
        let mut last = Instant::now();
        loop {
            match sftp.read(&handle, offset, CHUNK as u32).await {
                Ok(data) => {
                    ensure!(
                        !data.data.is_empty() && data.data.len() <= CHUNK,
                        "Invalid SFTP data length"
                    );
                    offset += data.data.len() as u64;
                    ensure!(offset <= limit, "File grew beyond transfer limit");
                    hash.update(&data.data);
                    file.write_all(&data.data).await?;
                    progress(state, wake, "Downloading", offset, &mut last);
                }
                Err(SftpError::Status(s)) if s.status_code == StatusCode::Eof => break,
                Err(e) => return Err(e.into()),
            }
        }
        ensure!(
            Some(offset) == attrs.size,
            "Remote file changed size during download; partial retained"
        );
        file.sync_all().await?;
        Ok(FileResult {
            bytes: offset,
            properties: Some(attrs.clone().into()),
            fingerprint: Some(fingerprint(&attrs, hash)),
            ..Default::default()
        })
    }
    .await;
    let closed = sftp.close(handle).await;
    drop(file);
    let result =
        result.with_context(|| format!("Partial download retained at {}", partial.display()))?;
    closed?;
    tokio::fs::rename(&partial, local).await.with_context(|| {
        format!(
            "Could not finish download; partial retained at {}",
            partial.display()
        )
    })?;
    Ok(result)
}

/// Map a remote filename to a name Windows accepts.
pub fn windows_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .take(100)
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim_end_matches(['.', ' ']);
    let stem = cleaned
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4;
    if cleaned.is_empty() || reserved {
        format!("remote-{cleaned}")
    } else {
        cleaned.to_owned()
    }
}

async fn list(sftp: &RawSftpSession, path: &str) -> Result<Vec<(String, FileAttributes)>> {
    let handle = sftp.opendir(path).await?.handle;
    let result = async {
        let mut entries = Vec::new();
        let mut name_bytes = 0usize;
        loop {
            match sftp.readdir(handle.clone()).await {
                Ok(batch) => {
                    ensure!(
                        !batch.files.is_empty(),
                        "Server returned an empty directory batch"
                    );
                    for f in batch.files {
                        if f.filename == "." || f.filename == ".." {
                            continue;
                        }
                        ensure!(
                            !f.filename.is_empty()
                                && !f.filename.contains(['/', '\0'])
                                && entries.len() < MAX_TREE_ENTRIES,
                            "Server returned an invalid or oversized directory listing"
                        );
                        name_bytes = name_bytes.saturating_add(f.filename.len());
                        ensure!(
                            name_bytes <= MAX_TREE_NAME_BYTES,
                            "Directory listing exceeds the 1 MiB filename budget"
                        );
                        entries.push((f.filename, f.attrs));
                    }
                }
                Err(SftpError::Status(s)) if s.status_code == StatusCode::Eof => break,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(entries)
    }
    .await;
    let _ = sftp.close(handle).await;
    result
}

async fn set_mode(sftp: &RawSftpSession, path: &str, mode: u32) -> Result<()> {
    sftp.setstat(
        path,
        FileAttributes {
            permissions: Some(mode),
            ..Default::default()
        },
    )
    .await?;
    Ok(())
}

/// Lists the whole tree first and changes directories deepest first, so a mode
/// without read or execute cannot stop the walk halfway.
async fn chmod_tree(sftp: &RawSftpSession, path: &str, mode: u32) -> Result<FileResult> {
    let root = path.trim_end_matches('/').to_owned();
    let mut result = FileResult::default();
    let mut directories = vec![root.clone()];
    let mut files = Vec::new();
    let mut pending = vec![(root, 0)];
    while let Some((remote, depth)) = pending.pop() {
        for (name, attrs) in list(sftp, &remote).await? {
            ensure!(
                directories.len() + files.len() < MAX_TREE_ENTRIES,
                "Recursive permissions stopped at {MAX_TREE_ENTRIES} entries; nothing was changed"
            );
            let child = format!("{remote}/{name}");
            ensure!(
                child.len() <= 4096,
                "Remote child path exceeds the 4096-byte limit; nothing was changed"
            );
            if attrs.is_symlink() || !(attrs.is_dir() || attrs.is_regular()) {
                result.skipped += 1;
            } else if attrs.is_dir() {
                ensure!(
                    depth < MAX_TREE_DEPTH,
                    "{child} is nested deeper than {MAX_TREE_DEPTH} levels; nothing was changed"
                );
                directories.push(child.clone());
                pending.push((child, depth + 1));
            } else {
                files.push(child);
            }
        }
    }
    // Parents are listed before their children, so reversing puts children first.
    for target in files.iter().chain(directories.iter().rev()) {
        set_mode(sftp, target, mode).await.with_context(|| {
            format!(
                "{target}: recursive permissions stopped after {} entries",
                result.changed
            )
        })?;
        result.changed += 1;
    }
    Ok(result)
}

async fn download_tree(
    sftp: &RawSftpSession,
    path: &str,
    local: &Path,
    state: &watch::Sender<Snapshot>,
    wake: &Wake,
) -> Result<FileResult> {
    let attrs = sftp.lstat(path).await?.attrs;
    if !attrs.is_dir() || attrs.is_symlink() {
        return download(sftp, path, local, MAX_TRANSFER, state, wake).await;
    }
    let mut result = FileResult::default();
    let mut visited = 0;
    let mut pending = vec![(path.trim_end_matches('/').to_owned(), local.to_owned(), 0)];
    while let Some((remote, target, depth)) = pending.pop() {
        // create_dir fails on an existing path, so nothing local is merged or overwritten.
        tokio::fs::create_dir(&target)
            .await
            .with_context(|| format!("Cannot create {}", target.display()))?;
        let mut names = std::collections::HashSet::new();
        for (name, attrs) in list(sftp, &remote).await? {
            visited += 1;
            ensure!(
                visited <= MAX_TREE_ENTRIES,
                "Directory download stopped at {MAX_TREE_ENTRIES} entries; completed files remain"
            );
            let child = format!("{remote}/{name}");
            ensure!(
                child.len() <= 4096,
                "Remote child path exceeds the 4096-byte limit; completed files remain"
            );
            let local_name = windows_name(&name);
            ensure!(
                names.insert(local_name.to_lowercase()),
                "{child} collides with another name on Windows; completed files remain"
            );
            let local_child = target.join(&local_name);
            if attrs.is_symlink() || !(attrs.is_dir() || attrs.is_regular()) {
                result.skipped += 1;
            } else if attrs.is_dir() {
                ensure!(
                    depth < MAX_TREE_DEPTH,
                    "{child} is nested deeper than {MAX_TREE_DEPTH} levels"
                );
                pending.push((child, local_child, depth + 1));
            } else {
                let file = download(
                    sftp,
                    &child,
                    &local_child,
                    MAX_TRANSFER - result.bytes,
                    state,
                    wake,
                )
                .await
                .with_context(|| format!("{child}: directory download stopped"))?;
                result.bytes += file.bytes;
            }
        }
    }
    Ok(result)
}

async fn upload(
    sftp: &RawSftpSession,
    path: &str,
    local: &Path,
    expected: Option<(&String, Option<&str>)>,
    posix_rename: bool,
    state: &watch::Sender<Snapshot>,
    wake: &Wake,
) -> Result<FileResult> {
    let original = if let Some((expected, _)) = expected {
        ensure!(
            posix_rename,
            "Server lacks posix-rename@openssh.com; safe replacement is unavailable"
        );
        let (current, attrs) = remote_hash(sftp, path).await?;
        ensure!(
            &current == expected,
            "Conflict: the remote file changed. Your local edit is retained; download/review the remote version first."
        );
        Some(attrs)
    } else {
        None
    };
    let mut file = tokio::fs::File::open(local).await?;
    let metadata = file.metadata().await?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_TRANSFER,
        "Select a regular local file up to 1 GiB"
    );
    if let Some(attrs) = original.as_ref() {
        ensure!(
            metadata.len() <= MAX_EDIT,
            "Edited files are limited to 64 MiB"
        );
        ensure!(
            attrs.uid.is_some() && attrs.gid.is_some() && attrs.permissions.is_some(),
            "Server did not provide owner/group/mode; replacement is disabled"
        );
    }
    let parent = path
        .rsplit_once('/')
        .map_or(".", |(p, _)| if p.is_empty() { "/" } else { p });
    let temporary = format!(
        "{}/.yuuterm-{:032x}.part",
        parent.trim_end_matches('/'),
        rand::random::<u128>()
    );
    let handle = sftp
        .open(
            &temporary,
            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE,
            FileAttributes {
                permissions: Some(0o600),
                ..Default::default()
            },
        )
        .await?
        .handle;
    let result = async {
        let mut hash = Sha256::new();
        let mut offset = 0;
        let mut buffer = vec![0; CHUNK];
        let mut last = Instant::now();
        loop {
            let n = file.read(&mut buffer).await?;
            if n == 0 {
                break;
            }
            ensure!(
                offset + n as u64 <= MAX_TRANSFER,
                "Local file grew beyond transfer limit"
            );
            hash.update(&buffer[..n]);
            sftp.write(&handle, offset, buffer[..n].to_vec()).await?;
            offset += n as u64;
            progress(state, wake, "Uploading", offset, &mut last);
        }
        ensure!(
            offset == metadata.len(),
            "Local file changed size during upload"
        );
        Ok((offset, crate::hex::lower(hash.finalize())))
    }
    .await;
    let closed = sftp.close(handle).await;
    let (bytes, uploaded_hash) =
        result.with_context(|| format!("Incomplete remote upload retained at {temporary}"))?;
    closed?;
    if let Some((_, Some(expected_local))) = expected {
        ensure!(
            uploaded_hash == expected_local,
            "Local file changed again; the selected saved revision was not uploaded. Temporary upload retained."
        );
    }
    let mut verify = tokio::fs::File::open(local).await?;
    let mut local_hash = Sha256::new();
    let mut read = 0;
    let mut buffer = vec![0; CHUNK];
    loop {
        let n = verify.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        read += n as u64;
        ensure!(
            read <= MAX_TRANSFER,
            "Local file grew beyond transfer limit"
        );
        local_hash.update(&buffer[..n]);
    }
    ensure!(
        read == bytes && crate::hex::lower(local_hash.finalize()) == uploaded_hash,
        "Local file changed during upload; remote temporary upload retained at {temporary}"
    );
    if let Some(attrs) = original {
        let mode = attrs.permissions.map(|m| m & 0o7777);
        // Windows OpenSSH maps owners to 0/0 but rejects chown, including a
        // redundant 0/0 request. Do not change ownership when the newly created
        // sibling already has exactly the required owner and group. On any
        // mismatch retain the existing strict chown-and-verify requirement.
        let temporary_attrs = sftp.lstat(&temporary).await?.attrs;
        if temporary_attrs.uid != attrs.uid || temporary_attrs.gid != attrs.gid {
            sftp.setstat(
            &temporary,
            FileAttributes {
                uid: attrs.uid,
                gid: attrs.gid,
                ..Default::default()
            },
        )
        .await
        .context(
            "Could not preserve original permissions/owner/group; remote temporary file retained",
        )?;
        }
        // chown can clear set-ID bits; restore the mode only after owner/group.
        sftp.setstat(
            &temporary,
            FileAttributes {
                permissions: mode,
                ..Default::default()
            },
        )
        .await
        .context("Could not preserve original mode; remote temporary file retained")?;
        let preserved = sftp.lstat(&temporary).await?.attrs;
        ensure!(
            preserved.is_regular()
                && preserved.uid == attrs.uid
                && preserved.gid == attrs.gid
                && preserved.permissions.map(|m| m & 0o7777) == mode,
            "Server did not preserve original owner/group/mode; remote temporary file retained"
        );
        let (current, _) = remote_hash(sftp, path).await?;
        ensure!(
            Some(&current) == expected.map(|(remote, _)| remote),
            "Conflict during upload: remote file changed. Both original and temporary upload are retained."
        );
        let mut packet = Vec::new();
        for value in [&temporary, path] {
            packet.extend_from_slice(&(value.len() as u32).to_be_bytes());
            packet.extend_from_slice(value.as_bytes());
        }
        match sftp.extended("posix-rename@openssh.com", packet).await? {
            Packet::Status(s) if s.status_code == StatusCode::Ok => {}
            _ => bail!("Atomic replacement failed; remote temporary upload retained"),
        }
    } else {
        ensure_missing(sftp, path).await?;
        sftp.rename(&temporary, path).await?;
    }
    let fingerprint = if expected.is_some() {
        let (current, _) = remote_hash(sftp, path).await?;
        ensure!(
            current.ends_with(&format!(":{uploaded_hash}")),
            "Upload completed but remote content changed again; local copy retained"
        );
        Some(current)
    } else {
        None
    };
    Ok(FileResult {
        bytes,
        fingerprint,
        ..Default::default()
    })
}

/// Hash only a staged regular file, bounded by the edit limit.
pub async fn local_revision(path: &Path) -> Result<String> {
    let before = tokio::fs::symlink_metadata(path).await?;
    ensure!(
        before.is_file() && before.len() <= MAX_EDIT,
        "Local edit must be a regular file up to 64 MiB"
    );
    let mut file = tokio::fs::File::open(path).await?;
    let mut buffer = vec![0; CHUNK];
    let mut hash = Sha256::new();
    let mut read = 0u64;
    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        read += n as u64;
        ensure!(read <= MAX_EDIT, "Local edit grew beyond 64 MiB");
        hash.update(&buffer[..n]);
    }
    let after = tokio::fs::symlink_metadata(path).await?;
    ensure!(
        read == before.len()
            && after.len() == before.len()
            && after.modified()? == before.modified()?,
        "Local edit is still being saved"
    );
    Ok(crate::hex::lower(hash.finalize()))
}

#[cfg(test)]
mod editor_tests {
    #[test]
    fn windows_drive_paths_and_single_names_are_unambiguous() {
        for path in ["/C:/Users/test/a", "C:/Users/test/a", "c:\\Users\\test\\a"] {
            assert!(super::windows_drive_path(path));
            assert!(super::nano_command(path).is_err());
        }
        for path in ["C:relative", "/tmp/C:/file", "/home/test", "//server/share"] {
            assert!(!super::windows_drive_path(path));
        }
        for name in ["../escape", "..\\escape", "dir\\file"] {
            assert!(super::validate_name(name).is_err());
        }
        assert!(super::validate_name("quote ' space Ω.txt").is_ok());
    }
    #[test]
    fn windows_names_are_sanitized() {
        use super::windows_name;
        assert_eq!(windows_name("a:b?.txt"), "a_b_.txt");
        assert_eq!(windows_name("trailing. "), "trailing");
        assert_eq!(windows_name("con.log"), "remote-con.log");
        assert_eq!(windows_name("COM1"), "remote-COM1");
        assert_eq!(windows_name("..."), "remote-");
        assert_eq!(windows_name("normal name.txt"), "normal name.txt");
    }
    #[test]
    fn nano_path_is_quoted_and_control_input_is_refused() {
        let bytes = super::nano_command("/tmp/a'$(touch nope).txt").unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "nano -- '/tmp/a'\"'\"'$(touch nope).txt'\r"
        );
        for path in ["relative", "/tmp/a\nwhoami", "/tmp/a\u{1b}"] {
            assert!(super::nano_command(path).is_err());
        }
    }
}
