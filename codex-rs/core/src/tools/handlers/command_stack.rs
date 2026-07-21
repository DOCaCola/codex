use std::collections::BTreeMap;
use std::sync::Arc;

use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::ResponseInputItem;
use codex_tools::JsonSchema;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use futures::future::join_all;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::command_stack_spec::create_command_stack_tool;
use crate::tools::handlers::parse_arguments;
use crate::tools::parallel::ToolCallRuntime;
use crate::tools::registry::AnyToolResult;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;
use crate::tools::router::ToolCall;
use crate::tools::router::ToolRouter;

const MAX_COMMANDS: usize = 8;
const MAX_STEP: u8 = 4;

pub(crate) struct CommandStackHandler {
    child_router: Arc<ToolRouter>,
    exec_arguments: JsonSchema,
    write_stdin_arguments: JsonSchema,
    include_environment_id: bool,
}

impl CommandStackHandler {
    pub(crate) fn new(
        child_router: Arc<ToolRouter>,
        exec_arguments: JsonSchema,
        write_stdin_arguments: JsonSchema,
        include_environment_id: bool,
    ) -> Self {
        Self {
            child_router,
            exec_arguments,
            write_stdin_arguments,
            include_environment_id,
        }
    }
}

impl ToolExecutor<ToolInvocation> for CommandStackHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("command_stack")
    }

    fn spec(&self) -> ToolSpec {
        create_command_stack_tool(
            self.exec_arguments.clone(),
            self.write_stdin_arguments.clone(),
            self.include_environment_id,
        )
    }

    fn handle(&self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'_> {
        Box::pin(self.handle_call(invocation))
    }
}

impl CoreToolRuntime for CommandStackHandler {
    fn waits_for_runtime_cancellation(&self) -> bool {
        true
    }

    fn pre_tool_use_payload(
        &self,
        _invocation: &ToolInvocation,
    ) -> Option<crate::tools::registry::PreToolUsePayload> {
        None
    }

    fn post_tool_use_payload(
        &self,
        _invocation: &ToolInvocation,
        _result: &dyn crate::tools::context::ToolOutput,
    ) -> Option<crate::tools::registry::PostToolUsePayload> {
        None
    }
}

impl CommandStackHandler {
    async fn handle_call(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Box<dyn crate::tools::context::ToolOutput>, FunctionCallError> {
        let ToolPayload::Function { arguments } = &invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "command_stack received unsupported payload".to_string(),
            ));
        };
        let args: CommandStackArgs = parse_arguments(arguments)?;
        validate_commands(&args.commands)?;
        let tool_runtime = ToolCallRuntime::new(
            Arc::clone(&self.child_router),
            Arc::clone(&invocation.session),
            Arc::clone(&invocation.step_context),
            Arc::clone(&invocation.tracker),
        );

        let mut grouped = BTreeMap::<u8, Vec<(usize, CommandItem)>>::new();
        for (index, command) in args.commands.into_iter().enumerate() {
            grouped
                .entry(command.step)
                .or_default()
                .push((index, command));
        }

        let mut results = Vec::new();
        let mut stopped = false;
        for commands in grouped.into_values() {
            let contains_patch = commands
                .iter()
                .any(|(_, command)| command.command_type == CommandType::ApplyPatch);
            let step_executions = if contains_patch {
                let mut step_executions = Vec::with_capacity(commands.len());
                for (index, command) in commands {
                    let execution = self
                        .execute_command(&invocation, tool_runtime.clone(), index, command)
                        .await;
                    let stop_current_step = execution.stop_current_step;
                    step_executions.push(execution);
                    if stop_current_step {
                        break;
                    }
                }
                step_executions
            } else {
                join_all(commands.into_iter().map(|(index, command)| {
                    self.execute_command(&invocation, tool_runtime.clone(), index, command)
                }))
                .await
            };
            let step_blocks_later = step_executions
                .iter()
                .any(|execution| execution.blocks_later_steps);
            results.extend(
                step_executions
                    .into_iter()
                    .map(|execution| execution.result),
            );
            stopped = step_blocks_later;
            if stopped {
                break;
            }
        }
        results.sort_by_key(|result| result.index);

        let success = results.iter().all(|result| result.success);
        let output =
            serde_json::to_string(&CommandStackOutput { results, stopped }).map_err(|err| {
                FunctionCallError::Fatal(format!("serialize command_stack output: {err}"))
            })?;
        Ok(boxed_tool_output(FunctionToolOutput::from_text(
            output,
            Some(success),
        )))
    }

    async fn execute_command(
        &self,
        parent: &ToolInvocation,
        tool_runtime: ToolCallRuntime,
        index: usize,
        command: CommandItem,
    ) -> CommandExecution {
        let command_type = command.command_type;
        let step = command.step;
        let continue_on_failure = command.continue_on_failure;
        let child = match build_child_call(&parent.call_id, index, command) {
            Ok(child) => child,
            Err(error) => {
                return CommandExecution::from_result(
                    CommandResult {
                        index,
                        step,
                        command_type,
                        success: false,
                        output: error.to_string(),
                    },
                    continue_on_failure,
                );
            }
        };

        CommandExecution::from_result(
            normalize_child_result(
                index,
                step,
                command_type,
                tool_runtime
                    .handle_tool_call_with_source(
                        child,
                        parent.source.clone(),
                        parent.cancellation_token.clone(),
                    )
                    .await,
            ),
            continue_on_failure,
        )
    }
}

