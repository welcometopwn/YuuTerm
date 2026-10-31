use crate::Profile;
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

const MAX_BYTES: u64 = 1024 * 1024;
fn default_scrollback() -> u32 {
    500
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub profiles: Vec<Profile>,
    pub mocha: bool,
    /// Theme id; empty means the legacy `mocha` choice decides.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub theme: String,
    pub font_size: f32,
    #[serde(default = "default_terminal_font")]
    pub font_family: String,
    #[serde(default = "default_scrollback")]
    pub scrollback_lines: u32,
    #[serde(default)]
    pub highlighting: Highlighting,
    #[serde(default)]
    pub statistics: StatisticsSettings,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub enum NetworkUnit {
    #[default]
    Megabytes,
    Kilobytes,
    Megabits,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatisticsSettings {
    pub enabled: bool,
    pub network_unit: NetworkUnit,
}
impl Default for StatisticsSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            network_unit: NetworkUnit::Megabytes,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HighlightRule {
    pub keywords: String,
    pub color: String,
    #[serde(default)]
    pub regex: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Highlighting {
    pub enabled: bool,
    pub ipv4: bool,
    #[serde(default = "default_ipv6")]
    pub ipv6: bool,
    pub ipv4_color: String,
    #[serde(default = "default_ip_color")]
    pub ipv6_color: String,
    pub rules: Vec<HighlightRule>,
}
fn default_terminal_font() -> String {
    "YuuTerm JetBrains Mono".into()
}
fn default_ipv6() -> bool {
    true
}
fn default_ip_color() -> String {
    "#89b4fa".into()
}
pub fn validate_terminal_font(name: &str) -> bool {
    !name.trim().is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || " ._-".contains(c))
}
pub fn validate_theme(id: &str) -> bool {
    id.len() <= 32
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
impl Default for Highlighting {
    fn default() -> Self {
        Self {
            enabled: false,
            ipv4: true,
            ipv6: true,
            ipv4_color: "#89b4fa".into(),
            ipv6_color: default_ip_color(),
            rules: [
                ("error, failed, failure, fatal, denied", "#f38ba8"),
                ("warning, warn, deprecated", "#f9e2af"),
                ("success, successful, passed, ready", "#a6e3a1"),
            ]
            .into_iter()
            .map(|(keywords, color)| HighlightRule {
                keywords: keywords.into(),
                color: color.into(),
                regex: false,
            })
            .collect(),
        }
    }
}
impl Highlighting {
    pub fn validate(&self) -> Result<(), String> {
        let color = |s: &str| {
            s.len() == 7
                && s.starts_with('#')
                && s.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        };
        if self.rules.len() > 12 || !color(&self.ipv4_color) || !color(&self.ipv6_color) {
            return Err("Highlighting needs at most 12 rules and #RRGGBB colors.".into());
        }
        for rule in &self.rules {
            if rule.keywords.len() > 1100 || !color(&rule.color) {
                return Err("Invalid highlighting rule or color.".into());
            }
            if rule.regex {
                if rule.keywords.is_empty()
                    || rule.keywords.len() > 128
                    || rule.keywords.chars().any(char::is_control)
                {
                    return Err(
                        "Regex patterns need 1–128 bytes without control characters.".into(),
                    );
                }
                regex::RegexBuilder::new(&rule.keywords)
                    .case_insensitive(true)
                    .size_limit(64 * 1024)
                    .dfa_size_limit(256 * 1024)
                    .build()
                    .map_err(|_| "Invalid or overly complex regular expression".to_string())?;
                continue;
            }
            let words: Vec<_> = rule
                .keywords
                .split(|c: char| c == ',' || c.is_ascii_whitespace())
                .filter(|s| !s.is_empty())
                .collect();
            if words.is_empty()
                || words.len() > 32
                || words.iter().any(|word| {
                    word.len() > 32
                        || !word
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                })
            {
                return Err("Each rule needs 1–32 literal ASCII words (letters, digits, _ or -), up to 32 characters each.".into());
            }
        }
        Ok(())
    }
}

impl Preferences {
    /// Change group metadata in one candidate; bookmarks and credential identities
    /// stay intact. None removes the group by making its saved sessions ungrouped.
    pub fn with_group_change(&self, group: &str, new_name: Option<&str>) -> Result<Self, String> {
        if group.is_empty()
            || !self
                .profiles
                .iter()
                .any(|p| !p.temporary && p.group == group)
        {
            return Err("Group no longer exists".into());
        }
        let target = match new_name {
            Some(name) => {
                let name = name.trim();
                if name.is_empty() || name.len() > 64 || name.chars().any(char::is_control) {
                    return Err(
                        "Group name must contain 1–64 UTF-8 bytes and no control characters".into(),
                    );
                }
                if name != group
                    && self
                        .profiles
                        .iter()
                        .any(|p| !p.temporary && p.group == name)
                {
                    return Err("A group with that name already exists".into());
                }
                name
            }
            None => "",
        };
        let mut candidate = self.clone();
        for profile in &mut candidate.profiles {
            if !profile.temporary && profile.group == group {
                profile.group = target.into();
            }
        }
        candidate.validate()?;
        Ok(candidate)
    }

    pub fn with_duplicate(&self, source_id: u64) -> Result<Self, String> {
        self.validate()?;
        if self.profiles.len() >= 1000 {
            return Err("Session limit reached.".into());
        }
        let mut copy = self
            .profiles
            .iter()
            .find(|p| p.id == source_id)
            .ok_or("Session no longer exists")?
            .clone();
        let source_position = self
            .profiles
            .iter()
            .position(|p| p.id == source_id)
            .ok_or("Session no longer exists")?;
        // A copy never inherits the source's saved password.
        copy.credential_id.clear();
        copy.id = self
            .profiles
            .iter()
            .map(|p| p.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or("Session ID limit reached")?;
        for n in 1..=1001 {
            let suffix = if n == 1 {
                " (copy)".to_string()
            } else {
                format!(" (copy {n})")
            };
            let mut end = copy.name.len().min(128 - suffix.len());
            while !copy.name.is_char_boundary(end) {
                end -= 1;
            }
            let name = format!("{}{}", &copy.name[..end], suffix);
            if !self.profiles.iter().any(|p| p.name == name) {
                copy.name = name;
                let mut candidate = self.clone();
                candidate.profiles.insert(source_position + 1, copy);
                candidate.validate()?;
                return Ok(candidate);
            }
        }
        Err("No free copy name available".into())
    }

    pub fn fresh() -> Self {
        Self {
            version: 1,
            font_size: 14.0,
            font_family: default_terminal_font(),
            scrollback_lines: default_scrollback(),
            ..Default::default()
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.highlighting.validate()?;
        if self.version != 1
            || !validate_terminal_font(&self.font_family)
            || !validate_theme(&self.theme)
            || !(0..=10_000).contains(&self.scrollback_lines)
            || self.profiles.len() > 1000
            || !self.font_size.is_finite()
            || !(8.0..=32.0).contains(&self.font_size)
        {
            return Err("Unsupported or invalid settings file.".into());
        }
        let mut ids = std::collections::HashSet::new();
        for p in &self.profiles {
            p.validate()?;
            if p.id == 0 || !ids.insert(p.id) {
                return Err("Duplicate or missing session ID.".into());
            }
        }
        Ok(())
    }
}

/// Portable mode: a `yuuterm.portable` file next to the executable keeps local
/// data (settings, host keys, editor recovery, WebView profile) in a `data`
/// folder beside it instead of %LOCALAPPDATA%\YuuTerm. Saved passwords stay in
/// the current Windows user's Credential Manager.
pub fn portable_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    dir.join("yuuterm.portable")
        .is_file()
        .then(|| dir.join("data"))
}

pub fn path() -> io::Result<PathBuf> {
    if let Some(dir) = portable_dir() {
        return Ok(dir.join("settings.json"));
    }
    std::env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join("YuuTerm").join("settings.json"))
        .ok_or_else(|| io::Error::other("LOCALAPPDATA is unavailable; settings were not saved."))
}

pub fn load() -> Result<Preferences, String> {
    let path = path().map_err(|e| e.to_string())?;
    load_from(&path)
}

fn load_from(path: &Path) -> Result<Preferences, String> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Preferences::fresh()),
        Err(e) => return Err(e.to_string()),
    };
    use io::Read;
    let mut data = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_BYTES {
        return Err("Settings exceed the 1 MiB limit.".into());
    }
    let prefs: Preferences = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
    prefs.validate()?;
    Ok(prefs)
}

