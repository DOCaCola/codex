use codex_protocol::ThreadId;
use codex_protocol::models::ShellCommandToolCallParams;
use codex_shell_command::is_safe_command::is_known_safe_command;
use codex_tools::ShellCommandBackendConfig;
use codex_tools::ToolName;
use std::path::PathBuf;

use crate::exec::ExecCapturePolicy;
use crate::exec::ExecParams;
use crate::exec_env::create_env;
use crate::function_tool::FunctionCallError;
use crate::maybe_emit_implicit_skill_invocation;
use crate::session::turn_context::TurnContext;
use crate::shell::Shell;
use crate::shell::get_shell;
use crate::shell_detect::detect_shell_type;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments_with_base_path;
use crate::tools::handlers::resolve_workdir_base_path;
use crate::tools::hook_names::HookToolName;
use crate::tools::registry::PostToolUsePayload;
use crate::tools::registry::PreToolUsePayload;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use crate::tools::runtimes::shell::ShellRuntimeBackend;
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
    options: Option<ShellCommandHandlerOptions>,
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
        Self {
            options: Some(options),
            ..Self::from(options.backend_config)
        }
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
            session.set_user_shell(Self::effective_shell(params, session)?);
        }

        Ok(())
    }

    pub(super) fn to_exec_params(
        params: &ShellCommandToolCallParams,
        session: &crate::session::session::Session,
        turn_context: &TurnContext,
        thread_id: ThreadId,
        allow_login_shell: bool,
    ) -> Result<ExecParams, FunctionCallError> {
        let shell = Self::effective_shell(params, session)?;
        Self::validate_command_supported(&shell, &params.command)?;
        let use_login_shell = Self::resolve_use_login_shell(params.login, allow_login_shell)?;
        let command = Self::base_command(&shell, &params.command, use_login_shell);

        Ok(ExecParams {
            command,
            cwd: turn_context.resolve_path(params.workdir.clone()),
            expiration: params.timeout_ms.into(),
            capture_policy: ExecCapturePolicy::ShellTool,
            env: create_env(&turn_context.shell_environment_policy, Some(thread_id)),
            network: turn_context.network.clone(),
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
    fn from(config: ShellCommandBackendConfig) -> Self {
        let backend = match config {
            ShellCommandBackendConfig::Classic => ShellCommandBackend::Classic,
            ShellCommandBackendConfig::ZshFork => ShellCommandBackend::ZshFork,
        };
        Self {
            backend,
            options: None,
        }
    }
}

impl ToolHandler for ShellCommandHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("shell_command")
    }

    fn spec(&self) -> Option<ToolSpec> {
        self.options.map(|options| {
            create_shell_command_tool(CommandToolOptions {
                allow_login_shell: options.allow_login_shell,
                exec_permission_approvals_enabled: options.exec_permission_approvals_enabled,
                active_shell_name: None,
            })
        })
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        self.options.is_some()
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }

    async fn is_mutating(&self, invocation: &ToolInvocation) -> bool {
        let ToolPayload::Function { arguments } = &invocation.payload else {
            return true;
        };

        serde_json::from_str::<ShellCommandToolCallParams>(arguments)
            .map(|params| {
                let use_login_shell = match Self::resolve_use_login_shell(
                    params.login,
                    invocation.turn.tools_config.allow_login_shell,
                ) {
                    Ok(use_login_shell) => use_login_shell,
                    Err(_) => return true,
                };
                let shell = match Self::effective_shell(&params, invocation.session.as_ref()) {
                    Ok(shell) => shell,
                    Err(_) => return true,
                };
                let command = Self::base_command(&shell, &params.command, use_login_shell);
                !is_known_safe_command(&command)
            })
            .unwrap_or(true)
    }

    fn pre_tool_use_payload(&self, invocation: &ToolInvocation) -> Option<PreToolUsePayload> {
        shell_command_payload_command(&invocation.payload).map(|command| PreToolUsePayload {
            tool_name: HookToolName::bash(),
            tool_input: serde_json::json!({ "command": command }),
        })
    }

    fn post_tool_use_payload(
        &self,
        invocation: &ToolInvocation,
        result: &Self::Output,
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

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            tracker,
            call_id,
            payload,
            ..
        } = invocation;

        let ToolPayload::Function { arguments } = payload else {
            return Err(FunctionCallError::RespondToModel(format!(
                "unsupported payload for shell_command handler: {}",
                self.tool_name().display()
            )));
        };

        let cwd = resolve_workdir_base_path(&arguments, &turn.cwd)?;
        let params: ShellCommandToolCallParams = parse_arguments_with_base_path(&arguments, &cwd)?;
        Self::maybe_update_session_shell(&params, session.as_ref())?;
        let workdir = turn.resolve_path(params.workdir.clone());
        maybe_emit_implicit_skill_invocation(
            session.as_ref(),
            turn.as_ref(),
            &params.command,
            &workdir,
        )
        .await;
        let prefix_rule = params.prefix_rule.clone();
        let exec_params = Self::to_exec_params(
            &params,
            session.as_ref(),
            turn.as_ref(),
            session.conversation_id,
            turn.tools_config.allow_login_shell,
        )?;
        run_exec_like(RunExecLikeArgs {
            tool_name: self.tool_name().display(),
            exec_params,
            hook_command: params.command,
            additional_permissions: params.additional_permissions.clone(),
            prefix_rule,
            session,
            turn,
            tracker,
            call_id,
            freeform: true,
            shell_runtime_backend: self.shell_runtime_backend(),
        })
        .await
    }
}
