use codex_tools::JsonSchema;
use codex_tools::ToolSpec;
use serde_json::json;

pub(super) fn configure_exec(
    tool: &mut ToolSpec,
    idle_continuation: bool,
    policy: Option<crate::unified_exec::background_policy::BackgroundJobPolicy>,
) {
    let ToolSpec::Function(tool) = tool else {
        return;
    };
    if idle_continuation {
        tool.description.push_str(
            "\nAuto/background commands keep running after the turn ends. Continue independent work \
             or finish the turn; completion starts a follow-up turn unless interrupted or in Plan mode, \
             when results wait for user input. Use interactive for persistent work and foreground \
             when final results are required now.",
        );
    } else if policy.is_some() {
        tool.description.push_str(
            "\nFinite commands deliver completion automatically. Continue independent work without polling. \
             When blocked, use write_stdin with wait_mode=\"completion\". The runtime keeps the turn open \
             while commands run. A started command is not a completed task. Use execution_mode=\"interactive\" \
             for servers, watchers, REPLs and other persistent work.",
        );
    } else {
        tool.description.push_str(
            "\nFinite commands deliver completion automatically. Continue independent work without polling. \
             When blocked, use write_stdin with wait_mode=\"completion\". The runtime keeps the turn open \
             while commands run, until a time limit passes without any finishing; ending or interrupting \
             the turn also stops automatic delivery. A started command is not a completed task. Use \
             execution_mode=\"interactive\" for servers, watchers, REPLs and other persistent work.",
        );
    }
    let properties = tool.parameters.properties.get_or_insert_default();
    properties.insert("execution_mode".into(), JsonSchema::string_enum(
        vec![json!("auto"), json!("foreground"), json!("background"), json!("interactive")],
        Some("auto (default) waits initially then delivers completion; foreground waits until exit; background returns promptly and delivers completion; interactive uses manual reads. tty=true defaults to interactive.".into()),
    ));
    properties.insert("timeout_ms".into(), JsonSchema::number(Some(
        policy.map_or_else(
            || "Positive execution deadline for finite commands, including background time. Foreground defaults to 300000 ms; auto/background have no implicit deadline. This is separate from yield_time_ms.".into(),
            |policy| policy.deadline_guidance(),
        ),
    )));
    if let Some(yield_time) = properties.get_mut("yield_time_ms") {
        if let Some(policy) = policy {
            yield_time.description = Some(format!(
                "Initial auto wait, default {} ms. Only this omitted auto default bypasses the floor and clamp. Explicit yield_time_ms and interactive keep legacy runtime bounds.",
                policy.defaults.auto_yield.as_millis(),
            ).into_boxed_str());
        }
        yield_time.description = Some(format!(
            "{} Applies only to auto/interactive; background returns promptly and foreground waits until exit or its execution deadline.",
            yield_time.description.as_deref().unwrap_or_default(),
        ).into_boxed_str());
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
