//! Behavioral coverage: unchanged jobs cause no inference, completion resumes
//! the same logical turn, cleanup revokes delivery, foreground stays synchronous,
//! and stack dependencies wait for exit rather than an initial yielded handle.
#![allow(clippy::unwrap_used)]

use super::*;
use core_test_support::TestTargetOs;
use core_test_support::responses::ev_custom_tool_call;
use core_test_support::test_target_os;
use pretty_assertions::assert_eq;

fn gated_command() -> &'static str {
    match test_target_os() {
        TestTargetOs::Windows => {
            "while (!(Test-Path release)) { Start-Sleep -Milliseconds 50 }; Write-Output completed-marker"
        }
        TestTargetOs::Linux | TestTargetOs::MacOs => {
            "while [ ! -e release ]; do sleep 0.05; done; echo completed-marker"
        }
    }
}

async fn managed_harness() -> Result<TestCodexHarness> {
    TestCodexHarness::with_auto_env_builder(test_codex().with_model("gpt-5.4").with_config(
        |config| {
            config.features.enable(Feature::UnifiedExec).unwrap();
            config
                .features
                .enable(Feature::BackgroundCommandDelivery)
                .unwrap();
            config.features.disable(Feature::CommandStack).unwrap();
            config.features.disable(Feature::CodeMode).unwrap();
        },
    ))
    .await
}

#[test_case::test_case("auto"; "automatic")]
#[test_case::test_case("background"; "explicit_background")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_delivery_parks_without_model_requests_and_resumes_once(
    mode: &str,
) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = managed_harness().await?;
    let log = mount_sse_sequence(
        harness.server(),
        vec![
            sse(vec![
                ev_function_call(
                    "launch",
                    "exec_command",
                    &json!({
                        "cmd": gated_command(), "execution_mode": mode, "timeout_ms": 30000,
                    })
                    .to_string(),
                ),
                ev_completed("r1"),
            ]),
            sse(vec![
                ev_assistant_message("waiting", "waiting-for-command"),
                ev_completed("r2"),
            ]),
            sse(vec![
                ev_assistant_message("done", "finished"),
                ev_completed("r3"),
            ]),
        ],
    )
    .await;
    let test = harness.test();
    submit_unified_exec_turn(test, "run", PermissionProfile::Disabled).await?;
    wait_for_event_with_timeout(&test.codex, |event| {
        matches!(event, EventMsg::AgentMessage(message) if message.message == "waiting-for-command")
    }, Duration::from_secs(30)).await;
    let finished = tokio::time::timeout(
        Duration::from_millis(400),
        wait_for_event(&test.codex, |event| {
            matches!(event, EventMsg::TurnComplete(_))
        }),
    )
    .await;
    assert!(
        finished.is_err(),
        "logical turn must remain active while waiting"
    );
    assert_eq!(log.requests().len(), 2);
    harness.write_file("release", "go").await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    assert_eq!(log.requests().len(), 3);
    let final_input = log.last_request().unwrap().body_json()["input"].to_string();
    assert_eq!(final_input.matches("<command_completion>").count(), 1);
    assert!(final_input.contains("completed-marker"));
    assert!(final_input.contains("exit_code=Some(0)"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_delivery_cleanup_revokes_pending_completion() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = managed_harness().await?;
    let log =
        mount_sse_sequence(
            harness.server(),
            vec![
                sse(vec![
            ev_function_call("launch", "exec_command", &json!({
                "cmd": gated_command(), "execution_mode": "background", "timeout_ms": 30000,
            }).to_string()),
            ev_completed("r1"),
        ]),
                sse(vec![
                    ev_assistant_message("waiting", "waiting-for-command"),
                    ev_completed("r2"),
                ]),
            ],
        )
        .await;
    let test = harness.test();
    submit_unified_exec_turn(test, "run", PermissionProfile::Disabled).await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::AgentMessage(message) if message.message == "waiting-for-command")
    }).await;
    test.codex.submit(Op::CleanBackgroundTerminals).await?;
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    assert_eq!(log.requests().len(), 2);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_delivery_foreground_returns_final_result_directly() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = managed_harness().await?;
    let log = mount_sse_sequence(
        harness.server(),
        vec![
            sse(vec![
                ev_function_call(
                    "launch",
                    "exec_command",
                    &json!({
                        "cmd": "echo foreground-marker", "execution_mode": "foreground",
                    })
                    .to_string(),
                ),
                ev_completed("r1"),
            ]),
            sse(vec![
                ev_assistant_message("done", "finished"),
                ev_completed("r2"),
            ]),
        ],
    )
    .await;
    submit_unified_exec_turn(harness.test(), "run", PermissionProfile::Disabled).await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    assert_eq!(log.requests().len(), 2);
    let output = log
        .last_request()
        .unwrap()
        .function_call_output_text("launch")
        .unwrap();
    assert!(output.contains("foreground-marker"));
    assert!(output.contains("Process exited with code 0"));
    assert!(
        !log.last_request().unwrap().body_json()["input"]
            .to_string()
            .contains("<command_completion>")
    );
    Ok(())
}

