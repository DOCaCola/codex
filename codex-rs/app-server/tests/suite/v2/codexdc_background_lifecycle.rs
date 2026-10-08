//! Automatic no-subscriber unload respects opted-in session work.
//! All tests are written, not run under the production-machine prohibition.
use anyhow::Result;
use app_test_support::MockResponsesConfig;
use app_test_support::TestAppServer;
use app_test_support::to_response;
use codex_app_server_protocol::*;
use codex_features::Feature;
use core_test_support::TestTargetOs;
use core_test_support::responses::*;
use core_test_support::test_target_os;
use pretty_assertions::assert_eq;
use serde_json::json;
use tempfile::TempDir;
use tokio::time::{Duration, timeout};
use wiremock::MockServer;

const UNLOAD_DELAY: u64 = 1;
const READ_TIMEOUT: Duration = Duration::from_secs(20);

async fn interrupted_job(model: &MockServer, enabled: bool) -> Result<(TempDir, TestAppServer, String)> {
    let home = TempDir::new()?;
    let config = MockResponsesConfig::new(&model.uri())
        .with_model("gpt-5.5")
        .with_sandbox_mode("danger-full-access")
        .with_root_config(&format!("thread_unload_delay_secs = {UNLOAD_DELAY}"))
        .enable_feature(Feature::BackgroundCommandDelivery)
        .disable_feature(Feature::CodeMode)
        .disable_feature(Feature::CommandStack);
    let config = if enabled {
        config.enable_feature(Feature::BackgroundJobs)
    } else {
        config.disable_feature(Feature::BackgroundJobs)
    };
    config.write(home.path())?;
    let mut server = TestAppServer::builder().with_codex_home(home.path())
        .without_managed_config().build_initialized().await?;
    let thread_id = server.start_thread(ThreadStartParams::default()).await?.thread.id;
    let started: TurnStartResponse = server.request(|request_id| ClientRequest::TurnStart {
        request_id,
        params: TurnStartParams {
            thread_id: thread_id.clone(),
            input: vec![UserInput::Text { text: "launch".into(), text_elements: vec![] }],
            ..Default::default()
        },
    }).await?;
    timeout(READ_TIMEOUT, server.read_stream_until_matching_notification("parked", |n| {
        n.method == "item/completed" && n.params.as_ref()
            .is_some_and(|p| p["item"]["type"] == "agentMessage")
    })).await??;
    let _: TurnInterruptResponse = server.request(|request_id| ClientRequest::TurnInterrupt {
        request_id,
        params: TurnInterruptParams { thread_id: thread_id.clone(), turn_id: started.turn.id },
    }).await?;
    let ended: TurnCompletedNotification = timeout(READ_TIMEOUT, server.read_notification("turn/completed")).await??;
    assert_eq!(ended.turn.status, TurnStatus::Interrupted);
    Ok((home, server, thread_id))
}

async fn responses(model: &MockServer, follow_up: bool) -> ResponseMock {
    let command = match test_target_os() {
        TestTargetOs::Windows => "while (!(Test-Path release)) { Start-Sleep -Milliseconds 50 }; Set-Content finished done; Write-Output completed-marker",
        TestTargetOs::Linux | TestTargetOs::MacOs => "while [ ! -e release ]; do sleep 0.05; done; echo done > finished; echo completed-marker",
    };
    let mut sequence = vec![
        sse(vec![ev_function_call("launch", "exec_command", &json!({
            "cmd": command, "execution_mode": "background",
        }).to_string()), ev_completed("launch")]),
        sse(vec![ev_assistant_message("wait", "parked"), ev_completed("park")]),
    ];
    if follow_up {
        sequence.push(sse(vec![ev_assistant_message("done", "delivered"), ev_completed("done")]));
    }
    mount_sse_sequence(model, sequence).await
}

async fn jobs(server: &mut TestAppServer, thread_id: &str) -> Result<CodexdcBackgroundJobsListResponse> {
    let id = server.send_request("codexdc/backgroundJobs/list", Some(json!({"threadId": thread_id}))).await?;
    to_response(timeout(READ_TIMEOUT, server.read_stream_until_response_message(RequestId::Integer(id))).await??)
}

