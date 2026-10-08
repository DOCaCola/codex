//! Written, not run. Snapshot expectations are hand-authored and unverified.
use super::*;
use crate::exec_cell::CommandOutput;
use crate::exec_cell::new_active_exec_command;
use crate::history_cell::HistoryCell;
use codex_app_server_protocol::CommandExecutionSource;
use codex_protocol::parse_command::ParsedCommand;
use pretty_assertions::assert_eq;
use std::time::Duration;

fn metadata() -> CodexdcBackgroundJobTimer {
    CodexdcBackgroundJobTimer { item_id: "job".into(), process_id: "1".into(), started_at_ms: 0,
        explicit_timeout_seconds: None, duration_seconds: None }
}
fn text(lines: Vec<ratatui::text::Line<'static>>) -> String {
    lines.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n")
}
fn cell(timers: &JobTimers) -> crate::exec_cell::ExecCell {
    let mut cell = new_active_exec_command("job".into(), vec!["sleep".into(), "10".into()],
        vec![ParsedCommand::Unknown { cmd: "sleep 10".into() }], CommandExecutionSource::UnifiedExecStartup,
        /*interaction_input*/ None, /*animations_enabled*/ false);
    cell.set_job_timers(timers);
    cell
}

#[test]
fn compact_formatter_covers_seconds_minutes_hours_and_exact_limits() {
    let actual = [0, 45, 90, 192, 1800, 3840, 5400].map(compact_duration);
    assert_eq!(actual, ["0s", "45s", "1m 30s", "3m 12s", "30m", "1h 4m", "1h 30m"]);
}

#[test]
fn explicit_limit_is_shown_host_default_is_hidden_and_completion_is_frozen() {
    let mut data = metadata();
    let host_default = JobTimer::new(data.clone()).label_at(/*now_ms*/ 192_000);
    data.explicit_timeout_seconds = Some(1800);
    let explicit = JobTimer::new(data.clone()).label_at(/*now_ms*/ 192_000);
    data.explicit_timeout_seconds = Some(90);
    let short = JobTimer::new(data.clone()).label_at(/*now_ms*/ 45_000);
    data.duration_seconds = Some(192);
    let done = JobTimer::new(data);
    assert_eq!((host_default, explicit, short, done.label_at(/*now_ms*/ 192_000), done.label_at(/*now_ms*/ 999_000)),
        ("(3m 12s)".into(), "(3m 12s · timeout 30m)".into(), "(45s · timeout 1m 30s)".into(),
         "(3m 12s)".into(), "(3m 12s)".into()));
}

#[test]
fn flag_off_keeps_original_command_rendering_and_cache_behavior() {
    let mut timers = JobTimers::default();
    let original = cell(&timers).display_lines(/*width*/ 80);
    timers.update(/*enabled*/ false, vec![metadata()]);
    let disabled = cell(&timers);
    assert_eq!(disabled.display_lines(/*width*/ 80), original);
    assert_eq!((disabled.has_stable_transcript_height(), disabled.transcript_animation_tick()), (true, None));
    assert!(timers.get("job").is_none());
}

#[test]
fn shared_completion_updates_existing_cells_and_stays_frozen() {
    let mut timers = JobTimers::default();
    timers.update(/*enabled*/ true, vec![metadata()]);
    let live = timers.get("job").unwrap();
    let mut completed = metadata(); completed.duration_seconds = Some(45);
    timers.update(/*enabled*/ true, vec![completed]);
    assert_eq!((live.label_at(/*now_ms*/ 90_000), live.label_at(/*now_ms*/ 180_000)), ("(45s)".into(), "(45s)".into()));
}

#[test]
fn running_cell_and_ps_show_timer_with_explicit_limit() {
    let mut data = metadata();
    // A future start clamps elapsed to zero deterministically, as after a wall-clock adjustment.
    data.started_at_ms = i64::MAX;
    data.explicit_timeout_seconds = Some(90);
    let mut timers = JobTimers::default(); timers.update(/*enabled*/ true, vec![data]);
    let running = text(cell(&timers).display_lines(/*width*/ 100));
    let ps = crate::history_cell::new_unified_exec_processes_output(vec![crate::history_cell::UnifiedExecProcessDetails {
        command_display: "sleep 10".into(), recent_chunks: Vec::new(), timer: Some(timers.process("1")),
    }]);
    let interaction = crate::history_cell::new_unified_exec_interaction(Some("sleep 10".into()), String::new())
        .with_job_timer(Some(timers.process("1")));
    let rendered = format!("Command:\n{running}\nProcesses:\n{}\nInteraction:\n{}",
        text(ps.display_lines(/*width*/ 100)), text(interaction.transcript_lines(/*width*/ 100)));
    insta::assert_snapshot!("claude_running_command_and_ps", rendered);
}

#[test]
fn finished_timeout_cell_preserves_outcome_and_freezes_duration_on_all_views() {
    let mut data = metadata(); data.duration_seconds = Some(192); data.explicit_timeout_seconds = Some(1800);
    let mut timers = JobTimers::default(); timers.update(/*enabled*/ true, vec![data]);
    let mut cell = cell(&timers);
    assert!(cell.complete_call("job", CommandOutput::new(/*exit_code*/ 124, "timed out".into()), Duration::from_secs(192)));
    let compact = text(cell.compact_hyperlink_lines(/*width*/ 100).into_iter().map(|line| line.line).collect());
    let rendered = format!("Chat:\n{}\nOwned:\n{compact}\nTranscript:\n{}",
        text(cell.display_lines(/*width*/ 100)), text(cell.transcript_lines(/*width*/ 100)));
    insta::assert_snapshot!("claude_finished_timeout_views", rendered);
}

#[test]
fn late_metadata_updates_existing_begin_process_row_and_note() {
    let mut timers = JobTimers::default();
    let mut started = cell(&timers);
    let ps = crate::history_cell::new_unified_exec_processes_output(vec![crate::history_cell::UnifiedExecProcessDetails {
        command_display: "sleep 10".into(), recent_chunks: Vec::new(), timer: Some(timers.process("1")),
    }]);
    let note = crate::history_cell::new_unified_exec_interaction(Some("sleep 10".into()), String::new())
        .with_job_timer(Some(timers.process("1")));
    let before = [text(started.display_lines(/*width*/ 100)), text(ps.display_lines(/*width*/ 100)),
        text(note.transcript_lines(/*width*/ 100))];
    assert!(before.iter().all(|view| !view.contains("timeout")));
    let mut data = metadata(); data.started_at_ms = i64::MAX; data.explicit_timeout_seconds = Some(90);
    timers.update(/*enabled*/ true, vec![data]);
    // The widget binds active item IDs on the existing snapshot event, without delaying begin.
    started.set_job_timers(&timers);
    let after = [text(started.display_lines(/*width*/ 100)), text(ps.display_lines(/*width*/ 100)),
        text(note.transcript_lines(/*width*/ 100))];
    assert!(after.iter().all(|view| view.contains("sleep 10 (0s · timeout 1m 30s)")));
    let compact = started.compact_hyperlink_lines(/*width*/ 40);
    assert!(compact.iter().all(|line| line.line.width() <= 40));
    assert!(text(compact.into_iter().map(|line| line.line).collect()).contains("timeout 1m 30s"));
}
