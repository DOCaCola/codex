use codex_tools::FreeformTool;
use codex_tools::FreeformToolFormat;
use codex_tools::ToolSpec;

const HPATCH_LARK_GRAMMAR: &str = include_str!("hpatch.lark");

pub(crate) fn create_hpatch_freeform_tool(include_environment_id: bool) -> ToolSpec {
    let definition = if include_environment_id {
        HPATCH_LARK_GRAMMAR.to_string()
    } else {
        HPATCH_LARK_GRAMMAR
            .replace("environment_id? ", "")
            .replace("environment_id: \"*** Environment ID: \" PATH NL\n", "")
    };
    ToolSpec::Freeform(FreeformTool {
        name: "hpatch".to_string(),
        description: "Edit files with a compact baseline-oriented script. Select an existing file with `in PATH` or create one with `new PATH`; select text with `tsel LINE \"TEXT\" [COUNT]` or whole lines with `rsel START:END`; then use `type`, `del`, `copy`, `cut`, `paste`, or `mv`. Multiline text uses `type <<PATCH`. Submit one complete script directly, without JSON or shell wrapping.".to_string(),
        format: FreeformToolFormat {
            r#type: "grammar".to_string(),
            syntax: "lark".to_string(),
            definition,
        },
    })
}

#[cfg(test)]
#[path = "hpatch_spec_tests.rs"]
mod tests;
