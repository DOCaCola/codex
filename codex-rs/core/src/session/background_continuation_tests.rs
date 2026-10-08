//! Written, not run: capability, readiness, Plan and suppression admission.
use super::*;
use pretty_assertions::assert_eq;

#[test]
fn background_wake_cannot_recreate_a_removed_or_replaced_reservation() {
    let active = crate::state::ActiveTurn::default();
    let reservation = BackgroundWakeReservation(Arc::clone(&active.turn_state));
    assert!(reservation.matches(Some(&active)));
    assert!(!reservation.matches(None));
    assert!(!reservation.matches(Some(&crate::state::ActiveTurn::default())));
}

#[tokio::test]
async fn interruption_guard_releases_on_drop_and_signals_existing_channel() {
    let (mut session, _) = crate::session::tests::make_session_and_context().await;
    session.background_continuation = BackgroundContinuation::new(/*enabled*/ true);
    let changed = session.services.unified_exec_manager.background_work_changed();
    tokio::pin!(changed);
    changed.as_mut().enable();
    let guard = session.begin_background_interrupt(Some(&TurnAbortReason::Interrupted), /*origin*/ None).await.unwrap();
    assert_eq!(session.background_continuation.interruptions.load(Ordering::Acquire), 1);
    drop(guard);
    assert_eq!(session.background_continuation.interruptions.load(Ordering::Acquire), 0);
    changed.await;
}

#[tokio::test]
async fn background_completion_predicate_respects_mode_readiness_and_suppression() {
    let capability = BackgroundContinuation::new(/*enabled*/ true);
    let candidate = WakeCandidate { job_id: Uuid::new_v4(), originating_turn: "origin".into(), ready: true };
    assert!(capability.can_wake(ModeKind::Default, /*shutting_down*/ false));
    assert!(!capability.can_wake(ModeKind::Plan, /*shutting_down*/ false));
    assert!(!capability.can_wake(ModeKind::Default, /*shutting_down*/ true));
    assert!(capability.permits(&candidate).await);
    capability.suppression.lock().await.jobs.insert(candidate.job_id);
    assert!(!capability.permits(&candidate).await);
    capability.suppression.lock().await.jobs.clear();
    capability.suppression.lock().await.turns.insert(candidate.originating_turn.clone());
    assert!(!capability.permits(&candidate).await);
    capability.suppression.lock().await.turns.clear();
    let candidate = WakeCandidate { ready: false, ..candidate };
    assert!(!capability.permits(&candidate).await);
    capability.interruptions.fetch_add(1, Ordering::AcqRel);
    assert!(!capability.can_wake(ModeKind::Default, /*shutting_down*/ false));
    capability.interruptions.fetch_sub(1, Ordering::AcqRel);
    capability.stop.cancel();
    assert!(!capability.can_wake(ModeKind::Default, /*shutting_down*/ false));
    let disabled = BackgroundContinuation::new(/*enabled*/ false);
    assert!(!disabled.can_wake(ModeKind::Default, /*shutting_down*/ false));
}

#[tokio::test]
async fn user_started_execution_ends_the_background_suppression_epoch() {
    let (mut session, turn) = crate::session::tests::make_session_and_context().await;
    session.background_continuation = BackgroundContinuation::new(/*enabled*/ true);
    let candidate = WakeCandidate { job_id: Uuid::new_v4(), originating_turn: "interrupted".into(), ready: true };
    session.background_continuation.suppression.lock().await.jobs.insert(candidate.job_id);
    assert!(!session.background_continuation.permits(&candidate).await);
    session.begin_background_user_turn(&turn).await;
    assert!(!session.background_continuation.permits(&candidate).await, "internal turns do not reset suppression");
    turn.extension_data.insert(UserStartedTurn);
    session.begin_background_user_turn(&turn).await;
    assert!(session.background_continuation.permits(&candidate).await);
}
