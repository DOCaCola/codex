use super::*;
use codex_protocol_test_support::ContentItemTestExt;

#[test]
fn still_running_notice_is_bounded_for_many_long_commands() {
    let command = "🦀".repeat(10_000);
    let commands = (0..100).map(|id| (i32::MIN + id, command.as_str(), Duration::MAX));
    let body = CommandsStillRunning::new(Duration::MAX, commands)
        .body()
        .expect_text();
    assert!(body.len() + 64 < 3000, "{}", body.len());
    assert!(body.contains("- 92 more sessions"));
}

#[test]
fn still_running_notice_lists_sessions_and_explains_retrieval() {
    let commands = [
        (
            7816,
            "ssh lab 'snapshot status'",
            Duration::from_secs(3_120),
        ),
        (94937, "ssh lab 'strace probe'", Duration::from_secs(3_178)),
    ];
    insta::assert_snapshot!(CommandsStillRunning::new(Duration::from_secs(600), commands.into_iter()).body().expect_text(), @r#"
    No command finished during 600s of waiting, so this turn no longer waits for these running command sessions and will not receive their results automatically:
    - session 7816, running 3120s: ssh lab 'snapshot status'
    - session 94937, running 3178s: ssh lab 'strace probe'
    If a result is still required, use write_stdin with chars="" and wait_mode="completion" to wait for it, or read current output with write_stdin. Otherwise continue; the commands keep running as background sessions.
    "#);
}
