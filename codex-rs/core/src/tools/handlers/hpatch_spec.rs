use codex_tools::FreeformTool;
use codex_tools::FreeformToolFormat;
use codex_tools::ToolSpec;

const HPATCH_LARK_GRAMMAR: &str = include_str!("hpatch.lark");

pub(crate) const HPATCH_SELECTOR_GUIDANCE: &str = "`tsel FROM_LINE \"TEXT\" [COUNT]` is exact and non-syntax-aware; `TEXT` must be a nonempty fragment on one logical line. Never encode `\\n` or a multiline block in `TEXT`. Start at stable non-whitespace content, avoid leading indentation, choose `FROM_LINE` near the intended occurrence, and verify short or repeated text with `rg -nF`. Use `rsel START:END` for complete lines, multiline regions, or indentation-sensitive edits. The first `in` freezes an immutable baseline; selectors use it until `commit`. Use `commit` only when later selectors must target content changed earlier in the same script. Rejection changes nothing.";

fn configured_hpatch_grammar(include_environment_id: bool) -> String {
    if include_environment_id {
        HPATCH_LARK_GRAMMAR.to_string()
    } else {
        HPATCH_LARK_GRAMMAR
            .replace("environment_id? ", "")
            .replace("environment_id: \"*** Environment ID: \" PATH NL\n", "")
    }
}

pub(crate) fn hpatch_lark_rules(include_environment_id: bool) -> String {
    configured_hpatch_grammar(include_environment_id)
        .split_once('\n')
        .map(|(_, rules)| rules.to_string())
        .expect("hpatch grammar must start with a start rule")
}

pub(crate) fn create_hpatch_freeform_tool(include_environment_id: bool) -> ToolSpec {
    ToolSpec::Freeform(FreeformTool {
        name: "hpatch".to_string(),
        description: format!(
            "Edit files with a compact baseline-oriented script. Use an optional `*** Working Directory: PATH` header to change the path base. Select files with `in PATH` or `new PATH`. {HPATCH_SELECTOR_GUIDANCE} Then edit with `type`, `del`, `copy`, `cut`, `paste`, or `mv`. Multiline replacement text uses `type <<PATCH`. Submit one complete script without JSON or shell wrapping."
        ),
        defer_loading: None,
        format: FreeformToolFormat {
            r#type: "grammar".to_string(),
            syntax: "lark".to_string(),
            definition: configured_hpatch_grammar(include_environment_id),
        },
    })
}

#[cfg(test)]
#[path = "hpatch_spec_tests.rs"]
mod tests;
