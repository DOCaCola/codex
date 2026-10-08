//! Session-owned job records. Terminal handles are an index, never completion identity.
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::sync::Mutex;
use uuid::Uuid;

use super::Job;
use super::RETAINED_RESULTS;

#[derive(Default)]
pub(super) struct SessionJobRegistry {
    state: Mutex<RegistryState>,
}

#[derive(Default)]
struct RegistryState {
    jobs: BTreeMap<Uuid, RegistryJob>,
    processes: BTreeMap<i32, Uuid>,
    completions: VecDeque<Uuid>,
}

struct RegistryJob {
    process_id: i32,
    job: Arc<Job>,
    completion_enqueued: bool,
}

impl SessionJobRegistry {
    pub(super) async fn insert(&self, process_id: i32, job_id: Uuid, job: Arc<Job>) {
        let mut state = self.state.lock().await;
        state.processes.insert(process_id, job_id);
        state.jobs.insert(
            job_id,
            RegistryJob {
                process_id,
                job,
                completion_enqueued: false,
            },
        );
    }

    pub(super) async fn get(&self, process_id: i32) -> Option<Arc<Job>> {
        let state = self.state.lock().await;
        state
            .processes
            .get(&process_id)
            .and_then(|id| state.jobs.get(id))
            .map(|entry| Arc::clone(&entry.job))
    }

    pub(super) async fn jobs(&self) -> Vec<(i32, Arc<Job>)> {
        let state = self.state.lock().await;
        // Preserve the accepted delivery-only path's process-ID ordering.
        state
            .processes
            .values()
            .map(|id| {
                let entry = &state.jobs[id];
                (entry.process_id, Arc::clone(&entry.job))
            })
            .collect()
    }

    /// Identity-owned records, including results whose terminal ID was reused.
    pub(super) async fn all_jobs(&self) -> Vec<Arc<Job>> {
        self.state
            .lock()
            .await
            .jobs
            .values()
            .map(|entry| Arc::clone(&entry.job))
            .collect()
    }

    pub(super) async fn enqueue(&self, job_id: Uuid) {
        let mut state = self.state.lock().await;
        // A retired job's late exit cannot enqueue a replacement terminal handle.
        if let Some(entry) = state.jobs.get_mut(&job_id)
            && !entry.completion_enqueued
        {
            entry.completion_enqueued = true;
            state.completions.push_back(job_id);
        }
    }

    pub(super) async fn queued(&self) -> Vec<(i32, Arc<Job>)> {
        let state = self.state.lock().await;
        state
            .completions
            .iter()
            .map(|id| {
                let entry = &state.jobs[id];
                (entry.process_id, Arc::clone(&entry.job))
            })
            .collect()
    }

    pub(super) async fn has_ready_completion(&self) -> bool {
        let state = self.state.lock().await;
        state.completions.iter().any(|id| {
            let job = &state.jobs[id].job;
            job.initial_response_done.is_cancelled()
                && !job.initial_response_consumed.load(Ordering::Acquire)
                && !job.acknowledged.load(Ordering::Acquire)
                && !job.revoked.is_cancelled()
                && job.state.try_lock().is_ok()
        })
    }

    pub(super) async fn contains(&self, job_id: Uuid) -> bool {
        self.state.lock().await.jobs.contains_key(&job_id)
    }

    pub(super) async fn consumed(&self, job_id: Uuid) {
        self.state
            .lock()
            .await
            .completions
            .retain(|id| *id != job_id);
    }

    pub(super) async fn evict_retired(&self) {
        let mut state = self.state.lock().await;
        if state.jobs.len() >= RETAINED_RESULTS {
            state.jobs.retain(|_, entry| {
                let job = &entry.job;
                !job.process.has_exited()
                    // A claimed completion still owns its terminal reservation, even
                    // if cleanup revokes it while output/hooks are being accepted.
                    || job.state.try_lock().is_err()
                    || !(job.acknowledged.load(Ordering::Acquire)
                        || job.revoked.is_cancelled()
                        || job.initial_response_consumed.load(Ordering::Acquire))
            });
            let RegistryState {
                jobs,
                processes,
                completions,
            } = &mut *state;
            processes.retain(|_, id| jobs.contains_key(id));
            completions.retain(|id| jobs.contains_key(id));
        }
    }
}

#[cfg(test)]
#[path = "managed_registry_tests.rs"]
mod tests;
