use codex_protocol::models::ShellCommandToolCallParams;
use codex_tools::ShellCommandBackendConfig;
use codex_tools::ToolName;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::path::PathBuf;

use crate::exec::ExecCapturePolicy;
use crate::exec::ExecParams;
use crate::exec_env::create_env;
use crate::exec_env::inject_permission_profile_env;
use crate::function_tool::FunctionCallError;
use crate::maybe_emit_implicit_skill_invocation;
use crate::session::turn_context::TurnContext;
use crate::session::turn_context::TurnEnvironment;
use crate::shell::Shell;
use crate::shell::get_shell;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::parse_arguments_with_base_path;
use crate::tools::handlers::resolve_workdir_base_path;
use crate::tools::handlers::rewrite_function_string_argument;
use crate::tools::handlers::updated_hook_command;
use crate::tools::hook_names::HookToolName;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::PostToolUsePayload;
use crate::tools::registry::PreToolUsePayload;
use crate::tools::registry::ToolExecutor;
use crate::tools::runtimes::shell::ShellRuntimeBackend;
use codex_shell_command::shell_detect::detect_shell_type;
use codex_tools::ToolSpec;

use super::super::shell_spec::CommandToolOptions;
use super::super::shell_spec::create_shell_command_tool;
use super::RunExecLikeArgs;
use super::run_exec_like;
use super::shell_command_payload_command;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellCommandBackend {
    Classic,
    ZshFork,
}

pub struct ShellCommandHandler {
    backend: ShellCommandBackend,
    options: ShellCommandHandlerOptions,
}

#[derive(Clone, Copy)]
pub(crate) struct ShellCommandHandlerOptions {
    pub(crate) backend_config: ShellCommandBackendConfig,
    pub(crate) allow_login_shell: bool,
    pub(crate) exec_permission_approvals_enabled: bool,
}

impl ShellCommandHandler {
    const WINDOWS_INLINE_WRITE_GUARD_LEN: usize = 4096;

    pub(crate) fn new(options: ShellCommandHandlerOptions) -> Self {
        let backend = match options.backend_config {
            ShellCommandBackendConfig::Classic => ShellCommandBackend::Classic,
            ShellCommandBackendConfig::ZshFork => ShellCommandBackend::ZshFork,
        };
        Self { backend, options }
    }

    fn shell_runtime_backend(&self) -> ShellRuntimeBackend {
        match self.backend {
            ShellCommandBackend::Classic => ShellRuntimeBackend::ShellCommandClassic,
            ShellCommandBackend::ZshFork => ShellRuntimeBackend::ShellCommandZshFork,
        }
    }

    pub(super) fn resolve_use_login_shell(
        login: Option<bool>,
        allow_login_shell: bool,
    ) -> Result<bool, FunctionCallError> {
        if !allow_login_shell && login == Some(true) {
            return Err(FunctionCallError::RespondToModel(
                "login shell is disabled by config; omit `login` or set it to false.".to_string(),
            ));
        }

        Ok(login.unwrap_or(allow_login_shell))
    }

    pub(super) fn base_command(shell: &Shell, command: &str, use_login_shell: bool) -> Vec<String> {
        shell.derive_exec_args(command, use_login_shell)
    }

    fn command_looks_like_large_inline_file_write(command: &str) -> bool {
        command.len() >= Self::WINDOWS_INLINE_WRITE_GUARD_LEN
            && (command.contains("<<") || command.contains("cat >") || command.contains("cat >>"))
    }

    fn validate_command_supported(shell: &Shell, command: &str) -> Result<(), FunctionCallError> {
        let is_windows_bash_family = cfg!(windows)
            && matches!(
                shell.shell_type,
                crate::shell::ShellType::Bash
                    | crate::shell::ShellType::Zsh
                    | crate::shell::ShellType::Sh
            );

        if is_windows_bash_family && Self::command_looks_like_large_inline_file_write(command) {
            return Err(FunctionCallError::RespondToModel(
                "large inline file writes through `shell_command` are unreliable on Windows bash. Use `apply_patch` for source files, or `exec_command` + `write_stdin` to stream file contents instead of a giant heredoc.".to_string(),
            ));
        }

        Ok(())
    }

    fn shell_from_requested_path(shell: &str) -> Result<Shell, FunctionCallError> {
        let requested_path = PathBuf::from(shell);
        let Some(shell_type) = detect_shell_type(&requested_path) else {
            return Err(FunctionCallError::RespondToModel(format!(
                "unsupported shell `{shell}`; expected a shell name or executable path for bash, sh, zsh, powershell, or cmd"
            )));
        };

        let Some(mut parsed_shell) = get_shell(shell_type, Some(&requested_path)) else {
            return Err(FunctionCallError::RespondToModel(format!(
                "requested shell `{shell}` is recognized but not available on this machine"
            )));
        };

        parsed_shell.shell_snapshot = crate::shell::empty_shell_snapshot_receiver();
        Ok(parsed_shell)
    }

