//! Exec continuation admission, exit, interruption and output.
use super::*;
use crate::event_processor::EventProcessor;
use crate::event_processor_with_jsonl_output::EventProcessorWithJsonOutput;
use codex_app_server_protocol::*;
use pretty_assertions::assert_eq;

fn turn(id: &str, message: Option<&str>) -> Turn {
    Turn {
        id: id.into(),
        root_turn_id: Some(id.into()),
        status: TurnStatus::Completed,
        error: None,
        items_view: TurnItemsView::Full,
        started_at: None,
        completed_at: None,
        duration_ms: None,
        items: message
            .map(|text| {
                vec![ThreadItem::untimed(ThreadItemPayload::AgentMessage {
                    id: id.into(),
                    text: text.into(),
                    phase: None,
                    memory_citation: None,
                    delivery: None,
                    questions: None,
                })]
            })
            .unwrap_or_default(),
    }
}
fn start(id: &str) -> ServerNotification {
    let mut turn = turn(id, None);
    turn.status = TurnStatus::InProgress;
    ServerNotification::TurnStarted(TurnStartedNotification {
        thread_id: "primary".into(),
        turn,
    })
}
fn state(id: &str, pending: bool, active: bool) -> CodexdcBackgroundWorkState {
    CodexdcBackgroundWorkState {
        background_jobs_enabled: true,
        job_timers: Vec::new(),
        thread_id: "primary".into(),
        pending,
        active,
        last_turn_id: Some(id.into()),
        idle_continuation: true,
        unattended_deadline_ms: 30_000,
    }
}
fn runner(enabled: bool) -> BackgroundWait {
    BackgroundWait::new(enabled, "first".into(), Duration::from_secs(30))
}

#[test]
fn flag_off_exits_after_first_turn_with_pending_work() {
    // Flag-off startup has no policy read and no idle deadline.
    let mut wait = BackgroundWait::new(false, "first".into(), Duration::ZERO);
    wait.complete(/*failed*/ false);
    assert!(wait.finished());
    assert!(wait.deadline.is_none());
    assert!(!wait.adopt(&start("next"), "primary"));
}

#[test]
fn enabled_run_without_jobs_exits_and_any_failed_turn_stays_failed() {
    let mut empty = runner(/*enabled*/ true);
    empty.complete(/*failed*/ false);
    empty.observe(&state(
        "first", /*pending*/ false, /*active*/ false,
    ));
    assert!(empty.finished() && !empty.failed);
    let mut wait = runner(/*enabled*/ true);
    wait.complete(/*failed*/ true);
    wait.observe(&state(
        "first", /*pending*/ true, /*active*/ false,
    ));
    assert!(!wait.finished() && wait.failed);
    assert!(wait.adopt(&start("next"), "primary"));
    wait.complete(/*failed*/ false);
    wait.observe(&state(
        "next", /*pending*/ false, /*active*/ false,
    ));
    assert!(wait.finished() && wait.failed);
}

#[test]
fn enabled_wait_adopts_continuation_and_exits_only_after_last_turn() {
    let mut wait = runner(/*enabled*/ true);
    wait.complete(/*failed*/ false);
    wait.observe(&state(
        "first", /*pending*/ true, /*active*/ false,
    ));
    assert!(!wait.finished());
    // A read can see a drained continuation before its queued start/completion arrive.
    wait.observe(&state(
        "next", /*pending*/ false, /*active*/ false,
    ));
    assert!(!wait.finished());
    wait.observe(&state(
        "first", /*pending*/ false, /*active*/ false,
    ));
    assert!(
        !wait.finished(),
        "stale notifications cannot undo the newer read"
    );
    assert!(wait.adopt(&start("next"), "primary"));
    wait.complete(/*failed*/ false);
    wait.observe(&state(
        "next", /*pending*/ false, /*active*/ false,
    ));
    assert!(wait.finished());
}

#[tokio::test]
async fn idle_wait_bound_expires_and_fails_run() {
    let mut wait = runner(/*enabled*/ true);
    wait.bound = Duration::from_millis(1);
    wait.complete(/*failed*/ false);
    tokio::time::timeout(Duration::from_secs(1), wait.wait_for_bound())
        .await
        .unwrap();
    wait.expire();
    assert!(wait.failed && wait.finished());
}

#[test]
fn control_c_skips_background_wait_in_both_states() {
    for idle in [false, true] {
        let mut wait = runner(/*enabled*/ true);
        if idle {
            wait.complete(/*failed*/ false);
        }
        wait.interrupt();
        assert_eq!(wait.finished(), idle);
        if !idle {
            wait.complete(/*failed*/ true);
        }
        assert!(wait.failed && wait.finished());
    }
}

#[test]
fn output_uses_last_tracked_turn_and_processors_do_not_request_shutdown() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("last.txt");
    let mut output = EventProcessorWithJsonOutput::new(Some(path.clone()));
    output.set_background_continuations(/*enabled*/ true);
    for (id, message) in [
        ("first", Some("waiting")),
        ("next", Some("final")),
        ("empty", None),
    ] {
        output.collect_thread_events(start(id));
        let collected = output.collect_thread_events(ServerNotification::TurnCompleted(
            TurnCompletedNotification {
                thread_id: "primary".into(),
                turn: turn(id, message),
            },
        ));
        assert_eq!(
            collected.status,
            crate::event_processor::CodexStatus::Running
        );
        output.print_final_output();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            message.unwrap_or_default()
        );
    }
}
