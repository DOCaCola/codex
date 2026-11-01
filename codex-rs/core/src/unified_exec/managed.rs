//! Finite commands owned by a session. Waiting never schedules inference.
//!
//! The model loop consumes completions between sampling requests and parks before
//! ending a turn. Keeping that turn open preserves existing desktop/exec lifecycle
//! semantics without adding a client protocol. Delivery-only commands belong to
//! their launching turn and release to explicit reads when it stops waiting.
//! Opted-in background jobs instead retain their session-owned completion queue.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::StreamExt;
use futures::stream::FuturesUnordered;
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio::sync::OwnedSemaphorePermit;
use tokio::sync::Semaphore;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::TerminalSession;
use super::UnifiedExecContext;
use super::UnifiedExecError;
use super::UnifiedExecProcessManager;
use super::WriteStdinRequest;
use crate::tools::context::ExecCommandToolOutput;

#[path = "managed_delivery.rs"]
mod delivery;
#[path = "managed_listing.rs"]
mod listing;
#[path = "managed_queue_delivery.rs"]
mod queue_delivery;
#[path = "managed_registry.rs"]
mod registry;
#[path = "managed_release.rs"]
mod release;

#[derive(Clone, Copy, Debug, Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExecutionMode {
    Auto,
    Foreground,
    Background,
    Interactive,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Execution {
    pub mode: ExecutionMode,
    pub timeout: Option<Duration>,
    pub explicit_timeout: Option<Duration>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WaitMode {
    #[default]
    Timed,
    Completion,
}

pub(super) struct ManagedCommands {
    jobs: Mutex<BTreeMap<i32, Arc<Job>>>,
    registry: Arc<registry::SessionJobRegistry>,
    changed: Arc<Notify>,
    slots: Arc<Semaphore>,
}

impl Default for ManagedCommands {
    fn default() -> Self {
        Self {
            jobs: Mutex::default(),
            registry: Arc::default(),
            changed: Arc::default(),
            slots: Arc::new(Semaphore::new(
                RETAINED_RESULTS + super::MAX_UNIFIED_EXEC_PROCESSES,
            )),
        }
    }
}

impl ManagedCommands {
    async fn get(&self, process_id: i32) -> Option<Arc<Job>> {
        if let Some(job) = self.registry.get(process_id).await {
            return Some(job);
        }
        self.jobs.lock().await.get(&process_id).cloned()
    }

    async fn records(&self) -> Vec<(i32, Arc<Job>)> {
        let mut jobs = self
            .jobs
            .lock()
            .await
            .iter()
            .map(|(id, job)| (*id, Arc::clone(job)))
            .collect::<Vec<_>>();
        jobs.extend(self.registry.jobs().await);
        jobs.sort_by_key(|(id, _)| *id);
        jobs
    }
}

struct Job {
    background_jobs: bool,
    origin: Arc<crate::session::step_context::StepContext>,
    metadata: Mutex<metadata::JobMetadata>,
    _slot: Arc<OwnedSemaphorePermit>,
    terminal: TerminalSession,
    initial_response_done: CancellationToken,
    initial_response_consumed: AtomicBool,
    acknowledged: AtomicBool,
    /// Delivery-only records release to explicit reads when their turn stops waiting.
    released: AtomicBool,
    revoked: CancellationToken,
    state: Arc<Mutex<JobState>>,
}

impl Job {
    fn delivers_to(&self, turn_id: &str) -> bool {
        self.background_jobs
            || (!self.released.load(Ordering::Acquire) && self.origin.turn.sub_id == turn_id)
    }
}

#[derive(Default)]
struct JobState {
    result: Option<ExecCommandToolOutput>,
    error: Option<String>,
    last_poll: Option<Instant>,
    poll_level: usize,
}

impl JobState {
    fn next_poll_wait(&mut self, requested: u64) -> u64 {
        self.poll_level = if self
            .last_poll
            .is_some_and(|last| last.elapsed() < Duration::from_secs(60))
        {
            (self.poll_level + 1).min(WAIT_LADDER.len() - 1)
        } else {
            0
        };
        requested.max(WAIT_LADDER[self.poll_level])
    }
}

pub(super) struct LaunchGuard {
    job: Arc<Job>,
    changed: Arc<Notify>,
}

impl LaunchGuard {
    pub(super) fn acknowledge(&self) {
        self.job
            .initial_response_consumed
            .store(true, Ordering::Release);
    }
}

impl Drop for LaunchGuard {
    fn drop(&mut self) {
        self.job.initial_response_done.cancel();
        self.changed.notify_waiters();
    }
}

const WAIT_LADDER: [u64; 5] = [5_000, 10_000, 30_000, 60_000, 300_000];
const RETAINED_RESULTS: usize = 64;

#[path = "managed_metadata.rs"]
pub(crate) mod metadata;

struct NotifyOnDrop(Arc<Notify>);

impl Drop for NotifyOnDrop {
    fn drop(&mut self) {
        self.0.notify_waiters();
    }
}

impl UnifiedExecProcessManager {
    pub(crate) async fn resolve_terminal(
        &self,
        process_id: i32,
    ) -> Result<TerminalSession, UnifiedExecError> {
        if let Some(entry) = self.process_store.lock().await.processes.get(&process_id) {
            return Ok(TerminalSession {
                process_id,
                process: Arc::clone(&entry.process),
                call_id: entry.call_id.clone(),
                environment_id: entry.environment_id.clone(),
            });
        }
        // Finite results outlive the live process store, but retain the same
        // terminal identity and environment for explicit artifact reads.
        self.managed
            .get(process_id)
            .await
            .map(|job| job.terminal.clone())
            .ok_or(UnifiedExecError::UnknownProcessId { process_id })
    }

    pub(super) async fn reserve_managed_command(
        &self,
    ) -> Result<Arc<OwnedSemaphorePermit>, UnifiedExecError> {
        self.evict_retired_managed_commands().await;
        Arc::clone(&self.managed.slots).try_acquire_owned().map(Arc::new).map_err(|_| {
            UnifiedExecError::process_failed("managed command capacity reached; consume pending results before starting more commands".into())
        })
    }

    pub(super) async fn evict_retired_managed_commands(&self) {
        {
            let mut jobs = self.managed.jobs.lock().await;
            // Preserve delivery-only retention when background_jobs is disabled.
            if jobs.len() >= RETAINED_RESULTS {
                jobs.retain(|_, job| {
                    !job.terminal.process.has_exited()
                        || !(job.acknowledged.load(Ordering::Acquire)
                            || job.revoked.is_cancelled()
                            || job.initial_response_consumed.load(Ordering::Acquire)
                            || job.released.load(Ordering::Acquire))
                });
            }
        }
        self.managed.registry.evict_retired().await;
    }

    pub(super) async fn track_managed_command(
        &self,
        process_id: i32,
        context: &UnifiedExecContext,
        metadata: metadata::JobMetadata,
        terminal: TerminalSession,
        slot: Arc<OwnedSemaphorePermit>,
    ) -> LaunchGuard {
        let job_id = metadata.job_id;
        let background_jobs = context.session.background_jobs.enabled;
        let job = Arc::new(Job {
            background_jobs,
            origin: Arc::clone(&context.step_context),
            metadata: Mutex::new(metadata),
            _slot: slot,
            terminal,
            initial_response_done: CancellationToken::new(),
            initial_response_consumed: AtomicBool::new(false),
            acknowledged: AtomicBool::new(false),
            released: AtomicBool::new(false),
            revoked: CancellationToken::new(),
            state: Arc::new(Mutex::new(JobState::default())),
        });
        if job.background_jobs {
            self.managed
                .registry
                .insert(process_id, job_id, Arc::clone(&job))
                .await;
        } else {
            self.managed
                .jobs
                .lock()
                .await
                .insert(process_id, Arc::clone(&job));
        }
        let registry = Arc::clone(&self.managed.registry);
        let changed = Arc::clone(&self.managed.changed);
        let finishing = Arc::clone(&job);
        tokio::spawn(async move {
            finishing
                .terminal
                .process
                .cancellation_token()
                .cancelled()
                .await;
            finishing.metadata.lock().await.finish(
                &finishing.terminal.process,
                finishing.revoked.is_cancelled(),
            );
            if finishing.background_jobs {
                registry.enqueue(job_id).await;
                changed.notify_waiters();
            }
        });
        self.managed.changed.notify_waiters();
        LaunchGuard {
            job,
            changed: Arc::clone(&self.managed.changed),
        }
    }

    pub(super) async fn managed_id_reserved(&self, process_id: i32) -> bool {
        self.managed.get(process_id).await.is_some()
    }

    pub(super) async fn revoke_managed_commands(&self) {
        for (_, job) in self.managed.records().await {
            job.revoked.cancel();
        }
        self.managed.changed.notify_waiters();
    }

    pub(super) async fn revoke_managed_command(&self, process_id: i32) {
        if let Some(job) = self.managed.get(process_id).await {
            job.revoked.cancel();
        }
        self.managed.changed.notify_waiters();
    }

    pub(crate) async fn read_managed_command(
        &self,
        context: &UnifiedExecContext,
        mut request: WriteStdinRequest<'_>,
        wait: WaitMode,
    ) -> Result<ExecCommandToolOutput, UnifiedExecError> {
        let job = self
            .managed
            .get(request.terminal.process_id)
            .await
            .filter(|job| Arc::ptr_eq(&job.terminal.process, &request.terminal.process));
        let Some(job) = job else {
            if wait == WaitMode::Completion {
                return Err(UnifiedExecError::process_failed(
                    "completion waits require a finite command with automatic delivery".into(),
                ));
            }
            return self.write_stdin(context, request).await;
        };
        if !job.initial_response_done.is_cancelled() {
            tokio::select! {
                biased;
                _ = context.cancellation_token.cancelled() => return Err(UnifiedExecError::process_failed("wait cancelled".into())),
                _ = job.revoked.cancelled() => return Err(UnifiedExecError::process_failed("command delivery cancelled".into())),
                _ = job.initial_response_done.cancelled() => {}
            }
        }
        // Serializes automatic delivery with explicit reads, including code-mode
        // calls that are still executing after their cell yielded.
        // Declared before the lock so cancellation releases the lock before
        // waking a parked turn that observed this in-flight reader.
        let _reader_finished = NotifyOnDrop(Arc::clone(&self.managed.changed));
        let mut state = Arc::clone(&job.state).lock_owned().await;
        if job.background_jobs && (state.result.is_some() || state.error.is_some()) {
            return self
                .read_cached_background_job(context, &job, &mut state, request)
                .await;
        }
        if let Some(error) = &state.error {
            return Err(UnifiedExecError::process_failed(error.clone()));
        }
        if let Some(result) = &state.result {
            if !request.input.is_empty() {
                return Err(UnifiedExecError::StdinClosed);
            }
            let mut result = result.clone();
            result.max_output_tokens = request.max_output_tokens;
            result.truncation_policy = request.truncation_policy;
            // PostToolUse belongs to the first consumption, not artifact reads.
            result.hook_command = None;
            return Ok(result);
        }
        if job.revoked.is_cancelled() {
            return Err(UnifiedExecError::process_failed(
                "command delivery cancelled".into(),
            ));
        }
        if wait == WaitMode::Completion {
            let exited = job.terminal.process.cancellation_token();
            tokio::select! {
                biased;
                _ = context.cancellation_token.cancelled() => return Err(UnifiedExecError::process_failed("wait cancelled".into())),
                _ = job.revoked.cancelled() => return Err(UnifiedExecError::process_failed("command delivery cancelled".into())),
                _ = exited.cancelled() => {}
            }
        } else if request.input.is_empty() {
            request.yield_time_ms = state.next_poll_wait(request.yield_time_ms);
        }
        let result = self.write_stdin(context, request).await.map(|mut output| {
            output.completion_delivery = Some(true);
            output
        });
        if job.background_jobs {
            let result = self
                .accept_background_job_read(context, &job, &mut state, result)
                .await;
            drop(state);
            drop(job);
            self.evict_retired_managed_commands().await;
            return result;
        }
        let result = match result {
            Ok(output) if output.process_id.is_none() => {
                self.accept_managed_output(context, &job, output, &[&job.terminal.environment_id])
                    .await
            }
            result => result,
        };
        if let Ok(output) = &result {
            state.last_poll = Some(Instant::now());
            if !output.raw_output.text.is_empty() {
                state.last_poll = None;
                state.poll_level = 0;
            }
            if output.process_id.is_none() {
                state.result = Some(output.clone());
                job.acknowledged.store(true, Ordering::Release);
            }
        } else if job.terminal.process.has_exited() {
            state.error = result.as_ref().err().map(ToString::to_string);
            job.acknowledged.store(true, Ordering::Release);
        }
        drop(state);
        drop(job);
        self.evict_retired_managed_commands().await;
        result
    }

    /// Park the current logical turn; wake only for completion, input or cleanup.
    async fn wait_for_managed_commands(&self, context: &UnifiedExecContext) -> bool {
        let turn_id = &context.step_context.turn.sub_id;
        loop {
            let changed = self.managed.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let jobs = self.managed.records().await;
            let queued_ready = if context.session.background_jobs.enabled {
                Some(self.managed.registry.has_ready_completion().await)
            } else {
                None
            };
            let mut exits = FuturesUnordered::new();
            let mut waiting_reader = false;
            for (_, job) in jobs {
                if job.revoked.is_cancelled() || !job.delivers_to(turn_id) {
                    continue;
                }
                if job.initial_response_consumed.load(Ordering::Acquire) {
                    continue;
                }
                if !job.initial_response_done.is_cancelled() {
                    exits.push(job.initial_response_done.clone().cancelled_owned());
                    continue;
                }
                let Ok(_state) = job.state.try_lock() else {
                    waiting_reader = true;
                    continue;
                };
                if job.acknowledged.load(Ordering::Acquire) {
                    continue;
                }
                let exited = job.terminal.process.cancellation_token();
                if exited.is_cancelled() {
                    if queued_ready != Some(false) {
                        return true;
                    }
                    waiting_reader = true;
                    continue;
                }
                exits.push(exited.cancelled_owned());
            }
            if exits.is_empty() && !waiting_reader {
                return false;
            }
            let turn_state = context
                .session
                .input_queue
                .turn_state_for_sub_id(
                    &context.session.active_turn,
                    &context.step_context.turn.sub_id,
                )
                .await;
            let (mut activity, pending) = context
                .session
                .input_queue
                .subscribe_activity(turn_state.as_deref())
                .await;
            if pending.is_some() {
                return true;
            }
            tokio::select! {
                biased;
                _ = context.cancellation_token.cancelled() => return false,
                _ = activity.changed() => return true,
                _ = exits.next(), if !exits.is_empty() => return true,
                _ = &mut changed => {}
            }
        }
    }

    pub(crate) async fn settle_managed_commands(
        &self,
        context: &UnifiedExecContext,
        model_needs_follow_up: bool,
    ) -> bool {
        let delivered = self.record_command_completions(context).await;
        if delivered || model_needs_follow_up || context.session.background_continuation.enabled {
            return delivered;
        }
        let limit = context
            .step_context
            .turn
            .config
            .background_command_turn_wait;
        let deadline = (!context.session.background_jobs.enabled).then(|| Instant::now() + limit);
        loop {
            if context
                .session
                .input_queue
                .has_pending_input(&context.session.active_turn)
                .await
            {
                return true;
            }
            if let Some(deadline) = deadline {
                match tokio::time::timeout_at(deadline, self.wait_for_managed_commands(context))
                    .await
                {
                    Ok(true) => {}
                    Ok(false) => return false,
                    Err(_) => {
                        return self.record_command_completions(context).await
                            || self.release_stalled_commands(context, limit).await;
                    }
                }
            } else if !self.wait_for_managed_commands(context).await {
                return false;
            }
            if self.record_command_completions(context).await {
                return true;
            }
        }
    }
}

#[cfg(test)]
#[path = "managed_tests.rs"]
mod tests;
