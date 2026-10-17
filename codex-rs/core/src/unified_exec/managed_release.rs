//! Ends a turn's claim on commands it launched. Released commands keep running
//! and stay readable through `write_stdin`, including completion waits.
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::Job;
use crate::context::CommandsStillRunning;
use crate::context::ContextualUserFragment;
use crate::unified_exec::UnifiedExecContext;
use crate::unified_exec::UnifiedExecProcessManager;

impl UnifiedExecProcessManager {
    /// Releases delivery-only records when their turn ends. Opted-in background
    /// jobs live in the separate session registry and retain their queue policy.
    pub(crate) async fn release_turn_commands(&self, turn_id: &str) {
        self.release_unsettled(turn_id).await;
    }

    /// Ends a park that saw no completion within `waited` and tells the model
    /// which commands are still running.
    pub(super) async fn release_stalled_commands(
        &self,
        context: &UnifiedExecContext,
        waited: Duration,
    ) -> bool {
        let released = self
            .release_unsettled(&context.step_context.turn.sub_id)
            .await;
        if released.is_empty() {
            return false;
        }
        let mut summaries = Vec::with_capacity(released.len());
        for (id, job) in &released {
            let metadata = job.metadata.lock().await;
            summaries.push((
                *id,
                metadata.command.clone(),
                metadata.clock_started_at.elapsed(),
            ));
        }
        let notice = CommandsStillRunning::new(
            waited,
            summaries
                .iter()
                .map(|(id, command, age)| (*id, command.as_str(), *age)),
        );
        context
            .session
            .record_conversation_items(
                &context.step_context.turn,
                &context.step_context.settings.model_info,
                &[ContextualUserFragment::into(notice)],
            )
            .await;
        true
    }

    async fn release_unsettled(&self, turn_id: &str) -> Vec<(i32, Arc<Job>)> {
        let released = self
            .managed
            .jobs
            .lock()
            .await
            .iter()
            .filter(|(_, job)| {
                !job.background_jobs
                    && job.delivers_to(turn_id)
                    && !job.revoked.is_cancelled()
                    && !job.initial_response_consumed.load(Ordering::Acquire)
                    && !job.acknowledged.load(Ordering::Acquire)
            })
            .map(|(id, job)| (*id, Arc::clone(job)))
            .collect::<Vec<_>>();
        for (_, job) in &released {
            job.released.store(true, Ordering::Release);
        }
        self.managed.changed.notify_waiters();
        released
    }
}
