const START_MARKER: &str = "<tool_surface_instructions>";
const END_MARKER: &str = "</tool_surface_instructions>";
const LEGACY_HPATCH_START_MARKER: &str = "<hpatch_instructions>";
const LEGACY_HPATCH_END_MARKER: &str = "</hpatch_instructions>";

const DIRECT_HPATCH_BODY: &str = "When `hpatch` is available without `command_stack`, use it directly for manual file edits. Earlier base-instruction references to `apply_patch` are superseded; do not call or shell-wrap `apply_patch`.";
const STACK_HPATCH_BODY: &str = " For `hpatch` bodies, start with `in PATH` or `new PATH`; select with `tsel FROM_LINE \"TEXT\" [COUNT]` or `rsel START:END`. `tsel` selects only matching text; use `rsel N:N` for a whole line. Then use `type`, `del`, `copy`, `cut`, `paste`, `mv`, or `rm`; use `type <<PATCH` for multiline text.";

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
        let mut body = format!(
            "When `command_stack` is available, it is the model-visible interface for shell, terminal continuation, and manual file-edit operations. Earlier base-instruction references to calling `exec_command`, `write_stdin`, `apply_patch`, or `hpatch` directly are superseded; use `exec_command`, `write_stdin`, and `{patch_tool}` only as `command_stack` operation types. Write each operation as `<type> [step=N] [continue_on_failure]`, put its body on following lines, then write `end`; step defaults to 1. `exec_command` and `write_stdin` bodies are JSON argument objects. A single operation is valid. Commands in the same step must be independent; use a later call when an operation depends on unseen output."
        );
        if hpatch_enabled {
            body.push_str(STACK_HPATCH_BODY);
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

        assert!(configured.contains("and `apply_patch` only as `command_stack` operation types"));
        assert!(configured.contains("put its body on following lines, then write `end`"));
        assert!(configured.contains("step defaults to 1"));
        assert!(!configured.contains("For `hpatch` bodies"));
    }

    #[test]
    fn command_stack_and_hpatch_compose_into_one_compact_section() {
        let configured = configure_tool_surface_instructions("base".to_string(), true, true);

        assert!(configured.contains("and `hpatch` only as `command_stack` operation types"));
        assert!(configured.contains("For `hpatch` bodies"));
        assert!(configured.contains("put its body on following lines, then write `end`"));
        assert_eq!(configured.matches(START_MARKER).count(), 1);
        assert!(configured.len() - "base".len() < 1100);
    }

    #[test]
    fn reconfiguration_is_idempotent_and_tracks_current_mode() {
        let stacked = configure_tool_surface_instructions("base".to_string(), true, false);
        let repeated = configure_tool_surface_instructions(stacked.clone(), true, false);
        let switched = configure_tool_surface_instructions(stacked, true, true);

        assert_eq!(repeated.matches(START_MARKER).count(), 1);
        assert!(repeated.contains("and `apply_patch` only"));
        assert!(switched.contains("and `hpatch` only"));
        assert!(!switched.contains("and `apply_patch` only"));
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
