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
        description: "Edit files with a compact baseline-oriented script. Use an optional `*** Working Directory: PATH` header to change the path base. Select files with `in PATH` or `new PATH`, text with `tsel LINE \"TEXT\" [COUNT]` or lines with `rsel START:END`, then edit with `type`, `del`, `copy`, `cut`, `paste`, or `mv`. Multiline text uses `type <<PATCH`. Submit one complete script without JSON or shell wrapping.".to_string(),
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
