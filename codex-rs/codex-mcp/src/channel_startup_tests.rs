use super::*;
use pretty_assertions::assert_eq;

#[test]
fn channel_opt_in_changes_connection_identity() {
    let runtime = reusable_server_runtime_context();
    let mut config = reusable_server_config("http://localhost/mcp");
    let ordinary = reusable_server_identity("docs", &config, &runtime);
    config.channel = true;
    let channel = reusable_server_identity("docs", &config, &runtime);
    assert!(!ordinary.has_same_connection_config(&channel));
}

#[tokio::test]
async fn channel_events_require_the_current_registration() {
    let mut manager = McpConnectionSet::new_uninitialized(
        &Constrained::allow_any(AskForApproval::OnRequest),
        &Constrained::allow_any(PermissionProfile::default()),
        /*prefix_mcp_tool_names*/ true,
    );
    manager.insert_test_client("docs", create_ready_async_managed_client(Vec::new()).await);
    let registration = Arc::clone(&manager.test_client("docs").channel_registration);
    let mut message = crate::McpChannelMessage {
        server_name: "docs".to_string(),
        registration: Arc::clone(&registration),
        notification: codex_rmcp_client::ChannelNotification {
            content: "event".to_string(),
            meta: Default::default(),
            instructions: None,
            connection: None,
        },
    };
    assert!(manager.accepts_channel_message(&message));
    message.registration = Arc::new(());
    assert!(!manager.accepts_channel_message(&message));
    message.registration = registration;
    manager.servers.clear();
    assert!(!manager.accepts_channel_message(&message));
}

#[tokio::test]
async fn cached_channel_catalog_does_not_defer_live_startup() -> anyhow::Result<()> {
    let home = tempdir()?;
    let environments = Arc::new(environment_manager_without_environments());
    environments.upsert_environment(
        "channel-fixture".to_string(),
        "ws://127.0.0.1:1".to_string(),
        /*connect_timeout*/ None,
    )?;
    let runtime_context = McpRuntimeContext::new(environments, home.path().to_path_buf());
    let cache = McpToolCatalogCache::default();
    for channel in [false, true] {
        let server_config: McpServerConfig = serde_json::from_value(serde_json::json!({
            "command": "unused-channel-fixture",
            "channel": channel,
            "environment_id": "channel-fixture",
        }))?;
        let input = || {
            let cancellation = CancellationToken::new();
            cancellation.cancel();
            McpRuntimeInput {
                startup_policy: McpStartupPolicy::LazyWhenCached,
                config: Arc::new(crate::mcp::tests::test_mcp_config(
                    home.path().to_path_buf(),
                )),
                plugins_available: false,
                ready_selected_capability_roots: Vec::new(),
                mcp_servers: HashMap::from([(
                    "docs".to_string(),
                    EffectiveMcpServer::from_host_config(server_config.clone()),
                )]),
                submit_id: "test".to_string(),
                tx_event: None,
                startup_cancellation_token: cancellation,
                runtime_context: runtime_context.clone(),
                codex_apps_tools_cache: ConnectorRuntimeManager::default(),
                tool_catalog_cache: cache.clone(),
                codex_apps_tools_cache_key: ConnectorRuntimeContextKey::personal(
                    /*account_id*/ None, /*chatgpt_user_id*/ None,
                ),
                client_mcp_extensions: ClientMcpExtensions::default(),
                auth: None,
                auth_manager: None,
                elicitation_reviewer: None,
                elicitation_lifecycle: None,
                mcp_channel_sender: crate::rmcp_client::ignore_mcp_channel_messages(),
            }
        };
        let first = McpConnectionSet::new(
            /*previous*/ None,
            McpPublicationGate::already_published(),
            input(),
            ElicitationRequestRouter::default(),
        )
        .await;
        let cache_context = first.servers["docs"]
            .connection
            .client
            .tool_catalog_cache_context
            .as_ref()
            .expect("cache context");
        cache_context.publish_if_newest(
            cache_context.begin_fetch(),
            &[create_test_tool("docs", "cached_tool")],
        );
        // A new registration can share its catalog without sharing a live connection.
        let previous = McpConnectionSet::new_uninitialized(
            &Constrained::allow_any(AskForApproval::OnRequest),
            &Constrained::allow_any(PermissionProfile::default()),
            /*prefix_mcp_tool_names*/ true,
        );
        let refreshed = McpConnectionSet::new(
            Some(&previous),
            McpPublicationGate::already_published(),
            input(),
            ElicitationRequestRouter::default(),
        )
        .await;
        assert_eq!(
            refreshed.servers["docs"].connection.startup_is_dormant(),
            !channel,
        );
    }
    Ok(())
}
