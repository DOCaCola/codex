use super::*;
use crate::context::CommandCompletion;
use crate::context::ContextualUserFragment;

#[test]
fn managed_fragment_bounds_escaped_metadata_and_multibyte_output() {
    let hostile = "\u{1}\n🦀".repeat(10_000);
    let job = JobMetadata {
        job_id: Uuid::new_v4(),
        originating_turn: hostile.clone(),
        call_id: format!("{hostile}:1:0"),
        stack: Some(crate::tools::context::CommandStackIdentity {
            parent_call_id: hostile.clone(),
            step: 1,
            index: 0,
        }),
        item_id: hostile.clone(),
        command: hostile.clone(),
        mode: ExecutionMode::Background,
        started_at: Utc::now(),
        explicit_timeout_ms: None,
        deadline_source: None, deadline_origin: None,
        effective_deadline: None,
        completed_at: Some(Utc::now()),
        exit_code: Some(-1),
        terminal_state: Some(TerminalState::Failed),
        duration_ms: u128::MAX,
        deadline_fired: false,
        stopped: false,
        clock_started_at: Instant::now(),
    };
    let body = CommandCompletion::failed(i32::MIN, &hostile)
        .with_metadata(&job)
        .body();
    assert!(body.len() + 64 < 1600, "fragment bytes: {}", body.len());
}

#[test]
fn background_jobs_snapshot_maps_state_delivery_and_deadline() {
    use crate::background_jobs::*;
    let start = Instant::now();
    let mut job = JobMetadata {
        job_id: Uuid::new_v4(),
        originating_turn: "turn".into(),
        call_id: "call".into(),
        stack: None,
        item_id: "item".into(),
        command: "cmd".into(),
        mode: ExecutionMode::Background,
        started_at: DateTime::from_timestamp_millis(1_000_000).unwrap(),
        explicit_timeout_ms: Some(30_000),
        deadline_source: None, deadline_origin: None,
        effective_deadline: Some(start + Duration::from_secs(30)),
        completed_at: None,
        exit_code: None,
        terminal_state: None,
        duration_ms: 0,
        deadline_fired: false,
        stopped: false,
        clock_started_at: start,
    };
    for (mode, terminal, facts, expected) in [
        (
            ExecutionMode::Background,
            None,
            DeliveryFacts::default(),
            (BackgroundJobState::Running, BackgroundJobDelivery::Pending),
        ),
        (
            ExecutionMode::Auto,
            Some(TerminalState::Completed),
            DeliveryFacts::default(),
            (
                BackgroundJobState::Completed,
                BackgroundJobDelivery::Pending,
            ),
        ),
        (
            ExecutionMode::Background,
            Some(TerminalState::Failed),
            DeliveryFacts {
                acknowledged: true,
                ..Default::default()
            },
            (BackgroundJobState::Failed, BackgroundJobDelivery::Delivered),
        ),
        (
            ExecutionMode::Background,
            Some(TerminalState::TimedOut),
            DeliveryFacts::default(),
            (BackgroundJobState::TimedOut, BackgroundJobDelivery::Pending),
        ),
        (
            ExecutionMode::Background,
            Some(TerminalState::Stopped),
            DeliveryFacts {
                revoked: true,
                ..Default::default()
            },
            (BackgroundJobState::Stopped, BackgroundJobDelivery::None),
        ),
        (
            ExecutionMode::Foreground,
            Some(TerminalState::Completed),
            DeliveryFacts {
                initial_response_consumed: true,
                ..Default::default()
            },
            (
                BackgroundJobState::Completed,
                BackgroundJobDelivery::Delivered,
            ),
        ),
        (
            ExecutionMode::Interactive,
            None,
            DeliveryFacts::default(),
            (BackgroundJobState::Running, BackgroundJobDelivery::None),
        ),
        (
            ExecutionMode::Background,
            Some(TerminalState::Completed),
            DeliveryFacts {
                revoked: true,
                acknowledged: true,
                ..Default::default()
            },
            (
                BackgroundJobState::Completed,
                BackgroundJobDelivery::Delivered,
            ),
        ),
    ] {
        job.mode = mode;
        job.terminal_state = terminal;
        let snapshot = BackgroundJob::from_metadata(&job, 42, facts).unwrap();
        assert_eq!(
            (
                snapshot.state,
                snapshot.delivery,
                snapshot.deadline_at_ms,
                snapshot.timeout_ms
            ),
            (expected.0, expected.1, Some(1_030_000), Some(30_000))
        );
    }
    job.mode = ExecutionMode::Foreground;
    job.explicit_timeout_ms = None;
    job.effective_deadline = Some(start + Duration::from_secs(300));
    let snapshot = BackgroundJob::from_metadata(&job, 42, DeliveryFacts::default()).unwrap();
    assert_eq!(
        (snapshot.timeout_ms, snapshot.deadline_at_ms),
        (None, Some(1_300_000))
    );
}
