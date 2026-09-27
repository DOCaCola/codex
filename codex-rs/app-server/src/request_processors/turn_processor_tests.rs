use super::*;
use codex_utils_path_uri::PathUri;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn repeated_default_workspace_preserves_owner_configuration_and_updates_task_roots()
-> anyhow::Result<()> {
    use codex_protocol::config_types::WindowsSandboxLevel;
    use codex_protocol::models::PermissionProfileSnapshot;
    use codex_protocol::protocol::EnvironmentConfig;
    use codex_protocol::protocol::ThreadSettingsOverrides;
    use core_test_support::submit_thread_settings;
    use core_test_support::test_codex::test_codex;

    let server = core_test_support::responses::start_mock_server().await;
    let test = test_codex().build_with_auto_env(&server).await?;
    let cwd = test.config.cwd.clone();
    let roots = vec![cwd.clone()];
    let mut environments = test
        .thread_manager
        .default_environment_selections(&cwd, &roots);
    let owner_config = EnvironmentConfig {
        allow_login_shell: false,
        workspace_roots: environments[0].workspace_roots.clone(),
        permission_profile: PermissionProfileSnapshot::legacy(
            test.config.permissions.permission_profile().clone(),
        ),
        shell_environment_policy: test.config.permissions.shell_environment_policy.clone(),
        windows_sandbox_level: WindowsSandboxLevel::from_config(&test.config),
        windows_sandbox_type: test.config.permissions.windows_sandbox_type,
        use_legacy_landlock: test.config.features.use_legacy_landlock(),
        exec_policy: None,
        mcp_policy: None,
        network_policy: None,
        selected_capability_roots: vec![],
    };
    for config in [
        EnvironmentConfigState::Pending,
        EnvironmentConfigState::Ready(owner_config),
        EnvironmentConfigState::Failed("owner unavailable".into()),
    ] {
        environments[0].config = config;
        submit_thread_settings(
            &test.codex,
            ThreadSettingsOverrides {
                environments: Some(TurnEnvironmentSelections::new(
                    cwd.clone(),
                    environments.clone(),
                )),
                runtime_workspace_roots: Some(vec![]),
                ..Default::default()
            },
        )
        .await?;

        let overrides = TurnRequestProcessor::build_environment_override(
            &test.thread_manager,
            &test.codex,
            Some(cwd.clone()),
            Some(roots.clone()),
            /*environment_selections*/ None,
        )
        .await;
        assert_eq!(overrides.environments, None);
        assert_eq!(overrides.runtime_workspace_roots, Some(roots.clone()));
        test.codex
            .preview_thread_settings_overrides(CodexThreadSettingsOverrides {
                environments: overrides.environments.clone(),
                runtime_workspace_roots: overrides.runtime_workspace_roots.clone(),
                ..Default::default()
            })
            .await?;
        submit_thread_settings(
            &test.codex,
            ThreadSettingsOverrides {
                environments: overrides.environments,
                runtime_workspace_roots: overrides.runtime_workspace_roots,
                ..Default::default()
            },
        )
        .await?;
        assert_eq!(test.codex.environment_selections().await, environments);
        assert_eq!(
            test.codex
                .thread_settings_snapshot()
                .await
                .runtime_workspace_roots,
            Some(roots.clone())
        );

        // A real attachment change must still pass the core ownership guard.
        let changed = TurnRequestProcessor::build_environment_override(
            &test.thread_manager,
            &test.codex,
            Some(cwd.join("other-workspace")),
            Some(roots.clone()),
            /*environment_selections*/ None,
        )
        .await;
        assert!(
            test.codex
                .preview_thread_settings_overrides(CodexThreadSettingsOverrides {
                    environments: changed.environments,
                    runtime_workspace_roots: changed.runtime_workspace_roots,
                    ..Default::default()
                })
                .await
                .is_err()
        );
    }
    Ok(())
}

fn selection(
    environment_id: &str,
    cwd: &AbsolutePathBuf,
    config: EnvironmentConfigState,
) -> TurnEnvironmentSelection {
    let cwd = PathUri::from_abs_path(cwd);
    TurnEnvironmentSelection {
        environment_id: environment_id.to_string(),
        cwd: cwd.clone(),
        workspace_roots: vec![cwd],
        config,
    }
}

#[test]
fn same_environment_attachments_ignores_configuration_ownership() {
    let cwd = AbsolutePathBuf::current_dir().expect("cwd");
    let current = vec![selection(
        LOCAL_ENVIRONMENT_ID,
        &cwd,
        EnvironmentConfigState::Pending,
    )];
    let requested = vec![selection(
        LOCAL_ENVIRONMENT_ID,
        &cwd,
        EnvironmentConfigState::FromThread,
    )];

    assert!(same_environment_attachments(&current, &requested));
}

#[test]
fn same_environment_attachments_detects_a_new_cwd() {
    let cwd = AbsolutePathBuf::current_dir().expect("cwd");
    let current = vec![selection(
        LOCAL_ENVIRONMENT_ID,
        &cwd,
        EnvironmentConfigState::Pending,
    )];
    let requested = vec![selection(
        LOCAL_ENVIRONMENT_ID,
        &cwd.join("new-cwd"),
        EnvironmentConfigState::FromThread,
    )];

    assert!(!same_environment_attachments(&current, &requested));
}
