use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use codex_exec_server::CODEX_HPATCH_COMPANION_ARGV0;
use codex_exec_server::ExecMetadata;
use codex_exec_server::ExecOutputStream;
use codex_exec_server::ExecParams;
use codex_exec_server::ProcessSignal;
use codex_exec_server::WriteStatus;
use codex_protocol::models::PermissionProfile;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use codex_utils_path_uri::PathUri;
use uuid::Uuid;

use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::ApplyPatchHandler;
use crate::tools::handlers::hpatch_spec::create_hpatch_freeform_tool;
use crate::tools::handlers::resolve_tool_environment;
use crate::tools::handlers::updated_hook_command;
use crate::tools::hook_names::HookToolName;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::PostToolUsePayload;
use crate::tools::registry::PreToolUsePayload;
use crate::tools::registry::ToolExecutor;

const TRANSLATE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_TRANSLATOR_ERROR_BYTES: usize = 8 * 1024;

pub(crate) struct HpatchHandler {
    multi_environment: bool,
}

impl HpatchHandler {
    pub(crate) fn new(multi_environment: bool) -> Self {
        Self { multi_environment }
    }

    async fn handle_call(
        &self,
        invocation: ToolInvocation,
    ) -> Result<Box<dyn crate::tools::context::ToolOutput>, FunctionCallError> {
        let ToolPayload::Custom { input } = &invocation.payload else {
            return Err(FunctionCallError::RespondToModel(
                "hpatch received unsupported payload".to_string(),
            ));
        };
        let original_input = input.clone();
        let (environment_id, workdir, script) = parse_input(input, self.multi_environment)?;
        let Some(turn_environment) = resolve_tool_environment(
            &invocation.step_context.environments,
            environment_id.as_deref(),
        )?
        else {
            return Err(FunctionCallError::RespondToModel(
                "hpatch is unavailable in this session".to_string(),
            ));
        };
        let (cwd, root) = resolve_translation_paths(
            turn_environment.cwd(),
            turn_environment.workspace_roots(),
            workdir.as_deref(),
        )?;
        let mut sandbox = turn_environment.sandbox_context(None);
        sandbox.permissions = PermissionProfile::read_only();

        let started = turn_environment
            .environment
            .get_exec_backend()
            .start(ExecParams {
                process_id: format!("hpatch-{}", Uuid::new_v4()).into(),
                metadata: Some(ExecMetadata {
                    thread_id: Some(invocation.session.thread_id()),
                    tool_call_id: Some(invocation.call_id.clone()),
                }),
                argv: vec![
                    CODEX_HPATCH_COMPANION_ARGV0.to_string(),
                    "translate".to_string(),
                    "--root".to_string(),
                    root.inferred_native_path_string(),
                    "--cwd".to_string(),
                    cwd.inferred_native_path_string(),
                ],
                cwd: cwd.clone(),
                env_policy: None,
                env: HashMap::from([(
                    "CODEX_HPATCH_DISABLE_USER_DATA".to_string(),
                    "1".to_string(),
                )]),
                tty: false,
                pipe_stdin: true,
                arg0: None,
                shell_snapshot: None,
                sandbox: Some(sandbox),
                enforce_managed_network: false,
                managed_network: None,
                network_proxy: None,
            })
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!(
                    "failed to start bundled hpatch companion: {err}"
                ))
            })?;
        let process = started.process;
        let write = process.write(script.into_bytes()).await.map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to send hpatch script: {err}"))
        })?;
        if write.status != WriteStatus::Accepted {
            process.terminate().await.ok();
            return Err(FunctionCallError::RespondToModel(format!(
                "failed to send hpatch script: stdin status was {:?}",
                write.status
            )));
        }
        process
            .signal(ProcessSignal::CloseStdin)
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!("failed to close hpatch stdin: {err}"))
            })?;

        let translation = tokio::time::timeout(
            TRANSLATE_TIMEOUT,
            collect_translation(Arc::clone(&process), invocation.cancellation_token.clone()),
        )
        .await;
        let (exit_code, stdout, stderr) = match translation {
            Ok(result) => result?,
            Err(_) => {
                process.terminate().await.ok();
                return Err(FunctionCallError::RespondToModel(
                    "hpatch translation timed out after 30 seconds".to_string(),
                ));
            }
        };
        if exit_code != 0 {
            return Err(FunctionCallError::RespondToModel(format!(
                "hpatch rejected the script:\n{}",
                truncate_error(&stderr)
            )));
        }
        if stdout.trim().is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "hpatch produced an empty translated patch".to_string(),
            ));
        }

        let translated_patch = insert_environment_id(stdout, environment_id.as_deref())?;
        let apply_invocation = ToolInvocation {
            payload: ToolPayload::Custom {
                input: translated_patch,
            },
            ..invocation
        };
        ApplyPatchHandler::for_hpatch(self.multi_environment, original_input, root)
            .handle(apply_invocation)
            .await
    }
}

