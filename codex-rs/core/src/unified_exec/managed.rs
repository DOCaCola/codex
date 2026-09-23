//! Finite commands owned by a session. Waiting never schedules inference.
//!
//! The model loop consumes completions between sampling requests and parks before
//! ending a turn. Keeping that turn open preserves existing desktop/exec lifecycle
//! semantics without adding a client protocol.

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

use super::UnifiedExecContext;
use super::UnifiedExecError;
use super::UnifiedExecProcess;
use super::UnifiedExecProcessManager;
use super::WriteStdinRequest;
use crate::tools::context::ExecCommandToolOutput;

#[path = "managed_delivery.rs"]
mod delivery;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
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
    changed: Arc<Notify>,
    slots: Arc<Semaphore>,
}

impl Default for ManagedCommands {
    fn default() -> Self {
        Self {
            jobs: Mutex::default(),
            changed: Arc::default(),
            slots: Arc::new(Semaphore::new(
                RETAINED_RESULTS + super::MAX_UNIFIED_EXEC_PROCESSES,
            )),
        }
    }
}

struct Job {
    _slot: Arc<OwnedSemaphorePermit>,
    process: Arc<UnifiedExecProcess>,
    initial_response_done: CancellationToken,
    initial_response_consumed: AtomicBool,
    revoked: CancellationToken,
    state: Arc<Mutex<JobState>>,
}

#[derive(Default)]
struct JobState {
    acknowledged: bool,
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

struct NotifyOnDrop(Arc<Notify>);

impl Drop for NotifyOnDrop {
    fn drop(&mut self) {
        self.0.notify_waiters();
    }
}

impl UnifiedExecProcessManager {
    pub(super) async fn reserve_managed_command(
        &self,
    ) -> Result<Arc<OwnedSemaphorePermit>, UnifiedExecError> {
        let mut jobs = self.managed.jobs.lock().await;
        // Retained results are addressable through write_stdin, including on a
        // remote executor. Never evict an unconsumed completion.
        if jobs.len() >= RETAINED_RESULTS {
            let retired = jobs.iter().find_map(|(id, job)| {
                job.state.try_lock().ok().and_then(|state| {
                    (state.acknowledged
                        || job.revoked.is_cancelled()
                        || job.initial_response_consumed.load(Ordering::Acquire))
                    .then_some(*id)
                })
            });
            if let Some(id) = retired {
                jobs.remove(&id);
            }
        }
        Arc::clone(&self.managed.slots).try_acquire_owned().map(Arc::new).map_err(|_| {
            UnifiedExecError::process_failed("managed command capacity reached; consume pending results before starting more commands".into())
        })
    }

    pub(super) async fn track_managed_command(
        &self,
        process_id: i32,
        process: Arc<UnifiedExecProcess>,
        slot: Arc<OwnedSemaphorePermit>,
    ) -> LaunchGuard {
        let job = Arc::new(Job {
            _slot: slot,
            process,
            initial_response_done: CancellationToken::new(),
            initial_response_consumed: AtomicBool::new(false),
            revoked: CancellationToken::new(),
            state: Arc::new(Mutex::new(JobState::default())),
        });
        self.managed
            .jobs
            .lock()
            .await
            .insert(process_id, Arc::clone(&job));
        self.managed.changed.notify_waiters();
        LaunchGuard {
            job,
            changed: Arc::clone(&self.managed.changed),
        }
    }

    pub(super) async fn managed_id_reserved(&self, process_id: i32) -> bool {
        self.managed.jobs.lock().await.contains_key(&process_id)
    }

    pub(super) async fn revoke_managed_commands(&self) {
        for job in self.managed.jobs.lock().await.values() {
            job.revoked.cancel();
        }
        self.managed.changed.notify_waiters();
    }

    pub(super) async fn revoke_managed_command(&self, process_id: i32) {
        if let Some(job) = self.managed.jobs.lock().await.get(&process_id) {
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
            .jobs
            .lock()
            .await
            .get(&request.process_id)
            .cloned();
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
            let exited = job.process.cancellation_token();
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
        let result = match result {
            Ok(output) if output.process_id.is_none() => {
                self.accept_managed_output(context, output).await
            }
            result => result,
        };
        if let Ok(output) = &result {
            state.last_poll = Some(Instant::now());
            if !output.raw_output.is_empty() {
                state.last_poll = None;
                state.poll_level = 0;
            }
            if output.process_id.is_none() {
                state.acknowledged = true;
                state.result = Some(output.clone());
            }
        } else if job.process.has_exited() {
            state.acknowledged = true;
            state.error = result.as_ref().err().map(ToString::to_string);
        }
        result
    }

    /// Park the current logical turn; wake only for completion, input or cleanup.
    async fn wait_for_managed_commands(&self, context: &UnifiedExecContext) -> bool {
        loop {
            let changed = self.managed.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let jobs = self
                .managed
                .jobs
                .lock()
                .await
                .values()
                .cloned()
                .collect::<Vec<_>>();
            let mut exits = FuturesUnordered::new();
            let mut waiting_reader = false;
            for job in jobs {
                if job.revoked.is_cancelled() {
                    continue;
                }
                if job.initial_response_consumed.load(Ordering::Acquire) {
                    continue;
                }
                if !job.initial_response_done.is_cancelled() {
                    exits.push(job.initial_response_done.clone().cancelled_owned());
                    continue;
                }
                let Ok(state) = job.state.try_lock() else {
                    waiting_reader = true;
                    continue;
                };
                if state.acknowledged {
                    continue;
                }
                let exited = job.process.cancellation_token();
                if exited.is_cancelled() {
                    return true;
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
        if delivered || model_needs_follow_up {
            return delivered;
        }
        loop {
            if context
                .session
                .input_queue
                .has_pending_input(&context.session.active_turn)
                .await
            {
                return true;
            }
            if !self.wait_for_managed_commands(context).await {
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
