//! In-memory files behind real SFTP packets; never maps to user or server paths.
use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Packet, Status, StatusCode, Version,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
#[derive(Clone)]
pub struct FixtureFile {
    pub data: Vec<u8>,
    pub attrs: FileAttributes,
}
pub type MemoryFiles = Arc<Mutex<HashMap<String, FixtureFile>>>;
pub fn fixture_files() -> MemoryFiles {
    let mut data = b"YuuTerm fixture: edit me safely.\n".to_vec();
    data.resize(42, b' ');
    Arc::new(Mutex::new(HashMap::from([
        (
            "/etc/passwd".into(),
            FixtureFile {
                data: b"root:x:0:0:root:/root:/bin/zsh\nfixture-user:x:1000:1000::/:/bin/sh\n"
                    .to_vec(),
                attrs: FileAttributes::default(),
            },
        ),
        (
            "/etc/group".into(),
            FixtureFile {
                data: b"root:x:0:\nfixture-group:x:1000:\n".to_vec(),
                attrs: FileAttributes::default(),
            },
        ),
        (
            "/fixture/read me.txt".into(),
            FixtureFile {
                data,
                attrs: FileAttributes {
                    size: Some(42),
                    permissions: Some(0o100600),
                    uid: Some(1000),
                    gid: Some(1000),
                    atime: Some(1700000000),
                    mtime: Some(1700000000),
                    ..Default::default()
                },
            },
        ),
        (
            "/fixture/logs".into(),
            FixtureFile {
                data: vec![],
                attrs: FileAttributes {
                    permissions: Some(0o040755),
                    ..Default::default()
                },
            },
        ),
    ])))
}
pub struct SftpFixture {
    read: usize,
    files: MemoryFiles,
    handles: HashMap<String, String>,
    next: u64,
    posix: bool,
}
impl SftpFixture {
    pub fn new(files: MemoryFiles, posix: bool) -> Self {
        Self {
            read: 0,
            files,
            handles: HashMap::new(),
            next: 0,
            posix,
        }
    }
}
fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: String::new(),
        language_tag: "en".into(),
    }
}
impl russh_sftp::server::Handler for SftpFixture {
    type Error = StatusCode;
    fn unimplemented(&self) -> StatusCode {
        StatusCode::OpUnsupported
    }
    async fn init(&mut self, _: u32, _: HashMap<String, String>) -> Result<Version, StatusCode> {
        let mut version = Version::new();
        if self.posix {
            version
                .extensions
                .insert("posix-rename@openssh.com".into(), "1".into());
        }
        Ok(version)
    }
    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, StatusCode> {
        let mut path = match path.as_str() {
            "." | "/fixture/logs/.." => "/fixture".to_owned(),
            _ => path,
        };
        while let Some(parent) = path.strip_suffix("/..") {
            path = parent
                .rsplit_once('/')
                .map_or("/", |(base, _)| if base.is_empty() { "/" } else { base })
                .to_owned();
        }
        let files = self.files.lock().unwrap();
        for _ in 0..16 {
            if path == "/fixture" || files.get(&path).is_some_and(|f| !f.attrs.is_symlink()) {
                return Ok(Name {
                    id,
                    files: vec![File::new(path, FileAttributes::default())],
                });
            }
            let file = files.get(&path).ok_or(StatusCode::NoSuchFile)?;
            if !file.attrs.is_symlink() {
                return Err(StatusCode::Failure);
            }
            let target = std::str::from_utf8(&file.data).map_err(|_| StatusCode::Failure)?;
            path = if target.starts_with('/') {
                target.into()
            } else {
                format!("{}/{target}", path.rsplit_once('/').unwrap().0)
            };
        }
        Err(StatusCode::Failure)
    }
    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, StatusCode> {
        self.read = 0;
        let handle = format!("dir:{path}");
        Ok(Handle { id, handle })
    }
    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, StatusCode> {
        let prefix = format!("{}/", handle.trim_start_matches("dir:"));
        let mut files = self
            .files
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(path, file)| {
                let name = path.strip_prefix(&prefix)?;
                if name.contains('/') {
                    return None;
                }
                Some(File::new(name, file.attrs.clone()))
            })
            .collect::<Vec<_>>();
        files.sort_by(|a, b| a.filename.cmp(&b.filename));
        if self.read >= files.len() {
            return Err(StatusCode::Eof);
        }
        // Split adversarial listings into bounded, genuine protocol packets.
        let files = files
            .into_iter()
            .skip(self.read)
            .take(32)
            .collect::<Vec<_>>();
        self.read += files.len();
        Ok(Name { id, files })
    }
    async fn open(
        &mut self,
        id: u32,
        filename: String,
        flags: OpenFlags,
        attrs: FileAttributes,
    ) -> Result<Handle, StatusCode> {
        let mut files = self.files.lock().unwrap();
        if flags.contains(OpenFlags::CREATE) {
            if flags.contains(OpenFlags::EXCLUDE) && files.contains_key(&filename) {
                return Err(StatusCode::Failure);
            }
            files.entry(filename.clone()).or_insert(FixtureFile {
                data: vec![],
                attrs: FileAttributes {
                    size: Some(0),
                    permissions: Some(0o100000 | attrs.permissions.unwrap_or(0o600)),
                    uid: Some(1000),
                    gid: Some(1000),
                    atime: Some(1700000001),
                    mtime: Some(1700000001),
                    ..Default::default()
                },
            });
        }
        if !files.contains_key(&filename) {
            return Err(StatusCode::NoSuchFile);
        }
        self.next += 1;
        let handle = format!("file:{}", self.next);
        self.handles.insert(handle.clone(), filename);
        Ok(Handle { id, handle })
    }
    async fn close(&mut self, id: u32, handle: String) -> Result<Status, StatusCode> {
        self.handles.remove(&handle);
        Ok(ok(id))
    }
    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        length: u32,
    ) -> Result<Data, StatusCode> {
        if self
            .handles
            .get(&handle)
            .is_some_and(|p| p.ends_with("slow.bin"))
        {
            let path = self.handles.get(&handle).unwrap().clone();
            self.files
                .lock()
                .unwrap()
                .get_mut(&path)
                .unwrap()
                .attrs
                .mtime = Some(1700000999);
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        }
        let files = self.files.lock().unwrap();
        let path = self.handles.get(&handle).ok_or(StatusCode::Failure)?;
        let file = files.get(path).ok_or(StatusCode::NoSuchFile)?;
        let start = offset as usize;
        if start >= file.data.len() {
            return Err(StatusCode::Eof);
        }
        Ok(Data {
            id,
            data: file.data[start..(start + length as usize).min(file.data.len())].to_vec(),
        })
    }
    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, StatusCode> {
        let mut files = self.files.lock().unwrap();
        let path = self.handles.get(&handle).ok_or(StatusCode::Failure)?;
        let file = files.get_mut(path).ok_or(StatusCode::NoSuchFile)?;
        let end = offset as usize + data.len();
        if end > 2 * 1024 * 1024 {
            return Err(StatusCode::Failure);
        }
        file.data.resize(file.data.len().max(end), 0);
        file.data[offset as usize..end].copy_from_slice(&data);
        file.attrs.size = Some(file.data.len() as u64);
        file.attrs.mtime = file.attrs.mtime.map(|m| m + 1);
        Ok(ok(id))
    }
    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        let files = self.files.lock().unwrap();
        if path == "/fixture" {
            return Ok(Attrs {
                id,
                attrs: FileAttributes {
                    permissions: Some(0o040755),
                    ..Default::default()
                },
            });
        }
        Ok(Attrs {
            id,
            attrs: files
                .get(&path)
                .ok_or(StatusCode::NoSuchFile)?
                .attrs
                .clone(),
        })
    }
    async fn setstat(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, StatusCode> {
        let mut files = self.files.lock().unwrap();
        if (attrs.uid.is_some() || attrs.gid.is_some())
            && files.contains_key("/fixture/reject-chown")
        {
            return Err(StatusCode::PermissionDenied);
        }
        let ignore_mode = files.contains_key("/fixture/ignore-mode");
        let file = files.get_mut(&path).ok_or(StatusCode::NoSuchFile)?;
        if let Some(mode) = attrs.permissions
            && !ignore_mode
        {
            file.attrs.permissions =
                Some((file.attrs.permissions.unwrap_or(0) & 0o170000) | (mode & 0o7777));
        }
        if let Some(uid) = attrs.uid {
            file.attrs.uid = Some(uid);
        }
        if let Some(gid) = attrs.gid {
            file.attrs.gid = Some(gid);
        }
        Ok(ok(id))
    }
    async fn rename(&mut self, id: u32, old: String, new: String) -> Result<Status, StatusCode> {
        let mut files = self.files.lock().unwrap();
        if files.contains_key(&new) {
            return Err(StatusCode::Failure);
        }
        let file = files.remove(&old).ok_or(StatusCode::NoSuchFile)?;
        files.insert(new, file);
        Ok(ok(id))
    }
    async fn remove(&mut self, id: u32, path: String) -> Result<Status, StatusCode> {
        let mut files = self.files.lock().unwrap();
        if files.get(&path).is_some_and(|f| f.attrs.is_dir()) {
            return Err(StatusCode::Failure);
        }
        files.remove(&path).ok_or(StatusCode::NoSuchFile)?;
        Ok(ok(id))
    }
    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, StatusCode> {
        let mut files = self.files.lock().unwrap();
        if !files.get(&path).is_some_and(|f| f.attrs.is_dir())
            || files.keys().any(|p| p.starts_with(&format!("{path}/")))
        {
            return Err(StatusCode::Failure);
        }
        files.remove(&path);
        Ok(ok(id))
    }
    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, StatusCode> {
        let mut files = self.files.lock().unwrap();
        if files.contains_key(&path) {
            return Err(StatusCode::Failure);
        }
        let parent = path.rsplit_once('/').map(|v| v.0).unwrap_or_default();
        if parent != "/fixture" && !files.get(parent).is_some_and(|f| f.attrs.is_dir()) {
            return Err(StatusCode::NoSuchFile);
        }
        files.insert(
            path,
            FixtureFile {
                data: vec![],
                attrs: FileAttributes {
                    permissions: Some(0o040000 | attrs.permissions.unwrap_or(0o700)),
                    ..Default::default()
                },
            },
        );
        Ok(ok(id))
    }
    async fn extended(
        &mut self,
        id: u32,
        request: String,
        data: Vec<u8>,
    ) -> Result<Packet, StatusCode> {
        if request != "posix-rename@openssh.com" {
            return Err(StatusCode::OpUnsupported);
        }
        let mut bytes = &data[..];
        let mut string = || -> Result<String, StatusCode> {
            if bytes.len() < 4 {
                return Err(StatusCode::BadMessage);
            }
            let len = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
            bytes = &bytes[4..];
            if bytes.len() < len {
                return Err(StatusCode::BadMessage);
            }
            let value =
                String::from_utf8(bytes[..len].to_vec()).map_err(|_| StatusCode::BadMessage)?;
            bytes = &bytes[len..];
            Ok(value)
        };
        let old = string()?;
        let new = string()?;
        let mut files = self.files.lock().unwrap();
        let file = files.remove(&old).ok_or(StatusCode::NoSuchFile)?;
        files.insert(new, file);
        Ok(Packet::Status(ok(id)))
    }
}

pub fn add_navigation_tree(files: &MemoryFiles) {
    let mut files = files.lock().unwrap();
    for path in ["/fixture/logs/one", "/fixture/logs/one/two"] {
        files.insert(
            path.into(),
            FixtureFile {
                data: Vec::new(),
                attrs: FileAttributes {
                    permissions: Some(0o040755),
                    ..Default::default()
                },
            },
        );
    }
}

pub fn add_links(files: &MemoryFiles) {
    let mut files = files.lock().unwrap();
    let regular = files["/fixture/read me.txt"].clone();
    files.insert("/fixture/logs/inner.txt".into(), regular.clone());
    for (name, target) in [
        ("file-link", "read me.txt"),
        ("dir-link", "logs"),
        ("broken-link", "missing"),
        ("cycle-link", "cycle-link"),
    ] {
        let mut link = regular.clone();
        link.data = target.as_bytes().to_vec();
        link.attrs.permissions = Some(0o120777);
        link.attrs.size = Some(link.data.len() as u64);
        files.insert(format!("/fixture/{name}"), link);
    }
}
