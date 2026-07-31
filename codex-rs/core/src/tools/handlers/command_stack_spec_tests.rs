use super::*;

fn arguments(required: &str, optional: &[&str]) -> JsonSchema {
    let properties = std::iter::once(required)
        .chain(optional.iter().copied())
        .map(|name| (name.to_string(), JsonSchema::default()))
        .collect();
    JsonSchema::object(
        properties,
        Some(vec![required.to_string()]),
        Some(false.into()),
    )
}

#[test]
fn command_stack_uses_one_freeform_block_surface_with_apply_patch() {
    let exec_arguments = arguments("cmd", &["workdir", "shell"]);
    let write_stdin_arguments = arguments("session_id", &["chars", "yield_time_ms"]);
    let ToolSpec::Freeform(tool) = create_command_stack_tool(
        exec_arguments,
        write_stdin_arguments,
        /*include_environment_id*/ false,
        CommandStackPatchTool::ApplyPatch,
    ) else {
        panic!("expected freeform tool");
    };

    assert!(tool.description.contains("Every operation is a block"));
    assert!(tool.description.contains("Step defaults to 1"));
    assert!(
        tool.description
            .contains("Exec and stdin bodies are JSON argument objects")
    );
    assert!(tool.description.contains("`exec_command` requires `cmd`"));
    assert!(
        tool.description
            .contains("optional keys: `shell`, `workdir`")
    );
    assert!(
        tool.description
            .contains("`write_stdin` requires `session_id`")
    );
    assert!(tool.description.contains("terminate it with `end`"));
    assert!(tool.description.contains("Patch operations run serially"));
    assert!(tool.format.definition.contains("exec_command_operation"));
    assert!(tool.format.definition.contains("write_stdin_operation"));
    assert!(
        tool.format
            .definition
            .contains("patch_operation: \"apply_patch\"")
    );
    assert!(tool.format.definition.contains("stack_step"));
    assert!(tool.format.definition.contains("JSON_STRING"));
    assert!(tool.format.definition.contains("*** Begin Patch"));
    assert!(
        !tool
            .format
            .definition
            .contains("patch_operation: \"hpatch\"")
    );
    assert!(!tool.format.definition.contains("environment_id:"));
}

#[test]
fn command_stack_keeps_the_same_surface_when_hpatch_is_selected() {
    let exec_arguments = arguments("cmd", &["environment_id", "tty"]);
    let write_stdin_arguments = arguments("session_id", &["chars"]);
    let ToolSpec::Freeform(tool) = create_command_stack_tool(
        exec_arguments,
        write_stdin_arguments,
        /*include_environment_id*/ true,
        CommandStackPatchTool::Hpatch,
    ) else {
        panic!("expected freeform tool");
    };

    assert!(tool.description.contains("Every operation is a block"));
    assert!(
        tool.description
            .contains("Exec and stdin bodies are JSON argument objects")
    );
    assert!(
        tool.description
            .contains("optional keys: `environment_id`, `tty`")
    );
    assert!(tool.description.contains("An hpatch body"));
    assert!(tool.format.definition.contains("exec_command_operation"));
    assert!(tool.format.definition.contains("write_stdin_operation"));
    assert!(
        tool.format
            .definition
            .contains("patch_operation: \"hpatch\"")
    );
    assert!(tool.format.definition.contains("working_directory"));
    assert!(tool.format.definition.contains("tsel_command"));
    assert!(tool.format.definition.contains("environment_id:"));
    assert!(
        !tool
            .format
            .definition
            .contains("patch_operation: \"apply_patch\"")
    );
}
