//! Interrupt preserves session jobs; explicit stop/shutdown revoke delivery.
//! All cases in this module are written, not run under the production-machine rule.
use super::*;
use codex_core::background_jobs::BackgroundJobDelivery;
use codex_core::background_jobs::BackgroundJobState;
use pretty_assertions::assert_eq;

async fn interrupt_harness(enabled: bool) -> Result<TestCodexHarness> {
    TestCodexHarness::with_auto_env_builder(test_codex().with_model("gpt-5.5").with_config(
        move |config| {
            config
                .features
                .enable(Feature::BackgroundCommandDelivery)
                .unwrap();
            config
                .features
                .set_enabled(Feature::BackgroundJobs, enabled)
                .unwrap();
            config.features.disable(Feature::CommandStack).unwrap();
            config.features.disable(Feature::CodeMode).unwrap();
        },
    ))
    .await
}

async fn interrupt_parked_job(harness: &TestCodexHarness) -> Result<()> {
    let test = harness.test();
    submit_unified_exec_turn(test, "launch", PermissionProfile::Disabled).await?;
    wait_for_event(
        &test.codex,
        |event| matches!(event, EventMsg::AgentMessage(message) if message.message == "parked"),
    )
    .await;
    test.codex.submit(Op::Interrupt).await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnAborted(_))
    })
    .await;
    Ok(())
}

async fn terminal_jobs(
    harness: &TestCodexHarness,
) -> Result<Vec<codex_core::background_jobs::BackgroundJob>> {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let jobs = harness.test().codex.list_background_jobs().await?;
            if jobs.len() == 1 && jobs[0].state != BackgroundJobState::Running {
                return Ok(jobs);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?
}

fn launch_response() -> String {
    sse(vec![
        ev_function_call(
            "launch",
            "exec_command",
            &json!({
                "cmd": gated_command(), "execution_mode": "background",
            })
            .to_string(),
        ),
        ev_completed("launch"),
    ])
}

fn parked_response() -> String {
    sse(vec![
        ev_assistant_message("wait", "parked"),
        ev_completed("park"),
    ])
}

#[test_case::test_case(true; "jobs_before_next_user_sample")]
#[test_case::test_case(false; "flag_off_post_sampling_parity")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_jobs_interrupt_keeps_completion_without_starting_a_turn(
    enabled: bool,
) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = interrupt_harness(enabled).await?;
    let mut responses = vec![
        launch_response(),
        parked_response(),
        sse(vec![
            ev_assistant_message("next", "next-user"),
            ev_completed("next"),
        ]),
    ];
    if !enabled {
        responses.push(sse(vec![
            ev_assistant_message("result", "result-seen"),
            ev_completed("result"),
        ]));
    }
    responses.push(sse(vec![
        ev_assistant_message("later", "later-user"),
        ev_completed("later"),
    ]));
    let log = mount_sse_sequence(harness.server(), responses).await;
    interrupt_parked_job(&harness).await?;
    let running = harness.test().codex.list_background_jobs().await?;
    assert_eq!(running.len(), 1);
    assert_eq!(
        (&running[0].state, &running[0].delivery),
        (
            &BackgroundJobState::Running,
            &BackgroundJobDelivery::Pending
        )
    );
    harness.write_file("release", "go").await?;
    let finished = terminal_jobs(&harness).await?;
    assert_eq!(
        (&finished[0].state, &finished[0].delivery),
        (
            &BackgroundJobState::Completed,
            &BackgroundJobDelivery::Pending
        )
    );
    assert_eq!(running[0].job_id, finished[0].job_id);
    // No automatic wake exists: wait for a late completion event, not a turn.
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::ExecCommandEnd(_))
    })
    .await;
    assert!(
        tokio::time::timeout(
            Duration::from_millis(400),
            wait_for_event(&harness.test().codex, |event| matches!(
                event,
                EventMsg::TurnStarted(_)
            ))
        )
        .await
        .is_err(),
        "completion must wait for user input"
    );
    assert_eq!(log.requests().len(), 2);
    submit_unified_exec_turn(harness.test(), "continue", PermissionProfile::Disabled).await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    let requests = log.requests();
    let first = requests[2].body_json()["input"].to_string();
    assert_eq!(
        first.matches("<command_completion>").count(),
        usize::from(enabled)
    );
    let last = requests.last().unwrap().body_json()["input"].to_string();
    assert_eq!(last.matches("<command_completion>").count(), 1);
    assert!(last.contains("completed-marker"));
    if enabled {
        assert_eq!(last.matches(&finished[0].job_id).count(), 1);
    }
    assert_eq!(
        harness.test().codex.list_background_jobs().await?[0].delivery,
        BackgroundJobDelivery::Delivered
    );
    submit_unified_exec_turn(harness.test(), "again", PermissionProfile::Disabled).await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    assert_eq!(
        log.last_request().unwrap().body_json()["input"]
            .to_string()
            .matches("<command_completion>")
            .count(),
        1
    );
    Ok(())
}

#[derive(Clone, Copy)]
enum StopPath {
    TerminalApi,
    StdinControl,
}

