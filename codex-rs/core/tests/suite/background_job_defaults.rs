//! Written, not run: real managed deadlines and requested dependency semantics.
use super::*;
use codex_core::CodexThread;
use codex_core::background_jobs::BackgroundJob;
use codex_core::background_jobs::BackgroundJobMode;
use codex_core::background_jobs::BackgroundJobState;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::InternalSessionSource;
use codex_protocol::protocol::SubAgentSource;
use pretty_assertions::assert_eq;
use std::sync::Arc;

async fn policy_harness(source: SessionSource, enabled: bool, stack: bool) -> Result<(TestCodexHarness, Arc<CodexThread>)> {
    let harness = TestCodexHarness::with_auto_env_builder(test_codex().with_model("gpt-5.5")
        .with_config(move |config| {
            config.features.enable(Feature::UnifiedExec).unwrap();
            config.features.enable(Feature::BackgroundCommandDelivery).unwrap();
            config.features.set_enabled(Feature::BackgroundJobs, enabled).unwrap();
            config.features.disable(Feature::BackgroundJobsIdleContinuation).unwrap();
            config.features.set_enabled(Feature::CommandStack, stack).unwrap();
            config.features.disable(Feature::CodeMode).unwrap();
            config.background_jobs = codex_config::BackgroundJobsConfigToml::default().resolve().unwrap();
            config.background_jobs.unattended_deadline = Duration::from_secs(30);
            config.background_jobs.foreground_timeout = Duration::from_secs(1);
            config.background_jobs.auto_yield = Duration::from_millis(25);
        })).await?;
    let test = harness.test();
    let thread = test.thread_manager.start_thread(StartThreadOptions {
        session_source: Some(source),
        environments: Some(test.codex.config_snapshot().await.environments.environments),
        ..StartThreadOptions::new(test.config.clone())
    }).await?.thread;
    Ok((harness, thread))
}

async fn policy_submit(harness: &TestCodexHarness, thread: &CodexThread) -> Result<()> {
    let test = harness.test();
    let (sandbox_policy, permission_profile) = turn_permission_fields(PermissionProfile::Disabled, test.config.cwd.as_path());
    thread.start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
        text: "run".into(), text_elements: vec![],
    }]).with_thread_settings(ThreadSettingsOverrides {
        approval_policy: Some(AskForApproval::Never), sandbox_policy: Some(sandbox_policy),
        permission_profile, ..Default::default()
    })).await?;
    Ok(())
}

async fn registered_job(thread: &CodexThread) -> Result<BackgroundJob> {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let mut jobs = thread.list_background_jobs().await?;
            if jobs.len() == 1 { return anyhow::Ok(jobs.remove(0)); }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await?
}

async fn advance_job_time() {
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    tokio::time::resume();
}

#[test_case::test_case(SessionSource::Exec, "background", None; "unattended_background_deadline")]
#[test_case::test_case(SessionSource::Exec, "auto", None; "unattended_auto_deadline_and_short_yield")]
#[test_case::test_case(SessionSource::VSCode, "background", None; "interactive_unlimited")]
#[test_case::test_case(SessionSource::Exec, "background", Some(60_000); "explicit_overrides_host_default")]
#[tokio::test]
async fn background_defaults_preserve_host_class_and_explicit_precedence(
    source: SessionSource, mode: &str, explicit: Option<u64>,
) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let unattended = source == SessionSource::Exec;
    let (harness, thread) = policy_harness(source, /*enabled*/ true, /*stack*/ false).await?;
    let mut args = json!({"cmd": gated_command(), "execution_mode": mode});
    if let Some(timeout) = explicit { args["timeout_ms"] = json!(timeout); }
    let log = mount_sse_sequence(harness.server(), vec![
        sse(vec![ev_function_call("launch", "exec_command", &args.to_string()), ev_completed("launch")]),
        sse(vec![ev_assistant_message("waiting", "waiting"), ev_completed("wait")]),
        sse(vec![ev_assistant_message("done", "finished"), ev_completed("done")]),
    ]).await;
    policy_submit(&harness, &thread).await?;
    tokio::time::timeout(Duration::from_secs(5), wait_for_event(&thread,
        |event| matches!(event, EventMsg::AgentMessage(message) if message.message == "waiting"))).await?;
    let before = registered_job(&thread).await?;
    let expected = explicit.or(unattended.then_some(30_000));
    assert_eq!(before.deadline_at_ms.map(|deadline| deadline - before.started_at_ms), expected.map(|ms| ms as i64));
    assert_eq!(before.timeout_ms, explicit.map(|ms| ms as i64), "host defaults are not explicit timeout labels");
    advance_job_time().await;
    let deadline_fires = unattended && explicit.is_none();
    if !deadline_fires {
        assert_eq!(registered_job(&thread).await?.state, BackgroundJobState::Running);
        harness.write_file("release", "go").await?;
    }
    wait_for_event(&thread, |event| matches!(event, EventMsg::TurnComplete(_))).await;
    let after = registered_job(&thread).await?;
    assert_eq!(after.state, if deadline_fires { BackgroundJobState::TimedOut } else { BackgroundJobState::Completed });
    let input = log.last_request().unwrap().message_input_texts("user").join("\n");
    assert!(input.contains("<command_completion>"));
    assert!(input.contains(&format!("\"deadline_fired\":{deadline_fires}")));
    assert_eq!(log.requests().len(), 3);
    Ok(())
}

