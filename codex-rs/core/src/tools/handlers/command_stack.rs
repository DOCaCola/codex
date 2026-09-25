use std::collections::BTreeMap;
use std::sync::Arc;

use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::ResponseItem;
use codex_tools::JsonSchema;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use futures::future::join_all;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolCallState;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::command_stack_freeform::parse_command_stack;
use crate::tools::handlers::command_stack_spec::CommandStackPatchTool;

use crate::tools::handlers::command_stack_spec::create_command_stack_tool;

use crate::tools::parallel::ToolCallRuntime;
use crate::tools::registry::AnyToolResult;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;
use crate::tools::router::ToolCall;
use crate::tools::router::ToolRouter;

#[path = "command_stack_managed.rs"]
mod managed;

const MAX_COMMANDS: usize = 8;
const MAX_STEP: u8 = 4;

pub(crate) struct CommandStackHandler {
    child_router: Arc<ToolRouter>,
    exec_arguments: JsonSchema,
    write_stdin_arguments: JsonSchema,
    include_environment_id: bool,
    patch_tool: CommandStackPatchTool,
}

impl CommandStackHandler {
    pub(crate) fn new(
        child_router: Arc<ToolRouter>,
        exec_arguments: JsonSchema,
        write_stdin_arguments: JsonSchema,
        include_environment_id: bool,
        patch_tool: CommandStackPatchTool,
    ) -> Self {
        Self {
            child_router,
            exec_arguments,
            write_stdin_arguments,
            include_environment_id,
            patch_tool,
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
            self.patch_tool,
        )
    }

    fn handle<'a>(&'a self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'a>
    where
        ToolInvocation: 'a,
    {
        Box::pin(self.handle_call(invocation))
    }
}

impl CoreToolRuntime for CommandStackHandler {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Custom { .. })
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
        let ToolPayload::Custom { input } = &invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "command_stack received unsupported payload".to_string(),
            ));
        };
        let CommandStackArgs {
            workdir,
            mut commands,
        } = parse_command_stack(input)?;
        apply_stack_workdir(&mut commands, workdir.as_deref());

        validate_commands(&commands, self.patch_tool)?;
        if invocation
            .turn
            .config
            .features
            .enabled(codex_features::Feature::BackgroundCommandDelivery)
        {
            managed::prepare_dependencies(&mut commands)?;
        }
        let tool_runtime = ToolCallRuntime::new_with_router(
            Arc::clone(&invocation.session),
            Arc::clone(&invocation.step_context),
            Arc::clone(&invocation.tracker),
            Arc::clone(&self.child_router),
        );

        let mut grouped = BTreeMap::<u8, Vec<(usize, CommandItem)>>::new();
        for (index, command) in commands.into_iter().enumerate() {
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
                .any(|(_, command)| command.command_type.is_patch());
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
                        Arc::clone(&parent.step_context),
                        child,
                        parent.source.clone(),
                        parent.cancellation_token.clone(),
                        Arc::new(ToolCallState::default()),
                    )
                    .await,
            ),
            continue_on_failure,
        )
    }
}

#[derive(Deserialize)]
pub(super) struct CommandStackArgs {
    #[serde(default)]
    pub(super) workdir: Option<String>,
    pub(super) commands: Vec<CommandItem>,
}

#[derive(Clone, Deserialize)]
pub(super) struct CommandItem {
    pub(super) command_type: CommandType,

    pub(super) step: u8,

    pub(super) arguments: Value,

    #[serde(default)]
    pub(super) continue_on_failure: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum CommandType {
    ExecCommand,
    WriteStdin,
    ApplyPatch,
    Hpatch,
}

impl CommandType {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::ExecCommand => "exec_command",
            Self::WriteStdin => "write_stdin",
            Self::ApplyPatch => "apply_patch",
            Self::Hpatch => "hpatch",
        }
    }

    fn is_patch(self) -> bool {
        matches!(self, Self::ApplyPatch | Self::Hpatch)
    }
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
        let patch_failed = failed && result.command_type.is_patch();
        Self {
            blocks_later_steps: failed && (patch_failed || !continue_on_failure),
            stop_current_step: patch_failed,
            result,
        }
    }
}

