use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::PathBuf,
};

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileData {
    version: u32,
    hosts: BTreeMap<String, String>,
}

pub(crate) struct TrustStore {
    path: PathBuf,
    data: FileData,
}

impl TrustStore {
    pub(crate) fn snapshot(&self) -> BTreeMap<String, String> {
        self.data.hosts.clone()
    }
    pub fn load(path: PathBuf) -> Result<Self> {
        let data = match std::fs::File::open(&path) {
            Ok(f) => {
                let mut bytes = Vec::new();
                f.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
                anyhow::ensure!(bytes.len() <= 1024 * 1024, "Host-key store exceeds limit");
                let d: FileData = serde_json::from_slice(&bytes)
                    .context("Invalid host-key store; refusing to connect")?;
                anyhow::ensure!(
                    d.version == 1 && d.hosts.len() <= 1000,
                    "Invalid host-key store version/size"
                );
                d
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileData {
                version: 1,
                ..Default::default()
            },
            Err(e) => return Err(e).context("Cannot read host-key store; refusing to connect"),
        };
        Ok(Self { path, data })
    }

    pub fn check(&self, host: &str, key: &str) -> Result<bool> {
        match self.data.hosts.get(host) {
            Some(saved) if saved == key => Ok(true),
            Some(_) => bail!(
                "HOST KEY CHANGED for {host}. Connection blocked. Verify the server independently before reviewing the saved trust entry."
            ),
            None => Ok(false),
        }
    }

    /// Recheck under the caller's lock: concurrent prompts must never replace trust.
    pub fn remember(&mut self, host: &str, key: &str) -> Result<()> {
        if self.check(host, key)? {
            return Ok(());
        }
        anyhow::ensure!(self.data.hosts.len() < 1000, "Host-key store is full");
        let mut hosts = self.data.hosts.clone();
        hosts.insert(host.into(), key.into());
        let candidate = FileData { version: 1, hosts };
        let bytes = serde_json::to_vec_pretty(&candidate)?;
        anyhow::ensure!(bytes.len() <= 1024 * 1024, "Host-key store exceeds limit");
        std::fs::create_dir_all(self.path.parent().context("Host-key store has no parent")?)?;
        let temp = self.path.with_extension("json.tmp");
        let mut f = std::fs::File::create(&temp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(temp, &self.path)?;
        self.data = candidate;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trust_persists_and_changes_including_algorithm_changes_block() {
        let dir = std::env::temp_dir().join(format!("yuuterm-trust-{}", std::process::id()));
        let path = dir.join("hosts.json");
        let mut s = TrustStore::load(path.clone()).unwrap();
        assert!(!s.check("[example]:22", "ed25519 key1").unwrap());
        s.remember("[example]:22", "ed25519 key1").unwrap();
        let mut s = TrustStore::load(path.clone()).unwrap();
        assert!(s.check("[example]:22", "ed25519 key1").unwrap());
        assert!(s.remember("[example]:22", "rsa different").is_err());
        assert!(s.check("[example]:22", "ed25519 key1").unwrap());
        std::fs::write(&path, b"broken").unwrap();
        assert!(TrustStore::load(path.clone()).is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
