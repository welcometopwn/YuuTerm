//! Atomic bookmark batch edits against the latest preferences.
use crate::{Profile, config::Preferences};
use std::collections::HashSet;
pub fn selected<'a>(prefs: &'a Preferences, ids: &[u64]) -> Result<Vec<&'a Profile>, String> {
    if ids.is_empty() || ids.len() > 1000 {
        return Err("Select 1–1000 sessions".into());
    }
    let unique: HashSet<_> = ids.iter().copied().collect();
    if unique.len() != ids.len()
        || prefs
            .profiles
            .iter()
            .filter(|p| unique.contains(&p.id))
            .count()
            != ids.len()
    {
        return Err("The selected sessions changed; select them again".into());
    }
    Ok(prefs
        .profiles
        .iter()
        .filter(|p| unique.contains(&p.id))
        .collect())
}
pub fn move_to(
    prefs: &Preferences,
    ids: &[u64],
    group: &str,
    before: Option<u64>,
) -> Result<Preferences, String> {
    prefs.validate()?;
    let mut moving: Vec<_> = selected(prefs, ids)?.into_iter().cloned().collect();
    if group.len() > 64 || group.trim() != group || group.chars().any(char::is_control) {
        return Err(
            "Group names have up to 64 UTF-8 bytes and no control characters or outer spaces"
                .into(),
        );
    }
    if let Some(id) = before {
        let target = prefs
            .profiles
            .iter()
            .find(|p| p.id == id)
            .ok_or("Drop target no longer exists")?;
        if target.group != group {
            return Err("Drop target group changed".into());
        }
        if ids.contains(&id) {
            return Ok(prefs.clone());
        }
    }
    for p in &mut moving {
        p.group = group.into();
    }
    let mut next = prefs.clone();
    next.profiles.retain(|p| !ids.contains(&p.id));
    let position = before
        .and_then(|id| next.profiles.iter().position(|p| p.id == id))
        .unwrap_or(next.profiles.len());
    next.profiles.splice(position..position, moving);
    next.validate()?;
    Ok(next)
}
pub fn remove(prefs: &Preferences, ids: &[u64]) -> Result<Preferences, String> {
    prefs.validate()?;
    selected(prefs, ids)?;
    let mut next = prefs.clone();
    next.profiles.retain(|p| !ids.contains(&p.id));
    next.validate()?;
    Ok(next)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn moves_are_atomic_ordered_and_preserve_bookmark_metadata() {
        let mut prefs = Preferences::fresh();
        prefs.profiles = (1..=4)
            .map(|id| Profile {
                id,
                name: format!("Session {id}"),
                host: "example.test".into(),
                group: "Old".into(),
                mac_address: "02:11:22:33:44:55".into(),
                ..Default::default()
            })
            .collect();
        let moved = move_to(&prefs, &[3, 1], "New", None).unwrap();
        assert_eq!(
            moved.profiles.iter().map(|p| p.id).collect::<Vec<_>>(),
            [2, 4, 1, 3]
        );
        assert!(
            moved.profiles[2..]
                .iter()
                .all(|p| p.group == "New" && p.mac_address == prefs.profiles[0].mac_address)
        );
        let ordered = move_to(&prefs, &[3, 4], "Old", Some(1)).unwrap();
        assert_eq!(
            ordered.profiles.iter().map(|p| p.id).collect::<Vec<_>>(),
            [3, 4, 1, 2]
        );
        assert!(move_to(&prefs, &[1, 99], "New", None).is_err());
        assert!(move_to(&prefs, &[1, 1], "New", None).is_err());
        assert!(move_to(&prefs, &[1], &"界".repeat(22), None).is_err());
        assert!(move_to(&prefs, &[1], "New", Some(4)).is_err());
        assert!(remove(&prefs, &[1, 99]).is_err());
        assert_eq!(remove(&prefs, &[1, 3]).unwrap().profiles.len(), 2);
        assert_eq!(prefs.profiles.len(), 4);
        let roundtrip: Preferences =
            serde_json::from_slice(&serde_json::to_vec(&moved).unwrap()).unwrap();
        assert_eq!(
            roundtrip.profiles[2].mac_address,
            prefs.profiles[0].mac_address
        );
    }
}
