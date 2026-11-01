//! One identity and outcome record per managed command, independent of terminal handles.
use super::ExecutionMode;
use crate::unified_exec::ExecCommandRequest;
use crate::unified_exec::UnifiedExecContext;
use crate::unified_exec::UnifiedExecProcess;
use chrono::DateTime;
use chrono::Utc;
use serde::Serialize;
use tokio::time::Duration;
use tokio::time::Instant;
use uuid::Uuid;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TerminalState {
    Completed,
    Failed,
    TimedOut,
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DeadlineSource {
    Explicit,
    HostDefault,
}

#[derive(Serialize)]
pub(crate) struct JobMetadata {
    pub job_id: Uuid,
    pub originating_turn: String,
    pub call_id: String,
    pub stack: Option<crate::tools::context::CommandStackIdentity>,
    pub item_id: String,
    pub command: String,
    pub mode: ExecutionMode,
    pub started_at: DateTime<Utc>,
    pub explicit_timeout_ms: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline_source: Option<DeadlineSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline_origin: Option<&'static str>,
    #[serde(skip)]
    pub effective_deadline: Option<Instant>,
    pub completed_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub terminal_state: Option<TerminalState>,
    pub duration_ms: u128,
    pub deadline_fired: bool,
    pub stopped: bool,
    #[serde(skip)]
    pub(super) clock_started_at: Instant,
}

impl JobMetadata {
    pub(crate) fn deadline_at_ms(&self) -> std::io::Result<Option<i64>> {
        self.effective_deadline
            .map(|deadline| {
                let offset =
                    i64::try_from(deadline.duration_since(self.clock_started_at).as_millis())
                        .map_err(std::io::Error::other)?;
                self.started_at
                    .timestamp_millis()
                    .checked_add(offset)
                    .ok_or_else(|| {
                        std::io::Error::other("job deadline exceeds the epoch millisecond range")
                    })
            })
            .transpose()
    }
    pub(in crate::unified_exec) fn new(
        request: &ExecCommandRequest,
        context: &UnifiedExecContext,
        execution: super::Execution,
        process: &UnifiedExecProcess,
        start: Instant,
        deadline: Option<Duration>,
    ) -> Self {
        Self {
            job_id: Uuid::new_v4(),
            originating_turn: context.step_context.turn.sub_id.clone(),
            call_id: context.call_id.clone(),
            stack: request.command_stack.clone(),
            item_id: context.call_id.clone(),
            command: request.prepared.hook_command.clone(),
            mode: execution.mode,
            started_at: if context.session.background_jobs.enabled {
                process.started_at_utc
            } else {
                Utc::now()
            },
            deadline_source: (context.session.background_jobs.enabled && deadline.is_some())
                .then_some(if execution.explicit_timeout.is_some() {
                    DeadlineSource::Explicit
                } else {
                    DeadlineSource::HostDefault
                }),
            deadline_origin: (context.session.background_jobs.enabled && deadline.is_some())
                .then_some("process_start"),
            explicit_timeout_ms: execution
                .explicit_timeout
                .map(|timeout| timeout.as_millis()),
            effective_deadline: deadline.and_then(|timeout| start.checked_add(timeout)),
            completed_at: None,
            exit_code: None,
            terminal_state: None,
            duration_ms: 0,
            deadline_fired: false,
            stopped: false,
            clock_started_at: start,
        }
    }

    pub(super) fn finish(&mut self, process: &UnifiedExecProcess, stopped: bool) {
        if self.completed_at.is_none() {
            self.completed_at = Some(Utc::now());
            self.duration_ms = self.clock_started_at.elapsed().as_millis();
        }
        self.exit_code = process.exit_code();
        self.deadline_fired = self.effective_deadline.is_some() && process.timed_out();
        self.stopped = !self.deadline_fired
            && process.failure_message().is_none()
            && (stopped || process.stopped());
        self.terminal_state = Some(if self.deadline_fired {
            TerminalState::TimedOut
        } else if self.stopped {
            TerminalState::Stopped
        } else if self.exit_code == Some(0) && process.failure_message().is_none() {
            TerminalState::Completed
        } else {
            TerminalState::Failed
        });
    }
}

#[cfg(test)]
#[path = "managed_metadata_tests.rs"]
mod tests;
