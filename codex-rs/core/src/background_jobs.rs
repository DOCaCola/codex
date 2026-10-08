//! Read-only snapshots of the session's managed job records.
use crate::unified_exec::managed::ExecutionMode;
use crate::unified_exec::managed::metadata::JobMetadata;
use crate::unified_exec::managed::metadata::TerminalState;
use std::io;

#[derive(Debug, PartialEq, Eq)]
pub enum BackgroundJobMode {
    Auto,
    Background,
    Foreground,
    Interactive,
}
#[derive(Debug, PartialEq, Eq)]
pub enum BackgroundJobState {
    Running,
    Completed,
    Failed,
    TimedOut,
    Stopped,
}
#[derive(Debug, PartialEq, Eq)]
pub enum BackgroundJobDelivery {
    Pending,
    Delivered,
    None,
}

#[derive(Debug)]
pub struct BackgroundJob {
    pub job_id: String,
    pub item_id: String,
    pub process_id: String,
    pub mode: BackgroundJobMode,
    pub state: BackgroundJobState,
    pub started_at_ms: i64,
    pub timeout_ms: Option<i64>,
    pub deadline_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
    pub exit_code: Option<i32>,
    pub delivery: BackgroundJobDelivery,
}

#[derive(Default)]
pub(crate) struct DeliveryFacts {
    pub revoked: bool,
    pub acknowledged: bool,
    pub initial_response_consumed: bool,
}

impl BackgroundJob {
    pub(crate) fn from_metadata(
        job: &JobMetadata,
        process_id: i32,
        facts: DeliveryFacts,
    ) -> io::Result<Self> {
        let state = match job.terminal_state {
            None => BackgroundJobState::Running,
            Some(TerminalState::Completed) => BackgroundJobState::Completed,
            Some(TerminalState::Failed) => BackgroundJobState::Failed,
            Some(TerminalState::TimedOut) => BackgroundJobState::TimedOut,
            Some(TerminalState::Stopped) => BackgroundJobState::Stopped,
        };
        // Consumption wins, including an initial/foreground tool response.
        // Unconsumed revoked/manual jobs never auto-deliver; auto/background
        // jobs remain pending both while running and after exit.
        let delivery = if facts.acknowledged || facts.initial_response_consumed {
            BackgroundJobDelivery::Delivered
        } else if facts.revoked
            || matches!(
                job.mode,
                ExecutionMode::Foreground | ExecutionMode::Interactive
            )
        {
            BackgroundJobDelivery::None
        } else {
            BackgroundJobDelivery::Pending
        };
        let mode = match job.mode {
            ExecutionMode::Auto => BackgroundJobMode::Auto,
            ExecutionMode::Background => BackgroundJobMode::Background,
            ExecutionMode::Foreground => BackgroundJobMode::Foreground,
            ExecutionMode::Interactive => BackgroundJobMode::Interactive,
        };
        let timeout_ms = job
            .explicit_timeout_ms
            .map(i64::try_from)
            .transpose()
            .map_err(io::Error::other)?;
        Ok(Self {
            job_id: job.job_id.to_string(),
            item_id: job.item_id.clone(),
            process_id: process_id.to_string(),
            mode,
            state,
            started_at_ms: job.started_at.timestamp_millis(),
            timeout_ms,
            deadline_at_ms: job.deadline_at_ms()?,
            completed_at_ms: job.completed_at.map(|time| time.timestamp_millis()),
            exit_code: job.exit_code,
            delivery,
        })
    }
}
