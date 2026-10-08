//! Written, not run. The new snapshot is hand-authored and unverified.
use super::*;
use crate::background_job_timer::JobTimer;
use codex_app_server_protocol::CodexdcBackgroundJobTimer;
use pretty_assertions::assert_eq;

#[test]
fn background_footer_and_inline_summary_show_explicit_job_timer() {
    let mut footer = UnifiedExecFooter::new();
    footer.set_processes(vec!["sleep 10".into()]);
    let original = footer.summary_text();
    footer.set_job_timers(Vec::new());
    assert_eq!(footer.summary_text(), original);
    footer.set_job_timers(vec![JobTimer::new(CodexdcBackgroundJobTimer {
        item_id: "job".into(), process_id: "1".into(), started_at_ms: i64::MAX,
        explicit_timeout_seconds: Some(1800), duration_seconds: None,
    })]);
    let rendered = footer.summary_text().unwrap();
    insta::assert_snapshot!("claude_background_footer_summary", rendered);
}