async fn release(server: &TestAppServer) -> Result<()> {
    let env = server.auto_env()?;
    env.environment().get_filesystem().write_file(&env.selection().cwd.join("release")?,
        b"go".to_vec(), Default::default(), /*sandbox*/ None).await?;
    Ok(())
}

async fn queued(server: &mut TestAppServer, thread_id: &str) -> Result<CodexdcBackgroundJobsListResponse> {
    timeout(READ_TIMEOUT, async {
        loop {
            let result = jobs(server, thread_id).await?;
            if result.data.len() == 1 && result.data[0].state == BackgroundJobState::Completed {
                return anyhow::Ok(result);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await?
}

async fn unsubscribe(server: &mut TestAppServer, thread_id: &str) -> Result<()> {
    let result: ThreadUnsubscribeResponse = server.request(|request_id| ClientRequest::ThreadUnsubscribe {
        request_id, params: ThreadUnsubscribeParams { thread_id: thread_id.into() },
    }).await?;
    assert_eq!(result.status, ThreadUnsubscribeStatus::Unsubscribed);
    Ok(())
}

async fn stays_loaded(server: &mut TestAppServer, thread_id: &str) -> Result<()> {
    assert!(timeout(Duration::from_millis(1300),
        server.read_stream_until_notification_message("thread/closed")).await.is_err());
    let loaded: ThreadLoadedListResponse = server.request(|request_id| ClientRequest::ThreadLoadedList {
        request_id, params: ThreadLoadedListParams::default(),
    }).await?;
    assert_eq!(loaded.data, vec![thread_id.to_string()]);
    Ok(())
}

async fn closed(server: &mut TestAppServer, thread_id: &str) -> Result<()> {
    let notification: ThreadClosedNotification = timeout(READ_TIMEOUT, server.read_notification("thread/closed")).await??;
    assert_eq!(notification.thread_id, thread_id);
    let loaded: ThreadLoadedListResponse = server.request(|request_id| ClientRequest::ThreadLoadedList {
        request_id, params: ThreadLoadedListParams::default(),
    }).await?;
    assert!(loaded.data.is_empty());
    Ok(())
}

#[test_case::test_case(false; "running_after_interrupt")]
#[test_case::test_case(true; "queued_before_unsubscribe")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codexdc_background_work_blocks_idle_unload_until_delivery(already_queued: bool) -> Result<()> {
    let model = start_mock_server().await;
    let log = responses(&model, /*follow_up*/ true).await;
    let (_home, mut server, thread_id) = interrupted_job(&model, /*enabled*/ true).await?;
    if already_queued {
        release(&server).await?;
        queued(&mut server, &thread_id).await?;
    }
    unsubscribe(&mut server, &thread_id).await?;
    stays_loaded(&mut server, &thread_id).await?;
    if !already_queued {
        let running = jobs(&mut server, &thread_id).await?;
        assert_eq!(running.data[0].state, BackgroundJobState::Running);
        release(&server).await?;
    }
    let pending = queued(&mut server, &thread_id).await?;
    assert_eq!(pending.data[0].delivery, BackgroundJobDelivery::Pending);
    stays_loaded(&mut server, &thread_id).await?;
    assert_eq!(log.requests().len(), 2, "pending completion never starts inference");
    let _: ThreadResumeResponse = server.request(|request_id| ClientRequest::ThreadResume {
        request_id, params: ThreadResumeParams { thread_id: thread_id.clone(), ..Default::default() },
    }).await?;
    server.start_turn_and_wait_for_completion(TurnStartParams {
        thread_id: thread_id.clone(),
        input: vec![UserInput::Text { text: "continue".into(), text_elements: vec![] }],
        ..Default::default()
    }).await?;
    let delivered = jobs(&mut server, &thread_id).await?;
    assert_eq!(delivered.data[0].delivery, BackgroundJobDelivery::Delivered);
    assert_eq!(delivered.data[0].job_id, pending.data[0].job_id);
    let input = log.last_request().unwrap().body_json()["input"].to_string();
    assert_eq!(input.matches("<command_completion>").count(), 1);
    assert_eq!(input.matches(&pending.data[0].job_id).count(), 1);
    unsubscribe(&mut server, &thread_id).await?;
    closed(&mut server, &thread_id).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codexdc_revoked_queue_restarts_idle_unload_without_turn_status_change() -> Result<()> {
    let model = start_mock_server().await;
    let log = responses(&model, /*follow_up*/ false).await;
    let (_home, mut server, thread_id) = interrupted_job(&model, /*enabled*/ true).await?;
    release(&server).await?;
    let pending = queued(&mut server, &thread_id).await?;
    unsubscribe(&mut server, &thread_id).await?;
    stays_loaded(&mut server, &thread_id).await?;
    let _: ThreadBackgroundTerminalsTerminateResponse = server.request(|request_id| {
        ClientRequest::ThreadBackgroundTerminalsTerminate {
            request_id,
            params: ThreadBackgroundTerminalsTerminateParams {
                thread_id: thread_id.clone(), process_id: pending.data[0].process_id.clone(),
            },
        }
    }).await?;
    assert_eq!(jobs(&mut server, &thread_id).await?.data[0].delivery, BackgroundJobDelivery::None);
    closed(&mut server, &thread_id).await?;
    assert_eq!(log.requests().len(), 2);
    Ok(())
}

async fn assert_job_was_killed(server: &TestAppServer) -> Result<()> {
    // A surviving process would cross its gate and write finished. The executor
    // filesystem works on local and remote targets without host-specific PID probes.
    release(server).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let env = server.auto_env()?;
    let result = env.environment().get_filesystem().read_file(
        &env.selection().cwd.join("finished")?, Default::default(), /*sandbox*/ None,
    ).await;
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::NotFound);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codexdc_background_flag_off_keeps_existing_idle_unload() -> Result<()> {
    let model = start_mock_server().await;
    let log = responses(&model, /*follow_up*/ false).await;
    let (_home, mut server, thread_id) = interrupted_job(&model, /*enabled*/ false).await?;
    let terminals: ThreadBackgroundTerminalsListResponse = server.request(|request_id| {
        ClientRequest::ThreadBackgroundTerminalsList {
            request_id, params: ThreadBackgroundTerminalsListParams {
                thread_id: thread_id.clone(), cursor: None, limit: None,
            },
        }
    }).await?;
    assert_eq!(terminals.data.len(), 1);
    assert_eq!(jobs(&mut server, &thread_id).await?, CodexdcBackgroundJobsListResponse {
        enabled: false, data: vec![],
    });
    unsubscribe(&mut server, &thread_id).await?;
    closed(&mut server, &thread_id).await?;
    assert_job_was_killed(&server).await?;
    assert_eq!(log.requests().len(), 2);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codexdc_explicit_archive_shutdown_bypasses_background_guard() -> Result<()> {
    let model = start_mock_server().await;
    let log = responses(&model, /*follow_up*/ false).await;
    let (_home, mut server, thread_id) = interrupted_job(&model, /*enabled*/ true).await?;
    unsubscribe(&mut server, &thread_id).await?;
    stays_loaded(&mut server, &thread_id).await?;
    let _: ThreadArchiveResponse = server.request(|request_id| ClientRequest::ThreadArchive {
        request_id, params: ThreadArchiveParams { thread_id: thread_id.clone() },
    }).await?;
    let loaded: ThreadLoadedListResponse = server.request(|request_id| ClientRequest::ThreadLoadedList {
        request_id, params: ThreadLoadedListParams::default(),
    }).await?;
    assert!(loaded.data.is_empty());
    assert_job_was_killed(&server).await?;
    assert_eq!(log.requests().len(), 2);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codexdc_explicit_server_shutdown_kills_guarded_background_job() -> Result<()> {
    let model = start_mock_server().await;
    let log = responses(&model, /*follow_up*/ false).await;
    let (_home, mut server, thread_id) = interrupted_job(&model, /*enabled*/ true).await?;
    unsubscribe(&mut server, &thread_id).await?;
    stays_loaded(&mut server, &thread_id).await?;
    assert!(timeout(READ_TIMEOUT, server.shutdown_gracefully()).await??.success());
    assert_job_was_killed(&server).await?;
    assert_eq!(log.requests().len(), 2);
    Ok(())
}
