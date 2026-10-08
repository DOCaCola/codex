//! Read-only snapshots of the session's managed job records.
use crate::unified_exec::managed::ExecutionMode;
use crate::unified_exec::managed::metadata::JobMetadata;
use crate::unified_exec::managed::metadata::TerminalState;
use std::io;

/// Runtime host policy inherited by a session's child configs, never a TOML setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundJobHostClass {
    Interactive,
    Unattended,
}

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

/// Process timing for opted-in auto/background jobs, excluding host-default limits.
pub struct BackgroundJobTimer {
    pub item_id: String,
    pub process_id: String,
    pub started_at_ms: i64,
    pub explicit_timeout_seconds: Option<u64>,
    pub duration_seconds: Option<u64>,
}

/// Captured policy and current work, read together with turn admission.
pub struct BackgroundWorkState {
    pub pending: bool,
    pub active: bool,
    pub last_turn_id: Option<String>,
    pub idle_continuation: bool,
    pub unattended_deadline: std::time::Duration,
    pub job_timers: Vec<BackgroundJobTimer>,
}

impl crate::codex_thread::CodexThread {
    pub fn background_jobs_enabled(&self) -> bool {
        self.session.background_jobs.enabled
    }

    /// Snapshot under turn admission's lock, so a drained queue cannot hide an admitted turn.
    pub async fn background_work_state(&self) -> BackgroundWorkState {
        let active = self.session.active_turn.lock().await;
        let pending = self.has_live_background_work().await;
        let last = self.session.state.lock().await.last_started_turn_id.clone();
        BackgroundWorkState {
            pending,
            active: active.as_ref().is_some_and(|turn| turn.task.is_some()),
            last_turn_id: last,
            idle_continuation: self.session.background_continuation.enabled,
            unattended_deadline: self.session.background_jobs.defaults.unattended_deadline,
            job_timers: if self.session.background_jobs.enabled {
                self.session.services.unified_exec_manager.background_job_timers().await
            } else {
                Vec::new()
            },
        }
    }

    /// Opted-in registry work that must survive automatic no-subscriber unload.
    pub async fn has_live_background_work(&self) -> bool {
        self.session
            .services
            .unified_exec_manager
            .has_live_background_work()
            .await
    }

    /// Reuses the managed registry notification; callers register before reading work.
    pub fn background_work_changed(&self) -> tokio::sync::futures::OwnedNotified {
        self.session
            .services
            .unified_exec_manager
            .background_work_changed()
    }
}