#[test_case::test_case(true; "configured_foreground_deadline")]
#[test_case::test_case(false; "flag_off_keeps_300_seconds")]
#[tokio::test]
async fn background_foreground_default_is_gated(enabled: bool) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let (harness, thread) = policy_harness(SessionSource::Exec, enabled, /*stack*/ false).await?;
    let log = mount_sse_sequence(harness.server(), vec![
        sse(vec![ev_function_call("launch", "exec_command", &json!({
            "cmd": format!("echo started; {}", gated_command()), "execution_mode": "foreground",
        }).to_string()), ev_completed("launch")]),
        sse(vec![ev_assistant_message("done", "finished"), ev_completed("done")]),
    ]).await;
    policy_submit(&harness, &thread).await?;
    wait_for_event(&thread, |event| matches!(event, EventMsg::ExecCommandOutputDelta(delta)
        if String::from_utf8_lossy(&delta.chunk).contains("started"))).await;
    if enabled {
        let before = registered_job(&thread).await?;
        assert_eq!(before.deadline_at_ms.map(|deadline| deadline - before.started_at_ms), Some(1000));
        assert_eq!(before.timeout_ms, None);
    }
    advance_job_time().await;
    if !enabled {
        assert_eq!(thread.list_background_terminals().await.len(), 1);
        harness.write_file("release", "go").await?;
    }
    wait_for_event(&thread, |event| matches!(event, EventMsg::TurnComplete(_))).await;
    if enabled { assert_eq!(registered_job(&thread).await?.state, BackgroundJobState::TimedOut); }
    else { assert!(log.last_request().unwrap().function_call_output_text("launch").unwrap().contains("completed-marker")); }
    assert_eq!(log.requests().len(), 2);
    Ok(())
}

#[test_case::test_case("background", BackgroundJobMode::Background, 30_000)]
#[test_case::test_case("foreground", BackgroundJobMode::Foreground, 1000)]
#[tokio::test]
async fn background_stack_dependency_preserves_requested_mode_deadline(
    mode: &str, expected_mode: BackgroundJobMode, deadline_ms: i64,
) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let (harness, thread) = policy_harness(SessionSource::Exec, /*enabled*/ true, /*stack*/ true).await?;
    let stack = format!("exec_command\n{}\nend\nexec_command step=2\n{}\nend",
        json!({"cmd": gated_command(), "execution_mode": mode}), json!({"cmd": "echo should-not-run"}));
    let log = mount_sse_sequence(harness.server(), vec![
        sse(vec![ev_custom_tool_call("stack", "command_stack", &stack), ev_completed("launch")]),
        sse(vec![ev_assistant_message("done", "finished"), ev_completed("done")]),
    ]).await;
    policy_submit(&harness, &thread).await?;
    let before = registered_job(&thread).await?;
    assert_eq!((before.mode, before.deadline_at_ms.map(|deadline| deadline - before.started_at_ms)), (expected_mode, Some(deadline_ms)));
    advance_job_time().await;
    wait_for_event(&thread, |event| matches!(event, EventMsg::TurnComplete(_))).await;
    assert_eq!(registered_job(&thread).await?.state, BackgroundJobState::TimedOut);
    let output = log.last_request().unwrap().custom_tool_call_output("stack");
    let result: Value = serde_json::from_str(output["output"].as_str().unwrap())?;
    assert_eq!(result["stopped"], true);
    assert_eq!(result["results"].as_array().unwrap().len(), 1);
    Ok(())
}

#[test_case::test_case(false; "spawned_subagent")]
#[test_case::test_case(true; "internal_owner")]
#[tokio::test]
async fn background_child_inherits_unattended_owner(internal: bool) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let (harness, owner) = policy_harness(SessionSource::Exec, /*enabled*/ true, /*stack*/ false).await?;
    let test = harness.test();
    let options = StartThreadOptions {
        session_source: Some(if internal { SessionSource::Internal(InternalSessionSource::MemoryConsolidation) }
            else { SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: owner.startup_metadata().thread_id, depth: 1, agent_path: None, agent_nickname: None, agent_role: None,
            }) }),
        environments: Some(owner.config_snapshot().await.environments.environments),
        ..StartThreadOptions::new(test.config.clone())
    };
    let child = if internal { test.thread_manager.spawn_internal_session(owner.startup_metadata().thread_id, options).await? }
        else { test.thread_manager.start_thread(options).await? }.thread;
    mount_sse_sequence(harness.server(), vec![
        sse(vec![ev_function_call("launch", "exec_command", &json!({
            "cmd": gated_command(), "execution_mode": "background",
        }).to_string()), ev_completed("launch")]),
        sse(vec![ev_assistant_message("waiting", "waiting"), ev_completed("wait")]),
        sse(vec![ev_assistant_message("done", "finished"), ev_completed("done")]),
    ]).await;
    policy_submit(&harness, &child).await?;
    let before = registered_job(&child).await?;
    assert_eq!(before.deadline_at_ms.map(|deadline| deadline - before.started_at_ms), Some(30_000));
    harness.write_file("release", "go").await?;
    wait_for_event(&child, |event| matches!(event, EventMsg::TurnComplete(_))).await;
    Ok(())
}
