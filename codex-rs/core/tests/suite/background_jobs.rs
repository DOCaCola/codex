//! Toggle boundaries and originating-turn attribution for deferred results.
use super::*;
use pretty_assertions::assert_eq;
use pretty_assertions::assert_ne;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_jobs_both_off_preserves_head_running_stack_output() -> Result<()> {
    skip_if_no_network!(Ok(()));
    let harness = TestCodexHarness::with_auto_env_builder(
        test_codex().with_model("gpt-5.5").with_config(|config| {
            config.features.disable(Feature::BackgroundJobs).unwrap();
            config
                .features
                .disable(Feature::BackgroundCommandDelivery)
                .unwrap();
            config.features.enable(Feature::CommandStack).unwrap();
            config.features.disable(Feature::CodeMode).unwrap();
        }),
    )
    .await?;
    let stack = format!(
        "exec_command\n{}\nend",
        json!({"cmd": gated_command(), "yield_time_ms": 1})
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
    wait_for_event_with_timeout(
        &harness.test().codex,
        |event| matches!(event, EventMsg::TurnComplete(_)),
        Duration::from_secs(30),
    )
    .await;
    let output = log.last_request().unwrap().custom_tool_call_output("stack");
    let result: Value = serde_json::from_str(output["output"].as_str().unwrap())?;
    assert_eq!(result["results"][0]["success"], true);
    assert!(result["results"][0].get("running").is_none());
    assert_eq!(
        harness.test().codex.list_background_terminals().await.len(),
        1
    );
    harness.write_file("release", "go").await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::ExecCommandEnd(_))
    })
    .await;
    Ok(())
}

