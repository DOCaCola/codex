//! Opted-in session capability and event-driven completion admission.
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use codex_protocol::config_types::ModeKind;
use codex_protocol::protocol::TurnAbortReason;
use codex_protocol::turn_input::TurnInputMode;
use codex_protocol::turn_input::TurnInputRequest;
use codex_protocol::turn_input::TurnInputSubmission;
use codex_protocol::turn_input::TurnStartOptions;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::session::Session;
use crate::context::ContextualUserFragment;
use crate::context::InternalContextSource;
use crate::context::InternalModelContextFragment;

pub(crate) const TRIGGER: &str = "background_job_completion";
pub(crate) struct UserStartedTurn;

pub(crate) struct BackgroundWakeReservation(pub Arc<Mutex<crate::state::TurnState>>);

impl BackgroundWakeReservation {
    pub(crate) fn matches(&self, active: Option<&crate::state::ActiveTurn>) -> bool {
        active.is_some_and(|turn| Arc::ptr_eq(&turn.turn_state, &self.0))
    }
}

pub(crate) struct WakeCandidate {
    pub job_id: Uuid,
    pub originating_turn: String,
    pub ready: bool,
}

#[derive(Default)]
struct Suppression {
    epoch: u64,
    output_schema: Option<serde_json::Value>,
    jobs: HashSet<Uuid>,
    turns: HashSet<String>,
}

pub(crate) struct BackgroundContinuation {
    pub enabled: bool,
    pub stop: CancellationToken,
    interruptions: AtomicUsize,
    suppression: Mutex<Suppression>,
}

impl BackgroundContinuation {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            stop: CancellationToken::new(),
            interruptions: AtomicUsize::new(0),
            suppression: Mutex::new(Suppression::default()),
        }
    }

    fn can_wake(&self, mode: ModeKind, shutting_down: bool) -> bool {
        self.enabled
            && !self.stop.is_cancelled()
            && !shutting_down
            && self.interruptions.load(Ordering::Acquire) == 0
            && mode != ModeKind::Plan
    }

    async fn permits(&self, candidate: &WakeCandidate) -> bool {
        let suppressed = self.suppression.lock().await;
        self.enabled
            && !self.stop.is_cancelled()
            && candidate.ready
            && !suppressed.jobs.contains(&candidate.job_id)
            && !suppressed.turns.contains(&candidate.originating_turn)
    }
}

pub(crate) struct BackgroundInterruption<'a> {
    counter: &'a AtomicUsize,
    changed: Arc<tokio::sync::Notify>,
}

impl Drop for BackgroundInterruption<'_> {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::AcqRel);
        self.changed.notify_waiters();
    }
}

impl Drop for BackgroundContinuation {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

impl Session {
    pub(crate) async fn begin_background_interrupt(
        &self,
        reason: Option<&TurnAbortReason>,
        origin: Option<&str>,
    ) -> Option<BackgroundInterruption<'_>> {
        if !self.background_continuation.enabled || reason != Some(&TurnAbortReason::Interrupted) {
            return None;
        }
        self.background_continuation.interruptions.fetch_add(1, Ordering::AcqRel);
        let guard = BackgroundInterruption {
            counter: &self.background_continuation.interruptions,
            changed: self.services.unified_exec_manager.background_work_notifications(),
        };
        let capability = &self.background_continuation;
        let epoch = {
            let mut suppression = capability.suppression.lock().await;
            suppression.epoch += 1;
            suppression.epoch
        };
        let originating_turn = match origin {
            Some(id) => Some(id.to_string()),
            None => self
                .active_turn
                .lock()
                .await
                .as_ref()
                .and_then(|turn| turn.task.as_ref())
                .map(|task| task.turn_context.sub_id.clone()),
        };
        let pending = self
            .services
            .unified_exec_manager
            .background_wake_candidates()
            .await;
        let mut suppression = capability.suppression.lock().await;
        // User-started execution ends the suppression epoch, including an abort
        // whose asynchronous snapshot began before that user turn started.
        if suppression.epoch == epoch {
            suppression
                .jobs
                .extend(pending.into_iter().map(|job| job.job_id));
            if let Some(turn) = originating_turn {
                suppression.turns.insert(turn);
            }
        }
        Some(guard)
    }

