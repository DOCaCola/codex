//! Claude-style managed-job labels. Metadata presence is the captured session capability.
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use codex_app_server_protocol::CodexdcBackgroundJobTimer;

pub(crate) fn compact_duration(seconds: u64) -> String {
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        let seconds = seconds % 60;
        return if seconds == 0 {
            format!("{minutes}m")
        } else {
            format!("{minutes}m {seconds}s")
        };
    }
    let hours = minutes / 60;
    let minutes = minutes % 60;
    if minutes == 0 {
        format!("{hours}h")
    } else {
        format!("{hours}h {minutes}m")
    }
}

#[derive(Clone, Debug)]
pub(crate) struct JobTimer(Arc<Mutex<CodexdcBackgroundJobTimer>>);

impl JobTimer {
    pub(crate) fn new(metadata: CodexdcBackgroundJobTimer) -> Self {
        Self(Arc::new(Mutex::new(metadata)))
    }

    pub(crate) fn label(&self) -> String {
        self.label_at(chrono::Utc::now().timestamp_millis())
    }

    pub(crate) fn label_at(&self, now_ms: i64) -> String {
        let timer = self.0.lock().unwrap();
        let duration = timer.duration_seconds;
        let seconds = duration.unwrap_or_else(|| {
            now_ms.saturating_sub(timer.started_at_ms).max(0) as u64 / 1000
        });
        let elapsed = compact_duration(seconds);
        if duration.is_none() && let Some(timeout) = timer.explicit_timeout_seconds {
            let timeout = compact_duration(timeout);
            format!("({elapsed} · timeout {timeout})")
        } else {
            format!("({elapsed})")
        }
    }

    pub(crate) fn running(&self) -> bool {
        self.0.lock().unwrap().duration_seconds.is_none()
    }
}

#[derive(Default)]
pub(crate) struct JobTimers {
    pub(crate) enabled: bool,
    by_item: Arc<Mutex<HashMap<String, JobTimer>>>,
}

impl JobTimers {
    pub(crate) fn update(&mut self, enabled: bool, timers: Vec<CodexdcBackgroundJobTimer>) {
        self.enabled = enabled;
        let mut by_item = self.by_item.lock().unwrap();
        if !enabled {
            by_item.clear();
            return;
        }
        let mut next = HashMap::new();
        for metadata in timers {
            let id = metadata.item_id.clone();
            let timer = by_item.remove(&id)
                .unwrap_or_else(|| JobTimer::new(metadata.clone()));
            *timer.0.lock().unwrap() = metadata;
            next.insert(id, timer);
        }
        *by_item = next;
    }

    pub(crate) fn for_process(&self, process_id: &str) -> Option<JobTimer> {
        self.by_item.lock().unwrap().values().find(|timer| timer.0.lock().unwrap().process_id == process_id).cloned()
    }

    pub(crate) fn process(&self, process_id: &str) -> ProcessJobTimer {
        ProcessJobTimer {
            by_item: Arc::clone(&self.by_item), process_id: process_id.to_string(),
            resolved: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn get(&self, item_id: &str) -> Option<JobTimer> {
        self.by_item.lock().unwrap().get(item_id).cloned()
    }
}

/// A process row can precede its metadata. Bind once it exists, then retain its shared final clock.
#[derive(Clone, Debug)]
pub(crate) struct ProcessJobTimer {
    by_item: Arc<Mutex<HashMap<String, JobTimer>>>,
    process_id: String,
    resolved: Arc<Mutex<Option<JobTimer>>>,
}

impl ProcessJobTimer {
    fn resolve(&self) -> Option<JobTimer> {
        let mut resolved = self.resolved.lock().unwrap();
        if resolved.is_none() {
            *resolved = self.by_item.lock().unwrap().values()
                .find(|timer| timer.0.lock().unwrap().process_id == self.process_id).cloned();
        }
        resolved.clone()
    }

    pub(crate) fn label(&self) -> Option<String> {
        self.resolve().map(|timer| timer.label())
    }

    pub(crate) fn present(&self) -> bool {
        self.resolve().is_some()
    }
}

#[cfg(test)]
#[path = "background_job_timer_tests.rs"]
mod tests;
