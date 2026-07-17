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
    include_environment_id: bool,
}

impl CommandStackHandler {
    pub(crate) fn new(
        child_router: Arc<ToolRouter>,
        exec_arguments: JsonSchema,
        include_environment_id: bool,
    ) -> Self {
        Self {
            child_router,
            exec_arguments,
            include_environment_id,
        }
    }
}

impl ToolExecutor<ToolInvocation> for CommandStackHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("command_stack")
    }

    fn spec(&self) -> ToolSpec {
        create_command_stack_tool(self.exec_arguments.clone(), self.include_environment_id)
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
            let step_results = if contains_patch {
                let mut step_results = Vec::with_capacity(commands.len());
                for (index, command) in commands {
                    let result = self
                        .execute_command(&invocation, tool_runtime.clone(), index, command)
                        .await;
                    let patch_failed =
                        result.command_type == CommandType::ApplyPatch && !result.success;
                    step_results.push(result);
                    if patch_failed {
                        stopped = true;
                        break;
                    }
                }
                step_results
            } else {
                join_all(commands.into_iter().map(|(index, command)| {
                    self.execute_command(&invocation, tool_runtime.clone(), index, command)
                }))
                .await
            };
            results.extend(step_results);
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
    ) -> CommandResult {
        let command_type = command.command_type;
        let step = command.step;
        let child = match build_child_call(&parent.call_id, index, command) {
            Ok(child) => child,
            Err(error) => {
                return CommandResult {
                    index,
                    step,
                    command_type,
                    success: false,
                    output: error.to_string(),
                };
            }
        };

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
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CommandType {
    ExecCommand,
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
            }])
            .is_err()
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
            },
        )
        .unwrap();

        assert_eq!(child.call_id, "call-command-stack:3:2");
        assert_eq!(child.tool_name, ToolName::plain("exec_command"));
    }

    #[tokio::test]
    async fn failed_patch_stops_later_steps() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let child_registry = ToolRegistry::from_tools([
            Arc::new(RecordingHandler::new(
                "exec_command",
                Arc::clone(&calls),
                false,
            )) as Arc<dyn CoreToolRuntime>,
            Arc::new(RecordingHandler::new(
                "apply_patch",
                Arc::clone(&calls),
                true,
            )) as Arc<dyn CoreToolRuntime>,
        ]);
        let child_router = Arc::new(ToolRouter::from_parts(child_registry, Vec::new()));
        let handler = CommandStackHandler::new(
            child_router,
            JsonSchema::object(Default::default(), None, Some(true.into())),
            false,
        );
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
            Box::pin(async move {
                calls.lock().await.push(invocation.tool_name.to_string());
                if fail {
                    return Err(FunctionCallError::RespondToModel(
                        "synthetic patch failure".to_string(),
                    ));
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