fn validate_commands(
    commands: &[CommandItem],
    patch_tool: CommandStackPatchTool,
) -> Result<(), FunctionCallError> {
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
    if commands
        .iter()
        .any(|command| command.command_type.is_patch() && command.continue_on_failure)
    {
        return Err(FunctionCallError::RespondToModel(
            "patch operations do not support continue_on_failure".to_string(),
        ));
    }
    if let Some(command) = commands.iter().find(|command| {
        command.command_type.is_patch()
            && match patch_tool {
                CommandStackPatchTool::ApplyPatch => {
                    command.command_type != CommandType::ApplyPatch
                }
                CommandStackPatchTool::Hpatch => command.command_type != CommandType::Hpatch,
            }
    }) {
        return Err(FunctionCallError::RespondToModel(format!(
            "{} is not available in this command_stack configuration",
            match command.command_type {
                CommandType::ApplyPatch => "apply_patch",
                CommandType::Hpatch => "hpatch",
                CommandType::ExecCommand | CommandType::WriteStdin => unreachable!(),
            }
        )));
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

fn apply_stack_workdir(commands: &mut [CommandItem], workdir: Option<&str>) {
    let Some(workdir) = workdir else {
        return;
    };

    for command in commands {
        let Some(arguments) = command.arguments.as_object_mut() else {
            continue;
        };
        match command.command_type {
            CommandType::ExecCommand => {
                arguments
                    .entry("workdir")
                    .or_insert_with(|| Value::String(workdir.to_string()));
            }
            CommandType::Hpatch => {
                let has_workdir_header = arguments
                    .get("script")
                    .and_then(Value::as_str)
                    .is_some_and(hpatch_has_workdir_header);
                if !has_workdir_header {
                    arguments
                        .entry("workdir")
                        .or_insert_with(|| Value::String(workdir.to_string()));
                }
            }
            CommandType::WriteStdin | CommandType::ApplyPatch => {}
        }
    }
}

fn hpatch_has_workdir_header(input: &str) -> bool {
    input.lines().take(2).any(|line| {
        line.trim_end_matches('\r')
            .starts_with("*** Working Directory: ")
    })
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
        CommandType::ApplyPatch | CommandType::Hpatch => {
            let PatchArguments {
                patch,
                script,
                environment_id,
                workdir,
            } = serde_json::from_value(command.arguments).map_err(|err| {
                FunctionCallError::RespondToModel(format!("failed to parse patch arguments: {err}"))
            })?;
            let (tool_name, input) = match command.command_type {
                CommandType::ApplyPatch => ("apply_patch", patch),
                CommandType::Hpatch => ("hpatch", script),
                CommandType::ExecCommand | CommandType::WriteStdin => unreachable!(),
            };
            let input = input.ok_or_else(|| {
                FunctionCallError::RespondToModel(format!(
                    "{tool_name} arguments are missing the edit input"
                ))
            })?;
            (
                ToolName::plain(tool_name),
                ToolPayload::Custom {
                    input: insert_patch_context(
                        input,
                        environment_id,
                        workdir,
                        command.command_type,
                    )?,
                },
            )
        }
    };

    Ok(ToolCall {
        call_id: format!("{parent_call_id}:{}:{index}", command.step),
        tool_name,
        payload,
        encrypted_function_args: None,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchArguments {
    #[serde(default)]
    patch: Option<String>,
    #[serde(default)]
    script: Option<String>,
    #[serde(default)]
    environment_id: Option<String>,
    #[serde(default)]
    workdir: Option<String>,
}

fn insert_patch_context(
    input: String,
    environment_id: Option<String>,
    workdir: Option<String>,
    command_type: CommandType,
) -> Result<String, FunctionCallError> {
    if environment_id.is_some() && input.contains("*** Environment ID: ") {
        return Err(FunctionCallError::RespondToModel(
            "environment_id was provided both separately and in the edit input".to_string(),
        ));
    }
    if workdir.is_some() && input.contains("*** Working Directory: ") {
        return Err(FunctionCallError::RespondToModel(
            "workdir was provided both separately and in the edit input".to_string(),
        ));
    }

    match command_type {
        CommandType::ApplyPatch => {
            if workdir.is_some() {
                return Err(FunctionCallError::RespondToModel(
                    "workdir is unavailable for apply_patch command_stack children".to_string(),
                ));
            }
            let Some(environment_id) = environment_id else {
                return Ok(input);
            };
            let Some(rest) = input.strip_prefix("*** Begin Patch\n") else {
                return Err(FunctionCallError::RespondToModel(
                    "apply_patch input must start with `*** Begin Patch`".to_string(),
                ));
            };
            Ok(format!(
                "*** Begin Patch\n*** Environment ID: {environment_id}\n{rest}"
            ))
        }
        CommandType::Hpatch => {
            if workdir.as_deref().is_some_and(str::is_empty) {
                return Err(FunctionCallError::RespondToModel(
                    "hpatch workdir must not be empty".to_string(),
                ));
            }
            let mut headers = String::new();
            if let Some(environment_id) = environment_id {
                headers.push_str(&format!("*** Environment ID: {environment_id}\n"));
            }
            if let Some(workdir) = workdir {
                headers.push_str(&format!("*** Working Directory: {workdir}\n"));
            }
            Ok(format!("{headers}{input}"))
        }
        CommandType::ExecCommand | CommandType::WriteStdin => unreachable!(),
    }
}

fn normalize_child_result(
    index: usize,
    step: u8,
    command_type: CommandType,
    result: Result<AnyToolResult, FunctionCallError>,
) -> CommandResult {
    match result {
        Ok(result) => {
            // Shell exit status governs dependency ordering, not whether the
            // tool transport succeeded. Preserve PostToolUse for nonzero exits.
            let dependency_succeeded = match command_type {
                CommandType::ExecCommand | CommandType::WriteStdin => {
                    let status = result.result.code_mode_result(&result.payload);
                    status.get("completion_delivery").is_none()
                        || status.get("session_id").is_some()
                        || status.get("exit_code").and_then(serde_json::Value::as_i64) == Some(0)
                }
                CommandType::ApplyPatch | CommandType::Hpatch => true,
            };
            let success = result.success_for_logging() && dependency_succeeded;
            let output = response_text(result.into_response().item);
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

fn response_text(item: ResponseItem) -> String {
    let body = match item {
        ResponseItem::FunctionCallOutput { output, .. }
        | ResponseItem::CustomToolCallOutput { output, .. } => output.body,
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
        assert!(validate_commands(&[], CommandStackPatchTool::ApplyPatch).is_err());
        assert!(
            validate_commands(
                &[CommandItem {
                    command_type: CommandType::ExecCommand,
                    step: 5,
                    arguments: serde_json::json!({"cmd": "echo ok"}),
                    continue_on_failure: false,
                }],
                CommandStackPatchTool::ApplyPatch
            )
            .is_err()
        );
        assert!(
            validate_commands(
                &[CommandItem {
                    command_type: CommandType::ApplyPatch,
                    step: 1,
                    arguments: serde_json::json!({
                        "patch": "*** Begin Patch\n*** End Patch\n"
                    }),
                    continue_on_failure: true,
                }],
                CommandStackPatchTool::ApplyPatch
            )
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

        let err = validate_commands(&commands, CommandStackPatchTool::ApplyPatch)
            .expect_err("shell switch must be isolated");
        assert!(
            err.to_string()
                .contains("must be the only command in step 1")
        );
    }

    #[test]
    fn stack_workdir_defaults_exec_and_hpatch_but_preserves_overrides() {
        let mut commands = vec![
            CommandItem {
                command_type: CommandType::ExecCommand,
                step: 1,
                arguments: serde_json::json!({"cmd": "echo default"}),
                continue_on_failure: false,
            },
            CommandItem {
                command_type: CommandType::ExecCommand,
                step: 1,
                arguments: serde_json::json!({"cmd": "echo override", "workdir": "explicit"}),
                continue_on_failure: false,
            },
            CommandItem {
                command_type: CommandType::Hpatch,
                step: 2,
                arguments: serde_json::json!({"script": "in src/lib.rs\ncommit"}),
                continue_on_failure: false,
            },
            CommandItem {
                command_type: CommandType::Hpatch,
                step: 2,
                arguments: serde_json::json!({
                    "script": "*** Working Directory: explicit\nin src/main.rs\ncommit"
                }),
                continue_on_failure: false,
            },
            CommandItem {
                command_type: CommandType::ApplyPatch,
                step: 3,
                arguments: serde_json::json!({
                    "patch": "*** Begin Patch\n*** End Patch\n"
                }),
                continue_on_failure: false,
            },
        ];

        apply_stack_workdir(&mut commands, Some("shared root"));

        assert_eq!(commands[0].arguments["workdir"], "shared root");
        assert_eq!(commands[1].arguments["workdir"], "explicit");
        assert_eq!(commands[2].arguments["workdir"], "shared root");
        assert!(commands[3].arguments.get("workdir").is_none());
        assert!(commands[4].arguments.get("workdir").is_none());
    }

    #[test]
    fn parsed_stack_workdir_reaches_hpatch_child_input() {
        let CommandStackArgs {
            workdir,
            mut commands,
        } = parse_command_stack("workdir nested repo\nhpatch\nin src/lib.rs\ncommit\nend")
            .expect("parse stack");
        apply_stack_workdir(&mut commands, workdir.as_deref());

        let child = build_child_call("call-command-stack", 0, commands.remove(0))
            .expect("build hpatch child");

        assert_eq!(
            child.payload,
            ToolPayload::Custom {
                input: "*** Working Directory: nested repo\nin src/lib.rs\ncommit".to_string(),
            }
        );
    }

    #[test]
    fn inserts_environment_id_after_begin_marker() {
        let patch = "*** Begin Patch\n*** End Patch\n".to_string();
        assert_eq!(
            insert_patch_context(
                patch,
                Some("remote".to_string()),
                None,
                CommandType::ApplyPatch,
            )
            .unwrap(),
            "*** Begin Patch\n*** Environment ID: remote\n*** End Patch\n"
        );
    }

    #[test]
    fn hpatch_child_includes_environment_and_workdir_headers() {
        let child = build_child_call(
            "call-command-stack",
            0,
            CommandItem {
                command_type: CommandType::Hpatch,
                step: 1,
                arguments: serde_json::json!({
                    "script": "in src/lib.rs\ncommit",
                    "environment_id": "remote",
                    "workdir": "codex"
                }),
                continue_on_failure: false,
            },
        )
        .unwrap();

        assert_eq!(child.tool_name, ToolName::plain("hpatch"));
        assert_eq!(
            child.payload,
            ToolPayload::Custom {
                input: "*** Environment ID: remote\n*** Working Directory: codex\nin src/lib.rs\ncommit"
                    .to_string(),
            }
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
        let invocation = invocation_for_stack(
            r#"apply_patch
*** Begin Patch
*** Add File: sample.txt
+hello
*** End Patch
end
exec_command step=2
{"cmd":"echo should-not-run"}
end"#,
        )
        .await;

        let output = handler.handle_call(invocation).await.unwrap();

        assert!(!output.success_for_logging());
        assert_eq!(calls.lock().await.as_slice(), ["apply_patch"]);
    }

    #[tokio::test]
    async fn failed_exec_stops_later_steps_by_default() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let handler = recording_stack_handler(Arc::clone(&calls), true, false, false);
        let invocation = invocation_for_stack(
            r#"exec_command
{"cmd":"false"}
end
apply_patch step=2
*** Begin Patch
*** Add File: sample.txt
+hello
*** End Patch
end"#,
        )
        .await;

        let output = handler.handle_call(invocation).await.unwrap();

        assert!(!output.success_for_logging());
        assert_eq!(calls.lock().await.as_slice(), ["exec_command"]);
    }

    #[tokio::test]
    async fn continue_on_failure_allows_later_steps_for_exec() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let handler = recording_stack_handler(Arc::clone(&calls), true, false, false);
        let invocation = invocation_for_stack(
            r#"exec_command continue_on_failure
{"cmd":"false"}
end
apply_patch step=2
*** Begin Patch
*** Add File: sample.txt
+hello
*** End Patch
end"#,
        )
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
        let invocation = invocation_for_stack(
            r#"write_stdin
{"session_id":42,"chars":"q"}
end"#,
        )
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
        let children = [
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
        ];
        let child_registry = ToolRegistry::from_tools(children);
        let child_router = Arc::new(ToolRouter::from_parts(
            child_registry,
            Vec::new(),
            codex_protocol::openai_models::ToolMode::Direct,
            BTreeMap::new(),
            /*tool_namespaces_info*/ None,
            &[],
        ));
        let arguments = JsonSchema::object(Default::default(), None, Some(true.into()));
        CommandStackHandler::new(
            child_router,
            arguments.clone(),
            arguments,
            false,
            CommandStackPatchTool::ApplyPatch,
        )
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

        fn handle<'a>(&'a self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'a>
        where
            ToolInvocation: 'a,
        {
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

    async fn invocation_for_stack(input: &str) -> ToolInvocation {
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
            payload: ToolPayload::Custom {
                input: input.to_string(),
            },
        }
    }
}
