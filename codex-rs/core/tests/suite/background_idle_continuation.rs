//! Written, not run: idle batches, user races, Plan, interruption and flag-off parking.
use super::*;
use codex_extension_api::ExtensionData;
use codex_extension_api::ExtensionFuture;
use codex_extension_api::ExtensionRegistryBuilder;
use codex_extension_api::TurnLifecycleContributor;
use codex_extension_api::TurnStartInput;
use codex_extension_api::TurnStartPhase;
use core_test_support::responses::mount_response_sequence;
use core_test_support::responses::sse_response;
use pretty_assertions::assert_eq;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use tokio::sync::Notify;

#[derive(Default)]
struct PreparationGate {
    armed: AtomicBool,
    entered: Notify,
    release: Notify,
}
impl TurnLifecycleContributor for PreparationGate {
    fn turn_start_phase(&self, _store: &ExtensionData) -> TurnStartPhase {
        TurnStartPhase::RegularTaskStart
    }
    fn on_turn_start<'a>(&'a self, _input: TurnStartInput<'a>) -> ExtensionFuture<'a, ()> {
        Box::pin(async move {
            if self.armed.swap(/*val*/ false, Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
        })
    }
}

async fn idle_harness(enabled: bool, gate: Arc<PreparationGate>) -> Result<TestCodexHarness> {
    let mut extensions = ExtensionRegistryBuilder::new();
    extensions.turn_lifecycle_contributor(gate);
    TestCodexHarness::with_auto_env_builder(test_codex().with_model("gpt-5.5")
        .with_extensions(Arc::new(extensions.build())).with_config(move |config| {
            config.features.enable(Feature::UnifiedExec).unwrap();
            config.features.enable(Feature::BackgroundCommandDelivery).unwrap();
            config.features.enable(Feature::BackgroundJobs).unwrap();
            config.features.set_enabled(Feature::BackgroundJobsIdleContinuation, enabled).unwrap();
            config.features.disable(Feature::CommandStack).unwrap();
            config.features.disable(Feature::CodeMode).unwrap();
        })).await
}

fn launch_response(count: usize) -> String {
    let mut calls = (0..count).map(|id| ev_function_call(&format!("job-{id}"), "exec_command",
        &json!({"cmd": gated_command().replace("release", &format!("release-{id}")), "execution_mode": "background"}).to_string()
    )).collect::<Vec<_>>();
    calls.push(ev_completed("launch"));
    sse(calls)
}
fn summary_response() -> String {
    sse(vec![ev_assistant_message("wait", "waiting"), ev_completed("summary")])
}
fn done_response() -> String {
    sse(vec![ev_assistant_message("done", "finished"), ev_completed("done")])
}
async fn completed_jobs(harness: &TestCodexHarness, count: usize) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let jobs = harness.test().codex.list_background_jobs().await?;
            if jobs.len() == count && jobs.iter().all(|job| job.completed_at_ms.is_some()) {
                return anyhow::Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await?
}
async fn launch(harness: &TestCodexHarness, mode: ModeKind) -> Result<()> {
    let test = harness.test();
    let (sandbox_policy, permission_profile) = turn_permission_fields(PermissionProfile::Disabled, test.config.cwd.as_path());
    test.codex.start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
        text: "launch".into(), text_elements: vec![],
    }]).with_thread_settings(ThreadSettingsOverrides {
        approval_policy: Some(AskForApproval::Never), sandbox_policy: Some(sandbox_policy), permission_profile,
        collaboration_mode: Some(CollaborationMode { mode, settings: Settings {
            model: test.session_configured.model.clone(), reasoning_effort: None, developer_instructions: None,
        }}), ..Default::default()
    })).await?;
    Ok(())
}
async fn finished(harness: &TestCodexHarness) {
    wait_for_event(&harness.test().codex, |event| matches!(event, EventMsg::TurnComplete(_))).await;
}

