use codex_tools::JsonSchema;
use codex_tools::ToolSpec;
use serde_json::json;

pub(super) fn configure_exec(tool: &mut ToolSpec) {
    let ToolSpec::Function(tool) = tool else {
        return;
    };
    tool.description.push_str(
        "\nFinite commands deliver completion automatically. Continue independent work without polling. \
         When blocked, use write_stdin with wait_mode=\"completion\". The runtime keeps the turn open \
         while commands run. A started command is not a completed task. Use execution_mode=\"interactive\" \
         for servers, watchers, REPLs and other persistent work.",
    );
    let properties = tool.parameters.properties.get_or_insert_default();
    properties.insert("execution_mode".into(), JsonSchema::string_enum(
        vec![json!("auto"), json!("foreground"), json!("background"), json!("interactive")],
        Some("auto (default) waits initially then delivers completion; foreground waits until exit; background returns promptly and delivers completion; interactive uses manual reads. tty=true defaults to interactive.".into()),
    ));
    properties.insert("timeout_ms".into(), JsonSchema::number(Some(
        "Positive execution deadline for finite commands, including background time. Foreground defaults to 300000 ms; auto/background have no implicit deadline. This is separate from yield_time_ms.".into(),
    )));
    if let Some(yield_time) = properties.get_mut("yield_time_ms") {
        yield_time.description.get_or_insert_default().push_str(
            " Applies only to auto/interactive; background returns promptly and foreground waits until exit or its execution deadline.",
        );
    }
    configure_result(tool);
}

pub(super) fn configure_wait(tool: &mut ToolSpec) {
    let ToolSpec::Function(tool) = tool else {
        return;
    };
    tool.description.push_str(
        "\nFinite commands deliver results automatically. Avoid status polling. When blocked, use \
         wait_mode=\"completion\" to wait in runtime code until exit. Empty timed checks back off \
         after unchanged results. Completed command output remains readable by session_id in a bounded \
         session-local cache; reads do not trigger another completion.",
    );
    tool.parameters.properties.get_or_insert_default().insert("wait_mode".into(), JsonSchema::string_enum(
        vec![json!("timed"), json!("completion")],
        Some("timed (default) returns output after yield_time_ms; completion waits for a finite command to exit and requires empty chars. Do not combine completion with yield_time_ms.".into()),
    ));
    configure_result(tool);
}

fn configure_result(tool: &mut codex_tools::ResponsesApiTool) {
    if let Some(schema) = tool.output_schema.take() {
        let mut schema = schema.into_value();
        if let Some(properties) = schema
            .get_mut("properties")
            .and_then(serde_json::Value::as_object_mut)
        {
            properties.insert("completion_delivery".into(), json!({
                "type": "boolean", "description": "Whether a running command will deliver its completion automatically."
            }));
        }
        tool.output_schema = Some(schema.into());
    }
}