#[test_case::test_case(false, false; "delivery_only")]
#[test_case::test_case(true, false; "background_jobs")]
#[test_case::test_case(true, true; "stack_identity")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn background_jobs_later_turn_preserves_hook_origin(rich: bool, stack: bool) -> Result<()> {
    skip_if_no_network!(Ok(()));
    let home = Arc::new(tempfile::tempdir()?);
    let script = home.path().join("post.py");
    let hook_log = home.path().join("post.json");
    std::fs::write(
        &script,
        format!(
            "import json,sys\nfrom pathlib import Path\nPath({:?}).write_text(json.dumps(json.load(sys.stdin)))\n",
            hook_log.to_string_lossy(),
        ),
    )?;
    std::fs::write(home.path().join("hooks.json"), json!({
        "hooks": {"PostToolUse": [{
            "matcher": "Bash",
            "hooks": [{"type": "command", "command": format!("python3 \"{}\"", script.display())}]
        }]}
    }).to_string())?;
    let harness = TestCodexHarness::with_auto_env_builder(
        test_codex()
            .with_home(home)
            .with_model("gpt-5.5")
            .with_config(move |config| {
                core_test_support::hooks::trust_discovered_hooks(config);
                config
                    .features
                    .enable(Feature::BackgroundCommandDelivery)
                    .unwrap();
                config
                    .features
                    .set_enabled(Feature::BackgroundJobs, rich)
                    .unwrap();
                config.features.enable(Feature::CodexHooks).unwrap();
                config.features.disable(Feature::CodeMode).unwrap();
                config
                    .features
                    .set_enabled(Feature::CommandStack, stack)
                    .unwrap();
            }),
    )
    .await?;
    let log = mount_sse_sequence(
        harness.server(),
        vec![
            sse(vec![
                if stack {
                    ev_custom_tool_call(
                        "launch:with:colons",
                        "command_stack",
                        &format!(
                            "exec_command\n{}\nend",
                            json!({"cmd": gated_command(), "execution_mode": "background"}),
                        ),
                    )
                } else {
                    ev_function_call(
                        "launch:with:colons",
                        "exec_command",
                        &json!({"cmd": gated_command(), "execution_mode": "background"})
                            .to_string(),
                    )
                },
                ev_completed("launch-response"),
            ]),
            sse(vec![
                ev_assistant_message("wait", "first-wait"),
                ev_completed("first"),
            ]),
            sse(vec![
                ev_assistant_message("wait2", "second-wait"),
                ev_completed("second"),
            ]),
            sse(vec![
                ev_assistant_message("done", "finished"),
                ev_completed("finish"),
            ]),
        ]
        .into_iter()
        .take(if rich { 4 } else { 2 })
        .collect(),
    )
    .await;
    submit_unified_exec_turn(harness.test(), "run", PermissionProfile::Disabled).await?;
    let EventMsg::TurnStarted(first) = wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::TurnStarted(_))
    })
    .await
    else {
        unreachable!()
    };
    wait_for_event(&harness.test().codex, |event| {
        matches!(event,
        EventMsg::AgentMessage(message) if message.message == "first-wait")
    })
    .await;
    harness.test().codex.submit(Op::Interrupt).await?;
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::TurnAborted(_))
    })
    .await;
    let retained =
        if !rich {
            let jobs = harness.test().codex.list_background_jobs().await?;
            assert_eq!(jobs.len(), 1);
            let process_id = jobs[0].process_id.parse::<i32>()?;
            harness.write_file("release", "go").await?;
            Some(
                mount_sse_sequence(
                    harness.server(),
                    vec![
                        sse(
                            vec![ev_function_call("read-retained", "write_stdin", &json!({
                "session_id": process_id, "chars": "", "wait_mode": "completion",
            }).to_string()), ev_completed("read-retained")],
                        ),
                        sse(vec![
                            ev_assistant_message("done", "finished"),
                            ev_completed("finish"),
                        ]),
                    ],
                )
                .await,
            )
        } else {
            None
        };
    submit_unified_exec_turn(harness.test(), "continue", PermissionProfile::Disabled).await?;
    let EventMsg::TurnStarted(second) = wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::TurnStarted(_))
    })
    .await
    else {
        unreachable!()
    };
    assert_ne!(first.turn_id, second.turn_id);
    if rich {
        wait_for_event(&harness.test().codex, |event| {
            matches!(event,
        EventMsg::AgentMessage(message) if message.message == "second-wait")
        })
        .await;
        harness.write_file("release", "go").await?;
    }
    wait_for_event(&harness.test().codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    let payload: Value = serde_json::from_str(&std::fs::read_to_string(&hook_log)?)?;
    assert_eq!(
        payload["turn_id"].as_str(),
        Some(if rich {
            first.turn_id.as_str()
        } else {
            second.turn_id.as_str()
        })
    );
    let call_id = if stack {
        "launch:with:colons:1:0"
    } else {
        "launch:with:colons"
    };
    assert_eq!(payload["tool_use_id"], call_id);
    if let Some(retained) = retained {
        let request = retained.last_request().unwrap();
        assert!(
            request
                .function_call_output("read-retained")
                .to_string()
                .contains("completed-marker")
        );
        assert!(
            !request.body_json()["input"]
                .to_string()
                .contains("<command_completion>")
        );
        assert_eq!(
            harness.test().codex.list_background_jobs().await?[0].delivery,
            codex_core::background_jobs::BackgroundJobDelivery::Delivered
        );
        return Ok(());
    }
    let input = log.last_request().unwrap().body_json();
    let fragment = input["input"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter_map(|item| item["text"].as_str())
        .find(|text| text.contains("<command_completion>"))
        .unwrap();
    let mut fragment = fragment.to_string();
    if rich {
        let serialized = fragment
            .split("Managed job metadata: ")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap();
        let mut metadata: Value = serde_json::from_str(serialized)?;
        assert!(uuid::Uuid::parse_str(metadata["job_id"].as_str().unwrap()).is_ok());
        assert_eq!(metadata["originating_turn"], first.turn_id);
        assert_eq!(
            metadata["stack"],
            if stack {
                json!({"parent_call_id": "launch:with:colons", "step": 1, "index": 0})
            } else {
                Value::Null
            }
        );
        assert_eq!(metadata["operation"], call_id);
        assert_eq!(
            metadata["command"],
            &gated_command()[..gated_command().floor_char_boundary(96.min(gated_command().len()))]
        );
        metadata["job_id"] = json!("<job-id>");
        metadata["originating_turn"] = json!("<origin-turn>");
        metadata["duration_ms"] = json!("<duration>");
        metadata["command"] = json!("<bounded-command>");
        metadata.sort_all_objects();
        fragment = fragment.replace(serialized, &metadata.to_string());
    } else {
        assert!(!fragment.contains("Managed job metadata"));
        let session = harness.test().codex.list_background_terminals().await;
        assert!(session.is_empty());
    }
    // Keep fragment content, normalizing JSON order and runtime identity/timing.
    let start = fragment.find("Command session ").unwrap() + "Command session ".len();
    let end = fragment[start..].find(' ').unwrap() + start;
    let id = fragment[start..end].to_string();
    fragment = fragment.replace(&id, "<session-id>");
    if stack {
        insta::assert_snapshot!("background_jobs_stack_completion", fragment);
    } else if rich {
        insta::assert_snapshot!("background_jobs_completion", fragment);
    } else {
        insta::assert_snapshot!("delivery_only_completion", fragment);
    }
    Ok(())
}
