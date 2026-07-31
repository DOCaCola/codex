const START_MARKER: &str = "<hpatch_instructions>";
const END_MARKER: &str = "</hpatch_instructions>";

const BODY: &str = r#"When the `hpatch` tool is available, use it for manual file edits. Earlier references to `apply_patch` in these base instructions are superseded for model-authored edits; do not call `apply_patch` or route hpatch through a shell.

hpatch uses immutable file baselines. Start with `in PATH` for an existing file or `new PATH`; select exact text with `tsel FROM_LINE "TEXT" [COUNT]` or inclusive full lines with `rsel START:END`; then use `type`, `del`, `copy`, `cut`, `paste`, or `mv`. Use `type <<PATCH` for multiline text. Submit one complete script per attempt. If coordinates depend on unseen command output or newly generated text, inspect first and use a later call. After success, run focused validation."#;

pub(crate) fn append_hpatch_instructions(base: String) -> String {
    if base.contains(START_MARKER) {
        return base;
    }
    if base.is_empty() {
        return format!("{START_MARKER}\n{BODY}\n{END_MARKER}");
    }
    format!("{base}\n\n{START_MARKER}\n{BODY}\n{END_MARKER}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_once_and_supersedes_apply_patch_wording() {
        let once = append_hpatch_instructions("base apply_patch text".to_string());
        let twice = append_hpatch_instructions(once.clone());
        assert_eq!(once, twice);
        assert!(once.contains("superseded"));
        assert_eq!(once.matches(START_MARKER).count(), 1);
    }
}
