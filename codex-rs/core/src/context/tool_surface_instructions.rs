use crate::tools::handlers::HPATCH_SELECTOR_GUIDANCE;

const START_MARKER: &str = "<tool_surface_instructions>";
const END_MARKER: &str = "</tool_surface_instructions>";
const LEGACY_HPATCH_START_MARKER: &str = "<hpatch_instructions>";
const LEGACY_HPATCH_END_MARKER: &str = "</hpatch_instructions>";

const DIRECT_HPATCH_BODY: &str = "When `hpatch` is available without `command_stack`, use it directly for manual file edits. Earlier base-instruction references to `apply_patch` are superseded; do not call or shell-wrap `apply_patch`.";
const STACK_HPATCH_BODY: &str = " For `hpatch` bodies use `in`/`new`, `tsel`/`rsel`, then `type`/`del`/`copy`/`cut`/`paste`/`mv`/`rm`. `type <<PATCH` is literal multiline file text. Follow structural edits with a focused build or check in `step=2`.";
const STACK_BATCHING_BODY: &str = r#" Tool calls are expensive. Include every currently known useful operation in one call. Independent operations must share the same step. Use later steps only for predetermined dependencies, and make another call only when the next operation depends on unseen output. Use one operation only when no other useful operation is currently known. Examples:
```text
exec_command
{"cmd":"git status --short"}
end

exec_command
{"cmd":"rg -n TODO src"}
end
```
The two operations above share step 1 and run together.
```text
exec_command
{"cmd":"cargo build"}
end

exec_command step=2
{"cmd":"cargo test"}
end
```
The step 2 operation waits for step 1."#;

pub(crate) fn configure_tool_surface_instructions(
    base: String,
    command_stack_enabled: bool,
    hpatch_enabled: bool,
) -> String {
    let base = strip_marked_section(base, LEGACY_HPATCH_START_MARKER, LEGACY_HPATCH_END_MARKER);
    let base = strip_marked_section(base, START_MARKER, END_MARKER);
    let Some(body) = instruction_body(command_stack_enabled, hpatch_enabled) else {
        return base;
    };

    if base.is_empty() {
        format!("{START_MARKER}\n{body}\n{END_MARKER}")
    } else {
        format!("{base}\n\n{START_MARKER}\n{body}\n{END_MARKER}")
    }
}

fn instruction_body(command_stack_enabled: bool, hpatch_enabled: bool) -> Option<String> {
    if command_stack_enabled {
        let patch_tool = if hpatch_enabled {
            "hpatch"
        } else {
            "apply_patch"
        };
        let workdir_guidance = if hpatch_enabled {
            "Optional leading `workdir PATH` defaults exec and hpatch operations; explicit operation workdirs override it."
        } else {
            "Optional leading `workdir PATH` defaults exec operations; apply_patch paths stay session-relative."
        };
        let mut body = format!(
            "When `command_stack` is available, use it for shell, terminal continuation, and manual edits; direct `exec_command`, `write_stdin`, `apply_patch`, and `hpatch` references are superseded. Use only `exec_command`, `write_stdin`, and `{patch_tool}` operation blocks: `<type> [step=N] [continue_on_failure]`, body, `end`; step defaults to 1 and exec/stdin bodies are JSON objects.{STACK_BATCHING_BODY} {workdir_guidance}"
        );
        if hpatch_enabled {
            body.push_str(STACK_HPATCH_BODY);
            body.push(' ');
            body.push_str(HPATCH_SELECTOR_GUIDANCE);
        }
        Some(body)
    } else if hpatch_enabled {
        Some(DIRECT_HPATCH_BODY.to_string())
    } else {
        None
    }
}

