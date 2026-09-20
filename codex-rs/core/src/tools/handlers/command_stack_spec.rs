use codex_tools::FreeformTool;
use codex_tools::FreeformToolFormat;
use codex_tools::JsonSchema;
use codex_tools::ToolSpec;

use crate::tools::handlers::apply_patch_spec::apply_patch_lark_rules;
use crate::tools::handlers::hpatch_spec::HPATCH_SELECTOR_GUIDANCE;
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
    let managed_guidance = if exec_arguments
        .properties
        .as_ref()
        .is_some_and(|properties| properties.contains_key("execution_mode"))
    {
        " execution_mode values: auto (default: initial wait then automatic completion delivery), background (return promptly with automatic delivery), foreground (wait until exit), interactive (manual supervision for servers/watchers; default with tty=true). timeout_ms is a positive execution deadline, separate from yield_time_ms; foreground defaults to 300000 ms and auto/background have no implicit deadline. Finite commands in earlier steps run to completion before dependent steps start. Their exit codes control continue_on_failure. Interactive commands cannot be dependencies. Final-step finite commands deliver completion automatically; continue independent work without polling. The runtime keeps the turn active while they run. Use foreground when the stack must return final results, or write_stdin wait_mode=\"completion\" when blocked (empty chars, no yield_time_ms). Empty timed reads back off; completed output remains readable via session_id in a bounded session-local cache."
    } else {
        ""
    };
    let (patch_rule, patch_rules) = match patch_tool {
        CommandStackPatchTool::ApplyPatch => {
            let environment = if include_environment_id {
                "environment_id? "
            } else {
                ""
            };
            (
                format!(
                    "patch_operation: \"apply_patch\" stack_step? STACK_NL stack_apply_patch_input \"end\"\nstack_apply_patch_input: begin_patch {environment}hunk+ stack_end_patch\nstack_end_patch: \"*** End Patch\" STACK_NL"
                ),
                apply_patch_lark_rules(include_environment_id),
            )
        }
        CommandStackPatchTool::Hpatch => {
            let environment = if include_environment_id {
                "environment_id? "
            } else {
                ""
            };
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
    let workdir_body = match patch_tool {
        CommandStackPatchTool::ApplyPatch => {
            "Optional leading `workdir PATH` supplies the default for exec operations; apply_patch paths remain session-relative."
        }
        CommandStackPatchTool::Hpatch => {
            "Optional leading `workdir PATH` supplies the default for exec and hpatch operations; explicit operation workdirs override it."
        }
    };

    let write_stdin_arguments = argument_summary("write_stdin", &write_stdin_arguments);
    let patch_body = match patch_tool {
        CommandStackPatchTool::ApplyPatch => {
            "An apply_patch body is a complete patch from `*** Begin Patch` through `*** End Patch`."
                .to_string()
        }
        CommandStackPatchTool::Hpatch => format!(
            "An hpatch body may start with a working-directory header, then uses `in`/`new`, `tsel`/`rsel`, and `type`/`del` commands. {HPATCH_SELECTOR_GUIDANCE}"
        ),
    };

    ToolSpec::Freeform(FreeformTool {
        name: "command_stack".to_string(),
        description: format!(
            "Runs 1-8 operations across up to 4 dependency steps. Tool calls are expensive: include every currently known useful operation in one call. Independent operations must share the same step. Use later steps only for predetermined dependencies, and make another call only when the next operation depends on unseen output. Use one operation only when no other useful operation is currently known. For example, omit `step` on independent blocks so they share step 1; use `step=2` for a known follow-up after step 1. Every operation is a block: write `exec_command`, `write_stdin`, or `{patch_tool}` followed by optional ` step=N`; `exec_command` and `write_stdin` may also add ` continue_on_failure`. Put the body on following lines and terminate it with `end`. Step defaults to 1. Exec and stdin bodies are JSON argument objects. {exec_arguments} {write_stdin_arguments} {workdir_body} Patch operations run serially and always stop on failure. {patch_body}{managed_guidance}",
            patch_tool = patch_tool.name(),
        ),
        defer_loading: None,
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
