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

async fn list(
    server: &mut TestAppServer,
    thread_id: &str,
) -> Result<CodexdcBackgroundJobsListResponse> {
    let id = server
        .send_request(
            "codexdc/backgroundJobs/list",
            Some(json!({"threadId": thread_id})),
        )
        .await?;
    to_response(
        timeout(
            Duration::from_secs(10),
            server.read_stream_until_response_message(RequestId::Integer(id)),
        )
        .await??,
    )
}

#[test_case::test_case(Some(60_000), false; "explicit_timeout")]
#[test_case::test_case(None, false; "no_timeout")]
#[test_case::test_case(None, true; "stack_child")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codexdc_background_jobs_tracks_running_completion_and_delivery(
    deadline: Option<u64>,
    stack: bool,
) -> Result<()> {
    let model = start_mock_server().await;
    let command = match test_target_os() {
        TestTargetOs::Windows => {
            "while (!(Test-Path release)) { Start-Sleep -Milliseconds 50 }; Write-Output done"
        }
        TestTargetOs::Linux | TestTargetOs::MacOs => {
            "while [ ! -e release ]; do sleep 0.05; done; echo done"
        }
    };
    let mut args = json!({"cmd": command, "execution_mode": "background"});
    if let Some(deadline) = deadline {
        args["timeout_ms"] = json!(deadline);
    }
    let launch = if stack {
        ev_custom_tool_call(
            "stack",
            "command_stack",
            &format!("exec_command\n{args}\nend"),
        )
    } else {
        ev_function_call("launch", "exec_command", &args.to_string())
    };
    let _responses = mount_sse_sequence(
        &model,
        vec![
            sse(vec![launch, ev_completed("launch")]),
            sse(vec![
                ev_assistant_message("wait", "waiting"),
                ev_completed("wait"),
            ]),
            sse(vec![
                ev_assistant_message("done", "finished"),
                ev_completed("done"),
            ]),
        ],
    )
    .await;
    let home = TempDir::new()?;
    let mut config = MockResponsesConfig::new(&model.uri())
        .with_model("gpt-5.5")
        .with_sandbox_mode("danger-full-access")
        .enable_feature(Feature::BackgroundJobs)
        .disable_feature(Feature::CodeMode);
    config = if stack {
        config.enable_feature(Feature::CommandStack)
    } else {
        config.disable_feature(Feature::CommandStack)
    };
    config.write(home.path())?;
    let mut server = TestAppServer::builder()
        .with_codex_home(home.path())
        .without_managed_config()
        .build_initialized()
        .await?;
    let thread = server
        .start_thread(ThreadStartParams::default())
        .await?
        .thread;
    server
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![UserInput::Text {
                text: "run".into(),
                text_elements: vec![],
            }],
            ..Default::default()
        })
        .await?;
    let started = timeout(
        Duration::from_secs(20),
        server.read_stream_until_matching_notification("command started", |n| {
            n.method == "item/started"
                && n.params
                    .as_ref()
                    .is_some_and(|p| p["item"]["type"] == "commandExecution")
        }),
    )
    .await??;
    let item_id = started.params.unwrap()["item"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    timeout(
        Duration::from_secs(20),
        server.read_stream_until_matching_notification("model waiting", |n| {
            n.method == "item/completed"
                && n.params
                    .as_ref()
                    .is_some_and(|p| p["item"]["type"] == "agentMessage")
        }),
    )
    .await??;
    let running = list(&mut server, &thread.id).await?;
    assert!(running.enabled);
    assert_eq!(running.data.len(), 1);
    let job = &running.data[0];
    assert_eq!(job.mode, BackgroundJobMode::Background);
    assert_eq!(
        (
            &job.state,
            &job.delivery,
            job.timeout_ms,
            job.completed_at_ms,
            job.exit_code
        ),
        (
            &BackgroundJobState::Running,
            &BackgroundJobDelivery::Pending,
            deadline.map(|ms| ms as i64),
            None,
            None
        )
    );
    assert_eq!(job.item_id, item_id);
    assert_eq!(job.item_id, if stack { "stack:1:0" } else { "launch" });
    assert!(uuid::Uuid::parse_str(&job.job_id).is_ok());
    assert_eq!(
        job.deadline_at_ms,
        deadline.map(|ms| job.started_at_ms + ms as i64)
    );
    let env = server.auto_env()?;
    env.environment()
        .get_filesystem()
        .write_file(
            &env.selection().cwd.join("release")?,
            b"go".to_vec(),
            Default::default(),
            /*sandbox*/ None,
        )
        .await?;
    timeout(
        Duration::from_secs(20),
        server.read_stream_until_notification_message("turn/completed"),
    )
    .await??;
    let completed = list(&mut server, &thread.id).await?;
    let result = &completed.data[0];
    assert_eq!(
        (
            &result.job_id,
            &result.item_id,
            &result.process_id,
            result.started_at_ms,
            result.timeout_ms,
            result.deadline_at_ms
        ),
        (
            &job.job_id,
            &job.item_id,
            &job.process_id,
            job.started_at_ms,
            job.timeout_ms,
            job.deadline_at_ms
        )
    );
    assert_eq!(
        (&result.state, &result.delivery, result.exit_code),
        (
            &BackgroundJobState::Completed,
            &BackgroundJobDelivery::Delivered,
            Some(0)
        )
    );
    assert!(result.completed_at_ms.unwrap() >= result.started_at_ms);
    Ok(())
}

#[tokio::test]
async fn codexdc_background_jobs_disabled_and_unknown_thread_match_upstream() -> Result<()> {
    let model = start_mock_server().await;
    let home = TempDir::new()?;
    MockResponsesConfig::new(&model.uri())
        .enable_feature(Feature::BackgroundJobs)
        .write(home.path())?;
    let mut server = TestAppServer::builder()
        .with_codex_home(home.path())
        .without_managed_config()
        .build_initialized()
        .await?;
    let thread = server
        .start_thread(ThreadStartParams {
            config: Some(std::collections::HashMap::from([
                ("features.background_jobs".to_string(), json!(false)),
                (
                    "features.background_command_delivery".to_string(),
                    json!(false),
                ),
            ])),
            ..Default::default()
        })
        .await?
        .thread;
    assert_eq!(
        list(&mut server, &thread.id).await?,
        CodexdcBackgroundJobsListResponse {
            enabled: false,
            data: vec![]
        }
    );
    let unknown = uuid::Uuid::new_v4().to_string();
    let mut errors = vec![];
    for method in [
        "codexdc/backgroundJobs/list",
        "thread/backgroundTerminals/list",
    ] {
        let id = server
            .send_request(method, Some(json!({"threadId": unknown})))
            .await?;
        errors.push(
            server
                .read_stream_until_error_message(RequestId::Integer(id))
                .await?
                .error,
        );
    }
    assert_eq!(errors[0], errors[1]);
    Ok(())
}
