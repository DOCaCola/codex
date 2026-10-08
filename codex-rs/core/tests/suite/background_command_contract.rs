//! Stack result, dependency deadline, and cache-pressure regressions.
use super::*;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::sse_response;
use pretty_assertions::assert_eq;
use wiremock::Mock;
use wiremock::matchers::method;

async fn stack_harness() -> Result<TestCodexHarness> {
    TestCodexHarness::with_auto_env_builder(test_codex().with_model("gpt-5.5").with_config(
        |config| {
            config.features.enable(Feature::UnifiedExec).unwrap();
            config
                .features
                .enable(Feature::BackgroundCommandDelivery)
                .unwrap();
            config.features.enable(Feature::CommandStack).unwrap();
            config.features.disable(Feature::CodeMode).unwrap();
        },
    ))
    .await
}

async fn mount_waiting_response(harness: &TestCodexHarness) {
    Mock::given(method("POST"))
        .respond_with(sse_response(sse(vec![
            ev_assistant_message("waiting", "waiting-for-command"),
            ev_completed("waiting-response"),
        ])))
        .with_priority(10)
        .mount(harness.server())
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_final_step_reports_pending_and_keeps_execution_events_running() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = stack_harness().await?;
    mount_waiting_response(&harness).await;
    let stack = format!(
        "exec_command\n{}\nend",
        json!({
            "cmd": gated_command(), "execution_mode": "background",
        })
    );
    mount_sse_once(
        harness.server(),
        sse(vec![
            ev_custom_tool_call("stack", "command_stack", &stack),
            ev_completed("launch"),
        ]),
    )
    .await;
    submit_unified_exec_turn(harness.test(), "run", PermissionProfile::Disabled).await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::AgentMessage(message) if message.message == "waiting-for-command")
    }).await;
    assert_eq!(
        harness.test().codex.list_background_terminals().await.len(),
        1
    );
    let requests = harness.server().received_requests().await.unwrap();
    let request: Value = requests.last().unwrap().body_json()?;
    let output = request["input"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["type"] == "custom_tool_call_output" && item["call_id"] == "stack")
        .unwrap()["output"]
        .as_str()
        .unwrap();
    let mut result: Value = serde_json::from_str(output)?;
    assert!(
        result["results"][0]["output"]
            .as_str()
            .unwrap()
            .contains("Process running")
    );
    result["results"][0]
        .as_object_mut()
        .unwrap()
        .remove("output");
    insta::assert_snapshot!(
        "running_stack_result",
        serde_json::to_string_pretty(&result)?
    );
    harness.write_file("release", "go").await?;
    let end = wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::ExecCommandEnd(_))
    })
    .await;
    let EventMsg::ExecCommandEnd(end) = end else {
        unreachable!()
    };
    assert_eq!(end.exit_code, 0);
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    Ok(())
}

#[test_case::test_case("auto")]
#[test_case::test_case("background")]
#[test_case::test_case("foreground")]
#[tokio::test]
async fn dependency_outlives_the_foreground_default_without_a_deadline(mode: &str) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = stack_harness().await?;
    let command = format!("echo dependency-ready; {}", gated_command());
    let stack = format!(
        "exec_command\n{}\nend\nexec_command step=2\n{}\nend",
        json!({"cmd": command, "execution_mode": mode}),
        json!({"cmd": "echo dependent-marker", "execution_mode": "foreground"})
    );
    let log = mount_sse_sequence(
        harness.server(),
        vec![
            sse(vec![
                ev_custom_tool_call("stack", "command_stack", &stack),
                ev_completed("launch"),
            ]),
            sse(vec![
                ev_assistant_message("done", "finished"),
                ev_completed("finish"),
            ]),
        ],
    )
    .await;
    submit_unified_exec_turn(harness.test(), "run", PermissionProfile::Disabled).await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::ExecCommandOutputDelta(delta)
            if String::from_utf8_lossy(&delta.chunk).contains("dependency-ready"))
    })
    .await;
    // Follow user_shell_cmd's time control: virtual deadline time, real process I/O.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(301)).await;
    tokio::time::resume();
    harness.write_file("release", "go").await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    let text = log.last_request().unwrap().custom_tool_call_output("stack")["output"]
        .as_str()
        .unwrap()
        .to_owned();
    let result: Value = serde_json::from_str(&text)?;
    assert_eq!(result["stopped"], false);
    assert_eq!(result["results"].as_array().unwrap().len(), 2);
    assert!(
        result["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["success"] == true)
    );
    assert!(text.contains("completed-marker") && text.contains("dependent-marker"));
    Ok(())
}

