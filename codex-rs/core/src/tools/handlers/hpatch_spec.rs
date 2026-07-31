use codex_tools::FreeformTool;
use codex_tools::FreeformToolFormat;
use codex_tools::ToolSpec;

const HPATCH_LARK_GRAMMAR: &str = include_str!("hpatch.lark");

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
        description: "Edit files with a compact baseline-oriented script. Use an optional `*** Working Directory: PATH` header to change the path base. Select files with `in PATH` or `new PATH`, text with `tsel LINE \"TEXT\" [COUNT]` or lines with `rsel START:END`; `tsel` selects only the matching text, so use `rsel N:N` to replace a whole line. Then edit with `type`, `del`, `copy`, `cut`, `paste`, or `mv`. Multiline text uses `type <<PATCH`. Submit one complete script without JSON or shell wrapping.".to_string(),
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
