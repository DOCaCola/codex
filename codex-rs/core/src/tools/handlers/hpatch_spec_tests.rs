use super::*;

#[test]
fn hpatch_spec_is_compact_and_freeform() {
    let ToolSpec::Freeform(tool) = create_hpatch_freeform_tool(false) else {
        panic!("expected freeform tool");
    };
    assert_eq!(tool.name, "hpatch");
    assert!(!tool.format.definition.contains("environment_id"));
    assert!(tool.format.definition.contains("tsel_command"));
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
    assert!(tool.description.len() < 1200);
}

#[test]
fn hpatch_spec_supports_environment_selection() {
    let ToolSpec::Freeform(tool) = create_hpatch_freeform_tool(true) else {
        panic!("expected freeform tool");
    };
    assert!(tool.format.definition.contains("environment_id?"));
}
