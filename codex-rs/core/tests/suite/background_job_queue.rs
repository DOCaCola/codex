//! Queue delivery races through real turns; lifecycle remains turn parking.
use super::*;
use codex_core::background_jobs::BackgroundJobDelivery;
use codex_core::background_jobs::BackgroundJobState;
use codex_extension_api::ExtensionData;
use codex_extension_api::ExtensionFuture;
use codex_extension_api::ExtensionRegistryBuilder;
use codex_extension_api::TurnLifecycleContributor;
use codex_extension_api::TurnStartInput;
use codex_extension_api::TurnStartPhase;
use pretty_assertions::assert_eq;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use tokio::sync::Notify;

#[derive(Default)]
struct PauseTurnSetup {
    armed: AtomicBool,
    entered: Notify,
    resume: Notify,
}

impl TurnLifecycleContributor for PauseTurnSetup {
    fn turn_start_phase(&self, _store: &ExtensionData) -> TurnStartPhase {
        TurnStartPhase::RegularTaskStart
    }

    fn on_turn_start<'a>(&'a self, _input: TurnStartInput<'a>) -> ExtensionFuture<'a, ()> {
        Box::pin(async move {
            if self.armed.swap(/*val*/ false, Ordering::SeqCst) {
                self.entered.notify_one();
                self.resume.notified().await;
            }
        })
    }
}

#[test_case::test_case(1, true; "during_turn_setup_before_first_sample")]
#[test_case::test_case(2, true; "two_completions_share_boundary")]
#[test_case::test_case(6, true; "four_then_two_without_loss")]
#[test_case::test_case(1, false; "flag_off_retains_explicit_completion")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_job_queue_drains_at_sampling_boundaries(
    count: usize,
    enabled: bool,
) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let pause = Arc::new(PauseTurnSetup::default());
    let mut extensions = ExtensionRegistryBuilder::new();
    extensions.turn_lifecycle_contributor(pause.clone());
    let harness = TestCodexHarness::with_auto_env_builder(
        test_codex()
            .with_model("gpt-5.5")
            .with_extensions(Arc::new(extensions.build()))
            .with_config(move |config| {
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
            }),
    )
    .await?;
    let mut launch = (0..count)
        .map(|index| {
            ev_function_call(
                &format!("job-{index}"),
                "exec_command",
                &json!({"cmd": gated_command(), "execution_mode": "background"}).to_string(),
            )
        })
        .collect::<Vec<_>>();
    launch.push(ev_completed("launch"));
    let launch_log = mount_sse_sequence(
        harness.server(),
        vec![
            sse(launch),
            sse(vec![
                ev_assistant_message("wait", "parked"),
                ev_completed("park"),
            ]),
        ],
    )
    .await;
    let test = harness.test();
    submit_unified_exec_turn(test, "launch", PermissionProfile::Disabled).await?;
    wait_for_event(
        &test.codex,
        |event| matches!(event, EventMsg::AgentMessage(message) if message.message == "parked"),
    )
    .await;
    assert_eq!(launch_log.requests().len(), 2);
    test.codex.submit(Op::Interrupt).await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnAborted(_))
    })
    .await;
    pause.armed.store(/*val*/ true, Ordering::SeqCst);
    submit_unified_exec_turn(test, "continue", PermissionProfile::Disabled).await?;
    tokio::time::timeout(Duration::from_secs(10), pause.entered.notified()).await?;
    harness.write_file("release", "go").await?;
    let completed = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let jobs = test.codex.list_background_jobs().await.unwrap();
            if jobs.len() == count
                && jobs
                    .iter()
                    .all(|job| job.state == BackgroundJobState::Completed)
            {
                break jobs;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await?;
    assert!(
        completed
            .iter()
            .all(|job| job.delivery == BackgroundJobDelivery::Pending)
    );
    assert_eq!(
        launch_log.requests().len(),
        2,
        "setup is paused while completions queue"
    );
    let mut responses = Vec::new();
    if !enabled {
        responses.push(sse(vec![
            ev_function_call(
                "read-retained",
                "write_stdin",
                &json!({
                    "session_id": completed[0].process_id.parse::<i32>()?,
                    "chars": "", "wait_mode": "completion",
                })
                .to_string(),
            ),
            ev_completed("read-retained"),
        ]));
    }
    responses.push(sse(vec![
        ev_assistant_message("new", "first-sample"),
        ev_completed("new"),
    ]));
    if enabled && count > 4 {
        responses.push(sse(vec![
            ev_assistant_message("rest", "next-boundary"),
            ev_completed("rest"),
        ]));
    }
    responses.push(sse(vec![
        ev_assistant_message("later", "later-turn"),
        ev_completed("later"),
    ]));
    let log = mount_sse_sequence(harness.server(), responses).await;
    pause.resume.notify_one();
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    let requests = log.requests();
    let fragments = |index: usize| {
        requests[index].body_json()["input"]
            .to_string()
            .matches("<command_completion>")
            .count()
    };
    assert_eq!(fragments(0), if enabled { count.min(4) } else { 0 });
    assert_eq!(requests.len(), if enabled && count <= 4 { 1 } else { 2 });
    assert_eq!(
        fragments(requests.len() - 1),
        if enabled { count } else { 0 }
    );
    if !enabled {
        assert!(
            requests
                .last()
                .unwrap()
                .function_call_output("read-retained")
                .to_string()
                .contains("completed-marker")
        );
    }
    if enabled {
        let final_input = requests.last().unwrap().body_json()["input"].to_string();
        for job in &completed {
            assert_eq!(final_input.matches(&job.job_id).count(), 1);
        }
    }
    assert!(
        test.codex
            .list_background_jobs()
            .await?
            .iter()
            .all(|job| job.delivery == BackgroundJobDelivery::Delivered)
    );
    submit_unified_exec_turn(test, "another turn", PermissionProfile::Disabled).await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    assert_eq!(log.requests().len(), requests.len() + 1);
    assert_eq!(
        log.last_request().unwrap().body_json()["input"]
            .to_string()
            .matches("<command_completion>")
            .count(),
        if enabled { count } else { 0 }
    );
    Ok(())
}
