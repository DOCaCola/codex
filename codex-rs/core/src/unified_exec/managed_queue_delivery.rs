//! Cancellation-safe acceptance for session-owned completions.
use std::sync::Arc;
use std::sync::atomic::Ordering;

use codex_protocol::models::ResponseItem;
use tokio::sync::OwnedMutexGuard;
use tokio::time::Instant;
use uuid::Uuid;

use super::Job;
use super::JobState;
use crate::context::CommandCompletion;
use crate::context::ContextualUserFragment;
use crate::tools::context::ExecCommandToolOutput;
use crate::unified_exec::UnifiedExecContext;
use crate::unified_exec::UnifiedExecError;
use crate::unified_exec::UnifiedExecProcess;
use crate::unified_exec::UnifiedExecProcessManager;
use crate::unified_exec::WriteStdinRequest;

struct PendingCompletion {
    job: Arc<Job>,
    job_id: Uuid,
    _claim: OwnedMutexGuard<JobState>,
    fragment: ResponseItem,
}

impl UnifiedExecProcessManager {
    /// Interrupt suppression includes eligible identities before their processes exit.
    pub(crate) async fn background_interrupt_job_ids(&self) -> Vec<Uuid> {
        let mut ids = Vec::new();
        for job in self.managed.registry.all_jobs().await {
            if job.revoked.is_cancelled()
                || job.acknowledged.load(Ordering::Acquire)
                || job.initial_response_consumed.load(Ordering::Acquire)
            {
                continue;
            }
            let metadata = job.metadata.lock().await;
            if matches!(
                metadata.mode,
                super::ExecutionMode::Auto | super::ExecutionMode::Background
            ) {
                ids.push(metadata.job_id);
            }
        }
        ids
    }

    pub(crate) async fn background_wake_candidates(
        &self,
    ) -> Vec<crate::session::background_continuation::WakeCandidate> {
        let mut candidates = Vec::new();
        for (_, job) in self.managed.registry.queued().await {
            if job.revoked.is_cancelled()
                || job.acknowledged.load(Ordering::Acquire)
                || job.initial_response_consumed.load(Ordering::Acquire)
            {
                continue;
            }
            let metadata = job.metadata.lock().await;
            candidates.push(crate::session::background_continuation::WakeCandidate {
                job_id: metadata.job_id,
                originating_turn: metadata.originating_turn.clone(),
                ready: job.process.has_exited()
                    && job.initial_response_done.is_cancelled()
                    && job.state.try_lock().is_ok(),
            });
        }
        candidates
    }

    pub(crate) fn background_work_notifications(&self) -> Arc<tokio::sync::Notify> {
        Arc::clone(&self.managed.changed)
    }

    pub(crate) fn background_work_changed(&self) -> tokio::sync::futures::OwnedNotified {
        self.background_work_notifications().notified_owned()
    }

    pub(crate) async fn has_live_background_work(&self) -> bool {
        // The flag-off map is deliberately excluded. A pending auto/background
        // record also protects the brief exit-to-queue publication handoff.
        for job in self.managed.registry.all_jobs().await {
            if !job.process.has_exited() {
                return true;
            }
            if job.revoked.is_cancelled()
                || job.acknowledged.load(Ordering::Acquire)
                || job.initial_response_consumed.load(Ordering::Acquire)
            {
                continue;
            }
            if matches!(
                job.metadata.lock().await.mode,
                super::ExecutionMode::Auto | super::ExecutionMode::Background
            ) {
                return true;
            }
        }
        false
    }

    pub(in crate::unified_exec) async fn finalize_managed_outcome(
        &self,
        process_id: i32,
        process: &Arc<UnifiedExecProcess>,
    ) {
        let Some(job) = self.managed.get(process_id).await else {
            return;
        };
        if job.background_jobs && !Arc::ptr_eq(&job.process, process) {
            return;
        }
        if job.background_jobs {
            let job_id = job.metadata.lock().await.job_id;
            self.managed.registry.enqueue(job_id).await;
            self.managed.changed.notify_waiters();
        }
        job.metadata
            .lock()
            .await
            .finish(&job.process, job.revoked.is_cancelled());
    }

    pub(super) async fn read_cached_background_job(
        &self,
        context: &UnifiedExecContext,
        job: &Job,
        state: &mut JobState,
        request: WriteStdinRequest<'_>,
    ) -> Result<ExecCommandToolOutput, UnifiedExecError> {
        if let Some(error) = &state.error {
            if !job.revoked.is_cancelled() {
                job.acknowledged.store(true, Ordering::Release);
            }
            return Err(UnifiedExecError::process_failed(error.clone()));
        }
        if !request.input.is_empty() {
            return Err(UnifiedExecError::StdinClosed);
        }
        let mut result = match &state.result {
            Some(result) => result.clone(),
            None => unreachable!("cached job read requires a result or error"),
        };
        if !job.revoked.is_cancelled() && result.hook_command.is_some() {
            result = match self.accept_managed_output(context, job, result).await {
                Ok(output) => output,
                Err(error) => {
                    state.result = None;
                    state.error = Some(error.to_string());
                    if !job.revoked.is_cancelled() {
                        job.acknowledged.store(true, Ordering::Release);
                    }
                    return Err(error);
                }
            };
            state.result = Some(result.clone());
        }
        result.max_output_tokens = request.max_output_tokens;
        result.truncation_policy = request.truncation_policy;
        if job.revoked.is_cancelled() {
            result.completion_delivery = Some(false);
        } else {
            job.acknowledged.store(true, Ordering::Release);
        }
        result.hook_command = None;
        Ok(result)
    }

