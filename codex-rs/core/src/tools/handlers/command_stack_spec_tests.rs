use super::*;

#[test]
fn command_stack_tool_describes_batching_and_dependency_rules() {
    let exec_arguments = JsonSchema::object(
        BTreeMap::from([(
            "cmd".to_string(),
            JsonSchema::string(Some("Shell command to execute.".to_string())),
        )]),
        Some(vec!["cmd".to_string()]),
        Some(false.into()),
    );
    let ToolSpec::Function(tool) =
        create_command_stack_tool(exec_arguments, /*include_environment_id*/ false)
    else {
        panic!("expected function tool");
    };

    assert!(tool.description.contains("between 1 and 8"));
    assert!(
        tool.description
            .contains("complete arguments are already known")
    );
    assert!(tool.description.contains("output you have not seen"));
    assert!(
        tool.description
            .contains("A failed apply_patch stops all later steps")
    );

    let commands = tool
        .parameters
        .properties
        .as_ref()
        .and_then(|properties| properties.get("commands"))
        .expect("commands schema");
    assert!(
        commands
            .description
            .as_deref()
            .is_some_and(|description| description.contains("do not add filler commands"))
    );

    let command_properties = commands
        .items
        .as_deref()
        .and_then(|items| items.properties.as_ref())
        .expect("command item properties");
    assert!(
        command_properties
            .get("step")
            .and_then(|schema| schema.description.as_deref())
            .is_some_and(|description| description.contains("must not depend on one another"))
    );
    assert!(
        command_properties
            .get("arguments")
            .and_then(|schema| schema.description.as_deref())
            .is_some_and(|description| description.contains("Complete arguments"))
    );
}