    pub(super) fn effective_shell(
        params: &ShellCommandToolCallParams,
        session: &crate::session::session::Session,
    ) -> Result<Shell, FunctionCallError> {
        if let Some(requested_shell) = params.shell.as_deref() {
            Self::shell_from_requested_path(requested_shell)
        } else {
            Ok(session.user_shell().as_ref().clone())
        }
    }

    pub(super) fn maybe_update_session_shell(
        params: &ShellCommandToolCallParams,
        session: &crate::session::session::Session,
    ) -> Result<(), FunctionCallError> {
        if params.shell.is_some() {
            let shell = Self::effective_shell(params, session)?;
            session.set_user_shell(shell.clone());
            session.services.turn_environments.update_local_shell(shell);
        }

        Ok(())
    }

    pub(super) fn to_exec_params(
        params: &ShellCommandToolCallParams,
        session: &crate::session::session::Session,
        turn_context: &TurnContext,
        turn_environment: &TurnEnvironment,
        cwd: AbsolutePathBuf,
        allow_login_shell: bool,
    ) -> Result<ExecParams, FunctionCallError> {
        let session_shell = session.user_shell();
        let shell = if params.shell.is_some() {
            Self::effective_shell(params, session)?
        } else {
            turn_environment
                .shell
                .as_ref()
                .map_or_else(|| session_shell.as_ref().clone(), Clone::clone)
        };
        Self::validate_command_supported(&shell, &params.command)?;
        let use_login_shell = Self::resolve_use_login_shell(params.login, allow_login_shell)?;
        let command = Self::base_command(&shell, &params.command, use_login_shell);

        let mut env = create_env(
            &turn_context.config.permissions.shell_environment_policy,
            Some(session.thread_id),
        );

        #[cfg(windows)]
        if matches!(
            shell.shell_type,
            crate::shell::ShellType::Bash
                | crate::shell::ShellType::Zsh
                | crate::shell::ShellType::Sh
        ) {
            let shell_dir = shell.shell_path.parent();
            let shell_dir_name = shell_dir
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let shell_parent_name = shell_dir
                .and_then(|path| path.parent())
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let candidates = if shell_dir_name.eq_ignore_ascii_case("bin")
                && shell_parent_name.eq_ignore_ascii_case("usr")
            {
                shell_dir
                    .and_then(|path| path.parent())
                    .and_then(|path| path.parent())
                    .map(|root| {
                        vec![
                            root.join("ucrt64").join("bin"),
                            root.join("usr").join("local").join("bin"),
                            root.join("usr").join("bin"),
                            root.join("bin"),
                        ]
                    })
                    .unwrap_or_default()
            } else if shell_dir_name.eq_ignore_ascii_case("bin") {
                shell_dir
                    .and_then(|path| path.parent())
                    .map(|root| vec![root.join("usr").join("bin"), root.join("bin")])
                    .unwrap_or_default()
            } else {
                shell_dir.into_iter().map(PathBuf::from).collect()
            };

            let path_keys = env
                .keys()
                .filter(|key| key.eq_ignore_ascii_case("PATH"))
                .cloned()
                .collect::<Vec<_>>();
            let path_key = path_keys
                .first()
                .cloned()
                .unwrap_or_else(|| "PATH".to_string());
            let inherited_path = path_keys
                .iter()
                .find_map(|key| env.get(key))
                .cloned()
                .unwrap_or_default();
            for key in path_keys {
                env.remove(&key);
            }

            let mut seen = std::collections::HashSet::new();
            let path_entries = candidates
                .into_iter()
                .filter(|path| path.is_dir())
                .map(|path| path.to_string_lossy().into_owned())
                .chain(inherited_path.split(';').map(str::to_string))
                .filter(|entry| !entry.is_empty())
                .filter(|entry| {
                    seen.insert(
                        entry
                            .replace('/', "\\")
                            .trim_end_matches('\\')
                            .to_ascii_lowercase(),
                    )
                })
                .collect::<Vec<_>>()
                .join(";");
            env.insert(path_key, path_entries);
        }

        let active_permission_profile = turn_context.config.permissions.active_permission_profile();
        inject_permission_profile_env(&mut env, active_permission_profile.as_ref());

        Ok(ExecParams {
            command,
            cwd,
            expiration: params.timeout_ms.into(),
            capture_policy: ExecCapturePolicy::ShellTool,
            env,
            network: turn_context.network.clone(),
            network_environment_id: Some(turn_environment.environment_id.clone()),
            sandbox_permissions: params.sandbox_permissions.unwrap_or_default(),
            windows_sandbox_level: turn_context.windows_sandbox_level,
            windows_sandbox_private_desktop: turn_context
                .config
                .permissions
                .windows_sandbox_private_desktop,
            justification: params.justification.clone(),
            arg0: None,
        })
    }
}