#[test_case::test_case("gpt-5.4"; "direct")]
#[test_case::test_case("gpt-5.6-sol"; "responses_lite_code_mode")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_delivery_stack_waits_for_dependencies_without_duplicate_delivery(
    model: &str,
) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = TestCodexHarness::with_auto_env_builder(
        test_codex().with_model(model).with_config(|config| {
            config.features.enable(Feature::UnifiedExec).unwrap();
            config
                .features
                .enable(Feature::BackgroundCommandDelivery)
                .unwrap();
            config.features.enable(Feature::CommandStack).unwrap();
            config.features.disable(Feature::CodeMode).unwrap();
        }),
    )
    .await?;
    let stack = format!(
        "exec_command\n{}\nend\nexec_command step=2\n{}\nend",
        json!({"cmd": gated_command(), "execution_mode": "background", "timeout_ms": 30000}),
        json!({"cmd": "echo dependent-marker", "execution_mode": "foreground"}),
    );
    let log = mount_sse_sequence(
        harness.server(),
        vec![
            sse(vec![
                ev_custom_tool_call("stack", "command_stack", &stack),
                ev_completed("r1"),
            ]),
            sse(vec![
                ev_assistant_message("done", "finished"),
                ev_completed("r2"),
            ]),
        ],
    )
    .await;
    submit_unified_exec_turn(harness.test(), "run", PermissionProfile::Disabled).await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::ExecCommandBegin(_))
    })
    .await;
    assert_eq!(log.requests().len(), 1);
    harness.write_file("release", "go").await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    assert_eq!(log.requests().len(), 2);
    let input = log.last_request().unwrap().body_json()["input"].to_string();
    assert!(input.contains("completed-marker"));
    assert!(input.contains("dependent-marker"));
    assert!(!input.contains("<command_completion>"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_delivery_deadline_terminates_and_delivers() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = managed_harness().await?;
    let log =
        mount_sse_sequence(
            harness.server(),
            vec![
                sse(vec![
            ev_function_call("launch", "exec_command", &json!({
                "cmd": gated_command(), "execution_mode": "background", "timeout_ms": 1000,
            }).to_string()),
            ev_completed("r1"),
        ]),
                sse(vec![
                    ev_assistant_message("waiting", "waiting-for-command"),
                    ev_completed("r2"),
                ]),
                sse(vec![
                    ev_assistant_message("done", "timed out"),
                    ev_completed("r3"),
                ]),
            ],
        )
        .await;
    submit_unified_exec_turn(harness.test(), "run", PermissionProfile::Disabled).await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    let requests = log.requests();
    let input = requests.last().unwrap().body_json()["input"].to_string();
    assert!(input.contains("<command_completion>"));
    assert!(!input.contains("exit_code=Some(0)"));
    assert_eq!(requests.len(), 3);
    Ok(())
}

#[test_case::test_case(false; "disabled")]
#[test_case::test_case(true; "interactive")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_delivery_manual_modes_do_not_park(enabled: bool) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness =
        TestCodexHarness::with_auto_env_builder(test_codex().with_model("gpt-5.4").with_config(
            move |config| {
                config.features.enable(Feature::UnifiedExec).unwrap();
                config.features.disable(Feature::CommandStack).unwrap();
                config.features.disable(Feature::CodeMode).unwrap();
                if enabled {
                    config
                        .features
                        .enable(Feature::BackgroundCommandDelivery)
                        .unwrap();
                } else {
                    config
                        .features
                        .disable(Feature::BackgroundCommandDelivery)
                        .unwrap();
                }
            },
        ))
        .await?;
    let mut args = json!({"cmd": gated_command(), "yield_time_ms": 1});
    if enabled {
        args["execution_mode"] = json!("interactive");
    }
    let log = mount_sse_sequence(
        harness.server(),
        vec![
            sse(vec![
                ev_function_call("launch", "exec_command", &args.to_string()),
                ev_completed("r1"),
            ]),
            sse(vec![
                ev_assistant_message("done", "finished"),
                ev_completed("r2"),
            ]),
        ],
    )
    .await;
    submit_unified_exec_turn(harness.test(), "run", PermissionProfile::Disabled).await?;
    wait_for_event_with_timeout(
        &harness.test().codex,
        |event| matches!(event, EventMsg::TurnComplete(_)),
        Duration::from_secs(30),
    )
    .await;
    harness
        .test()
        .codex
        .submit(Op::CleanBackgroundTerminals)
        .await?;
    assert_eq!(log.requests().len(), 2);
    let tool = log.requests()[0].body_json()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "exec_command")
        .unwrap()
        .clone();
    assert_eq!(
        tool["parameters"]["properties"]
            .get("execution_mode")
            .is_some(),
        enabled
    );
    assert!(
        log.function_call_output_text("launch")
            .unwrap()
            .contains("Process running with session ID")
    );
    Ok(())
}

