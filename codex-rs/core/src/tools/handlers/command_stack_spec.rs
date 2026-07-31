use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use serde_json::json;
use std::collections::BTreeMap;

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

    fn input_key(self) -> &'static str {
        match self {
            Self::ApplyPatch => "patch",
            Self::Hpatch => "script",
        }
    }

    fn input_description(self) -> &'static str {
        match self {
            Self::ApplyPatch => {
                "Complete apply_patch input, including Begin Patch and End Patch markers."
            }
            Self::Hpatch => "Complete hpatch editing script.",
        }
    }
}

pub(crate) fn create_command_stack_tool(
    exec_arguments: JsonSchema,
    write_stdin_arguments: JsonSchema,
    include_environment_id: bool,
    patch_tool: CommandStackPatchTool,
) -> ToolSpec {
    let mut patch_properties = BTreeMap::from([(
        patch_tool.input_key().to_string(),
        JsonSchema::string(Some(patch_tool.input_description().to_string())),
    )]);
    if include_environment_id {
        patch_properties.insert(
            "environment_id".to_string(),
            JsonSchema::string(Some("Environment in which to apply the patch.".to_string())),
        );
    }

    let command_properties = BTreeMap::from([
        (
            "command_type".to_string(),
            JsonSchema::string_enum(
                vec![
                    json!("exec_command"),
                    json!("write_stdin"),
                    json!(patch_tool.name()),
                ],
                Some("Child tool to invoke.".to_string()),
            ),
        ),
        (
            "step".to_string(),
            JsonSchema::integer(Some(
                "Dependency group from 1 through 4. Steps run in ascending order. Commands in the same step must not depend on one another."
                    .to_string(),
            )),
        ),
        (
            "arguments".to_string(),
            JsonSchema::any_of(
                vec![
                    exec_arguments,
                    write_stdin_arguments,
                    JsonSchema::object(
                        patch_properties,
                        Some(vec![patch_tool.input_key().to_string()]),
                        Some(false.into()),
                    ),
                ],
                Some(
                    format!("Complete arguments for the selected child tool. exec_command and write_stdin arguments must match their displayed schemas; {} arguments contain the complete edit input.", patch_tool.name()),
                ),
            ),
        ),
        (
            "continue_on_failure".to_string(),
            JsonSchema::boolean(Some(
                format!("For exec_command and write_stdin only, continue to later steps if this command fails. Defaults to false. {} failures always stop execution.", patch_tool.name()),
            )),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "command_stack".to_string(),
        description: format!(r#"Runs between 1 and 8 exec_command, write_stdin, and {patch_tool_name} operations across at most 4 ordered steps.
Include only operations whose complete arguments are already known. Put independent reads, searches, and other exec_command calls in the same step. Use a later step for known work that requires completed edits. If a command or patch must be chosen from output you have not seen, inspect that output first and make another command_stack call.
Use write_stdin in a later command_stack call after exec_command returns a session id. Independent exec_command and write_stdin calls in the same step may run concurrently. A step containing {patch_tool_name} runs serially. A failed command stops later steps by default; continue_on_failure may override this for exec_command and write_stdin. A failed {patch_tool_name} always stops execution.
"#, patch_tool_name = patch_tool.name()),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            BTreeMap::from([(
                "commands".to_string(),
                JsonSchema::array(
                    JsonSchema::object(
                        command_properties,
                        Some(vec![
                            "command_type".to_string(),
                            "step".to_string(),
                            "arguments".to_string(),
                        ]),
                        Some(false.into()),
                    ),
                    Some(
                        "Between 1 and 8 commands. A single command is valid; do not add filler commands."
                            .to_string(),
                    ),
                ),
            )]),
            Some(vec!["commands".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

#[cfg(test)]
#[path = "command_stack_spec_tests.rs"]
mod tests;