#[test_case::test_case(false; "two_jobs_one_wake_window")]
#[test_case::test_case(true; "user_input_races_admitted_wake")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_idle_continuation_batches_and_yields_to_user(race_user: bool) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let gate = Arc::new(PreparationGate::default());
    let harness = idle_harness(/*enabled*/ true, Arc::clone(&gate)).await?;
    let log = mount_sse_sequence(harness.server(), vec![launch_response(2), summary_response(), done_response()]).await;
    launch(&harness, ModeKind::Default).await?;
    finished(&harness).await;
    assert_eq!(log.requests().len(), 2, "turn ends while both jobs are still running");
    gate.armed.store(/*val*/ true, Ordering::SeqCst);
    harness.write_file("release-0", "go").await?;
    tokio::time::timeout(Duration::from_secs(20), gate.entered.notified()).await?;
    harness.write_file("release-1", "go").await?;
    completed_jobs(&harness, 2).await?;
    if race_user {
        submit_unified_exec_turn(harness.test(), "user-race-marker", PermissionProfile::Disabled).await?;
    }
    gate.release.notify_one();
    finished(&harness).await;
    assert_eq!(log.requests().len(), 3, "one continuation sample contains the batch");
    let input = log.last_request().unwrap().body_json()["input"].to_string();
    assert_eq!(input.matches("<command_completion>").count(), 2);
    assert!(input.contains("background_job_completion"));
    if race_user {
        assert!(input.contains("user-race-marker"));
    }
    let tools = log.last_request().unwrap().body_json()["tools"].to_string();
    assert!(!tools.contains("The runtime keeps the turn open"));
    assert!(tools.contains("completion starts a follow-up turn"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_idle_continuation_plan_waits_for_user_without_rejected_start_loop() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = idle_harness(/*enabled*/ true, Arc::default()).await?;
    let log = mount_sse_sequence(harness.server(), vec![launch_response(1), summary_response(), done_response()]).await;
    launch(&harness, ModeKind::Plan).await?;
    finished(&harness).await;
    harness.write_file("release-0", "go").await?;
    completed_jobs(&harness, 1).await?;
    assert!(tokio::time::timeout(Duration::from_millis(400), wait_for_event(&harness.test().codex,
        |event| matches!(event, EventMsg::TurnStarted(_)))).await.is_err());
    assert_eq!(log.requests().len(), 2);
    launch(&harness, ModeKind::Plan).await?;
    finished(&harness).await;
    assert_eq!(log.requests().len(), 3);
    assert_eq!(log.last_request().unwrap().body_json()["input"].to_string().matches("<command_completion>").count(), 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_idle_continuation_interrupt_suppresses_until_next_user_turn() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = idle_harness(/*enabled*/ true, Arc::default()).await?;
    let log = mount_response_sequence(harness.server(), vec![sse_response(launch_response(1)),
        sse_response(summary_response()).set_delay(Duration::from_secs(60)), sse_response(done_response())]).await;
    launch(&harness, ModeKind::Default).await?;
    tokio::time::timeout(Duration::from_secs(20), async {
        while log.requests().len() < 2 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await?;
    harness.test().codex.submit(Op::Interrupt).await?;
    wait_for_event(&harness.test().codex, |event| matches!(event, EventMsg::TurnAborted(_))).await;
    harness.write_file("release-0", "go").await?;
    completed_jobs(&harness, 1).await?;
    assert!(tokio::time::timeout(Duration::from_millis(400), wait_for_event(&harness.test().codex,
        |event| matches!(event, EventMsg::TurnStarted(_)))).await.is_err());
    assert_eq!(log.requests().len(), 2);
    submit_unified_exec_turn(harness.test(), "user-resumes", PermissionProfile::Disabled).await?;
    finished(&harness).await;
    assert_eq!(log.requests().len(), 3);
    assert_eq!(log.last_request().unwrap().body_json()["input"].to_string().matches("<command_completion>").count(), 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_idle_continuation_off_retains_parked_turn_and_instruction_text() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = idle_harness(/*enabled*/ false, Arc::default()).await?;
    let log = mount_sse_sequence(harness.server(), vec![launch_response(1), summary_response(), done_response()]).await;
    launch(&harness, ModeKind::Default).await?;
    wait_for_event(&harness.test().codex, |event| matches!(event, EventMsg::AgentMessage(message) if message.message == "waiting")).await;
    assert!(tokio::time::timeout(Duration::from_millis(400), wait_for_event(&harness.test().codex,
        |event| matches!(event, EventMsg::TurnComplete(_)))).await.is_err());
    assert_eq!(log.requests().len(), 2);
    assert!(log.last_request().unwrap().body_json()["tools"].to_string().contains("The runtime keeps the turn open"));
    harness.write_file("release-0", "go").await?;
    finished(&harness).await;
    assert_eq!(log.requests().len(), 3);
    Ok(())
}