    pub(super) async fn accept_background_job_read(
        &self,
        context: &UnifiedExecContext,
        job: &Job,
        state: &mut JobState,
        result: Result<ExecCommandToolOutput, UnifiedExecError>,
    ) -> Result<ExecCommandToolOutput, UnifiedExecError> {
        let result = match result {
            Ok(output) if output.process_id.is_none() && !job.revoked.is_cancelled() => {
                state.result = Some(output.clone());
                self.accept_managed_output(context, job, output).await
            }
            result => result,
        };
        if job.revoked.is_cancelled() {
            return result.map(|mut output| {
                output.completion_delivery = Some(false);
                output.hook_command = None;
                output
            });
        }
        if let Ok(output) = &result {
            state.last_poll = Some(Instant::now());
            if !output.raw_output.is_empty() {
                state.last_poll = None;
                state.poll_level = 0;
            }
            if output.process_id.is_none() {
                state.result = Some(output.clone());
                job.acknowledged.store(true, Ordering::Release);
            }
        } else if job.process.has_exited() {
            state.result = None;
            state.error = result.as_ref().err().map(ToString::to_string);
            job.acknowledged.store(true, Ordering::Release);
        }
        result
    }

    /// Ctrl-C for a finite job is an explicit stop, not turn interruption.
    pub(in crate::unified_exec) async fn stop_managed_command(
        &self,
        process_id: i32,
        process: &Arc<UnifiedExecProcess>,
    ) -> Result<bool, UnifiedExecError> {
        let Some(job) = self.managed.get(process_id).await else {
            return Ok(false);
        };
        if !job.background_jobs || !Arc::ptr_eq(&job.process, process) {
            return Ok(false);
        }
        job.revoked.cancel();
        self.managed.changed.notify_waiters();
        process.terminate_confirmed().await?;
        Ok(true)
    }

    pub(super) async fn record_queued_completions(&self, context: &UnifiedExecContext) -> bool {
        let mut pending = Vec::new();
        for (id, job) in self.managed.registry.queued().await {
            if context.cancellation_token.is_cancelled() {
                return false;
            }
            let job_id = job.metadata.lock().await.job_id;
            if !context
                .session
                .background_completion_deliverable(job_id, &job.origin.turn.sub_id)
                .await
            {
                continue;
            }
            if job.revoked.is_cancelled()
                || job.initial_response_consumed.load(Ordering::Acquire)
                || job.acknowledged.load(Ordering::Acquire)
            {
                self.managed.registry.consumed(job_id).await;
                continue;
            }
            if !job.initial_response_done.is_cancelled() || !job.process.has_exited() {
                continue;
            }
            let Ok(mut state) = Arc::clone(&job.state).try_lock_owned() else {
                continue;
            };
            if job.acknowledged.load(Ordering::Acquire)
                || !self.managed.registry.contains(job_id).await
            {
                continue;
            }
            // A cancelled turn may already have accepted output/hooks. Keep that
            // result with the record until a later turn commits its fragment.
            if state.result.is_none() && state.error.is_none() {
                let result = self
                    .write_stdin(
                        context,
                        WriteStdinRequest {
                            process_id: id,
                            input: "",
                            yield_time_ms: 5_000,
                            max_output_tokens: None,
                            truncation_policy: context
                                .step_context
                                .settings
                                .model_info
                                .truncation_policy
                                .into(),
                            interaction_event: None,
                        },
                    )
                    .await;
                match result {
                    Ok(mut output) => {
                        output.completion_delivery = Some(true);
                        state.result = Some(output);
                    }
                    Err(error) => state.error = Some(error.to_string()),
                }
            }
            if job.revoked.is_cancelled() {
                self.managed.registry.consumed(job_id).await;
                continue;
            }
            if let Some(output) = state.result.clone()
                && output.hook_command.is_some()
            {
                match self.accept_managed_output(context, &job, output).await {
                    Ok(output) => {
                        state.result = Some(output);
                    }
                    Err(error) => {
                        state.result = None;
                        state.error = Some(error.to_string());
                    }
                }
            }
            let fragment = if let Some(output) = &state.result {
                CommandCompletion::from_output(id, output)
            } else if let Some(error) = &state.error {
                CommandCompletion::failed(id, error)
            } else {
                unreachable!("accepted completion has either output or an error")
            };
            let fragment = {
                let mut metadata = job.metadata.lock().await;
                metadata.finish(&job.process, job.revoked.is_cancelled());
                ContextualUserFragment::into(fragment.with_metadata(&metadata))
            };
            pending.push(PendingCompletion {
                job,
                job_id,
                _claim: state,
                fragment,
            });
            if pending.len() == 4 {
                break;
            }
        }
        pending.retain(|completion| !completion.job.revoked.is_cancelled());
        if pending.is_empty() || context.cancellation_token.is_cancelled() {
            return false;
        }
        let session = Arc::clone(&context.session);
        let step = Arc::clone(&context.step_context);
        let registry = Arc::clone(&self.managed.registry);
        let changed = Arc::clone(&self.managed.changed);
        // This is the delivery commit boundary. Once admitted, append and
        // acknowledgement finish together even if abort drops the caller. It
        // records history only; it cannot schedule inference or start a turn.
        let commit = tokio::spawn(async move {
            let items = pending
                .iter()
                .map(|completion| completion.fragment.clone())
                .collect::<Vec<_>>();
            session
                .record_conversation_items(&step.turn, &step.settings.model_info, &items)
                .await;
            for completion in &pending {
                completion.job.acknowledged.store(true, Ordering::Release);
                registry.consumed(completion.job_id).await;
            }
            drop(pending);
            changed.notify_waiters();
        });
        if let Err(error) = commit.await {
            tracing::error!(%error, "background completion commit failed");
            return false;
        }
        self.evict_retired_managed_commands().await;
        true
    }
}
