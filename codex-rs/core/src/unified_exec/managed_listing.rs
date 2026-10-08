use super::UnifiedExecProcessManager;
use crate::background_jobs::BackgroundJob;
use crate::background_jobs::DeliveryFacts;
use std::sync::atomic::Ordering;

impl UnifiedExecProcessManager {
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