impl From<ShellCommandBackendConfig> for ShellCommandHandler {
    fn from(backend_config: ShellCommandBackendConfig) -> Self {
        Self::new(ShellCommandHandlerOptions {
            backend_config,
            allow_login_shell: false,
            exec_permission_approvals_enabled: false,
        })
    }
}

impl ToolExecutor<ToolInvocation> for ShellCommandHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("shell_command")
    }

    fn spec(&self) -> ToolSpec {
        create_shell_command_tool(CommandToolOptions {
            allow_login_shell: self.options.allow_login_shell,
            exec_permission_approvals_enabled: self.options.exec_permission_approvals_enabled,
            active_shell_name: None,
        })
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    fn handle(&self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'_> {
        Box::pin(self.handle_call(invocation))
    }
}

impl ShellCommandHandler {
    async fn handle_call(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Box<dyn crate::tools::context::ToolOutput>, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            step_context,
            cancellation_token,
            tracker,
            call_id,
            payload,
            ..
        } = invocation;

        let tool_name = self.tool_name();
        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(format!(
                "unsupported payload for shell_command handler: {tool_name}"
            )));
        };

        let Some(turn_environment) = step_context.environments.primary().cloned() else {
            return Err(FunctionCallError::RespondToModel(
                "shell is unavailable in this session".to_string(),
            ));
        };

        let environment_cwd = turn_environment.cwd().to_abs_path().map_err(|err| {
            FunctionCallError::RespondToModel(format!(
                "shell_command cwd `{}` is not native to the Codex host: {err}",
                turn_environment.cwd()
            ))
        })?;
        let cwd = resolve_workdir_base_path(&arguments, &environment_cwd)?;
        let params: ShellCommandToolCallParams = parse_arguments_with_base_path(&arguments, &cwd)?;
        Self::maybe_update_session_shell(&params, session.as_ref())?;
        maybe_emit_implicit_skill_invocation(
            session.as_ref(),
            turn.as_ref(),
            &params.command,
            &cwd,
        )
        .await;
        let prefix_rule = params.prefix_rule.clone();
        let exec_params = Self::to_exec_params(
            &params,
            session.as_ref(),
            turn.as_ref(),
            &turn_environment,
            cwd,
            turn.config.permissions.allow_login_shell,
        )?;
        let shell_type = Some(
            turn_environment
                .shell
                .as_ref()
                .map_or_else(|| session.user_shell().shell_type, |shell| shell.shell_type),
        );
        run_exec_like(RunExecLikeArgs {
            tool_name,
            exec_params,
            cancellation_token,
            hook_command: params.command,
            shell_type,
            additional_permissions: params.additional_permissions.clone(),
            prefix_rule,
            session,
            turn,
            turn_environment,
            tracker,
            call_id,
            shell_runtime_backend: self.shell_runtime_backend(),
        })
        .await
        .map(boxed_tool_output)
    }
}

impl CoreToolRuntime for ShellCommandHandler {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }

    fn waits_for_runtime_cancellation(&self) -> bool {
        true
    }

    fn pre_tool_use_payload(&self, invocation: &ToolInvocation) -> Option<PreToolUsePayload> {
        shell_command_payload_command(&invocation.payload).map(|command| PreToolUsePayload {
            tool_name: HookToolName::bash(),
            tool_input: serde_json::json!({ "command": command }),
        })
    }

    fn with_updated_hook_input(
        &self,
        mut invocation: ToolInvocation,
        updated_input: serde_json::Value,
    ) -> Result<ToolInvocation, FunctionCallError> {
        let ToolPayload::Function { arguments } = invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "hook input rewrite received unsupported shell_command payload".to_string(),
            ));
        };
        invocation.payload = ToolPayload::Function {
            arguments: rewrite_function_string_argument(
                &arguments,
                "shell_command",
                "command",
                updated_hook_command(&updated_input)?,
            )?,
        };
        Ok(invocation)
    }

    fn post_tool_use_payload(
        &self,
        invocation: &ToolInvocation,
        result: &dyn crate::tools::context::ToolOutput,
    ) -> Option<PostToolUsePayload> {
        let tool_response =
            result.post_tool_use_response(&invocation.call_id, &invocation.payload)?;
        let command = shell_command_payload_command(&invocation.payload)?;
        Some(PostToolUsePayload {
            tool_name: HookToolName::bash(),
            tool_use_id: invocation.call_id.clone(),
            tool_input: serde_json::json!({ "command": command }),
            tool_response,
        })
    }
}
