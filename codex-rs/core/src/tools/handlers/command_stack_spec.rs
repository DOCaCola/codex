use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use serde_json::json;
use std::collections::BTreeMap;

pub(crate) fn create_command_stack_tool(
    exec_arguments: JsonSchema,
    include_environment_id: bool,
) -> ToolSpec {
    let mut patch_properties = BTreeMap::from([(
        "patch".to_string(),
        JsonSchema::string(Some(
            "Complete apply_patch input, including Begin Patch and End Patch markers.".to_string(),
        )),
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
                vec![json!("exec_command"), json!("apply_patch")],
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
                    JsonSchema::object(
                        patch_properties,
                        Some(vec!["patch".to_string()]),
                        Some(false.into()),
                    ),
                ],
                Some(
                    "Complete arguments for the selected child tool. exec_command arguments must match its displayed schema; apply_patch arguments contain the complete patch."
                        .to_string(),
                ),
            ),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "command_stack".to_string(),
        description: r#"Runs between 1 and 8 exec_command and apply_patch operations across at most 4 ordered steps.
Include only operations whose complete arguments are already known. Put independent reads, searches, and other exec_command calls in the same step. Use a later step for known work that requires completed edits. If a command or patch must be chosen from output you have not seen, inspect that output first and make another command_stack call.
Independent exec_command calls in the same step may run concurrently. A step containing apply_patch runs serially. A failed apply_patch stops all later steps.
"#
        .to_string(),
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