#[test_case::test_case(StopPath::TerminalApi, true; "terminal_stop_jobs")]
#[test_case::test_case(StopPath::TerminalApi, false; "terminal_stop_flag_off")]
#[test_case::test_case(StopPath::StdinControl, true; "stdin_stop_jobs")]
#[test_case::test_case(StopPath::StdinControl, false; "stdin_stop_flag_off")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_jobs_explicit_stop_revokes_without_continuation(
    path: StopPath,
    enabled: bool,
) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = interrupt_harness(enabled).await?;
    let launch =
        mount_sse_sequence(harness.server(), vec![launch_response(), parked_response()]).await;
    interrupt_parked_job(&harness).await?;
    let running = harness.test().codex.list_background_jobs().await?;
    assert_eq!(running.len(), 1);
    let process_id: i32 = running[0].process_id.parse()?;
    let final_response = sse(vec![
        ev_assistant_message("done", "stopped"),
        ev_completed("stop"),
    ]);
    let responses = match path {
        StopPath::TerminalApi => {
            assert!(
                harness
                    .test()
                    .codex
                    .terminate_background_terminal(process_id)
                    .await
            );
            let jobs = terminal_jobs(&harness).await?;
            assert_eq!(jobs[0].delivery, BackgroundJobDelivery::None);
            assert_eq!(launch.requests().len(), 2, "stop does not start inference");
            vec![final_response]
        }
        StopPath::StdinControl => vec![
            sse(vec![
                ev_function_call(
                    "stop",
                    "write_stdin",
                    &json!({
                        "session_id": process_id, "chars": "\u{3}", "yield_time_ms": 1000,
                    })
                    .to_string(),
                ),
                ev_completed("control"),
            ]),
            final_response,
        ],
    };
    let log = mount_sse_sequence(harness.server(), responses).await;
    submit_unified_exec_turn(harness.test(), "stop", PermissionProfile::Disabled).await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    let jobs = terminal_jobs(&harness).await?;
    let expected = match (path, enabled) {
        (StopPath::StdinControl, false) => BackgroundJobDelivery::Delivered,
        (StopPath::TerminalApi, _) | (StopPath::StdinControl, true) => BackgroundJobDelivery::None,
    };
    assert_eq!(jobs[0].delivery, expected);
    assert_eq!(jobs[0].job_id, running[0].job_id);
    assert!(
        harness
            .test()
            .codex
            .list_background_terminals()
            .await
            .is_empty()
    );
    assert_eq!(
        log.requests().len(),
        match path {
            StopPath::TerminalApi => 1,
            StopPath::StdinControl => 2,
        }
    );
    assert!(
        !log.last_request().unwrap().body_json()["input"]
            .to_string()
            .contains("<command_completion>")
    );
    Ok(())
}

#[test_case::test_case(true; "jobs_shutdown_revokes")]
#[test_case::test_case(false; "flag_off_shutdown_parity")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_jobs_shutdown_terminates_and_revokes(enabled: bool) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = interrupt_harness(enabled).await?;
    let log =
        mount_sse_sequence(harness.server(), vec![launch_response(), parked_response()]).await;
    interrupt_parked_job(&harness).await?;
    harness.test().codex.shutdown_and_wait().await?;
    let jobs = terminal_jobs(&harness).await?;
    assert_eq!(jobs[0].delivery, BackgroundJobDelivery::None);
    assert!(
        harness
            .test()
            .codex
            .list_background_terminals()
            .await
            .is_empty()
    );
    assert_eq!(log.requests().len(), 2);
    Ok(())
}

#[test_case::test_case(true; "jobs_stop_queued_result")]
#[test_case::test_case(false; "flag_off_stop_queued_result")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_jobs_stop_revokes_an_already_queued_completion(enabled: bool) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = interrupt_harness(enabled).await?;
    let log = mount_sse_sequence(
        harness.server(),
        vec![
            launch_response(),
            parked_response(),
            sse(vec![
                ev_assistant_message("next", "no-result"),
                ev_completed("next"),
            ]),
        ],
    )
    .await;
    interrupt_parked_job(&harness).await?;
    harness.write_file("release", "go").await?;
    let finished = terminal_jobs(&harness).await?;
    assert_eq!(finished[0].delivery, BackgroundJobDelivery::Pending);
    let process_id = finished[0].process_id.parse()?;
    // The process can already be retired from the terminal store. The existing
    // stop API still revokes its retained managed record before terminal lookup.
    let _terminal_was_present = harness
        .test()
        .codex
        .terminate_background_terminal(process_id)
        .await;
    assert_eq!(
        harness.test().codex.list_background_jobs().await?[0].delivery,
        BackgroundJobDelivery::None
    );
    assert_eq!(log.requests().len(), 2);
    submit_unified_exec_turn(harness.test(), "continue", PermissionProfile::Disabled).await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    assert_eq!(log.requests().len(), 3);
    assert!(
        !log.last_request().unwrap().body_json()["input"]
            .to_string()
            .contains("<command_completion>")
    );
    Ok(())
}
