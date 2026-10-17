use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use codex_rmcp_client::ElicitationAction;
use codex_rmcp_client::ElicitationResponse;
use codex_rmcp_client::LocalStdioServerLauncher;
use codex_rmcp_client::RmcpClient;
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
async fn channel_message_is_forwarded() -> anyhow::Result<()> {
    let inbox = tempfile::NamedTempFile::new()?;
    let data = json!({ "content": "Build failed", "meta": { "chat_id": "17" } });
    let client = RmcpClient::new_stdio_client(
        stdio_server_bin()?.into(),
        Vec::<OsString>::new(),
        Some(HashMap::from([(
            "MCP_TEST_CHANNEL_INBOX".into(),
            inbox.path().as_os_str().to_owned(),
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

    let malformed = json!({"content": "invalid", "meta": {"chat_id": 17}});
    let oversized = json!({"content": "x".repeat(6_001)});
    std::fs::write(inbox.path(), format!("{malformed}\n{oversized}\n{data}\n"))?;
    let notification = tokio::time::timeout(Duration::from_secs(5), notification_rx.recv())
        .await?
        .expect("notification callback should remain connected");
    assert_eq!(notification.content, "Build failed");
    assert_eq!(
        notification.meta,
        std::collections::BTreeMap::from([("chat_id".to_string(), "17".to_string())])
    );
    assert!(notification.instructions.is_some());
    let peer = notification.connection.expect("notification transport");
    assert!(!peer.is_transport_closed());
    client.shutdown().await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !peer.is_transport_closed() {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    Ok(())
}

#[tokio::test]
async fn channel_registration_requires_live_server_capability() -> anyhow::Result<()> {
    let client = RmcpClient::new_stdio_client(
        stdio_server_bin()?.into(),
        Vec::<OsString>::new(),
        None,
        &[],
        /*cwd*/ None,
        Arc::new(LocalStdioServerLauncher::new(std::env::current_dir()?)),
    )
    .await?;
    let error = client
        .initialize(
            InitializeRequestParams::new(
                ClientCapabilities::default(),
                Implementation::new("channel-test", "1"),
            )
            .with_protocol_version(ProtocolVersion::V_2025_06_18),
            Some(Duration::from_secs(5)),
            Box::new(|_, _| async { anyhow::bail!("unexpected elicitation") }.boxed()),
            Some(Box::new(|_| async { Ok(()) }.boxed())),
        )
        .await
        .expect_err("server must declare channel capability");
    assert!(error.to_string().contains("claude/channel"));
    client.shutdown().await;
    Ok(())
}
