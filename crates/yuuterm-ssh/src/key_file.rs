use anyhow::{Context, Result};
use russh::keys::{Error, PrivateKey, decode_secret_key};
use std::io::Read;
use zeroize::Zeroizing;

const LIMIT: usize = 1024 * 1024;

fn read(path: &str) -> Result<Zeroizing<String>> {
    let f = std::fs::File::open(path).context("Cannot access the selected private-key file")?;
    anyhow::ensure!(f.metadata()?.is_file(), "Private-key path is not a file");
    let mut encoded = Zeroizing::new(String::new());
    f.take(LIMIT as u64 + 1).read_to_string(&mut encoded)?;
    anyhow::ensure!(
        encoded.len() <= LIMIT,
        "Private-key file exceeds the 1 MiB limit"
    );
    Ok(encoded)
}

fn needs_passphrase(encoded: &str, error: &Error) -> bool {
    // Encrypted PKCS#8 fails to parse without a passphrase instead of
    // reporting KeyIsEncrypted.
    matches!(error, Error::KeyIsEncrypted) || encoded.contains("BEGIN ENCRYPTED PRIVATE KEY")
}

pub fn encrypted(path: &str) -> bool {
    read(path).is_ok_and(|encoded| {
        decode_secret_key(&encoded, None).is_err_and(|e| needs_passphrase(&encoded, &e))
    })
}

pub(crate) fn load(path: &str, passphrase: Option<&str>) -> Result<Option<PrivateKey>> {
    let encoded = read(path)?;
    match decode_secret_key(&encoded, None) {
        Ok(key) => Ok(Some(key)),
        Err(e) if needs_passphrase(&encoded, &e) => {
            let Some(passphrase) = passphrase.filter(|p| !p.is_empty()) else {
                anyhow::bail!(
                    "The private key is encrypted and no passphrase was entered; for gateways, load it into the Windows agent"
                );
            };
            Ok(decode_secret_key(&encoded, Some(passphrase)).ok())
        }
        Err(e) => Err(anyhow::Error::new(e).context("Cannot load the private-key file")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, content: &str) -> String {
        let dir = std::env::temp_dir().join(format!("yuuterm-key-file-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn plain_keys_load_without_a_passphrase_and_ignore_one() {
        let key = PrivateKey::random(
            &mut russh::keys::key::safe_rng(),
            russh::keys::Algorithm::Ed25519,
        )
        .unwrap();
        let pem = key
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .unwrap();
        let path = file("plain", &pem);
        assert!(!encrypted(&path));
        assert!(load(&path, None).unwrap().is_some());
        assert!(load(&path, Some("x")).unwrap().is_some());
    }

    #[test]
    fn encrypted_keys_need_the_right_passphrase() {
        let key = PrivateKey::random(
            &mut russh::keys::key::safe_rng(),
            russh::keys::Algorithm::Ed25519,
        )
        .unwrap()
        .encrypt(&mut russh::keys::key::safe_rng(), "secret")
        .unwrap();
        let pem = key
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .unwrap();
        let path = file("encrypted", &pem);
        assert!(encrypted(&path));
        assert!(load(&path, None).is_err());
        assert!(load(&path, Some("wrong")).unwrap().is_none());
        assert!(load(&path, Some("secret")).unwrap().is_some());
    }

    #[test]
    fn unreadable_files_are_not_reported_as_encrypted() {
        assert!(!encrypted("C:/does/not/exist/key"));
        assert!(load(&file("garbage", "not a key"), None).is_err());
    }
}