    pub(crate) fn start_background_continuation_worker(self: &Arc<Self>) {
        if !self.background_continuation.enabled {
            return;
        }
        let weak = Arc::downgrade(self);
        let stop = self.background_continuation.stop.clone();
        tokio::spawn(async move {
            loop {
                if stop.is_cancelled() {
                    return;
                }
                let Some(session) = weak.upgrade() else {
                    return;
                };
                let changed = session
                    .services
                    .unified_exec_manager
                    .background_work_changed();
                tokio::pin!(changed);
                changed.as_mut().enable();
                session.maybe_start_turn_for_pending_work().await;
                drop(session);
                tokio::select! {
                    _ = stop.cancelled() => return,
                    _ = &mut changed => {}
                }
            }
        });
    }

    pub(crate) async fn begin_background_user_turn(&self, turn: &super::turn_context::TurnContext) {
        if self.background_continuation.enabled
            && turn.extension_data.remove::<UserStartedTurn>().is_some()
        {
            let mut suppression = self.background_continuation.suppression.lock().await;
            suppression.epoch += 1;
            suppression.output_schema = turn.final_output_json_schema.clone();
            suppression.jobs.clear();
            suppression.turns.clear();
        }
    }

    pub(crate) async fn background_completion_deliverable(&self, job_id: Uuid, origin: &str) -> bool {
        if !self.background_continuation.enabled {
            return true;
        }
        self.background_continuation
            .permits(&WakeCandidate {
                job_id,
                originating_turn: origin.to_string(),
                ready: true,
            })
            .await
    }

    pub(crate) async fn background_completion_ready(&self) -> bool {
        let candidates = self
            .services
            .unified_exec_manager
            .background_wake_candidates()
            .await;
        let mut ready = false;
        for candidate in candidates {
            if self.background_continuation.permits(&candidate).await {
                ready = true;
                break;
            }
        }
        ready && self.background_continuation.interruptions.load(Ordering::Acquire) == 0
    }

    pub(crate) async fn maybe_start_background_completion(self: &Arc<Self>, sub_id: String) {
        if !self.background_continuation.enabled {
            return;
        }
        let (expected, shutting_down) = {
            let state = self.state.lock().await;
            (state.last_started_turn_id.clone(), state.shutting_down)
        };
        if !self
            .background_continuation
            .can_wake(self.collaboration_mode().await.mode, shutting_down)
        {
            return;
        }
        let Some(expected) = expected else {
            return;
        };
        if !self.background_completion_ready().await {
            return;
        }
        let fragment = ContextualUserFragment::into(InternalModelContextFragment::new(
            InternalContextSource::from_static(TRIGGER),
            "Background command results are ready. Continue the existing work from the completion results. This is internal context, not new user authorization.",
        ));
        let request =
            TurnInputRequest::new(codex_protocol::turn_input::TurnInput::ResponseItem(fragment))
                .on_start(TurnStartOptions {
                    turn_trigger: Some(TRIGGER.into()),
                    final_output_json_schema: self.background_continuation.suppression.lock().await.output_schema.clone(),
                    ..Default::default()
                });
        match super::turn_input::handle(
            self,
            request,
            TurnInputMode::ContinueIfIdle {
                expected_previous_turn_id: expected,
            },
            sub_id,
        )
        .await
        {
            Ok(TurnInputSubmission::Started { .. }) => {}
            Ok(TurnInputSubmission::NotSubmitted { reason }) => {
                tracing::debug!(?reason, "background wake not admitted");
            }
            Ok(TurnInputSubmission::Steered { .. }) => unreachable!("internal continuation cannot steer"),
            Err(error) => tracing::warn!(%error, "background continuation admission failed"),
        }
    }
}

#[cfg(test)]
#[path = "background_continuation_tests.rs"]
mod tests;
