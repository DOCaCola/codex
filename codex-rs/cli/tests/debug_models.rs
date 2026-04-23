use std::path::Path;

use anyhow::Result;
use predicates::str::contains;
use tempfile::TempDir;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

fn codex_command(codex_home: &Path) -> Result<assert_cmd::Command> {
    let mut cmd = assert_cmd::Command::new(codex_utils_cargo_bin::cargo_bin("codex")?);
    cmd.env("CODEX_HOME", codex_home);
    Ok(cmd)
}

#[test]
fn debug_models_bundled_json_prints_json() -> Result<()> {
    let codex_home = TempDir::new()?;
    let mut cmd = codex_command(codex_home.path())?;
    let output = cmd
        .args(["debug", "models", "--bundled", "--json"])
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    let value: serde_json::Value = serde_json::from_str(&stdout)?;
    assert!(value["models"].is_array());
    assert!(!value["models"].as_array().unwrap_or(&Vec::new()).is_empty());

    Ok(())
}

#[test]
fn debug_models_default_prints_human_summary_without_auth() -> Result<()> {
    let codex_home = TempDir::new()?;
    let mut cmd = codex_command(codex_home.path())?;
    cmd.args(["debug", "models"])
        .assert()
        .success()
        .stdout(contains("Source: active (bundled + cache/remote:"))
        .stdout(contains("/models?client_version="))
        .stdout(contains("Models: "))
        .stdout(contains("slug"))
        .stdout(contains("display"));

    Ok(())
}

#[test]
fn debug_models_json_prints_catalog_json_without_auth() -> Result<()> {
    let codex_home = TempDir::new()?;
    let mut cmd = codex_command(codex_home.path())?;
    let output = cmd.args(["debug", "models", "--json"]).output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    let value: serde_json::Value = serde_json::from_str(&stdout)?;
    assert!(value["models"].is_array());
    assert!(!value["models"].as_array().unwrap_or(&Vec::new()).is_empty());

    Ok(())
}

#[tokio::test]
async fn debug_models_remote_json_fetches_active_model_provider_url() -> Result<()> {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/codex/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_raw(r#"{"models":[]}"#, "application/json"),
        )
        .mount(&server)
        .await;

    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join("config.toml"),
        format!(
            r#"model_provider = "custom-models"

[model_providers.custom-models]
name = "OpenAI"
base_url = "{}/api/codex"
wire_api = "responses"
requires_openai_auth = true
"#,
            server.uri()
        ),
    )?;

    let mut cmd = codex_command(codex_home.path())?;
    let output = cmd
        .env("OPENAI_API_KEY", "test-api-key")
        .args(["debug", "models", "--remote-json"])
        .output()?;

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    let value: serde_json::Value = serde_json::from_str(&stdout)?;
    assert_eq!(value, serde_json::json!({ "models": [] }));

    let requests = server.received_requests().await.unwrap_or_default();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/api/codex/models");
    assert!(
        requests[0]
            .url
            .query()
            .unwrap_or("")
            .contains("client_version=")
    );

    Ok(())
}

#[tokio::test]
async fn debug_models_human_output_shows_configured_source_url() -> Result<()> {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/codex/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_raw(r#"{"models":[]}"#, "application/json"),
        )
        .mount(&server)
        .await;

    let codex_home = TempDir::new()?;
    std::fs::write(
        codex_home.path().join("config.toml"),
        format!(
            r#"model_provider = "custom-models"

[model_providers.custom-models]
name = "OpenAI"
base_url = "{}/api/codex"
wire_api = "responses"
requires_openai_auth = true
"#,
            server.uri()
        ),
    )?;

    let mut cmd = codex_command(codex_home.path())?;
    cmd.env("OPENAI_API_KEY", "test-api-key")
        .args(["debug", "models"])
        .assert()
        .success()
        .stdout(contains(format!(
            "Source: active (bundled + cache/remote: {}/api/codex/models?client_version=",
            server.uri()
        )))
        .stdout(contains("Models: 5"));

    Ok(())
}
