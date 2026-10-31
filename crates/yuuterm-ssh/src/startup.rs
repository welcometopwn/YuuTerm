//! Capture-only phase durations. Fixed keys; no identity or endpoint data.
use crate::Snapshot;
use std::time::Instant;
use tokio::sync::watch;

pub(crate) struct Probe {
    last: Instant,
    state: watch::Sender<Snapshot>,
}
impl Probe {
    pub(crate) fn new(state: &watch::Sender<Snapshot>) -> Self {
        Self {
            last: Instant::now(),
            state: state.clone(),
        }
    }
    pub(crate) fn mark(&mut self, key: &'static str) {
        let now = Instant::now();
        let ms = now.duration_since(self.last).as_secs_f64() * 1000.0;
        self.last = now;
        self.state.send_modify(|s| {
            s.startup.insert(key, ms);
        });
    }
}
