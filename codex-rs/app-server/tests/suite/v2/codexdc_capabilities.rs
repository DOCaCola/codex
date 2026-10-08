use anyhow::Result;
use app_test_support::DEFAULT_CLIENT_NAME;
use app_test_support::TestAppServer;
use app_test_support::to_response;
use codex_app_server_protocol::ClientInfo;
use codex_app_server_protocol::CodexdcCapabilitiesParams;
use codex_app_server_protocol::CodexdcCapabilitiesResponse;
use codex_app_server_protocol::CodexdcFeatures;
use codex_app_server_protocol::InitializeCapabilities;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::JSONRPCMessage;
use codex_app_server_protocol::RequestId;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::time::Duration;
use tempfile::TempDir;
use tokio::process::Command;
use tokio::time::timeout;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::test]
async fn capabilities_identify_the_running_build() -> Result<()> {
    let codex_home = TempDir::new()?;
    let mut server = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .build_initialized()
        .await?;
    let request_id = server
        .send_request(
            "codexdc/capabilities",
            Some(serde_json::to_value(CodexdcCapabilitiesParams::default())?),
        )
        .await?;
    let response = timeout(
        DEFAULT_TIMEOUT,
        server.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;

    let version_output = timeout(
        DEFAULT_TIMEOUT,
        Command::new(codex_utils_cargo_bin::cargo_bin("codex-app-server")?)
            .arg("--version")
            .output(),
    )
    .await??;
    assert!(version_output.status.success());
    let version_output = String::from_utf8(version_output.stdout)?;
    let version = version_output
        .trim()
        .strip_prefix("codex-app-server ")
        .ok_or_else(|| anyhow::anyhow!("unexpected binary version: {version_output}"))?;
    assert_eq!(
        to_response::<CodexdcCapabilitiesResponse>(response)?,
        CodexdcCapabilitiesResponse {
            fork: "codexdc".to_string(),
            version: version.to_string(),
            protocol: BTreeMap::from([("backgroundJobs".to_string(), 1), ("backgroundWork".to_string(), 1)]),
            features: CodexdcFeatures {
                background_jobs: false
            },
        }
    );
    for enabled in [true, false] {
        std::fs::write(
            codex_home.path().join("config.toml"),
            format!("[features]\nbackground_jobs = {enabled}\n"),
        )?;
        let request_id = server
            .send_request("codexdc/capabilities", Some(serde_json::json!({})))
            .await?;
        let response = timeout(
            DEFAULT_TIMEOUT,
            server.read_stream_until_response_message(RequestId::Integer(request_id)),
        )
        .await??;
        assert_eq!(
            to_response::<CodexdcCapabilitiesResponse>(response)?.features,
            CodexdcFeatures {
                background_jobs: enabled
            }
        );
    }
    Ok(())
}

#[tokio::test]
async fn capabilities_require_experimental_api() -> Result<()> {
    let codex_home = TempDir::new()?;
    let mut server = TestAppServer::builder()
        .with_codex_home(codex_home.path())
        .build()
        .await?;
    let initialized = server
        .initialize_with_capabilities(
            ClientInfo {
                name: DEFAULT_CLIENT_NAME.to_string(),
                title: None,
                version: "0.1.0".to_string(),
            },
            Some(InitializeCapabilities::default()),
        )
        .await?;
    assert!(matches!(initialized, JSONRPCMessage::Response(_)));

    let request_id = server
        .send_request(
            "codexdc/capabilities",
            Some(serde_json::to_value(CodexdcCapabilitiesParams::default())?),
        )
        .await?;
    let error = timeout(
        DEFAULT_TIMEOUT,
        server.read_stream_until_error_message(RequestId::Integer(request_id)),
    )
    .await??;
    assert_eq!(
        error.error,
        JSONRPCErrorError {
            code: -32600,
            message: "codexdc/capabilities requires experimentalApi capability".to_string(),
            data: None,
        }
    );
    Ok(())
}
