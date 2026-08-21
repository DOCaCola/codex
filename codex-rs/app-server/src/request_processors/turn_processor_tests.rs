use super::*;
use codex_utils_path_uri::PathUri;

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
