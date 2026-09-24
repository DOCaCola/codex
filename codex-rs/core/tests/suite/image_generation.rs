use std::sync::Arc;

use anyhow::Context;
use anyhow::Result;
use codex_core::config::Config;
use codex_extension_api::ExtensionRegistry;
use codex_extension_api::ExtensionRegistryBuilder;
use codex_features::Feature;
use codex_image_generation_extension::install;
use codex_login::CodexAuth;
use codex_protocol::openai_models::InputModality;
use core_test_support::responses;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;
use pretty_assertions::assert_eq;
use serde_json::json;
use wiremock::Mock;
use wiremock::ResponseTemplate;
use wiremock::matchers::header;
use wiremock::matchers::method;
use wiremock::matchers::path;

fn image_extensions(auth: &CodexAuth) -> Arc<ExtensionRegistry<Config>> {
    let auth_manager = codex_core::test_support::auth_manager_from_auth(auth.clone());
    let mut builder = ExtensionRegistryBuilder::new();
    install(&mut builder, auth_manager, |_config| None);
    Arc::new(builder.build())
}

#[derive(Clone, Copy, Debug)]
enum ProviderAuth {
    LocalChatgpt,
    BearerToken,
    ActorAuthorization,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn image_generation_uses_request_auth_for_plan_gating_without_probes() -> Result<()> {
    skip_if_no_network!(Ok(()));

    for (provider_auth, plan, feature_enabled, expected_visible) in [
        (ProviderAuth::LocalChatgpt, "free", true, false),
        (ProviderAuth::LocalChatgpt, "plus", true, true),
        (ProviderAuth::BearerToken, "free", true, true),
        (ProviderAuth::ActorAuthorization, "free", true, true),
        (ProviderAuth::BearerToken, "free", false, false),
    ] {
        let server = responses::start_mock_server().await;
        let response_mock = responses::mount_sse_once(
            &server,
            responses::sse(vec![
                responses::ev_response_created("resp-1"),
                responses::ev_completed("resp-1"),
            ]),
        )
        .await;
        let auth = CodexAuth::from_external_chatgpt_tokens(
            "header.e30.signature",
            "local-account",
            Some(plan),
        )?;
        let extensions = image_extensions(&auth);
        let mut builder = test_codex()
            .with_auth(auth)
            .with_extensions(extensions)
            .with_model_info_override("gpt-5.4", |model| {
                model.input_modalities = vec![InputModality::Text, InputModality::Image];
            })
            .with_config(move |config| {
                match provider_auth {
                    ProviderAuth::LocalChatgpt => {}
                    ProviderAuth::BearerToken => {
                        config.model_provider.experimental_bearer_token =
                            Some("gateway-key".into());
                    }
                    ProviderAuth::ActorAuthorization => {
                        config.model_provider.requires_openai_auth = false;
                        config.model_provider.http_headers = Some(
                            [("x-openai-actor-authorization".into(), "codex-lb".into())]
                                .into_iter()
                                .collect(),
                        );
                    }
                }
                if !feature_enabled {
                    config.features.disable(Feature::ImageGeneration).unwrap();
                }
            });
        let test = builder.build_with_auto_env(&server).await?;
        test.submit_turn("Hello").await?;

        assert_eq!(
            response_mock
                .single_request()
                .tool_by_name("image_gen", "imagegen")
                .is_some(),
            expected_visible,
            "{provider_auth:?}, {plan}, feature_enabled={feature_enabled}"
        );
        let requests = server.received_requests().await.unwrap();
        assert!(
            requests
                .iter()
                .all(|request| !request.url.path().contains("/images/")),
            "tool discovery must not probe image generation"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn image_generation_gateway_rejection_is_returned_to_model() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    Mock::given(method("POST"))
        .and(path("/v1/images/generations"))
        .and(header("authorization", "Bearer gateway-key"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "error": {
                "message": "Image generation is not available for this upstream account",
                "type": "permission_error",
                "code": "image_generation_not_supported"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let response_mock = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse(vec![
                responses::ev_response_created("resp-1"),
                responses::ev_function_call_with_namespace(
                    "image-call",
                    "image_gen",
                    "imagegen",
                    &json!({"prompt": "A blue circle on a white background"}).to_string(),
                ),
                responses::ev_completed("resp-1"),
            ]),
            responses::sse(vec![
                responses::ev_response_created("resp-2"),
                responses::ev_assistant_message("msg-1", "The server rejected image generation."),
                responses::ev_completed("resp-2"),
            ]),
        ],
    )
    .await;
    let auth = CodexAuth::from_external_chatgpt_tokens(
        "header.e30.signature",
        "local-account",
        Some("free"),
    )?;
    let extensions = image_extensions(&auth);
    let mut builder = test_codex()
        .with_auth(auth)
        .with_extensions(extensions)
        .with_model_info_override("gpt-5.4", |model| {
            model.input_modalities = vec![InputModality::Text, InputModality::Image];
        })
        .with_config(|config| {
            config.model_provider.experimental_bearer_token = Some("gateway-key".into());
        });
    let test = builder.build_with_auto_env(&server).await?;
    test.submit_turn("Generate an image of a blue circle")
        .await?;

    let requests = response_mock.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].tool_by_name("image_gen", "imagegen").is_some());
    let output = requests[1]
        .function_call_output_content_and_success("image-call")
        .and_then(|(content, _)| content)
        .context("image rejection should be returned as tool output")?;
    assert!(output.starts_with("image generation failed:"), "{output}");
    assert!(
        output.contains("Image generation is not available for this upstream account"),
        "{output}"
    );
    let requests = server.received_requests().await.unwrap();
    let image_requests: Vec<_> = requests
        .iter()
        .filter(|request| request.url.path().contains("/images/"))
        .collect();
    assert_eq!(image_requests.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&image_requests[0].body)?;
    assert_eq!(body["model"], "gpt-image-2");
    assert_eq!(body["prompt"], "A blue circle on a white background");
    Ok(())
}
