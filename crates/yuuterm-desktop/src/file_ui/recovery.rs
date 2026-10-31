//! Restart recovery is a reviewed disk record, never a restored live connection.
use super::*;
use std::path::{Component, Path};
const RECORD_LIMIT: u64 = 16 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u8,
    origin: Origin,
    remote: String,
    filename: String,
    baseline: Option<String>,
}
impl Record {
    fn from_edit(e: &Edit) -> Result<Self, String> {
        Ok(Self {
            version: 1,
            origin: e.origin.clone(),
            remote: e.remote.clone(),
            filename: e
                .local
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("Invalid staged filename")?
                .into(),
            baseline: e.baseline.clone(),
        })
    }
    fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.origin.profile_id == 0
            || self.origin.port == 0
            || self.origin.host.is_empty()
            || self.origin.host.len() > 253
            || self.origin.username.len() > 256
            || self.origin.host.chars().any(char::is_control)
            || self.origin.username.chars().any(char::is_control)
            || !self.remote.starts_with('/')
            || self.remote.len() > 4096
            || self.remote.chars().any(char::is_control)
            || self.filename != safe_name(&self.remote)
            || self.baseline.as_ref().is_some_and(|b| {
                b.len() > 4096 || !valid_hash(b.rsplit(':').next().unwrap_or_default())
            })
        {
            return Err("Unsupported recovery record; files preserved".into());
        }
        Ok(())
    }
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn key_valid(key: &str) -> bool {
    key.starts_with("edit-")
        && key.len() <= 80
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
async fn plain(path: &Path, directory: bool) -> Result<(), String> {
    let m = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err("Recovery refuses reparse points".into());
        }
    }
    if m.file_type().is_symlink() || if directory { !m.is_dir() } else { !m.is_file() } {
        return Err("Recovery requires regular files/directories".into());
    }
    Ok(())
}
fn root(state: &Backend) -> Result<PathBuf, String> {
    if cfg!(feature = "capture") && lock(&state.config)?.fixture {
        Ok(PathBuf::from(
            std::env::var("YUUTERM_CAPTURE_PROFILE").map_err(|_| "Fixture profile missing")?,
        )
        .with_file_name("staging"))
    } else {
        Ok(yuuterm_core::config::path()
            .map_err(|e| e.to_string())?
            .with_file_name("staging"))
    }
}
async fn directory(root: &Path, key: &str) -> Result<PathBuf, String> {
    if !key_valid(key) {
        return Err("Invalid recovery identity".into());
    }
    // Inspect each existing ancestor before descending into the staging root.
    let mut parents = root.ancestors().collect::<Vec<_>>();
    parents.reverse();
    for path in parents {
        if path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err("Invalid staging path".into());
        }
        plain(path, true).await?;
    }
    let dir = root.join(key);
    plain(&dir, true).await?;
    Ok(dir)
}
pub(super) async fn persist(edit: &Edit) -> Result<(), String> {
    let record = Record::from_edit(edit)?;
    write_record(
        record,
        edit.local
            .parent()
            .ok_or("Missing staged directory")?
            .to_owned(),
    )
    .await
}
async fn write_record(record: Record, dir: PathBuf) -> Result<(), String> {
    record.validate()?;
    plain(&dir, true).await?;
    let bytes = serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new_in(&dir).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.as_file().sync_all().map_err(|e| e.to_string())?;
        file.persist(dir.join("recovery.json"))
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}
pub(super) async fn persist_id(state: &Backend, id: u64) -> Result<(), String> {
    let (record, local) = {
        let edits = lock(&state.edits)?;
        let Some(e) = edits.get(&id) else {
            return Ok(());
        };
        (Record::from_edit(e)?, e.local.clone())
    };
    write_record(
        record,
        local.parent().ok_or("Missing staged directory")?.to_owned(),
    )
    .await
}
#[derive(Serialize)]
pub(super) struct Item {
    key: String,
    remote: String,
    server: String,
    local: String,
    profile_id: Option<u64>,
    state: String,
    revision: Option<String>,
    verified: bool,
}
#[derive(Serialize)]
pub(crate) struct Listing {
    items: Vec<Item>,
    notice: String,
}
struct Loaded {
    record: Option<Record>,
    remote: String,
    server: String,
    local: PathBuf,
    directory: PathBuf,
}
async fn load(root: &Path, key: &str) -> Result<Loaded, String> {
    let dir = directory(root, key).await?;
    let path = dir.join("recovery.json");
    if tokio::fs::symlink_metadata(&path)
        .await
        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    {
        return Err("Not a pending recovery record".into());
    }
    plain(&path, false).await?;
    if tokio::fs::metadata(&path)
        .await
        .map_err(|e| e.to_string())?
        .len()
        > RECORD_LIMIT
    {
        return Err("Recovery record exceeds limit".into());
    }
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    tokio::fs::File::open(path)
        .await
        .map_err(|e| e.to_string())?
        .take(RECORD_LIMIT + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    if bytes.len() > RECORD_LIMIT as usize {
        return Err("Recovery record exceeds limit".into());
    }
    if let Ok(record) = serde_json::from_slice::<Record>(&bytes) {
        record.validate()?;
        return Ok(Loaded {
            local: dir.join(&record.filename),
            remote: record.remote.clone(),
            server: format!(
                "{}@{}:{}",
                record.origin.username, record.origin.host, record.origin.port
            ),
            record: Some(record),
            directory: dir,
        });
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Legacy {
        remote: String,
        local: PathBuf,
        endpoint: Option<String>,
    }
    let old: Legacy =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid recovery metadata; files preserved")?;
    if !old.remote.starts_with('/')
        || old.remote.len() > 4096
        || old.remote.chars().any(char::is_control)
        || old.local != dir.join(safe_name(&old.remote))
    {
        return Err("Unverified legacy path refused; files preserved".into());
    }
    Ok(Loaded {
        local: old.local,
        remote: old.remote,
        server: old
            .endpoint
            .filter(|v| v.len() <= 1024 && !v.chars().any(char::is_control))
            .unwrap_or_else(|| "Unknown server".into()),
        record: None,
        directory: dir,
    })
}
async fn local_revision_checked(local: &Path) -> Result<String, String> {
    plain(local, false).await?;
    let before = stamp(local).await?;
    let hash = yuuterm_ssh::files::local_revision(local)
        .await
        .map_err(|e| e.to_string())?;
    if stamp(local).await? != before {
        return Err("Local copy is changing; inspect again".into());
    }
    Ok(hash)
}
#[tauri::command]
pub(crate) async fn edit_restore_list(
    window: WebviewWindow,
    state: State<'_, Backend>,
    key: Option<String>,
) -> Result<Listing, String> {
    main_only(&window)?;
    let root = root(&state)?;
    if !tokio::fs::try_exists(&root)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(Listing {
            items: vec![],
            notice: String::new(),
        });
    }
    let active = lock(&state.edits)?
        .values()
        .map(|e| e.local.clone())
        .collect::<Vec<_>>();
    let keys = if let Some(key) = key {
        vec![key]
    } else {
        plain(&root, true).await?;
        let mut entries = tokio::fs::read_dir(&root)
            .await
            .map_err(|e| e.to_string())?;
        let mut keys = vec![];
        let mut count = 0;
        while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
            count += 1;
            if count > 256 {
                break;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if key_valid(&name) {
                keys.push(name)
            }
        }
        keys
    };
    let mut items = vec![];
    let mut skipped = 0;
    let mut budget = yuuterm_ssh::files::MAX_EDIT;
    for key in keys {
        if items.len() >= 32 {
            skipped += 1;
            continue;
        }
        let loaded = match load(&root, &key).await {
            Ok(v) => v,
            Err(e) => {
                if e == "Not a pending recovery record" {
                    continue;
                }
                skipped += 1;
                continue;
            }
        };
        if active.contains(&loaded.local) {
            continue;
        }
        let size = tokio::fs::symlink_metadata(&loaded.local)
            .await
            .ok()
            .map(|m| m.len());
        let (hash, status) = match size {
            None => (None, "Missing local copy".into()),
            Some(size) if size > budget => (
                None,
                "Not checked: 64 MiB scan budget; inspect this file separately".into(),
            ),
            Some(size) => {
                budget -= size;
                match local_revision_checked(&loaded.local).await {
                    Ok(hash) => {
                        let status = if let Some(baseline) =
                            loaded.record.as_ref().and_then(|r| r.baseline.as_ref())
                        {
                            if baseline.rsplit(':').next() == Some(&hash) {
                                "Unchanged"
                            } else {
                                "Changed"
                            }
                        } else {
                            "Unverified baseline"
                        };
                        (Some(hash), status.into())
                    }
                    Err(e) => (None, e),
                }
            }
        };
        items.push(Item {
            key,
            remote: loaded.remote,
            server: loaded.server,
            local: loaded.local.to_string_lossy().into(),
            profile_id: loaded.record.as_ref().map(|r| r.origin.profile_id),
            verified: loaded.record.as_ref().is_some_and(|r| r.baseline.is_some()),
            state: status,
            revision: hash,
        });
    }
    let empty = items.is_empty() && skipped == 0;
    Ok(Listing {
        items,
        notice: if empty {
            String::new()
        } else {
            format!(
                "Scans inspect at most 256 staging entries, 32 records and 64 MiB of local data. {skipped} unreadable/unsupported or excess records were left untouched. Legacy records without a baseline cannot upload."
            )
        },
    })
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Decision {
    Upload,
    Keep,
    Discard,
}
#[tauri::command]
pub(crate) async fn edit_restore_decide(
    window: WebviewWindow,
    state: State<'_, Backend>,
    key: String,
    decision: Decision,
    id: Option<u64>,
    revision: Option<String>,
) -> Result<(), String> {
    main_only(&window)?;
    let _poll = state.edit_polling.lock().await;
    let loaded = load(&root(&state)?, &key).await?;
    if lock(&state.edits)?
        .values()
        .any(|e| e.local == loaded.local)
    {
        return Err("Local copy is already watched; use its active edit actions".into());
    }
    if matches!(decision, Decision::Keep) {
        tokio::fs::remove_file(loaded.directory.join("recovery.json"))
            .await
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    let actual = local_revision_checked(&loaded.local).await?;
    if revision.as_deref() != Some(&actual) {
        return Err("Local copy changed; inspect and review its latest revision".into());
    }
    if matches!(decision, Decision::Discard) {
        let held = loaded.directory.join(format!(
            ".discard-review-{}",
            state.next_edit.fetch_add(1, Ordering::Relaxed)
        ));
        if tokio::fs::try_exists(&held)
            .await
            .map_err(|e| e.to_string())?
        {
            return Err("Discard backup already exists; local copy retained".into());
        }
        tokio::fs::rename(&loaded.local, &held)
            .await
            .map_err(|e| e.to_string())?;
        if local_revision_checked(&held).await?.as_str() != actual {
            return Err(format!(
                "Local copy changed while discarding; moved revision preserved at {}",
                held.display()
            ));
        }
        tokio::fs::remove_file(held)
            .await
            .map_err(|e| e.to_string())?;
        if tokio::fs::try_exists(&loaded.local)
            .await
            .map_err(|e| e.to_string())?
        {
            return Err("Reviewed revision discarded, but a new local save exists; record retained for review".into());
        }
        tokio::fs::remove_file(loaded.directory.join("recovery.json"))
            .await
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    let record = loaded
        .record
        .ok_or("Legacy recovery has no verified baseline; keep local copy and compare manually")?;
    let id = id.ok_or("Explicitly connect the original SSH bookmark before uploading")?;
    if recovery_origin(&state, id)? != record.origin {
        return Err("Recovery belongs to a different bookmark/server/user".into());
    }
    if lock(&state.edits)?.len() >= 32
        || lock(&state.edits)?
            .values()
            .any(|e| e.live_id == id && e.remote == record.remote)
    {
        return Err("Edit limit or file already open; local recovery copy retained".into());
    }
    let (client, lease) = begin(&state, id)?;
    // Replace independently hashes the remote baseline and checks this selected
    // local revision before publishing. No rebind or baseline refresh on conflict.
    let result = client
        .execute(
            FileAction::Replace {
                path: record.remote.clone(),
                local: loaded.local.clone(),
                expected: record.baseline.ok_or("Missing verified baseline")?,
                expected_local: Some(actual.clone()),
            },
            lease.cancel.clone(),
        )
        .await?;
    let observed = stamp(&loaded.local).await?;
    let latest = local_revision_checked(&loaded.local).await?;
    if stamp(&loaded.local).await? != observed {
        return Err(
            "Upload completed, but local copy changed again; recovery record retained for review"
                .into(),
        );
    }
    let pending = (latest != actual).then_some(latest);
    let edit = Edit {
        origin: record.origin,
        live_id: id,
        remote: record.remote,
        local: loaded.local.clone(),
        baseline: result.fingerprint.clone(),
        acknowledged: actual,
        stamp: observed,
        candidate: None,
        pending,
        remote_ack: metadata_part(result.fingerprint.as_deref().unwrap_or_default()).into(),
    };
    persist(&edit).await?;
    lock(&state.edits)?.insert(state.next_edit.fetch_add(1, Ordering::Relaxed), edit);
    Ok(())
}

#[tauri::command]
pub(crate) async fn capture_restore_probe(
    window: WebviewWindow,
    state: State<'_, Backend>,
    key: String,
    id: u64,
    operation: String,
    text: String,
) -> Result<String, String> {
    main_only(&window)?;
    if !cfg!(feature = "capture") || !lock(&state.config)?.fixture || text.len() > 65536 {
        return Err("Fixture only".into());
    }
    let loaded = load(&root(&state)?, &key).await?;
    if loaded
        .record
        .as_ref()
        .is_none_or(|r| Some(&r.origin) != recovery_origin(&state, id).ok().as_ref())
    {
        return Err("Wrong fixture origin".into());
    }
    if operation == "local-save" {
        plain(&loaded.local, false).await?;
        tokio::fs::write(loaded.local, text)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(String::new());
    }
    if operation == "local-read" {
        plain(&loaded.local, false).await?;
        if tokio::fs::metadata(&loaded.local)
            .await
            .map_err(|e| e.to_string())?
            .len()
            > 65536
        {
            return Err("Fixture text exceeds limit".into());
        }
        return tokio::fs::read_to_string(loaded.local)
            .await
            .map_err(|e| e.to_string());
    }
    if operation != "remote-read" {
        return Err("Unknown fixture operation".into());
    }
    let (client, lease) = begin(&state, id)?;
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let local = temp.path().join("probe");
    client
        .execute(
            FileAction::Download {
                path: loaded.remote,
                local: local.clone(),
                edit: true,
            },
            lease.cancel.clone(),
        )
        .await?;
    if tokio::fs::metadata(&local)
        .await
        .map_err(|e| e.to_string())?
        .len()
        > 65536
    {
        return Err("Fixture text exceeds limit".into());
    }
    tokio::fs::read_to_string(local)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record() -> Record {
        Record {
            version: 1,
            origin: Origin::from_profile(&yuuterm_core::Profile {
                id: 1,
                name: "Fixture".into(),
                host: "127.0.0.1".into(),
                username: "fixture".into(),
                ..Default::default()
            }),
            remote: "/fixture/read me.txt".into(),
            filename: safe_name("/fixture/read me.txt"),
            baseline: Some(format!("metadata:{}", "a".repeat(64))),
        }
    }
    #[tokio::test]
    async fn durable_baseline_updates_and_legacy_records_never_gain_upload_authority() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("edit-fixture");
        tokio::fs::create_dir(&dir).await.unwrap();
        let mut r = record();
        write_record(r.clone(), dir.clone()).await.unwrap();
        assert_eq!(
            load(root.path(), "edit-fixture")
                .await
                .unwrap()
                .record
                .unwrap()
                .baseline,
            r.baseline
        );
        r.baseline = Some(format!("uploaded:{}", "b".repeat(64)));
        write_record(r.clone(), dir.clone()).await.unwrap();
        assert_eq!(
            load(root.path(), "edit-fixture")
                .await
                .unwrap()
                .record
                .unwrap()
                .baseline,
            r.baseline
        );
        let legacy = serde_json::json!({"remote":r.remote,"local":dir.join(&r.filename),"endpoint":"fixture@127.0.0.1:22"});
        tokio::fs::write(
            dir.join("recovery.json"),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .await
        .unwrap();
        assert!(
            load(root.path(), "edit-fixture")
                .await
                .unwrap()
                .record
                .is_none()
        );
        tokio::fs::write(
            dir.join("recovery.json"),
            vec![b' '; RECORD_LIMIT as usize + 1],
        )
        .await
        .unwrap();
        assert!(load(root.path(), "edit-fixture").await.is_err());
    }
    #[tokio::test]
    async fn malformed_records_paths_symlinks_and_local_changes_are_refused() {
        let mut r = record();
        r.filename = "../outside".into();
        assert!(r.validate().is_err());
        r = record();
        r.version = 2;
        assert!(r.validate().is_err());
        r = record();
        r.baseline = Some("bad".into());
        assert!(r.validate().is_err());
        for key in ["../edit-x", "edit-x/other", "edit-x\\other", "edit-x:"] {
            assert!(!key_valid(key));
        }
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("local");
        tokio::fs::write(&path, b"before").await.unwrap();
        let old = local_revision_checked(&path).await.unwrap();
        tokio::fs::write(&path, b"after").await.unwrap();
        assert_ne!(local_revision_checked(&path).await.unwrap(), old);
        assert!(plain(root.path(), false).await.is_err());
        #[cfg(windows)]
        {
            let link = root.path().join("link");
            std::os::windows::fs::symlink_file(&path, &link).unwrap();
            assert!(plain(&link, false).await.is_err());
        }
    }
}
