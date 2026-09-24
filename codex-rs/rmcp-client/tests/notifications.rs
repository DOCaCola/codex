use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use codex_rmcp_client::ElicitationAction;
use codex_rmcp_client::ElicitationResponse;
use codex_rmcp_client::LocalStdioServerLauncher;
use codex_rmcp_client::RmcpClient;
use codex_rmcp_client::SurfaceNotification;
use codex_utils_cargo_bin::CargoBinError;
use futures::FutureExt as _;
use pretty_assertions::assert_eq;
use rmcp::model::ClientCapabilities;
use rmcp::model::Implementation;
use rmcp::model::InitializeRequestParams;
use rmcp::model::ProtocolVersion;
use serde_json::json;
use tokio::sync::mpsc;

fn stdio_server_bin() -> Result<PathBuf, CargoBinError> {
    codex_utils_cargo_bin::cargo_bin("test_stdio_server")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn standard_logging_notification_is_forwarded() -> anyhow::Result<()> {
    let data = json!({
        "eventId": "event-17",
        "kind": "debug.breakpointHit",
        "payload": { "line": 42 }
    });
    let client = RmcpClient::new_stdio_client(
        stdio_server_bin()?.into(),
        Vec::<OsString>::new(),
        Some(HashMap::from([(
            "MCP_TEST_LOGGING_NOTIFICATION".into(),
            data.to_string().into(),
        )])),
        &[],
        /*cwd*/ None,
        Arc::new(LocalStdioServerLauncher::new(std::env::current_dir()?)),
    )
    .await?;
    let (notification_tx, mut notification_rx) = mpsc::channel(1);

    client
        .initialize(
            InitializeRequestParams::new(
                ClientCapabilities::default(),
                Implementation::new("codex-test", "0.0.0-test"),
            )
            .with_protocol_version(ProtocolVersion::V_2025_06_18),
            Some(Duration::from_secs(5)),
            Box::new(|_, _| {
                async {
                    Ok(ElicitationResponse {
                        action: ElicitationAction::Accept,
                        content: Some(json!({})),
                        meta: None,
                    })
                }
                .boxed()
            }),
            Some(Box::new(move |notification| {
                let notification_tx = notification_tx.clone();
                async move {
                    notification_tx.send(notification).await?;
                    Ok(())
                }
                .boxed()
            })),
        )
        .await?;

    let notification = tokio::time::timeout(Duration::from_secs(5), notification_rx.recv())
        .await?
        .expect("notification callback should remain connected");
    assert_eq!(
        notification,
        SurfaceNotification {
            method: "notifications/message".to_string(),
            source: Some("codex.test".to_string()),
            message_id: Some("event-17".to_string()),
            payload: json!({
                "level": "info",
                "logger": "codex.test",
                "data": data,
            }),
        }
    );
    Ok(())
}