#[test_case::test_case(false; "stop")]
#[test_case::test_case(true; "continues_on_failure")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_delivery_stack_observes_exit_status(continue_on_failure: bool) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = TestCodexHarness::with_auto_env_builder(
        test_codex().with_model("gpt-5.4").with_config(|config| {
            config.features.enable(Feature::UnifiedExec).unwrap();
            config
                .features
                .enable(Feature::BackgroundCommandDelivery)
                .unwrap();
            config.features.enable(Feature::CommandStack).unwrap();
            config.features.disable(Feature::CodeMode).unwrap();
        }),
    )
    .await?;
    let modifier = if continue_on_failure {
        " continue_on_failure"
    } else {
        ""
    };
    let stack = format!(
        "exec_command{modifier}\n{{\"cmd\":\"exit 7\"}}\nend\nexec_command step=2\n{{\"cmd\":\"echo dependent-marker\",\"execution_mode\":\"foreground\"}}\nend",
    );
    let log = mount_sse_sequence(
        harness.server(),
        vec![
            sse(vec![
                ev_custom_tool_call("stack", "command_stack", &stack),
                ev_completed("r1"),
            ]),
            sse(vec![
                ev_assistant_message("done", "finished"),
                ev_completed("r2"),
            ]),
        ],
    )
    .await;
    submit_unified_exec_turn(harness.test(), "run", PermissionProfile::Disabled).await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    let output = log.last_request().unwrap().custom_tool_call_output("stack")["output"].to_string();
    assert!(output.contains("Process exited with code 7"));
    assert_eq!(output.contains("dependent-marker"), continue_on_failure);
    assert_eq!(log.requests().len(), 2);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_delivery_code_mode_completion_wait_consumes_once() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = TestCodexHarness::with_auto_env_builder(
        test_codex().with_model("gpt-5.4").with_config(|config| {
            config.features.enable(Feature::UnifiedExec).unwrap();
            config
                .features
                .enable(Feature::BackgroundCommandDelivery)
                .unwrap();
            config.features.enable(Feature::CodeMode).unwrap();
            config.features.disable(Feature::CommandStack).unwrap();
        }),
    )
    .await?;
    let args = json!({"cmd": gated_command(), "execution_mode": "background", "timeout_ms": 30000});
    let code = format!(
        "const job = await tools.exec_command({args});\n\
         await tools.exec_command({{cmd: 'echo wait-ready', execution_mode: 'foreground'}});\n\
         const result = await tools.write_stdin({{session_id: job.session_id, wait_mode: 'completion'}});\n\
         text(result);\n\
         text(await tools.write_stdin({{session_id: job.session_id}}));"
    );
    let log = mount_sse_sequence(
        harness.server(),
        vec![
            sse(vec![
                ev_custom_tool_call("cell", "exec", &code),
                ev_completed("r1"),
            ]),
            sse(vec![
                ev_assistant_message("done", "finished"),
                ev_completed("r2"),
            ]),
        ],
    )
    .await;
    submit_unified_exec_turn(harness.test(), "run", PermissionProfile::Disabled).await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::ExecCommandEnd(end) if end.aggregated_output.contains("wait-ready"))
    }).await;
    harness.write_file("release", "go").await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    let input = log.last_request().unwrap().body_json()["input"].to_string();
    assert!(input.contains("completed-marker"));
    assert!(!input.contains("<command_completion>"));
    assert_eq!(log.requests().len(), 2);
    Ok(())
}