fn strip_marked_section(base: String, start_marker: &str, end_marker: &str) -> String {
    let Some(start) = base.find(start_marker) else {
        return base;
    };
    let Some(end_offset) = base[start..].find(end_marker) else {
        return base;
    };
    let end = start + end_offset + end_marker.len();
    let prefix = base[..start].trim_end();
    let suffix = base[end..].trim_start();

    match (prefix.is_empty(), suffix.is_empty()) {
        (true, true) => String::new(),
        (true, false) => suffix.to_string(),
        (false, true) => prefix.to_string(),
        (false, false) => format!("{prefix}\n\n{suffix}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_hpatch_supersedes_apply_patch_without_repeating_tool_syntax() {
        let configured =
            configure_tool_surface_instructions("base apply_patch text".to_string(), false, true);

        assert!(configured.contains("use it directly for manual file edits"));
        assert!(configured.contains("references to `apply_patch` are superseded"));
        assert!(!configured.contains("`tsel FROM_LINE"));
    }

    #[test]
    fn command_stack_uses_apply_patch_with_shared_block_guidance() {
        let configured = configure_tool_surface_instructions("base".to_string(), true, false);

        assert!(configured.contains("and `apply_patch` operation blocks"));
        assert!(configured.contains("Tool calls are expensive"));
        assert!(configured.contains("Independent operations must share the same step"));
        assert!(
            configured.contains(
                "Use one operation only when no other useful operation is currently known"
            )
        );
        assert!(configured.contains("The two operations above share step 1 and run together"));
        assert!(configured.contains("exec_command step=2"));
        assert!(configured.contains("The step 2 operation waits for step 1"));

        assert!(configured.contains("`workdir PATH` defaults exec operations"));
        assert!(!configured.contains("For `hpatch` bodies"));
    }

    #[test]
    fn command_stack_and_hpatch_compose_into_one_compact_section() {
        let configured = configure_tool_surface_instructions("base".to_string(), true, true);

        assert!(configured.contains("and `hpatch` operation blocks"));
        assert!(configured.contains("For `hpatch` bodies"));
        assert!(configured.contains("Tool calls are expensive"));
        assert!(configured.contains("Independent operations must share the same step"));
        assert!(
            configured.contains(
                "Use one operation only when no other useful operation is currently known"
            )
        );
        assert!(configured.contains("The two operations above share step 1 and run together"));
        assert!(configured.contains("exec_command step=2"));
        assert!(configured.contains("The step 2 operation waits for step 1"));
        assert!(configured.contains("`workdir PATH` defaults exec and hpatch operations"));
        assert!(configured.contains("exact plain-text search"));
        assert!(configured.contains("do not JSON-escape quotes or newlines"));
        assert!(configured.contains("verify short or repeated fragments with `rg -nF`"));
        assert!(configured.contains("Follow structural edits"));
        assert!(configured.contains("selectors use it until `commit`"));
        assert_eq!(configured.matches(START_MARKER).count(), 1);
        assert!(configured.len() - "base".len() < 2400);
    }

    #[test]
    fn reconfiguration_is_idempotent_and_tracks_current_mode() {
        let stacked = configure_tool_surface_instructions("base".to_string(), true, false);
        let repeated = configure_tool_surface_instructions(stacked.clone(), true, false);
        let switched = configure_tool_surface_instructions(stacked, true, true);

        assert_eq!(repeated.matches(START_MARKER).count(), 1);
        assert!(repeated.contains("and `apply_patch` operation blocks"));
        assert!(switched.contains("and `hpatch` operation blocks"));
        assert!(!switched.contains("and `apply_patch` operation blocks"));
    }

    #[test]
    fn removes_reserved_sections_when_features_are_disabled() {
        let configured = configure_tool_surface_instructions("base".to_string(), true, true);
        let disabled = configure_tool_surface_instructions(configured, false, false);

        assert_eq!(disabled, "base");
    }

    #[test]
    fn migrates_legacy_hpatch_section_without_duplication() {
        let legacy =
            format!("base\n\n{LEGACY_HPATCH_START_MARKER}\nlegacy\n{LEGACY_HPATCH_END_MARKER}");
        let configured = configure_tool_surface_instructions(legacy, true, true);

        assert!(!configured.contains(LEGACY_HPATCH_START_MARKER));
        assert_eq!(configured.matches(START_MARKER).count(), 1);
    }
}
