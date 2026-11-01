use anyhow::Result;
use codex_features::Feature;
use codex_login::CodexAuth;
use codex_models_manager::model_info::model_info_from_slug;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ModelsResponse;
use codex_protocol::protocol::MultiAgentVersion;
use core_test_support::responses::ev_completed;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::namespace_child_tool;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::test_codex::test_codex;
use pretty_assertions::assert_eq;
use test_case::test_case;

#[test_case(MultiAgentVersion::V1, 5; "v1_exact_cap")]
#[test_case(MultiAgentVersion::V1, 6; "v1_truncated")]
#[test_case(MultiAgentVersion::V2, 5; "v2_exact_cap")]
#[test_case(MultiAgentVersion::V2, 6; "v2_truncated")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_agent_description_discloses_truncated_model_overrides(
    multi_agent_version: MultiAgentVersion,
    visible_model_count: usize,
) -> Result<()> {
    let server = start_mock_server().await;
    let mut models = (1..=visible_model_count)
        .map(|number| {
            let slug = format!("model-{number}");
            ModelInfo {
                display_name: format!("Model {number}"),
                description: Some(format!("Model number {number}")),
                visibility: ModelVisibility::List,
                supported_in_api: true,
                used_fallback_model_metadata: false,
                multi_agent_version: Some(multi_agent_version),
                priority: number as i32,
                ..model_info_from_slug(&slug)
            }
        })
        .collect::<Vec<_>>();
    let mut hidden = models[0].clone();
    hidden.slug = "hidden-model".to_string();
    hidden.visibility = ModelVisibility::Hide;
    hidden.priority = 0;
    models.insert(0, hidden);
    if multi_agent_version == MultiAgentVersion::V2 {
        let mut disabled = models[1].clone();
        disabled.slug = "disabled-model".to_string();
        disabled.multi_agent_version = Some(MultiAgentVersion::Disabled);
        disabled.priority = 0;
        models.insert(0, disabled);
    }
    let response = mount_sse_once(&server, sse(vec![ev_completed("resp1")])).await;
    let test = test_codex()
        .with_auth(CodexAuth::create_dummy_chatgpt_auth_for_testing())
        .with_model("model-1")
        .with_config(move |config| {
            config.model_catalog = Some(ModelsResponse { models });
            config
                .features
                .enable(Feature::Collab)
                .expect("enable collaboration");
            config
                .features
                .set_enabled(
                    Feature::MultiAgentV2,
                    multi_agent_version == MultiAgentVersion::V2,
                )
                .expect("select multi-agent version");
            config
                .features
                .disable(Feature::ModelCatalogInContext)
                .expect("keep model choices in the tool description");
            config.multi_agent_v2.hide_spawn_agent_metadata = false;
        })
        .build_with_auto_env(&server)
        .await?;
    test.submit_turn("hello").await?;

    let namespace = match multi_agent_version {
        MultiAgentVersion::V1 => "multi_agent_v1",
        MultiAgentVersion::V2 => "collaboration",
        MultiAgentVersion::Disabled => unreachable!("this test requires spawning"),
    };
    let body = response.single_request().body_json();
    let description = namespace_child_tool(&body, namespace, "spawn_agent")
        .and_then(|tool| tool.get("description"))
        .and_then(serde_json::Value::as_str)
        .expect("spawn_agent description");
    let advertised_models = description
        .lines()
        .filter_map(|line| line.strip_prefix("- `"))
        .map(|line| line.split_once('`').expect("model identifier").0)
        .collect::<Vec<_>>();
    assert_eq!(
        advertised_models,
        vec!["model-1", "model-2", "model-3", "model-4", "model-5"]
    );
    let notice = "This list is truncated to 5 entries. Other catalog models may also be valid; try an explicit override before declaring a model unavailable.";
    assert_eq!(description.contains(notice), visible_model_count > 5);
    Ok(())
}
