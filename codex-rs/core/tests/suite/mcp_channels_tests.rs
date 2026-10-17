use super::*;
use pretty_assertions::assert_eq;

async fn channel_fixture(
    server: &MockServer,
    inbox: &Path,
    no_tools: bool,
) -> anyhow::Result<TestCodex> {
    let command = stdio_server_bin()?;
    let inbox = inbox.to_string_lossy().into_owned();
    test_codex()
        .with_config(move |config| {
            let mut env = HashMap::from([
                ("MCP_TEST_CHANNEL_INBOX".to_string(), inbox),
                (
                    "MCP_TEST_SERVER_INSTRUCTIONS".to_string(),
                    "Channel routing instructions: use the echo tool to reply.".to_string(),
                ),
            ]);
            if no_tools {
                env.insert("MCP_TEST_CHANNEL_NO_TOOLS".to_string(), "1".to_string());
            }
            insert_mcp_server(
                config,
                "channel",
                stdio_transport(command, Some(env), Vec::new()),
                TestMcpServerOptions::default(),
            );
            let mut servers = config.mcp_servers.get().clone();
            servers.get_mut("channel").expect("channel server").channel = true;
            config.mcp_servers.set(servers).expect("channel config");
        })
        .build_with_auto_env(server)
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_channel_starts_turn_and_calls_mcp_reply_tool() -> anyhow::Result<()> {
    skip_if_wine_exec!(Ok(()), "requires host-native stdio test server");
    let server = start_mock_server().await;
    let response = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse(vec![
                responses::ev_function_call_with_namespace(
                    "reply",
                    "mcp__channel",
                    "echo",
                    r#"{"message":"channel reply"}"#,
                ),
                responses::ev_completed("tool"),
            ]),
            responses::sse(vec![responses::ev_completed("done")]),
        ],
    )
    .await;
    let inbox = tempfile::NamedTempFile::new()?;
    let fixture = channel_fixture(&server, inbox.path(), /*no_tools*/ false).await?;
    wait_for_mcp_server(&fixture.codex, "channel").await?;
    fs::write(
        inbox.path(),
        format!(
            "{}\n",
            json!({"content":"incoming", "meta":{"chat_id":"17"}})
        ),
    )?;
    let event = wait_for_event(&fixture.codex, |event| {
        matches!(event, EventMsg::McpToolCallEnd(_))
    })
    .await;
    let EventMsg::McpToolCallEnd(event) = event else {
        unreachable!()
    };
    assert!(event.result.is_ok());
    wait_for_event(&fixture.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    let requests = response.requests();
    let first = serde_json::to_string(&requests[0].input())?;
    assert!(first.contains("Channel routing instructions"));
    assert!(first.contains("chat_id=\\\"17\\\""));
    assert!(first.contains("incoming"));
    fixture.codex.submit(Op::Shutdown).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn busy_one_way_channel_batches_identical_events_in_order() -> anyhow::Result<()> {
    skip_if_wine_exec!(Ok(()), "requires host-native stdio test server");
    let server = start_mock_server().await;
    let response = responses::mount_response_sequence(
        &server,
        vec![
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(responses::sse(vec![responses::ev_completed("user-done")]))
                .set_delay(Duration::from_millis(800)),
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(responses::sse(vec![responses::ev_completed(
                    "channel-done",
                )])),
        ],
    )
    .await;
    let inbox = tempfile::NamedTempFile::new()?;
    let fixture = channel_fixture(&server, inbox.path(), /*no_tools*/ true).await?;
    wait_for_mcp_server(&fixture.codex, "channel").await?;
    fixture
        .codex
        .start_or_steer_turn(read_only_user_turn(&fixture, "work"))
        .await?;
    wait_for_event(&fixture.codex, |event| {
        matches!(event, EventMsg::TurnStarted(_))
    })
    .await;
    let message = json!({"content":"repeat-event", "meta":{"chat_id":"17"}});
    fs::write(inbox.path(), format!("{message}\n{message}\n"))?;
    for _ in 0..2 {
        wait_for_event(&fixture.codex, |event| {
            matches!(event, EventMsg::TurnComplete(_))
        })
        .await;
    }
    let requests = response.requests();
    assert_eq!(requests.len(), 2);
    let first = serde_json::to_string(&requests[0].input())?;
    let second = serde_json::to_string(&requests[1].input())?;
    assert!(!first.contains("repeat-event"));
    assert_eq!(second.matches("repeat-event").count(), 2);
    assert_eq!(second.matches("Channel routing instructions").count(), 1);
    fixture.codex.submit(Op::Shutdown).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plan_mode_queues_channels_until_mode_change() -> anyhow::Result<()> {
    skip_if_wine_exec!(Ok(()), "requires host-native stdio test server");
    let server = start_mock_server().await;
    let response = mount_sse_once(
        &server,
        responses::sse(vec![responses::ev_completed("channel-done")]),
    )
    .await;
    let inbox = tempfile::NamedTempFile::new()?;
    let fixture = channel_fixture(&server, inbox.path(), /*no_tools*/ true).await?;
    wait_for_mcp_server(&fixture.codex, "channel").await?;
    let settings = |mode| ThreadSettingsOverrides {
        collaboration_mode: Some(CollaborationMode {
            mode,
            settings: Settings {
                model: fixture.session_configured.model.clone(),
                reasoning_effort: None,
                developer_instructions: None,
            },
        }),
        ..Default::default()
    };
    submit_thread_settings(&fixture.codex, settings(ModeKind::Plan)).await?;
    fs::write(
        inbox.path(),
        format!("{}\n", json!({"content":"queued-in-plan"})),
    )?;
    sleep(Duration::from_millis(200)).await;
    assert!(response.requests().is_empty());
    submit_thread_settings(&fixture.codex, settings(ModeKind::Default)).await?;
    wait_for_event(&fixture.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    assert!(serde_json::to_string(&response.single_request().input())?.contains("queued-in-plan"));
    fixture.codex.submit(Op::Shutdown).await?;
    Ok(())
}

/// Run against an unmodified checkout of Anthropic's fakechat server.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires CODEX_FAKECHAT_BUN and CODEX_FAKECHAT_SERVER"]
async fn official_fakechat_round_trip() -> anyhow::Result<()> {
    use futures::SinkExt;
    use futures::StreamExt;
    use tokio_tungstenite::tungstenite::Message;

    let bun = std::env::var("CODEX_FAKECHAT_BUN")?;
    let script = std::env::var("CODEX_FAKECHAT_SERVER")?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    let server = start_mock_server().await;
    let response = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse(vec![
                responses::ev_function_call_with_namespace(
                    "reply",
                    "mcp__fakechat",
                    "reply",
                    r#"{"text":"Codex channel reply"}"#,
                ),
                responses::ev_completed("tool"),
            ]),
            responses::sse(vec![responses::ev_completed("done")]),
        ],
    )
    .await;
    let fixture = test_codex()
        .with_config(move |config| {
            config.permissions.approval_policy =
                codex_config::Constrained::allow_any(AskForApproval::Never);
            let mut transport = stdio_transport(
                bun,
                Some(HashMap::from([(
                    "FAKECHAT_PORT".to_string(),
                    port.to_string(),
                )])),
                Vec::new(),
            );
            let McpServerTransportConfig::Stdio { args, .. } = &mut transport else {
                unreachable!()
            };
            args.push(script);
            insert_mcp_server(
                config,
                "fakechat",
                transport,
                TestMcpServerOptions::default(),
            );
            let mut servers = config.mcp_servers.get().clone();
            let fakechat = servers.get_mut("fakechat").expect("fakechat");
            fakechat.channel = true;
            fakechat.default_tools_approval_mode = Some(codex_config::AppToolApproval::Approve);
            config.mcp_servers.set(servers).expect("fakechat config");
        })
        .build_with_auto_env(&server)
        .await?;
    wait_for_mcp_server(&fixture.codex, "fakechat").await?;
    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")).await?;
    socket
        .send(Message::Text(
            json!({"id":"proof-1", "text":"hello from fakechat"})
                .to_string()
                .into(),
        ))
        .await?;
    let reply = tokio::time::timeout(Duration::from_secs(20), socket.next())
        .await?
        .expect("fakechat reply")?
        .into_text()?;
    let reply: Value = serde_json::from_str(&reply)?;
    assert_eq!(reply["from"], "assistant");
    assert_eq!(reply["text"], "Codex channel reply");
    wait_for_event(&fixture.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    assert!(
        serde_json::to_string(&response.requests()[0].input())?.contains("hello from fakechat")
    );
    socket.close(/*msg*/ None).await?;
    fixture.codex.submit(Op::Shutdown).await?;
    Ok(())
}
