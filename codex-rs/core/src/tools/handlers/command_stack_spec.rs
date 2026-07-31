use codex_tools::FreeformTool;
use codex_tools::FreeformToolFormat;
use codex_tools::JsonSchema;
use codex_tools::ToolSpec;

use crate::tools::handlers::apply_patch_spec::apply_patch_lark_rules;
use crate::tools::handlers::hpatch_spec::hpatch_lark_rules;

const COMMAND_STACK_LARK_GRAMMAR: &str = include_str!("command_stack.lark");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommandStackPatchTool {
    ApplyPatch,
    Hpatch,
}

impl CommandStackPatchTool {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::ApplyPatch => "apply_patch",
            Self::Hpatch => "hpatch",
        }
    }
}

pub(crate) fn create_command_stack_tool(
    exec_arguments: JsonSchema,
    write_stdin_arguments: JsonSchema,
    include_environment_id: bool,
    patch_tool: CommandStackPatchTool,
) -> ToolSpec {
    let (patch_rule, patch_rules) = match patch_tool {
        CommandStackPatchTool::ApplyPatch => {
            let environment = include_environment_id
                .then_some("environment_id? ")
                .unwrap_or("");
            (
                format!(
                    "patch_operation: \"apply_patch\" stack_step? STACK_NL stack_apply_patch_input \"end\"\nstack_apply_patch_input: begin_patch {environment}hunk+ stack_end_patch\nstack_end_patch: \"*** End Patch\" STACK_NL"
                ),
                apply_patch_lark_rules(include_environment_id),
            )
        }
        CommandStackPatchTool::Hpatch => {
            let environment = include_environment_id
                .then_some("environment_id? ")
                .unwrap_or("");
            (
                format!(
                    "patch_operation: \"hpatch\" stack_step? STACK_NL stack_hpatch_input STACK_NL stack_blank_line* \"end\"\nstack_hpatch_input: {environment}working_directory? blank_line* script"
                ),
                hpatch_lark_rules(include_environment_id),
            )
        }
    };
    let definition = format!("{COMMAND_STACK_LARK_GRAMMAR}\n{patch_rule}\n\n{patch_rules}");
    let exec_arguments = argument_summary("exec_command", &exec_arguments);
    let write_stdin_arguments = argument_summary("write_stdin", &write_stdin_arguments);
    let patch_body = match patch_tool {
        CommandStackPatchTool::ApplyPatch => {
            "An apply_patch body is a complete patch from `*** Begin Patch` through `*** End Patch`."
        }
        CommandStackPatchTool::Hpatch => {
            "An hpatch body may start with a working-directory header, then uses `in`/`new`, `tsel`/`rsel`, and `type`/`del` commands. `tsel` selects only matching text; use `rsel N:N` for a whole line."
        }
    };

    ToolSpec::Freeform(FreeformTool {
        name: "command_stack".to_string(),
        description: format!(
            "Runs 1-8 operations across up to 4 dependency steps. Every operation is a block: write `exec_command`, `write_stdin`, or `{patch_tool}` followed by optional ` step=N`; `exec_command` and `write_stdin` may also add ` continue_on_failure`. Put the body on following lines and terminate it with `end`. Step defaults to 1. Exec and stdin bodies are JSON argument objects. {exec_arguments} {write_stdin_arguments} Commands in one step must be independent; steps run in order. Patch operations run serially and always stop on failure. {patch_body}",
            patch_tool = patch_tool.name(),
        ),
        format: FreeformToolFormat {
            r#type: "grammar".to_string(),
            syntax: "lark".to_string(),
            definition,
        },
    })
}

fn argument_summary(tool_name: &str, schema: &JsonSchema) -> String {
    let required = schema.required.as_deref().unwrap_or_default();
    let optional = schema
        .properties
        .as_ref()
        .map(|properties| {
            properties
                .keys()
                .filter(|name| !required.contains(name))
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let required = required
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>();

    match (required.is_empty(), optional.is_empty()) {
        (false, false) => format!(
            "`{tool_name}` requires {}; optional keys: {}.",
            required.join(", "),
            optional.join(", ")
        ),
        (false, true) => format!("`{tool_name}` requires {}.", required.join(", ")),
        (true, false) => format!("`{tool_name}` has optional keys: {}.", optional.join(", ")),
        (true, true) => format!("`{tool_name}` takes a JSON object."),
    }
}

#[cfg(test)]
#[path = "command_stack_spec_tests.rs"]
mod tests;
