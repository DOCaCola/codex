use super::*;

#[test]
fn parses_optional_environment_header() {
    let (environment, workdir, script) = parse_input(
        "*** Environment ID: remote-1\nin src/lib.rs\ntsel 1 \"old\"\ntype \"new\"",
        true,
    )
    .expect("valid input");
    assert_eq!(environment.as_deref(), Some("remote-1"));
    assert_eq!(workdir, None);
    assert_eq!(script, "in src/lib.rs\ntsel 1 \"old\"\ntype \"new\"");
}

#[test]
fn parses_workdir_after_environment_header() {
    let (environment, workdir, script) = parse_input(
        "*** Environment ID: remote-1\n*** Working Directory: codex\nin src/lib.rs\ncommit",
        true,
    )
    .expect("valid input");
    assert_eq!(environment.as_deref(), Some("remote-1"));
    assert_eq!(workdir.as_deref(), Some("codex"));
    assert_eq!(script, "in src/lib.rs\ncommit");
}

#[test]
fn workdir_changes_translation_cwd_but_preserves_workspace_root() {
    let default_cwd = PathUri::parse("file:///workspace").expect("valid cwd");
    let workspace_root = default_cwd.clone();

    let (cwd, root) =
        resolve_translation_paths(&default_cwd, &[workspace_root.clone()], Some("codex"))
            .expect("valid paths");

    assert_eq!(cwd, PathUri::parse("file:///workspace/codex").unwrap());
    assert_eq!(root, workspace_root);
}

#[test]
fn inserts_environment_header_into_translated_patch() {
    let patch = "*** Begin Patch\n*** Add File: hello.txt\n+hello\n*** End Patch\n";
    assert_eq!(
        insert_environment_id(patch.to_string(), Some("remote-1")).expect("valid patch"),
        "*** Begin Patch\n*** Environment ID: remote-1\n*** Add File: hello.txt\n+hello\n*** End Patch\n"
    );
}

#[test]
fn truncates_translator_errors_on_utf8_boundaries() {
    let error = "x".repeat(MAX_TRANSLATOR_ERROR_BYTES - 1) + "éé";
    let truncated = truncate_error(&error);
    assert!(truncated.ends_with("[output truncated]"));
    assert!(truncated.is_char_boundary(truncated.len()));
}

#[tokio::test]
async fn hooks_capture_the_selected_hpatch_environment_and_reject_redirection() {
    use crate::environment_selection::TurnEnvironmentState;
    use crate::session::step_context::StepContext;
    use crate::session::tests::make_session_and_context;
    use crate::tools::context::ToolCallSource;
    use crate::turn_diff_tracker::TurnDiffTracker;
    use pretty_assertions::assert_eq;
    use tokio::sync::Mutex;
    use tokio_util::sync::CancellationToken;

    let (session, turn) = make_session_and_context().await;
    let turn = Arc::new(turn);
    let mut step_context = StepContext::for_test(Arc::clone(&turn));
    let primary = step_context
        .environments
        .primary()
        .unwrap()
        .selection
        .environment_id
        .clone();
    let mut other = step_context.environments.primary().unwrap().clone();
    other.selection.environment_id = "other".to_string();
    Arc::get_mut(&mut step_context)
        .unwrap()
        .environments
        .environments
        .push(TurnEnvironmentState::Ready(other));
    let handler = HpatchHandler::new(/*multi_environment*/ true);
    let mut invocation = ToolInvocation {
        session: session.into(),
        turn,
        step_context,
        cancellation_token: CancellationToken::new(),
        tracker: Arc::new(Mutex::new(TurnDiffTracker::new())),
        call_id: "hpatch-hook".to_string(),
        tool_name: ToolName::plain("hpatch"),
        source: ToolCallSource::Direct,
        payload: ToolPayload::Custom {
            input: "*** Environment ID: other\nin file.txt\ncommit".to_string(),
        },
        environments: ToolEnvironments::default(),
    };
    invocation.environments = handler.environments_for_input(&invocation).unwrap();
    assert_eq!(invocation.environments.hook_ids(), vec!["other"]);
    for (target, accepted) in [
        (Some("other"), true),
        (None, true),
        (Some(primary.as_str()), false),
    ] {
        let command = target.map_or_else(
            || "in changed.txt\ncommit".to_string(),
            |target| format!("*** Environment ID: {target}\nin changed.txt\ncommit"),
        );
        let rewritten = handler
            .with_updated_hook_input(invocation.clone(), serde_json::json!({"command": command}));
        if accepted {
            let rewritten = rewritten.unwrap();
            assert_eq!(rewritten.environments.hook_ids(), vec!["other"]);
            assert_eq!(
                selected_executor(
                    &rewritten.environments,
                    &rewritten.step_context.environments
                )
                .unwrap()
                .selection
                .environment_id,
                "other"
            );
        } else {
            assert!(
                matches!(rewritten, Err(FunctionCallError::RespondToModel(message)) if message == "PreToolUse hook cannot change the tool environment")
            );
        }
    }
}