pub fn save(prefs: &Preferences) -> Result<(), String> {
    let path = path().map_err(|e| e.to_string())?;
    save_to(prefs, &path)
}

fn save_to(prefs: &Preferences, path: &Path) -> Result<(), String> {
    prefs.validate()?;
    // Quick-connect sessions are never persisted.
    let mut stored = prefs.clone();
    stored.profiles.retain(|p| !p.temporary);
    let data = serde_json::to_vec_pretty(&stored).map_err(|e| e.to_string())?;
    if data.len() as u64 > MAX_BYTES {
        return Err("Settings exceed the 1 MiB limit.".into());
    }
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.tmp");
    use io::Write;
    let mut file = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
    file.write_all(&data)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    // On Windows rename replaces an existing file through MoveFileExW.
    std::fs::rename(temp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn duplicate_is_inserted_after_its_source_without_copying_credentials() {
        let mut prefs = Preferences::fresh();
        prefs.profiles = (1..=3)
            .map(|id| Profile {
                id,
                name: format!("Session {id}"),
                host: "example.test".into(),
                group: "Servers".into(),
                mac_address: "02:11:22:33:44:55".into(),
                ..Default::default()
            })
            .collect();
        prefs.profiles[1].credential_id = "0f8fad5b-d9cb-469f-a165-70867728950e".into();
        let copy = prefs.with_duplicate(2).unwrap();
        assert_eq!(
            copy.profiles.iter().map(|p| p.id).collect::<Vec<_>>(),
            [1, 2, 4, 3]
        );
        assert_eq!(copy.profiles[2].group, "Servers");
        assert_eq!(copy.profiles[2].mac_address, prefs.profiles[1].mac_address);
        assert!(copy.profiles[2].credential_id.is_empty());
        assert_eq!(
            copy.profiles[1].credential_id,
            prefs.profiles[1].credential_id
        );
    }
    #[test]
    fn batch_order_groups_and_wol_survive_disk_reload_and_exchange() {
        let mut prefs = Preferences::fresh();
        prefs.profiles = (1..=3)
            .map(|id| crate::Profile {
                id,
                name: format!("Session {id}"),
                host: "example.test".into(),
                mac_address: "02:11:22:33:44:55".into(),
                ..Default::default()
            })
            .collect();
        let moved = crate::session_actions::move_to(&prefs, &[3], "", Some(1)).unwrap();
        let moved = crate::session_actions::move_to(&moved, &[1, 2], "Servers", None).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "yuuterm-batch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = dir.join("settings.json");
        save_to(&moved, &path).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(
            loaded.profiles.iter().map(|p| p.id).collect::<Vec<_>>(),
            [3, 1, 2]
        );
        assert_eq!(loaded.profiles[1].group, "Servers");
        assert_eq!(loaded.profiles[1].mac_address, "02:11:22:33:44:55");
        let imported =
            crate::session_transfer::parse(&crate::session_transfer::export(&loaded).unwrap())
                .unwrap();
        assert_eq!(imported[1].mac_address, loaded.profiles[1].mac_address);
        let removed = crate::session_actions::remove(&loaded, &[1, 2]).unwrap();
        save_to(&removed, &path).unwrap();
        assert_eq!(load_from(&path).unwrap().profiles.len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn quick_connect_sessions_are_never_saved_or_exported() {
        let mut prefs = Preferences::fresh();
        for (id, temporary) in [(1, false), (2, true)] {
            prefs.profiles.push(crate::Profile {
                id,
                name: format!("s{id}"),
                host: "host.test".into(),
                temporary,
                ..Default::default()
            });
        }
        let dir = std::env::temp_dir().join(format!("yuuterm-quick-{}", std::process::id()));
        let path = dir.join("settings.json");
        save_to(&prefs, &path).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(
            loaded.profiles.iter().map(|p| p.id).collect::<Vec<_>>(),
            [1]
        );
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("temporary")
        );
        let exported = String::from_utf8(crate::session_transfer::export(&prefs).unwrap()).unwrap();
        assert!(exported.contains("s1") && !exported.contains("s2"));
        let merged =
            crate::session_transfer::merge(&Preferences::fresh(), &prefs.profiles[1..]).unwrap();
        assert!(
            !merged.profiles[0].temporary,
            "imports are always real bookmarks"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn logging_defaults_paths_and_invalid_saves_preserve_settings() {
        let old = r#"{"id":1,"name":"test","protocol":"Ssh","host":"example.test","port":22,"username":"test","font_size":null}"#;
        let mut profile: crate::Profile = serde_json::from_str(old).unwrap();
        assert!(!profile.logging.enabled && profile.logging.file.is_empty());
        profile.logging.enabled = true;
        profile.logging.file = r"C:\blabla\custom.log".into();
        let mut prefs = Preferences::fresh();
        prefs.profiles.push(profile);
        let dir = std::env::temp_dir().join(format!("yuuterm-log-settings-{}", std::process::id()));
        let path = dir.join("settings.json");
        save_to(&prefs, &path).unwrap();
        let loaded = load_from(&path).unwrap();
        assert!(loaded.profiles[0].logging.enabled);
        assert_eq!(loaded.profiles[0].logging.file, r"C:\blabla\custom.log");
        let original = std::fs::read(&path).unwrap();
        for invalid in [
            "relative.log",
            r"C:\logs\NUL.log",
            r"C:\logs\COM¹.log",
            r"C:\logs\file:stream",
            r"\\.\COM1",
            "C:\\logs\\bad\n.log",
        ] {
            prefs.profiles[0].logging.file = invalid.into();
            assert!(save_to(&prefs, &path).is_err(), "{invalid}");
            assert_eq!(std::fs::read(&path).unwrap(), original);
        }
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    use super::*;
    #[test]
    fn statistics_defaults_overrides_units_and_persistence_are_explicit() {
        let old = br#"{"version":1,"profiles":[{"id":1,"name":"fixture","protocol":"Ssh","host":"example.test","port":22,"username":"test"}],"mocha":false,"font_size":14}"#;
        let mut prefs: Preferences = serde_json::from_slice(old).unwrap();
        assert!(prefs.statistics.enabled);
        assert!(matches!(
            prefs.statistics.network_unit,
            NetworkUnit::Megabytes
        ));
        assert_eq!(prefs.profiles[0].statistics_enabled, None);
        prefs.statistics.enabled = false;
        prefs.statistics.network_unit = NetworkUnit::Megabits;
        prefs.profiles[0].statistics_enabled = Some(true);
        let dir =
            std::env::temp_dir().join(format!("yuuterm-statistics-test-{}", std::process::id()));
        let path = dir.join("settings.json");
        save_to(&prefs, &path).unwrap();
        let loaded = load_from(&path).unwrap();
        assert!(!loaded.statistics.enabled);
        assert!(matches!(
            loaded.statistics.network_unit,
            NetworkUnit::Megabits
        ));
        assert_eq!(loaded.profiles[0].statistics_enabled, Some(true));
        prefs.profiles[0].statistics_enabled = Some(false);
        prefs.statistics.network_unit = NetworkUnit::Kilobytes;
        save_to(&prefs, &path).unwrap();
        assert_eq!(
            load_from(&path).unwrap().profiles[0].statistics_enabled,
            Some(false)
        );
        let invalid = std::fs::read_to_string(&path)
            .unwrap()
            .replace("Kilobytes", "UnknownUnit");
        assert!(serde_json::from_str::<Preferences>(&invalid).is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn appearance_defaults_regex_and_font_roundtrip_are_validated() {
        let old = br##"{"version":1,"profiles":[],"mocha":false,"font_size":14,"highlighting":{"enabled":true,"ipv4":true,"ipv4_color":"#89b4fa","rules":[{"keywords":"error","color":"#ffffff"}]}}"##;
        let mut prefs: Preferences = serde_json::from_slice(old).unwrap();
        assert_eq!(prefs.scrollback_lines, 500);
        assert_eq!(prefs.font_family, "YuuTerm JetBrains Mono");
        assert!(prefs.highlighting.ipv6);
        assert_eq!(prefs.highlighting.ipv6_color, "#89b4fa");
        prefs.highlighting.ipv6_color = "#ffffff".into();
        assert!(!prefs.highlighting.rules[0].regex);
        prefs.font_family = "Consolas".into();
        prefs.scrollback_lines = 1000;
        prefs.highlighting.rules[0].regex = true;
        prefs.highlighting.rules[0].keywords = "warn[0-9]+|error".into();
        prefs.validate().unwrap();
        let dir =
            std::env::temp_dir().join(format!("yuuterm-appearance-test-{}", std::process::id()));
        let path = dir.join("settings.json");
        save_to(&prefs, &path).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.font_family, "Consolas");
        assert_eq!(loaded.scrollback_lines, 1000);
        assert!(loaded.highlighting.rules[0].regex);
        assert_eq!(loaded.highlighting.ipv6_color, "#ffffff");
        assert_eq!(loaded.highlighting.ipv4_color, "#89b4fa");
        let original = std::fs::read(&path).unwrap();
        prefs.scrollback_lines = 10_001;
        assert!(save_to(&prefs, &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        prefs.scrollback_lines = 1000;
        prefs.highlighting.rules[0].keywords = "(?<=error)foo".into();
        assert!(save_to(&prefs, &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        prefs.highlighting.rules[0].keywords = "error".into();
        prefs.font_family = "Consolas; url(example)".into();
        assert!(prefs.validate().is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn duplicate_preserves_source_settings_and_roundtrips_with_distinct_names_and_ids() {
        let mut prefs = Preferences::fresh();
        let mut source = Profile {
            id: 7,
            name: "服务器".repeat(14),
            host: "example.test".into(),
            ..Default::default()
        };
        source.ssh.config_host = "sample-alias".into();
        source.font_size = Some(18.0);
        prefs.profiles.push(source);
        let original = serde_json::to_value(&prefs.profiles[0]).unwrap();
        let first = prefs.with_duplicate(7).unwrap();
        let second = first.with_duplicate(7).unwrap();
        assert_eq!(serde_json::to_value(&second.profiles[0]).unwrap(), original);
        assert_eq!(second.profiles[1].id, 9);
        assert_eq!(second.profiles[2].id, 8);
        assert_ne!(second.profiles[1].name, second.profiles[2].name);
        let mut copied = serde_json::to_value(&second.profiles[1]).unwrap();
        copied["id"] = original["id"].clone();
        copied["name"] = original["name"].clone();
        assert_eq!(copied, original);
        let dir = std::env::temp_dir().join(format!("yuuterm-copy-test-{}", std::process::id()));
        let path = dir.join("settings.json");
        save_to(&second, &path).unwrap();
        assert_eq!(
            serde_json::to_value(load_from(&path).unwrap()).unwrap(),
            serde_json::to_value(&second).unwrap()
        );
        assert!(prefs.with_duplicate(999).is_err());
        prefs.profiles[0].id = u64::MAX;
        assert!(prefs.with_duplicate(u64::MAX).is_err());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn replaces_and_reloads_metadata_without_losing_original_on_invalid_save() {
        let dir = std::env::temp_dir().join(format!("yuuterm-store-test-{}", std::process::id()));
        let path = dir.join("settings.json");
        let mut p = Preferences::fresh();
        save_to(&p, &path).unwrap();
        p.mocha = true;
        save_to(&p, &path).unwrap();
        assert!(load_from(&path).unwrap().mocha);
        p.theme = "tokyo-night".into();
        save_to(&p, &path).unwrap();
        assert_eq!(load_from(&path).unwrap().theme, "tokyo-night");
        p.theme = "Nord; url(x)".into();
        assert!(save_to(&p, &path).is_err());
        p.theme.clear();
        let original = std::fs::read(&path).unwrap();
        p.version = 999;
        assert!(save_to(&p, &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        // Only these test-owned files are removed.
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn refuses_invalid_and_oversized_files() {
        let dir = std::env::temp_dir().join(format!("yuuterm-bounds-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, b"not json").unwrap();
        assert!(load_from(&path).is_err());
        std::fs::write(&path, vec![b' '; (MAX_BYTES + 1) as usize]).unwrap();
        assert!(matches!(load_from(&path), Err(e) if e.contains("1 MiB")));
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
    #[test]
    fn reject_ambiguous_or_unbounded_settings() {
        let mut p = Preferences::fresh();
        assert!(p.validate().is_ok());
        p.version = 2;
        assert!(p.validate().is_err());
        p.version = 1;
        let profile = Profile {
            id: 1,
            name: "Test".into(),
            host: "localhost".into(),
            ..Default::default()
        };
        p.profiles = vec![profile.clone(), profile];
        assert!(p.validate().is_err());
    }

    #[test]
    fn highlighting_preserves_old_settings_and_rejects_unbounded_rules() {
        let old = br##"{"version":1,"profiles":[],"mocha":false,"font_size":14}"##;
        let mut p: Preferences = serde_json::from_slice(old).unwrap();
        assert!(!p.highlighting.enabled);
        p.highlighting.enabled = true;
        let data = serde_json::to_vec(&p).unwrap();
        let loaded: Preferences = serde_json::from_slice(&data).unwrap();
        assert!(loaded.highlighting.enabled);
        assert_eq!(loaded.highlighting.rules[0].color, "#f38ba8");
        assert!(p.validate().is_ok());
        p.highlighting.rules[0].keywords = "(a+)+$".into();
        assert!(p.validate().is_err());
        p.highlighting = Highlighting::default();
        p.highlighting.rules[0].color = "url(example)".into();
        assert!(p.validate().is_err());
        p.highlighting = Highlighting::default();
        p.highlighting.rules = vec![p.highlighting.rules[0].clone(); 13];
        assert!(p.validate().is_err());
        p.highlighting = Highlighting::default();
        p.highlighting.rules[0].keywords = "word ".repeat(33);
        assert!(p.validate().is_err());
    }
    #[test]
    fn group_changes_preserve_all_session_data_and_roundtrip() {
        let mut prefs = Preferences::fresh();
        for (id, group, temporary) in [
            (1, "Servers", false),
            (2, "Servers", false),
            (3, "Other", false),
            (4, "Servers", true),
        ] {
            prefs.profiles.push(crate::Profile {
                id,
                name: format!("Session {id}"),
                host: "example.test".into(),
                group: group.into(),
                temporary,
                credential_id: format!("00000000-0000-0000-0000-{id:012x}"),
                ..Default::default()
            });
        }
        let renamed = prefs
            .with_group_change("Servers", Some("  Renamed  "))
            .unwrap();
        let mut expected = prefs.clone();
        for p in &mut expected.profiles[..2] {
            p.group = "Renamed".into();
        }
        assert_eq!(
            serde_json::to_value(&renamed).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        let removed = renamed.with_group_change("Renamed", None).unwrap();
        for p in &mut expected.profiles[..2] {
            p.group.clear();
        }
        assert_eq!(
            serde_json::to_value(&removed).unwrap(),
            serde_json::to_value(&expected).unwrap()
        );
        let loaded: Preferences =
            serde_json::from_slice(&serde_json::to_vec(&removed).unwrap()).unwrap();
        loaded.validate().unwrap();
        assert_eq!(loaded.profiles.len(), 4);
        assert_eq!(
            loaded.profiles[0].credential_id,
            "00000000-0000-0000-0000-000000000001"
        );
        assert_eq!(prefs.profiles[0].group, "Servers");
    }

    #[test]
    fn invalid_group_changes_leave_source_unchanged() {
        let mut prefs = Preferences::fresh();
        for (id, group) in [(1, "Servers"), (2, "Other")] {
            prefs.profiles.push(crate::Profile {
                id,
                name: format!("Session {id}"),
                host: "example.test".into(),
                group: group.into(),
                ..Default::default()
            });
        }
        let before = serde_json::to_value(&prefs).unwrap();
        for name in ["", "   ", "Other", "bad\nname", &"é".repeat(33)] {
            assert!(prefs.with_group_change("Servers", Some(name)).is_err());
            assert_eq!(serde_json::to_value(&prefs).unwrap(), before);
        }
        assert!(prefs.with_group_change("", None).is_err());
        assert!(prefs.with_group_change("Missing", None).is_err());
        assert!(prefs.with_group_change("Servers", Some("Servers")).is_ok());
    }

    #[test]
    fn log_rotation_is_opt_in_bounded_and_invalid_save_preserves_disk() {
        use crate::{LogRotation, LoggingSettings};
        let old: LoggingSettings = serde_json::from_str(r#"{"enabled":true,"file":""}"#).unwrap();
        assert!(old.rotation.is_none());
        let mut p = Preferences::fresh();
        p.profiles.push(crate::Profile {
            id: 1,
            name: "log fixture".into(),
            host: "example.test".into(),
            ..Default::default()
        });
        p.profiles[0].logging = LoggingSettings {
            enabled: true,
            file: String::new(),
            rotation: Some(LogRotation {
                max_mib: 1,
                keep_files: 2,
            }),
        };
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/log-rotation-tests");
        std::fs::create_dir_all(&root).unwrap();
        let dir = root.join(format!("core-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        save_to(&p, &path).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.profiles[0].logging.rotation.unwrap().keep_files, 2);
        let original = std::fs::read(&path).unwrap();
        for (max_mib, keep_files) in [(0, 2), (65, 2), (1, 0), (1, 1), (1, 11)] {
            p.profiles[0].logging.rotation = Some(LogRotation {
                max_mib,
                keep_files,
            });
            assert!(save_to(&p, &path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), original);
        }
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn telnet_line_mode_defaults_off_and_roundtrips_as_metadata_only() {
        let old=br#"{"id":1,"name":"TELNET fixture","protocol":"Telnet","host":"example.test","port":23,"username":"","font_size":null}"#;
        let mut profile: crate::Profile = serde_json::from_slice(old).unwrap();
        assert!(!profile.telnet.local_line_editing);
        profile.telnet.local_line_editing = true;
        let loaded: crate::Profile =
            serde_json::from_slice(&serde_json::to_vec(&profile).unwrap()).unwrap();
        assert!(loaded.telnet.local_line_editing);
        assert!(loaded.validate().is_ok());
        assert!(
            serde_json::from_str::<crate::TelnetSettings>(r#"{"local_line_editing":"yes"}"#)
                .is_err()
        );
        assert!(
            serde_json::from_str::<crate::TelnetSettings>(r#"{"command":"fixture-secret"}"#)
                .is_err()
        );
    }
}
