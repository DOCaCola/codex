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