#[tokio::test]
async fn dependency_preserves_its_explicit_deadline() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = stack_harness().await?;
    let stack = format!(
        "exec_command\n{}\nend\nexec_command step=2\n{}\nend",
        json!({"cmd": format!("echo dependency-ready; {}", gated_command()), "timeout_ms": 1000}),
        json!({"cmd": "echo should-not-run"})
    );
    let log = mount_sse_sequence(
        harness.server(),
        vec![
            sse(vec![
                ev_custom_tool_call("stack", "command_stack", &stack),
                ev_completed("launch"),
            ]),
            sse(vec![
                ev_assistant_message("done", "finished"),
                ev_completed("finish"),
            ]),
        ],
    )
    .await;
    submit_unified_exec_turn(harness.test(), "run", PermissionProfile::Disabled).await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::ExecCommandOutputDelta(delta)
            if String::from_utf8_lossy(&delta.chunk).contains("dependency-ready"))
    })
    .await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(2)).await;
    tokio::time::resume();
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    let text = log.last_request().unwrap().custom_tool_call_output("stack")["output"]
        .as_str()
        .unwrap()
        .to_owned();
    let result: Value = serde_json::from_str(&text)?;
    assert_eq!(result["stopped"], true);
    assert_eq!(result["results"].as_array().unwrap().len(), 1);
    assert_eq!(result["results"][0]["success"], false);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cache_pressure_evicts_all_delivered_results_without_another_reservation() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = stack_harness().await?;
    mount_waiting_response(&harness).await;
    let mut old_ids = Vec::new();
    for round in 0..8 {
        let gate = format!("release-{round}");
        let command = gated_command().replace("release", &gate);
        let stack = (0..8)
            .map(|_| {
                format!(
                    "exec_command\n{}\nend",
                    json!({"cmd": command, "execution_mode": "background"})
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        mount_sse_once(
            harness.server(),
            sse(vec![
                ev_custom_tool_call(&format!("stack-{round}"), "command_stack", &stack),
                ev_completed("launch"),
            ]),
        )
        .await;
        submit_unified_exec_turn(harness.test(), "run", PermissionProfile::Disabled).await?;
        wait_for_event(&harness.test().codex, |event| {
            matches!(event, EventMsg::AgentMessage(message) if message.message == "waiting-for-command")
        }).await;
        let processes = harness.test().codex.list_background_terminals().await;
        assert_eq!(processes.len(), 8);
        if round < 7 {
            old_ids.extend(processes.into_iter().map(|process| process.process_id));
        }
        harness.write_file(&gate, "go").await?;
        wait_for_event(&harness.test().codex, |event| {
            matches!(event, EventMsg::TurnComplete(_))
        })
        .await;
    }
    let reads = old_ids
        .chunks(8)
        .enumerate()
        .map(|(index, ids)| {
            let input = ids
                .iter()
                .map(|id| {
                    format!(
                        "write_stdin\n{}\nend",
                        json!({"session_id": id.parse::<i32>().unwrap()})
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            ev_custom_tool_call(&format!("read-{index}"), "command_stack", &input)
        })
        .chain([ev_completed("read-cache")])
        .collect::<Vec<_>>();
    mount_sse_once(harness.server(), sse(reads)).await;
    submit_unified_exec_turn(
        harness.test(),
        "read retired output",
        PermissionProfile::Disabled,
    )
    .await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    let requests = harness.server().received_requests().await.unwrap();
    let request: Value = requests.last().unwrap().body_json()?;
    for index in 0..7 {
        let item = request["input"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| {
                item["type"] == "custom_tool_call_output"
                    && item["call_id"] == format!("read-{index}")
            })
            .unwrap();
        let result: Value = serde_json::from_str(item["output"].as_str().unwrap())?;
        assert_eq!(result["results"].as_array().unwrap().len(), 8);
        assert!(result["results"].as_array().unwrap().iter().all(|item| {
            item["success"] == false
                && item["output"]
                    .as_str()
                    .unwrap()
                    .contains("Unknown process id")
        }));
    }
    Ok(())
}
