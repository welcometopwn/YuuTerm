//! Read-only subset of OpenSSH identity policy. Never executes config commands.
use anyhow::{Context, Result, bail, ensure};
use russh::keys::{PublicKey, ssh_key::HashAlg};
use std::{
    collections::BTreeSet,
    io::Read,
    path::{Path, PathBuf},
};
use yuuterm_core::Profile;

#[derive(Default, Clone, Debug)]
struct Rules {
    only: Option<bool>,
    files: Vec<String>,
    hostname: Option<String>,
    user: Option<String>,
    port: Option<u16>,
}
struct Block {
    patterns: Vec<String>,
    lines: Vec<(String, Vec<String>)>,
}

fn words(text: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if quoted && chars.peek() == Some(&'"') => {
                word.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            '#' if !quoted => break,
            c if c.is_whitespace() && !quoted => {
                if !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
            }
            c => word.push(c),
        }
    }
    ensure!(!quoted, "Unclosed quote in SSH identity config");
    if !word.is_empty() {
        out.push(word);
    }
    ensure!(out.len() <= 64, "Too many arguments in SSH identity config");
    Ok(out)
}
fn wildcard(pattern: &str, host: &str) -> bool {
    let p = pattern.as_bytes();
    let h = host.as_bytes();
    let (mut i, mut j, mut star, mut retry) = (0, 0, None, 0);
    while j < h.len() {
        if i < p.len() && (p[i] == b'?' || p[i].eq_ignore_ascii_case(&h[j])) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == b'*' {
            star = Some(i);
            i += 1;
            retry = j;
        } else if let Some(s) = star {
            retry += 1;
            j = retry;
            i = s + 1;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == b'*' {
        i += 1;
    }
    i == p.len()
}
fn parse(text: &str) -> Result<Vec<Block>> {
    ensure!(
        text.len() <= 256 * 1024,
        "SSH identity config exceeds 256 KiB"
    );
    let mut blocks = vec![Block {
        patterns: vec!["*".into()],
        lines: Vec::new(),
    }];
    let mut patterns = 0;
    for (number, line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
        ensure!(
            number < 4096 && line.len() <= 16384,
            "SSH identity config exceeds line limits"
        );
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let end = line
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(line.len());
        let key = line[..end].to_ascii_lowercase();
        ensure!(
            !key.is_empty() && key.bytes().all(|b| b.is_ascii_alphanumeric()),
            "Malformed directive in SSH identity config"
        );
        let rest = line[end..].trim_start();
        let rest = rest.strip_prefix('=').unwrap_or(rest).trim_start();
        // Unsupported conditional scope must not silently broaden key selection.
        ensure!(
            !matches!(key.as_str(), "include" | "match"),
            "SSH identity config uses Include/Match, which is not supported yet; select an explicit fingerprint to bypass identity-config lookup"
        );
        if !matches!(
            key.as_str(),
            "host" | "hostname" | "user" | "port" | "identityfile" | "identitiesonly"
        ) {
            continue;
        }
        let values = words(rest)?;
        if key == "identitiesonly" {
            ensure!(
                values.len() == 1
                    && matches!(values[0].to_ascii_lowercase().as_str(), "yes" | "no"),
                "IdentitiesOnly must be yes or no"
            );
        }
        ensure!(!values.is_empty(), "Missing value in SSH identity config");
        if key == "host" {
            patterns += values.len();
            ensure!(
                patterns <= 256 && values.iter().all(|v| v.len() <= 512),
                "SSH identity host patterns exceed limits"
            );
            blocks.push(Block {
                patterns: values,
                lines: Vec::new(),
            });
        } else {
            ensure!(
                values.len() == 1,
                "Expected one value in SSH identity config"
            );
            blocks.last_mut().unwrap().lines.push((key, values));
        }
    }
    Ok(blocks)
}
fn rules(blocks: &[Block], host: &str) -> Result<Rules> {
    let mut out = Rules::default();
    for block in blocks {
        if block
            .patterns
            .iter()
            .any(|p| p.strip_prefix('!').is_some_and(|p| wildcard(p, host)))
            || !block
                .patterns
                .iter()
                .any(|p| !p.starts_with('!') && wildcard(p, host))
        {
            continue;
        }
        for (key, values) in &block.lines {
            let value = &values[0];
            match key.as_str() {
                "identitiesonly" if out.only.is_none() => {
                    out.only = Some(match value.to_ascii_lowercase().as_str() {
                        "yes" => true,
                        "no" => false,
                        _ => bail!("IdentitiesOnly must be yes or no"),
                    })
                }
                "identityfile" => {
                    ensure!(out.files.len() < 64, "Too many configured identity files");
                    if !out.files.contains(value) {
                        out.files.push(value.clone());
                    }
                }
                "hostname" if out.hostname.is_none() => out.hostname = Some(value.clone()),
                "user" if out.user.is_none() => out.user = Some(value.clone()),
                "port" if out.port.is_none() => {
                    let port = value
                        .parse::<u16>()
                        .context("Invalid SSH identity config port")?;
                    ensure!(port > 0, "Invalid SSH identity config port");
                    out.port = Some(port);
                }
                _ => {}
            }
        }
    }
    Ok(out)
}
fn effective(blocks: &[Block], p: &Profile) -> Result<(String, Rules)> {
    if !p.ssh.config_host.is_empty() {
        return Ok((
            p.ssh.config_host.clone(),
            rules(blocks, &p.ssh.config_host)?,
        ));
    }
    let direct = rules(blocks, &p.host)?;
    if !direct.files.is_empty() {
        return Ok((p.host.clone(), direct));
    }
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    for alias in blocks
        .iter()
        .flat_map(|b| &b.patterns)
        .filter(|v| !v.contains(['*', '?', '!']))
    {
        if !seen.insert(alias) {
            continue;
        }
        let r = rules(blocks, alias)?;
        if r.hostname
            .as_ref()
            .is_some_and(|h| h.eq_ignore_ascii_case(&p.host))
            && r.port.unwrap_or(22) == p.port
            && r.user.as_ref().is_none_or(|u| u == &p.username)
            && !r.files.is_empty()
        {
            candidates.push((alias.clone(), r));
        }
    }
    ensure!(
        candidates.len() <= 1,
        "Multiple SSH identity host entries match this endpoint; set OpenSSH identity host in session settings"
    );
    Ok(candidates.pop().unwrap_or((p.host.clone(), direct)))
}
fn public_path(value: &str, home: &Path, p: &Profile, alias: &str) -> Result<PathBuf> {
    ensure!(
        !value.contains("${"),
        "Environment expansion in IdentityFile is not supported"
    );
    let mut expanded = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            expanded.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => expanded.push('%'),
            Some('d') => expanded.push_str(&home.to_string_lossy()),
            Some('h') => expanded.push_str(&p.host),
            Some('n') => expanded.push_str(alias),
            Some('r') => expanded.push_str(&p.username),
            Some('p') => expanded.push_str(&p.port.to_string()),
            _ => bail!("Unsupported token in IdentityFile"),
        }
    }
    let path = if let Some(rest) = expanded
        .strip_prefix("~/")
        .or_else(|| expanded.strip_prefix("~\\"))
    {
        home.join(rest)
    } else {
        PathBuf::from(expanded)
    };
    ensure!(
        path.is_absolute() && !path.to_string_lossy().starts_with(r"\\"),
        "IdentityFile must use a local absolute or ~/ path"
    );
    // Agent mode only reads public files. Never opens a private-key file.
    Ok(if path.extension().is_some_and(|e| e == "pub") {
        path
    } else {
        let mut name = path.into_os_string();
        name.push(".pub");
        PathBuf::from(name)
    })
}
pub(crate) fn select(p: &Profile, home: &Path, keys: Vec<PublicKey>) -> Result<Vec<PublicKey>> {
    let mut seen = BTreeSet::new();
    let keys: Vec<_> = keys
        .into_iter()
        .filter(|k| seen.insert(k.fingerprint(HashAlg::Sha256).to_string()))
        .collect();
    if !p.ssh.identity_fingerprint.is_empty() {
        let selected: Vec<_> = keys
            .into_iter()
            .filter(|k| k.fingerprint(HashAlg::Sha256).to_string() == p.ssh.identity_fingerprint)
            .collect();
        ensure!(
            !selected.is_empty(),
            "Selected fingerprint is not available in the Windows SSH agent"
        );
        return Ok(selected);
    }
    let config = if p.ssh.config_file.is_empty() {
        home.join(".ssh/config")
    } else {
        PathBuf::from(&p.ssh.config_file)
    };
    ensure!(
        config.is_absolute() && !config.to_string_lossy().starts_with(r"\\"),
        "SSH identity config must be a local absolute path"
    );
    let text = match std::fs::File::open(config) {
        Ok(file) => {
            ensure!(
                file.metadata()?.is_file(),
                "SSH identity config is not a regular file"
            );
            let mut text = String::new();
            file.take(256 * 1024 + 1)
                .read_to_string(&mut text)
                .context("Cannot read UTF-8 SSH identity config")?;
            text
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && p.ssh.config_file.is_empty() => {
            String::new()
        }
        Err(_) => {
            bail!("Cannot access SSH identity config; refusing unfiltered agent authentication")
        }
    };
    let (alias, r) = effective(&parse(&text)?, p)?;
    let configured = !r.files.is_empty();
    let files = if configured {
        r.files.clone()
    } else {
        [
            "id_rsa",
            "id_ecdsa",
            "id_ed25519",
            "id_ecdsa_sk",
            "id_ed25519_sk",
        ]
        .iter()
        .map(|n| home.join(".ssh").join(n).to_string_lossy().into_owned())
        .collect()
    };
    let mut fingerprints = Vec::new();
    for value in files {
        if value.eq_ignore_ascii_case("none") {
            continue;
        }
        let path = public_path(&value, home, p, &alias)?;
        match std::fs::File::open(path) {
            Ok(file) => {
                ensure!(
                    file.metadata()?.is_file(),
                    "Identity public-key path is not a file"
                );
                let mut encoded = String::new();
                file.take(65537)
                    .read_to_string(&mut encoded)
                    .context("Cannot read identity public key")?;
                ensure!(
                    encoded.len() <= 65536,
                    "Identity public-key file exceeds 64 KiB"
                );
                let key = PublicKey::from_openssh(encoded.trim()).map_err(|_| {
                    anyhow::anyhow!(
                        "IdentityFile public-key companion is not a supported OpenSSH public key"
                    )
                })?;
                fingerprints.push(key.fingerprint(HashAlg::Sha256).to_string());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("Cannot access configured identity public key"),
        }
    }
    let mut selected = Vec::new();
    let mut remaining = keys;
    for fingerprint in fingerprints {
        if let Some(i) = remaining
            .iter()
            .position(|k| k.fingerprint(HashAlg::Sha256).to_string() == fingerprint)
        {
            selected.push(remaining.remove(i));
        }
    }
    if r.only == Some(true) {
        ensure!(
            !selected.is_empty(),
            "IdentitiesOnly=yes: no configured public key matches an agent identity; provide its .pub companion or select an explicit fingerprint"
        );
        return Ok(selected);
    }
    if selected.is_empty() && r.only.is_none() && remaining.len() > 1 {
        bail!(
            "Multiple agent identities are available without a resolved selection; configure IdentityFile/IdentitiesOnly or select a fingerprint (no keys offered)"
        );
    }
    selected.extend(remaining);
    ensure!(
        !selected.is_empty(),
        "No supported public keys are available in the Windows SSH agent"
    );
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::{Algorithm, PrivateKey};
    fn key() -> PublicKey {
        PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
            .unwrap()
            .public_key()
            .clone()
    }
    fn fixture() -> (tempfile::TempDir, Profile) {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join(".ssh")).unwrap();
        let p = Profile {
            name: "Fixture".into(),
            host: "192.0.2.9".into(),
            username: "fixture".into(),
            ..Default::default()
        };
        (home, p)
    }
    fn config(home: &Path, text: &str) {
        std::fs::write(home.join(".ssh/config"), text).unwrap();
    }
    #[test]
    fn host_patterns_first_values_and_additive_identity_files() {
        let b=parse("\u{feff}Host lab* !lab-bad\n IdentitiesOnly=yes # comment\n IdentityFile \"~/a key\"\n IdentityFile ~/b\nHost *\n IdentitiesOnly no\n IdentityFile ~/a\n").unwrap();
        let r = rules(&b, "LAB-good").unwrap();
        assert_eq!(r.only, Some(true));
        assert_eq!(r.files, vec!["~/a key", "~/b", "~/a"]);
        assert_eq!(rules(&b, "lab-bad").unwrap().only, Some(false));
        assert_eq!(rules(&b, "other").unwrap().files, vec!["~/a"]);
        assert!(parse("Match exec unsafe\n IdentityFile ~/key").is_err());
        assert!(parse("Include config.d/*").is_err());
        assert!(parse("IdentitiesOnly maybe").is_err());
        assert!(parse("\0Host other\nIdentityFile ~/key").is_err());
        assert!(parse(&"#".repeat(256 * 1024 + 1)).is_err());
        assert!(words("\"broken").is_err());
    }
    #[test]
    fn identities_only_selects_seventh_agent_key_and_never_falls_back() {
        let (home, p) = fixture();
        let keys: Vec<_> = (0..7).map(|_| key()).collect();
        std::fs::write(
            home.path().join(".ssh/chosen.pub"),
            keys[6].to_openssh().unwrap(),
        )
        .unwrap();
        config(
            home.path(),
            "Host *\n IdentitiesOnly yes\n IdentityFile ~/.ssh/chosen\n",
        );
        let selected = select(&p, home.path(), keys.clone()).unwrap();
        assert_eq!(selected, vec![keys[6].clone()]);
        assert!(select(&p, home.path(), keys[..6].to_vec()).is_err());
        std::fs::remove_file(home.path().join(".ssh/chosen.pub")).unwrap();
        // A private file at IdentityFile is never opened to derive its public key.
        std::fs::write(home.path().join(".ssh/chosen"), "not a public key").unwrap();
        assert!(select(&p, home.path(), keys).is_err());
    }
    #[test]
    fn unique_endpoint_alias_and_ambiguity_require_explicit_choice() {
        let (home, mut p) = fixture();
        let k = key();
        std::fs::write(home.path().join(".ssh/id.pub"), k.to_openssh().unwrap()).unwrap();
        let block = "Host appliance\n HostName 192.0.2.9\n User fixture\n Port 22\n IdentityFile ~/.ssh/id.pub\n IdentitiesOnly yes\n";
        config(home.path(), block);
        assert_eq!(
            select(&p, home.path(), vec![key(), k.clone()]).unwrap(),
            vec![k.clone()]
        );
        config(
            home.path(),
            &format!("{block}{}", block.replace("appliance", "other")),
        );
        assert!(select(&p, home.path(), vec![k.clone()]).is_err());
        p.ssh.config_host = "appliance".into();
        assert_eq!(select(&p, home.path(), vec![k.clone()]).unwrap(), vec![k]);
    }
    #[test]
    fn missing_policy_multiple_keys_and_explicit_pin_precedence() {
        let (home, mut p) = fixture();
        let a = key();
        let b = key();
        assert!(select(&p, home.path(), vec![a.clone(), b.clone()]).is_err());
        assert_eq!(
            select(&p, home.path(), vec![a.clone()]).unwrap(),
            vec![a.clone()]
        );
        config(home.path(), "Match exec must-not-run\n");
        p.ssh.identity_fingerprint = a.fingerprint(HashAlg::Sha256).to_string();
        assert_eq!(
            select(&p, home.path(), vec![b.clone(), a.clone(), a.clone()]).unwrap(),
            vec![a]
        );
        assert!(select(&p, home.path(), vec![b]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn supported_identity_directives_agree_with_windows_openssh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        // Entire config is synthetic and contains no executable/conditional hooks.
        let text = "Host demo* !demo-bad\n IdentitiesOnly=yes\n IdentityFile \"C:/fixture keys/a.pub\"\n IdentityFile C:/fixture/b.pub\nHost *\n IdentitiesOnly no\n IdentityFile C:/fixture/default.pub\n";
        std::fs::write(&path, text).unwrap();
        let blocks = parse(text).unwrap();
        let exe =
            PathBuf::from(std::env::var_os("WINDIR").unwrap()).join("System32/OpenSSH/ssh.exe");
        for host in ["demo-good", "demo-bad", "other"] {
            let result = std::process::Command::new(&exe)
                .arg("-G")
                .arg("-F")
                .arg(&path)
                .arg(host)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "Synthetic OpenSSH reference query failed"
            );
            let output = String::from_utf8(result.stdout).unwrap();
            let r = rules(&blocks, host).unwrap();
            let only = output
                .lines()
                .find_map(|l| l.strip_prefix("identitiesonly "))
                .unwrap();
            assert_eq!(only, if r.only.unwrap_or(false) { "yes" } else { "no" });
            let files: Vec<_> = output
                .lines()
                .filter_map(|l| l.strip_prefix("identityfile "))
                .collect();
            assert_eq!(files, r.files);
        }
    }
}