async fn collect_translation(
    process: Arc<dyn codex_exec_server::ExecProcess>,
    cancellation_token: tokio_util::sync::CancellationToken,
) -> Result<(i32, String, String), FunctionCallError> {
    let mut after_seq = 0;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    loop {
        let response = tokio::select! {
            _ = cancellation_token.cancelled() => {
                process.terminate().await.ok();
                return Err(FunctionCallError::RespondToModel("hpatch translation was cancelled".to_string()));
            }
            response = process.read(Some(after_seq), None, Some(5_000)) => response,
        }
        .map_err(|err| {
            FunctionCallError::RespondToModel(format!("failed to read hpatch output: {err}"))
        })?;
        for chunk in response.chunks {
            after_seq = after_seq.max(chunk.seq);
            match chunk.stream {
                ExecOutputStream::Stdout | ExecOutputStream::Pty => stdout.extend(chunk.chunk.0),
                ExecOutputStream::Stderr => stderr.extend(chunk.chunk.0),
            }
        }
        if let Some(failure) = response.failure {
            return Err(FunctionCallError::RespondToModel(format!(
                "hpatch process failed: {failure}"
            )));
        }
        if response.closed {
            return Ok((
                response.exit_code.unwrap_or(-1),
                String::from_utf8_lossy(&stdout).into_owned(),
                String::from_utf8_lossy(&stderr).into_owned(),
            ));
        }
    }
}

fn parse_input(
    input: &str,
    allow_environment_id: bool,
) -> Result<(Option<String>, Option<String>, String), FunctionCallError> {
    let mut remaining = input.strip_prefix('\n').unwrap_or(input);
    let environment_id = take_header(&mut remaining, "*** Environment ID: ");
    if environment_id.is_some() && !allow_environment_id {
        return Err(FunctionCallError::RespondToModel(
            "hpatch environment selection is unavailable for this turn".to_string(),
        ));
    }
    if environment_id.as_deref().is_some_and(str::is_empty) {
        return Err(FunctionCallError::RespondToModel(
            "hpatch environment ID must not be empty".to_string(),
        ));
    }

    let workdir = take_header(&mut remaining, "*** Working Directory: ");
    if workdir.as_deref().is_some_and(str::is_empty) {
        return Err(FunctionCallError::RespondToModel(
            "hpatch working directory must not be empty".to_string(),
        ));
    }

    Ok((environment_id, workdir, remaining.to_string()))
}

fn take_header(input: &mut &str, prefix: &str) -> Option<String> {
    let first_newline = input.find('\n')?;
    let first_line = input[..first_newline].trim_end_matches('\r');
    let value = first_line.strip_prefix(prefix)?;
    *input = &input[first_newline + 1..];
    Some(value.to_string())
}

fn resolve_translation_paths(
    default_cwd: &PathUri,
    workspace_roots: &[PathUri],
    workdir: Option<&str>,
) -> Result<(PathUri, PathUri), FunctionCallError> {
    let cwd = workdir
        .filter(|workdir| !workdir.is_empty())
        .map_or_else(
            || Ok(default_cwd.clone()),
            |workdir| default_cwd.join(workdir),
        )
        .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
    let root = workspace_roots
        .iter()
        .filter(|root| cwd.starts_with(root))
        .max_by_key(|root| root.encoded_path().len())
        .cloned()
        .unwrap_or_else(|| cwd.clone());
    Ok((cwd, root))
}

fn insert_environment_id(
    patch: String,
    environment_id: Option<&str>,
) -> Result<String, FunctionCallError> {
    let Some(environment_id) = environment_id else {
        return Ok(patch);
    };
    let Some(rest) = patch.strip_prefix("*** Begin Patch\n") else {
        return Err(FunctionCallError::RespondToModel(
            "hpatch produced an invalid translated patch".to_string(),
        ));
    };
    Ok(format!(
        "*** Begin Patch\n*** Environment ID: {environment_id}\n{rest}"
    ))
}

fn truncate_error(error: &str) -> String {
    if error.len() <= MAX_TRANSLATOR_ERROR_BYTES {
        return error.trim().to_string();
    }
    let mut end = MAX_TRANSLATOR_ERROR_BYTES;
    while !error.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[output truncated]", error[..end].trim())
}

impl ToolExecutor<ToolInvocation> for HpatchHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain("hpatch")
    }

    fn spec(&self) -> ToolSpec {
        create_hpatch_freeform_tool(self.multi_environment)
    }

    fn handle<'a>(&'a self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'a>
    where
        ToolInvocation: 'a,
    {
        Box::pin(self.handle_call(invocation))
    }
}

impl CoreToolRuntime for HpatchHandler {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Custom { .. })
    }

    fn pre_tool_use_payload(&self, invocation: &ToolInvocation) -> Option<PreToolUsePayload> {
        let ToolPayload::Custom { input } = &invocation.payload else {
            return None;
        };
        Some(PreToolUsePayload {
            tool_name: HookToolName::hpatch(),
            tool_input: serde_json::json!({ "command": input }),
        })
    }

    fn with_updated_hook_input(
        &self,
        mut invocation: ToolInvocation,
        updated_input: serde_json::Value,
    ) -> Result<ToolInvocation, FunctionCallError> {
        let script = updated_hook_command(&updated_input)?;
        invocation.payload = ToolPayload::Custom {
            input: script.to_string(),
        };
        Ok(invocation)
    }

    fn post_tool_use_payload(
        &self,
        invocation: &ToolInvocation,
        result: &dyn crate::tools::context::ToolOutput,
    ) -> Option<PostToolUsePayload> {
        let ToolPayload::Custom { input } = &invocation.payload else {
            return None;
        };
        Some(PostToolUsePayload {
            tool_name: HookToolName::hpatch(),
            tool_use_id: invocation.call_id.clone(),
            tool_input: serde_json::json!({ "command": input }),
            tool_response: result
                .post_tool_use_response(&invocation.call_id, &invocation.payload)?,
        })
    }
}

#[cfg(test)]
#[path = "hpatch_tests.rs"]
mod tests;