#[derive(Deserialize)]
struct CommandStackArgs {
    commands: Vec<CommandItem>,
}

#[derive(Clone, Deserialize)]
struct CommandItem {
    command_type: CommandType,
    step: u8,
    arguments: Value,
    #[serde(default)]
    continue_on_failure: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CommandType {
    ExecCommand,
    WriteStdin,
    ApplyPatch,
}

#[derive(Serialize)]
struct CommandStackOutput {
    results: Vec<CommandResult>,
    stopped: bool,
}

#[derive(Serialize)]
struct CommandResult {
    index: usize,
    step: u8,
    command_type: CommandType,
    success: bool,
    output: String,
}

struct CommandExecution {
    result: CommandResult,
    blocks_later_steps: bool,
    stop_current_step: bool,
}

impl CommandExecution {
    fn from_result(result: CommandResult, continue_on_failure: bool) -> Self {
        let failed = !result.success;
        let patch_failed = failed && result.command_type == CommandType::ApplyPatch;
        Self {
            blocks_later_steps: failed && (patch_failed || !continue_on_failure),
            stop_current_step: patch_failed,
            result,
        }
    }
}

fn validate_commands(commands: &[CommandItem]) -> Result<(), FunctionCallError> {
    if commands.is_empty() || commands.len() > MAX_COMMANDS {
        return Err(FunctionCallError::RespondToModel(format!(
            "command_stack requires between 1 and {MAX_COMMANDS} commands"
        )));
    }
    if let Some(command) = commands
        .iter()
        .find(|command| command.step == 0 || command.step > MAX_STEP)
    {
        return Err(FunctionCallError::RespondToModel(format!(
            "command_stack step {} is outside the supported range 1..={MAX_STEP}",
            command.step
        )));
    }
    if commands.iter().any(|command| {
        command.command_type == CommandType::ApplyPatch && command.continue_on_failure
    }) {
        return Err(FunctionCallError::RespondToModel(
            "apply_patch does not support continue_on_failure".to_string(),
        ));
    }
    for command in commands.iter().filter(|command| {
        command.command_type == CommandType::ExecCommand
            && command
                .arguments
                .get("shell")
                .and_then(Value::as_str)
                .is_some_and(|shell| !shell.is_empty())
    }) {
        if commands
            .iter()
            .filter(|other| other.step == command.step)
            .count()
            > 1
        {
            return Err(FunctionCallError::RespondToModel(format!(
                "exec_command with `shell` must be the only command in step {}; later steps inherit the selected local shell",
                command.step
            )));
        }
    }
    Ok(())
}

fn build_child_call(
    parent_call_id: &str,
    index: usize,
    command: CommandItem,
) -> Result<ToolCall, FunctionCallError> {
    let (tool_name, payload) = match command.command_type {
        CommandType::ExecCommand => {
            if !command.arguments.is_object() {
                return Err(FunctionCallError::RespondToModel(
                    "exec_command arguments must be an object".to_string(),
                ));
            }
            let arguments = serde_json::to_string(&command.arguments).map_err(|err| {
                FunctionCallError::RespondToModel(format!(
                    "failed to serialize exec_command arguments: {err}"
                ))
            })?;
            (
                ToolName::plain("exec_command"),
                ToolPayload::Function { arguments },
            )
        }
        CommandType::WriteStdin => {
            if !command.arguments.is_object() {
                return Err(FunctionCallError::RespondToModel(
                    "write_stdin arguments must be an object".to_string(),
                ));
            }
            let arguments = serde_json::to_string(&command.arguments).map_err(|err| {
                FunctionCallError::RespondToModel(format!(
                    "failed to serialize write_stdin arguments: {err}"
                ))
            })?;
            (
                ToolName::plain("write_stdin"),
                ToolPayload::Function { arguments },
            )
        }
        CommandType::ApplyPatch => {
            let PatchArguments {
                patch,
                environment_id,
            } = serde_json::from_value(command.arguments).map_err(|err| {
                FunctionCallError::RespondToModel(format!(
                    "failed to parse apply_patch arguments: {err}"
                ))
            })?;
            (
                ToolName::plain("apply_patch"),
                ToolPayload::Custom {
                    input: insert_environment_id(patch, environment_id)?,
                },
            )
        }
    };

    Ok(ToolCall {
        call_id: format!("{parent_call_id}:{}:{index}", command.step),
        tool_name,
        payload,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchArguments {
    patch: String,
    #[serde(default)]
    environment_id: Option<String>,
}

fn insert_environment_id(
    patch: String,
    environment_id: Option<String>,
) -> Result<String, FunctionCallError> {
    let Some(environment_id) = environment_id else {
        return Ok(patch);
    };
    if patch.contains("*** Environment ID: ") {
        return Err(FunctionCallError::RespondToModel(
            "apply_patch environment_id was provided both separately and in the patch".to_string(),
        ));
    }
    let Some(rest) = patch.strip_prefix("*** Begin Patch\n") else {
        return Err(FunctionCallError::RespondToModel(
            "apply_patch input must start with `*** Begin Patch`".to_string(),
        ));
    };
    Ok(format!(
        "*** Begin Patch\n*** Environment ID: {environment_id}\n{rest}"
    ))
}

fn normalize_child_result(
    index: usize,
    step: u8,
    command_type: CommandType,
    result: Result<AnyToolResult, FunctionCallError>,
) -> CommandResult {
    match result {
        Ok(result) => {
            let success = result.success_for_logging();
            let output = response_text(result.into_response());
            CommandResult {
                index,
                step,
                command_type,
                success,
                output,
            }
        }
        Err(error) => CommandResult {
            index,
            step,
            command_type,
            success: false,
            output: error.to_string(),
        },
    }
}

fn response_text(item: ResponseInputItem) -> String {
    let body = match item {
        ResponseInputItem::FunctionCallOutput { output, .. }
        | ResponseInputItem::CustomToolCallOutput { output, .. } => output.body,
        other => return format!("{other:?}"),
    };
    body.to_text().unwrap_or_else(|| match body {
        FunctionCallOutputBody::Text(text) => text,
        FunctionCallOutputBody::ContentItems(items) => {
            serde_json::to_string(&items).unwrap_or_default()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use codex_tools::ResponsesApiTool;
    use tokio::sync::Mutex;

    use crate::session::step_context::StepContext;
    use crate::session::tests::make_session_and_context;
    use crate::tools::context::ToolCallSource;
    use crate::tools::registry::ToolRegistry;
    use crate::tools::router::ToolRouter;
    use crate::turn_diff_tracker::TurnDiffTracker;

    #[test]
    fn validation_rejects_empty_and_out_of_range_steps() {
        assert!(validate_commands(&[]).is_err());
        assert!(
            validate_commands(&[CommandItem {
                command_type: CommandType::ExecCommand,
                step: 5,
                arguments: serde_json::json!({"cmd": "echo ok"}),
                continue_on_failure: false,
            }])
            .is_err()
        );
        assert!(
            validate_commands(&[CommandItem {
                command_type: CommandType::ApplyPatch,
                step: 1,
                arguments: serde_json::json!({
                    "patch": "*** Begin Patch\n*** End Patch\n"
                }),
                continue_on_failure: true,
            }])
            .is_err()
        );
    }

    #[test]
    fn validation_isolates_persistent_shell_switches() {
        let commands = [
            CommandItem {
                command_type: CommandType::ExecCommand,
                step: 1,
                arguments: serde_json::json!({"cmd": "echo switch", "shell": "bash"}),
                continue_on_failure: false,
            },
            CommandItem {
                command_type: CommandType::ExecCommand,
                step: 1,
                arguments: serde_json::json!({"cmd": "echo races"}),
                continue_on_failure: false,
            },
        ];

        let err = validate_commands(&commands).expect_err("shell switch must be isolated");
        assert!(
            err.to_string()
                .contains("must be the only command in step 1")
        );
    }

    #[test]
    fn inserts_environment_id_after_begin_marker() {
        let patch = "*** Begin Patch\n*** End Patch\n".to_string();
        assert_eq!(
            insert_environment_id(patch, Some("remote".to_string())).unwrap(),
            "*** Begin Patch\n*** Environment ID: remote\n*** End Patch\n"
        );
    }

    #[test]
    fn child_call_ids_are_stable_for_app_server_items() {
        let child = build_child_call(
            "call-command-stack",
            2,
            CommandItem {
                command_type: CommandType::ExecCommand,
                step: 3,
                arguments: serde_json::json!({"cmd": "echo ok"}),
                continue_on_failure: false,
            },
        )
        .unwrap();

        assert_eq!(child.call_id, "call-command-stack:3:2");
        assert_eq!(child.tool_name, ToolName::plain("exec_command"));
    }

    #[test]
    fn write_stdin_routes_to_the_continuation_child() {
        let child = build_child_call(
            "call-command-stack",
            0,
            CommandItem {
                command_type: CommandType::WriteStdin,
                step: 1,
                arguments: serde_json::json!({"session_id": 42, "chars": "q"}),
                continue_on_failure: false,
            },
        )
        .unwrap();

        assert_eq!(child.call_id, "call-command-stack:1:0");
        assert_eq!(child.tool_name, ToolName::plain("write_stdin"));
        assert!(matches!(child.payload, ToolPayload::Function { .. }));
    }

    #[tokio::test]
    async fn failed_patch_stops_later_steps() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let handler = recording_stack_handler(Arc::clone(&calls), false, false, true);
        let invocation = invocation_for_commands(serde_json::json!({
            "commands": [
                {
                    "command_type": "apply_patch",
                    "step": 1,
                    "arguments": {
                        "patch": "*** Begin Patch\n*** End Patch\n"
                    }
                },
                {
                    "command_type": "exec_command",
                    "step": 2,
                    "arguments": {
                        "cmd": "echo should-not-run"
                    }
                }
            ]
        }))
        .await;

        let output = handler.handle_call(invocation).await.unwrap();

        assert!(!output.success_for_logging());
        assert_eq!(calls.lock().await.as_slice(), ["apply_patch"]);
    }

    #[tokio::test]
    async fn failed_exec_stops_later_steps_by_default() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let handler = recording_stack_handler(Arc::clone(&calls), true, false, false);
        let invocation = invocation_for_commands(serde_json::json!({
            "commands": [
                {
                    "command_type": "exec_command",
                    "step": 1,
                    "arguments": {"cmd": "false"}
                },
                {
                    "command_type": "apply_patch",
                    "step": 2,
                    "arguments": {
                        "patch": "*** Begin Patch\n*** End Patch\n"
                    }
                }
            ]
        }))
        .await;

        let output = handler.handle_call(invocation).await.unwrap();

        assert!(!output.success_for_logging());
        assert_eq!(calls.lock().await.as_slice(), ["exec_command"]);
    }

    #[tokio::test]
    async fn continue_on_failure_allows_later_steps_for_exec() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let handler = recording_stack_handler(Arc::clone(&calls), true, false, false);
        let invocation = invocation_for_commands(serde_json::json!({
            "commands": [
                {
                    "command_type": "exec_command",
                    "step": 1,
                    "arguments": {"cmd": "false"},
                    "continue_on_failure": true
                },
                {
                    "command_type": "apply_patch",
                    "step": 2,
                    "arguments": {
                        "patch": "*** Begin Patch\n*** End Patch\n"
                    }
                }
            ]
        }))
        .await;

        let output = handler.handle_call(invocation).await.unwrap();

        assert!(!output.success_for_logging());
        assert_eq!(
            calls.lock().await.as_slice(),
            ["exec_command", "apply_patch"]
        );
    }

    #[tokio::test]
    async fn write_stdin_executes_through_the_child_router() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let handler = recording_stack_handler(Arc::clone(&calls), false, false, false);
        let invocation = invocation_for_commands(serde_json::json!({
            "commands": [{
                "command_type": "write_stdin",
                "step": 1,
                "arguments": {"session_id": 42, "chars": "q"}
            }]
        }))
        .await;

        let output = handler.handle_call(invocation).await.unwrap();

        assert!(output.success_for_logging());
        assert_eq!(calls.lock().await.as_slice(), ["write_stdin"]);
    }

    fn recording_stack_handler(
        calls: Arc<Mutex<Vec<String>>>,
        exec_fails: bool,
        write_stdin_fails: bool,
        patch_fails: bool,
    ) -> CommandStackHandler {
        let child_registry = ToolRegistry::from_tools([
            Arc::new(RecordingHandler::new(
                "exec_command",
                Arc::clone(&calls),
                exec_fails,
            )) as Arc<dyn CoreToolRuntime>,
            Arc::new(RecordingHandler::new(
                "write_stdin",
                Arc::clone(&calls),
                write_stdin_fails,
            )) as Arc<dyn CoreToolRuntime>,
            Arc::new(RecordingHandler::new("apply_patch", calls, patch_fails))
                as Arc<dyn CoreToolRuntime>,
        ]);
        let child_router = Arc::new(ToolRouter::from_parts(child_registry, Vec::new()));
        let arguments = JsonSchema::object(Default::default(), None, Some(true.into()));
        CommandStackHandler::new(child_router, arguments.clone(), arguments, false)
    }

    struct RecordingHandler {
        name: &'static str,
        calls: Arc<Mutex<Vec<String>>>,
        fail: bool,
    }

    impl RecordingHandler {
        fn new(name: &'static str, calls: Arc<Mutex<Vec<String>>>, fail: bool) -> Self {
            Self { name, calls, fail }
        }
    }

    impl ToolExecutor<ToolInvocation> for RecordingHandler {
        fn tool_name(&self) -> ToolName {
            ToolName::plain(self.name)
        }

        fn spec(&self) -> ToolSpec {
            ToolSpec::Function(ResponsesApiTool {
                name: self.name.to_string(),
                description: String::new(),
                strict: false,
                defer_loading: None,
                parameters: JsonSchema::default(),
                output_schema: None,
            })
        }

        fn handle(&self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'_> {
            let calls = Arc::clone(&self.calls);
            let fail = self.fail;
            let name = self.name;
            Box::pin(async move {
                calls.lock().await.push(invocation.tool_name.to_string());
                if fail {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "synthetic {name} failure"
                    )));
                }
                Ok(boxed_tool_output(FunctionToolOutput::from_text(
                    "ok".to_string(),
                    Some(true),
                )))
            })
        }
    }

    impl CoreToolRuntime for RecordingHandler {
        fn matches_kind(&self, payload: &ToolPayload) -> bool {
            match self.name {
                "apply_patch" => matches!(payload, ToolPayload::Custom { .. }),
                _ => matches!(payload, ToolPayload::Function { .. }),
            }
        }
    }

    async fn invocation_for_commands(arguments: Value) -> ToolInvocation {
        let (session, turn) = make_session_and_context().await;
        let turn = Arc::new(turn);
        ToolInvocation {
            session: session.into(),
            step_context: StepContext::for_test(Arc::clone(&turn)),
            turn,
            cancellation_token: tokio_util::sync::CancellationToken::new(),
            tracker: Arc::new(Mutex::new(TurnDiffTracker::new())),
            call_id: "call-command-stack".to_string(),
            tool_name: ToolName::plain("command_stack"),
            source: ToolCallSource::Direct,
            payload: ToolPayload::Function {
                arguments: arguments.to_string(),
            },
        }
    }
}
