use super::*;
use crate::tools::handlers::ExecCommandHandler;
use crate::tools::registry::ToolExecutor;
use pretty_assertions::assert_eq;

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
        /*idle_continuation*/ false,
        /*policy*/ None,
    ) else {
        panic!("expected freeform tool");
    };

    assert!(tool.description.contains("Every operation is a block"));
    assert!(tool.description.contains("Tool calls are expensive"));
    assert!(
        tool.description
            .contains("Independent operations must share the same step")
    );
    assert!(
        tool.description
            .contains("Use one operation only when no other useful operation is currently known")
    );
    assert!(
        tool.description
            .contains("omit `step` on independent blocks")
    );
    assert!(
        tool.description
            .contains("use `step=2` for a known follow-up")
    );
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
    assert!(
        tool.description
            .contains("default for exec operations; apply_patch paths remain session-relative")
    );
    assert!(tool.description.contains("Patch operations run serially"));
    assert!(tool.format.definition.contains("exec_command_operation"));
    assert!(tool.format.definition.contains("write_stdin_operation"));
    assert!(
        tool.format
            .definition
            .contains("patch_operation: \"apply_patch\"")
    );
    assert!(tool.format.definition.contains("stack_workdir"));
    assert!(tool.format.definition.contains("STACK_PATH"));
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
        /*idle_continuation*/ false,
        /*policy*/ None,
    ) else {
        panic!("expected freeform tool");
    };

    assert!(tool.description.contains("Every operation is a block"));
    assert!(tool.description.contains("Tool calls are expensive"));
    assert!(
        tool.description
            .contains("Independent operations must share the same step")
    );
    assert!(
        tool.description
            .contains("Use one operation only when no other useful operation is currently known")
    );
    assert!(
        tool.description
            .contains("omit `step` on independent blocks")
    );
    assert!(
        tool.description
            .contains("use `step=2` for a known follow-up")
    );
    assert!(
        tool.description
            .contains("Exec and stdin bodies are JSON argument objects")
    );
    assert!(
        tool.description
            .contains("optional keys: `environment_id`, `tty`")
    );
    assert!(tool.description.contains("An hpatch body"));
    assert!(tool.description.contains("exact plain-text search"));
    assert!(
        tool.description
            .contains("do not JSON-escape quotes or newlines")
    );
    assert!(
        tool.description
            .contains("verify short or repeated fragments with `rg -nF`")
    );
    assert!(tool.description.contains("selectors use it until `commit`"));
    assert!(
        tool.description
            .contains("default for exec and hpatch operations")
    );
    assert!(tool.format.definition.contains("exec_command_operation"));
    assert!(tool.format.definition.contains("write_stdin_operation"));
    assert!(
        tool.format
            .definition
            .contains("patch_operation: \"hpatch\"")
    );
    assert!(tool.format.definition.contains("stack_workdir"));
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

// Actual exec and stack specs select their captured capability.
#[test_case::test_case(false; "parked_turn")]
#[test_case::test_case(true; "idle_continuation")]
fn command_stack_and_exec_instructions_select_turn_lifetime(enabled: bool) {
    let ToolSpec::Function(exec) = ExecCommandHandler::default()
        .with_background_delivery(/*enabled*/ true)
        .with_idle_continuation(enabled)
        .spec()
    else {
        panic!("expected exec function tool");
    };
    assert_eq!(
        exec.description.contains("The runtime keeps the turn open"),
        !enabled
    );
    assert_eq!(
        exec.description
            .contains("completion starts a follow-up turn"),
        enabled
    );
    let ToolSpec::Freeform(stack) = create_command_stack_tool(
        exec.parameters,
        arguments("session_id", &["chars"]),
        /*include_environment_id*/ false,
        CommandStackPatchTool::ApplyPatch,
        enabled,
        /*policy*/ None,
    ) else {
        panic!("expected freeform tool");
    };
    assert_eq!(
        stack
            .description
            .contains("The runtime keeps the turn active while they run."),
        !enabled
    );
    assert_eq!(
        stack
            .description
            .contains("completion starts a follow-up turn"),
        enabled
    );
}
