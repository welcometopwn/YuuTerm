//! Capture-only observations: no endpoint names, credential data or terminal text.
use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};
use tokio::sync::watch;
use yuuterm_ssh::Snapshot;
#[derive(Clone, Serialize)]
pub struct Timing {
    profile_id: u64,
    start_observed_ms: f64,
    connected_observed_ms: Option<f64>,
    fixture_listing_observed_ms: Option<f64>,
    stages: BTreeMap<&'static str, f64>,
}
pub type Records = Arc<Mutex<BTreeMap<u64, Timing>>>;
pub fn observe(
    origin: Instant,
    profile_id: u64,
    mut changes: watch::Receiver<Snapshot>,
    records: Records,
) {
    let start = origin.elapsed().as_secs_f64() * 1000.0;
    tauri::async_runtime::spawn(async move {
        let mut timing = Timing {
            profile_id,
            start_observed_ms: start,
            connected_observed_ms: None,
            fixture_listing_observed_ms: None,
            stages: BTreeMap::new(),
        };
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let snapshot = changes.borrow_and_update().clone();
            timing.stages.clone_from(&snapshot.startup);
            let now = origin.elapsed().as_secs_f64() * 1000.0;
            if snapshot.connected && timing.connected_observed_ms.is_none() {
                timing.connected_observed_ms = Some(now)
            }
            if snapshot.connected
                && snapshot.files.len() >= 2
                && timing.fixture_listing_observed_ms.is_none()
            {
                timing.fixture_listing_observed_ms = Some(now)
            }
            if let Ok(mut entries) = records.lock() {
                entries.insert(profile_id, timing.clone());
            }
            if timing.fixture_listing_observed_ms.is_some() || snapshot.finished {
                break;
            }
            match tokio::time::timeout_at(deadline, changes.changed()).await {
                Ok(Ok(())) => {}
                _ => break,
            }
        }
    });
}
