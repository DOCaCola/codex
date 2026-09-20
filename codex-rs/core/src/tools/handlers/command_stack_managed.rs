use super::CommandItem;
use super::CommandType;
use crate::function_tool::FunctionCallError;
use serde_json::json;

/// Resolve finite dependencies before starting any command. A dependency stays
/// inside its original tool call, preserving output ownership and hook ordering.
pub(super) fn prepare_dependencies(commands: &mut [CommandItem]) -> Result<(), FunctionCallError> {
    let last_step = commands
        .iter()
        .map(|command| command.step)
        .max()
        .unwrap_or(1);
    for command in commands
        .iter_mut()
        .filter(|command| command.step < last_step)
    {
        match command.command_type {
            CommandType::ExecCommand => {
                let args = command.arguments.as_object_mut().ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "exec_command arguments must be an object".into(),
                    )
                })?;
                if args.get("tty").and_then(serde_json::Value::as_bool) == Some(true)
                    || args
                        .get("execution_mode")
                        .and_then(serde_json::Value::as_str)
                        == Some("interactive")
                {
                    return Err(FunctionCallError::RespondToModel(
                        "interactive commands cannot be dependencies of later command_stack steps; start services separately and check readiness explicitly".into(),
                    ));
                }
                if let Some(mode) = args.get("execution_mode")
                    && !matches!(mode.as_str(), Some("auto" | "foreground" | "background"))
                {
                    return Err(FunctionCallError::RespondToModel(
                        "invalid execution_mode".into(),
                    ));
                }
                args.insert("execution_mode".into(), json!("foreground"));
                args.remove("yield_time_ms");
            }
            CommandType::WriteStdin => {
                let args = command.arguments.as_object_mut().ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "write_stdin arguments must be an object".into(),
                    )
                })?;
                if args
                    .get("chars")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|chars| !chars.is_empty())
                {
                    return Err(FunctionCallError::RespondToModel(
                        "stdin writes cannot be completion dependencies; use a separate interaction".into(),
                    ));
                }
                args.insert("wait_mode".into(), json!("completion"));
                args.remove("yield_time_ms");
            }
            CommandType::ApplyPatch | CommandType::Hpatch => {}
        }
    }
    Ok(())
}
