//! Versioned bookmark exchange. Live sessions, trust and credentials are excluded.
use crate::{Profile, config::Preferences};
use serde::{Deserialize, Serialize};

pub const MAX_BYTES: usize = 1024 * 1024;
pub fn parse_import(bytes: &[u8]) -> Result<crate::mobaxterm::Import, String> {
    parse_import_with_icons(bytes, &std::collections::BTreeMap::new())
}
pub fn parse_import_with_icons(
    bytes: &[u8],
    icons: &std::collections::BTreeMap<u16, String>,
) -> Result<crate::mobaxterm::Import, String> {
    if bytes.iter().copied().find(|b| !b.is_ascii_whitespace()) == Some(b'{') {
        Ok(crate::mobaxterm::Import {
            format: "YuuTerm",
            profiles: parse(bytes)?,
            skipped: Vec::new(),
        })
    } else {
        crate::mobaxterm::parse_with_icons(bytes, icons)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: String,
    version: u32,
    profiles: Vec<Profile>,
}

pub fn export(prefs: &Preferences) -> Result<Vec<u8>, String> {
    prefs.validate()?;
    let document = Document {
        format: "YuuTerm sessions".into(),
        version: 1,
        profiles: prefs
            .profiles
            .iter()
            .filter(|p| !p.temporary)
            .cloned()
            .map(|mut p| {
                p.credential_id.clear();
                p
            })
            .collect(),
    };
    let bytes = serde_json::to_vec_pretty(&document).map_err(|_| "Cannot encode sessions")?;
    if bytes.len() > MAX_BYTES {
        return Err("Session export exceeds 1 MiB".into());
    }
    Ok(bytes)
}

pub fn parse(bytes: &[u8]) -> Result<Vec<Profile>, String> {
    if bytes.len() > MAX_BYTES {
        return Err("Session import exceeds 1 MiB".into());
    }
    // Do not reflect untrusted values or secret-bearing JSON in error messages.
    let document: Document =
        serde_json::from_slice(bytes).map_err(|_| "Not a supported YuuTerm session export")?;
    if document.format != "YuuTerm sessions" || document.version != 1 {
        return Err("Unsupported session export format/version".into());
    }
    if document.profiles.is_empty() {
        return Err("The export contains no sessions".into());
    }
    let mut check = Preferences::fresh();
    check.profiles = document.profiles;
    check.validate()?;
    // Imports must not activate automatic local file writes at attacker-chosen paths.
    for p in &mut check.profiles {
        p.logging = Default::default();
        p.credential_id.clear();
    }
    Ok(check.profiles)
}

pub fn merge(current: &Preferences, incoming: &[Profile]) -> Result<Preferences, String> {
    current.validate()?;
    if incoming.is_empty() || incoming.len() > 1000 {
        return Err("Import needs 1–1000 sessions within the saved-session limit".into());
    }
    let mut candidate = current.clone();
    let mut next = current.profiles.iter().map(|p| p.id).max().unwrap_or(0);
    for source in incoming {
        source.validate()?;
        if candidate.profiles.iter().any(|p| same_bookmark(p, source)) {
            continue;
        }
        if candidate.profiles.len() >= 1000 {
            return Err("Import exceeds the saved-session limit".into());
        }
        let mut p = source.clone();
        next = next.checked_add(1).ok_or("Session ID limit reached")?;
        p.id = next;
        p.logging = Default::default();
        p.credential_id.clear();
        p.temporary = false;
        if candidate.profiles.iter().any(|v| v.name == p.name) {
            for n in 1..=1001 {
                let name = collision_name(&source.name, n);
                if !candidate.profiles.iter().any(|v| v.name == name) {
                    p.name = name;
                    break;
                }
            }
        }
        candidate.profiles.push(p);
    }
    candidate.validate()?;
    Ok(candidate)
}

// Imported settings never overwrite an existing bookmark. Match its stable
// connection identity, including folder/name, rather than optional appearance,
// authentication or terminal settings the user may have edited after migration.
fn same_bookmark(existing: &Profile, source: &Profile) -> bool {
    !existing.temporary
        && existing.protocol == source.protocol
        && existing.host.eq_ignore_ascii_case(&source.host)
        && existing.port == source.port
        && existing.username == source.username
        && existing.group == source.group
        && (existing.name == source.name || imported_name_matches(&existing.name, &source.name))
}

fn imported_name_matches(existing: &str, source: &str) -> bool {
    let n = if existing.ends_with(" (imported)") {
        Some(1)
    } else {
        existing
            .strip_suffix(')')
            .and_then(|s| s.rsplit_once(" (imported "))
            .and_then(|(_, n)| n.parse::<usize>().ok())
            .filter(|n| (2..=1001).contains(n))
    };
    n.is_some_and(|n| existing == collision_name(source, n))
}

fn collision_name(source: &str, n: usize) -> String {
    let suffix = if n == 1 {
        " (imported)".into()
    } else {
        format!(" (imported {n})")
    };
    let mut end = source.len().min(128 - suffix.len());
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &source[..end], suffix)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prefs() -> Preferences {
        let mut p = Preferences::fresh();
        p.profiles.push(Profile {
            id: 7,
            name: "fixture".into(),
            host: "example.test".into(),
            username: "test".into(),
            ..Default::default()
        });
        p
    }
    #[test]
    fn saved_password_identity_never_leaves_or_enters_through_exchange_or_copy() {
        let mut p = prefs();
        p.profiles[0].credential_id = "0f8fad5b-d9cb-469f-a165-70867728950e".into();
        p.validate().unwrap();
        let bytes = export(&p).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("credential_id"));
        let mut forged: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        forged["profiles"][0]["credential_id"] = p.profiles[0].credential_id.clone().into();
        let incoming = parse(&serde_json::to_vec(&forged).unwrap()).unwrap();
        assert!(incoming[0].credential_id.is_empty());
        let mut smuggled = incoming.clone();
        smuggled[0].credential_id = p.profiles[0].credential_id.clone();
        let merged = merge(&p, &smuggled).unwrap();
        assert_eq!(merged.profiles.len(), 1);
        assert_eq!(
            merged.profiles[0].credential_id,
            p.profiles[0].credential_id
        );
        smuggled[0].host = "new.test".into();
        assert!(
            merge(&p, &smuggled).unwrap().profiles[1]
                .credential_id
                .is_empty()
        );
        let copied = p.with_duplicate(p.profiles[0].id).unwrap();
        assert!(copied.profiles.last().unwrap().credential_id.is_empty());
        assert_eq!(
            copied.profiles[0].credential_id,
            p.profiles[0].credential_id
        );
        for bad in [
            "not-a-uuid",
            "00000000-0000-0000-0000-000000000000",
            "0F8FAD5B-D9CB-469F-A165-70867728950E",
        ] {
            p.profiles[0].credential_id = bad.into();
            assert!(p.validate().is_err(), "{bad}");
        }
    }
    #[test]
    fn roundtrip_excludes_global_settings_and_disables_imported_logging() {
        let mut p = prefs();
        p.profiles[0].logging.enabled = true;
        p.profiles[0].logging.file = r"C:\fixture\output.log".into();
        p.profiles[0].logging.rotation = Some(crate::LogRotation {
            max_mib: 1,
            keep_files: 2,
        });
        let bytes = export(&p).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 3);
        let incoming = parse(&bytes).unwrap();
        assert!(!incoming[0].logging.enabled);
        assert!(incoming[0].logging.file.is_empty());
        assert!(incoming[0].logging.rotation.is_none());
        let merged = merge(&p, &incoming).unwrap();
        assert_eq!(merged.profiles[0].id, 7);
        assert!(merged.profiles[0].logging.enabled);
        assert_eq!(merged.profiles.len(), 1);
        assert_eq!(merged.profiles[0].logging.file, p.profiles[0].logging.file);
    }
    #[test]
    fn hostile_and_secret_fields_fail_without_reflecting_values() {
        let bytes = export(&prefs()).unwrap();
        for field in ["password", "private_key", "host_keys", "command"] {
            let mut json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            json["profiles"][0][field] = "fixture-secret-do-not-reflect".into();
            let error = parse(&serde_json::to_vec(&json).unwrap()).unwrap_err();
            assert!(!error.contains("fixture-secret"));
        }
        assert!(parse(&vec![b' '; MAX_BYTES + 1]).is_err());
        assert!(parse(br#"{"format":"YuuTerm sessions","version":2,"profiles":[]}"#).is_err());
        assert!(parse(br#"{"format":"YuuTerm sessions","version":1,"profiles":[]}"#).is_err());
    }
    #[test]
    fn imports_append_to_latest_state_without_reusing_ids_or_replacing_preferences() {
        let mut current = prefs();
        current.mocha = true;
        let incoming = parse(&export(&current).unwrap()).unwrap();
        current.profiles[0].name = "edited meanwhile".into();
        let merged = merge(&current, &incoming).unwrap();
        assert!(merged.mocha);
        assert_eq!(merged.profiles[0].name, "edited meanwhile");
        assert_eq!(merged.profiles[1].name, "fixture");
        let twice = merge(&merged, &incoming).unwrap();
        assert_eq!(twice.profiles.len(), 2);
        assert_eq!(current.profiles.len(), 1);
    }
    #[test]
    fn unicode_collisions_and_limits_are_bounded() {
        let mut p = prefs();
        p.profiles[0].name = "界".repeat(42);
        let mut incoming = parse(&export(&p).unwrap()).unwrap();
        incoming[0].host = "different.test".into();
        let merged = merge(&p, &incoming).unwrap();
        assert!(merged.profiles[1].name.len() <= 128);
        assert_eq!(merge(&merged, &incoming).unwrap().profiles.len(), 2);
        p.profiles[0].id = u64::MAX;
        incoming[0].host = "another-new.test".into();
        assert!(merge(&p, &incoming).is_err());
        p.profiles = (1..=1000)
            .map(|id| Profile {
                id,
                ..incoming[0].clone()
            })
            .collect();
        incoming[0].host = "beyond-capacity.test".into();
        assert!(merge(&p, &incoming).is_err());
    }

    #[test]
    fn repeated_and_in_file_duplicates_preserve_existing_settings() {
        let original = prefs();
        let incoming = original.profiles.clone();
        let mut current = original.clone();
        current.profiles[0].host = "EXAMPLE.TEST".into();
        current.profiles[0].font_size = Some(18.0);
        let merged = merge(&current, &[incoming[0].clone(), incoming[0].clone()]).unwrap();
        assert_eq!(
            serde_json::to_value(&merged).unwrap(),
            serde_json::to_value(&current).unwrap()
        );
        let mut distinct = incoming[0].clone();
        distinct.host = "other.test".into();
        let first = merge(&current, &[distinct.clone(), distinct.clone()]).unwrap();
        assert_eq!(first.profiles.len(), 2);
        assert_eq!(first.profiles[1].name, "fixture (imported)");
        assert_eq!(merge(&first, &[distinct]).unwrap().profiles.len(), 2);
        let mut group = incoming[0].clone();
        group.group = "Other".into();
        assert_eq!(merge(&current, &[group]).unwrap().profiles.len(), 2);
        let mut user = incoming[0].clone();
        user.username = "another-user".into();
        assert_eq!(merge(&current, &[user]).unwrap().profiles.len(), 2);
        let full = Preferences {
            profiles: (1..=1000)
                .map(|id| Profile {
                    id,
                    ..incoming[0].clone()
                })
                .collect(),
            ..Preferences::fresh()
        };
        assert_eq!(merge(&full, &incoming).unwrap().profiles.len(), 1000);
        current.profiles[0].temporary = true;
        let saved = merge(&current, &incoming).unwrap();
        assert_eq!(saved.profiles.len(), 2);
        assert!(!saved.profiles[1].temporary);
    }
}
