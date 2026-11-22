//! Union of recorded turn intervals: parallel agents share one session clock.
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Default)]
pub(in crate::lean::tui) struct SessionTime {
    completed_ms: u64,
    active: HashSet<String>,
    since_ms: Option<i64>,
    last_ms: i64,
    after: u64,
}

impl SessionTime {
    pub(super) fn observe(&mut self, event: &Value) {
        let seq = event["seq"].as_u64().unwrap_or(0);
        if seq <= self.after {
            return;
        }
        self.after = seq;
        let timestamp = event["created_ms"]
            .as_i64()
            .filter(|ms| *ms > 0)
            .map(|ms| ms.max(self.last_ms));
        if let Some(timestamp) = timestamp {
            self.last_ms = timestamp;
        }
        let Some(turn) = event["turn_id"].as_str().filter(|turn| !turn.is_empty()) else {
            return;
        };
        match event["kind"].as_str() {
            Some("turn_started") => {
                if let Some(timestamp) = timestamp {
                    if self.active.is_empty() {
                        self.since_ms = Some(timestamp);
                    }
                    self.active.insert(turn.to_owned());
                }
            }
            Some("turn_finished") => {
                if self.active.remove(turn)
                    && self.active.is_empty()
                    && let Some(start) = self.since_ms.take()
                {
                    self.completed_ms = self
                        .completed_ms
                        .saturating_add(self.last_ms.saturating_sub(start).max(0) as u64);
                }
            }
            _ => {}
        }
    }

    pub(in crate::lean::tui) fn elapsed_ms(&self, now_ms: i64) -> u64 {
        self.completed_ms
            .saturating_add(self.since_ms.map_or(0, |start| {
                now_ms.max(self.last_ms).saturating_sub(start).max(0) as u64
            }))
    }
}
