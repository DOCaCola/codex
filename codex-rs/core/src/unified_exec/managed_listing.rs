use super::UnifiedExecProcessManager;
use crate::background_jobs::BackgroundJob;
use crate::background_jobs::DeliveryFacts;
use std::sync::atomic::Ordering;

impl UnifiedExecProcessManager {
    pub(crate) async fn finish_background_job_timing(&self, item_id: &str) {
        for (_, job) in self.managed.records().await {
            if job.background_jobs && job.process.has_exited() {
                let mut metadata = job.metadata.lock().await;
                if metadata.item_id == item_id {
                    metadata.finish(&job.process, job.revoked.is_cancelled());
                }
            }
        }
        self.managed.changed.notify_waiters();
    }

    pub(crate) async fn background_job_timers(
        &self,
    ) -> Vec<crate::background_jobs::BackgroundJobTimer> {
        use super::ExecutionMode;
        use super::metadata::DeadlineSource;
        let mut timers = Vec::new();
        for (id, job) in self.managed.records().await {
            if !job.background_jobs {
                continue;
            }
            let metadata = job.metadata.lock().await;
            if !matches!(
                metadata.mode,
                ExecutionMode::Auto | ExecutionMode::Background
            ) {
                continue;
            }
            timers.push(crate::background_jobs::BackgroundJobTimer {
                item_id: metadata.item_id.clone(),
                process_id: id.to_string(),
                started_at_ms: metadata.started_at.timestamp_millis(),
                explicit_timeout_seconds: (metadata.deadline_source
                    == Some(DeadlineSource::Explicit))
                .then_some(metadata.explicit_timeout_ms)
                .flatten()
                .map(|ms| (ms / 1000) as u64),
                duration_seconds: metadata
                    .completed_at
                    .map(|_| (metadata.duration_ms / 1000) as u64),
            });
        }
        timers.sort_by(|left, right| {
            (left.started_at_ms, &left.item_id).cmp(&(right.started_at_ms, &right.item_id))
        });
        timers
    }

    pub(crate) async fn list_background_jobs(&self) -> std::io::Result<Vec<BackgroundJob>> {
        let jobs = self.managed.records().await;
        let mut data = Vec::with_capacity(jobs.len());
        for (id, job) in jobs {
            data.push(BackgroundJob::from_metadata(
                &*job.metadata.lock().await,
                id,
                DeliveryFacts {
                    revoked: job.revoked.is_cancelled(),
                    acknowledged: job.acknowledged.load(Ordering::Acquire),
                    initial_response_consumed: job
                        .initial_response_consumed
                        .load(Ordering::Acquire),
                },
            )?);
        }
        data.sort_by(|left, right| {
            (left.started_at_ms, &left.job_id).cmp(&(right.started_at_ms, &right.job_id))
        });
        Ok(data)
    }
}
