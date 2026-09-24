//! The single acceptance path for finite command results and their hooks.
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::context::CommandCompletion;
use crate::context::ContextualUserFragment;
use crate::tools::context::ExecCommandToolOutput;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::unified_exec::UnifiedExecContext;
use crate::unified_exec::UnifiedExecError;
use crate::unified_exec::UnifiedExecProcessManager;
use crate::unified_exec::WriteStdinRequest;

impl UnifiedExecProcessManager {
    /// Append at most four bounded completion fragments at a sampling boundary.
    pub(crate) async fn record_command_completions(&self, context: &UnifiedExecContext) -> bool {
        let jobs = self
            .managed
            .jobs
            .lock()
            .await
            .iter()
            .map(|(id, job)| (*id, Arc::clone(job)))
            .collect::<Vec<_>>();
        let mut items = Vec::new();
        for (id, job) in jobs {
            if job.revoked.is_cancelled()
                || !job.initial_response_done.is_cancelled()
                || job.initial_response_consumed.load(Ordering::Acquire)
                || !job.process.has_exited()
            {
                continue;
            }
            let Ok(mut state) = Arc::clone(&job.state).try_lock_owned() else {
                continue;
            };
            if state.acknowledged {
                continue;
            }
            let result = self
                .write_stdin(
                    context,
                    WriteStdinRequest {
                        process_id: id,
                        input: "",
                        yield_time_ms: 5_000,
                        max_output_tokens: None,
                        truncation_policy: context
                            .step_context
                            .settings
                            .model_info
                            .truncation_policy
                            .into(),
                        interaction_event: None,
                    },
                )
                .await;
            let result = match result {
                Ok(mut output) => {
                    output.completion_delivery = Some(true);
                    self.accept_managed_output(context, output).await
                }
                Err(error) => Err(error),
            };
            let fragment = match result {
                Ok(output) => {
                    let fragment = CommandCompletion::from_output(id, &output);
                    state.result = Some(output);
                    fragment
                }
                Err(error) => {
                    state.error = Some(error.to_string());
                    CommandCompletion::failed(id, &error.to_string())
                }
            };
            if !job.revoked.is_cancelled() {
                items.push(ContextualUserFragment::into(fragment));
            }
            state.acknowledged = true;
            if items.len() == 4 {
                break;
            }
        }
        if items.is_empty() {
            return false;
        }
        context
            .session
            .record_conversation_items(
                &context.step_context.turn,
                &context.step_context.settings.model_info,
                &items,
            )
            .await;
        true
    }

    pub(super) async fn accept_managed_output(
        &self,
        context: &UnifiedExecContext,
        mut output: ExecCommandToolOutput,
    ) -> Result<ExecCommandToolOutput, UnifiedExecError> {
        let payload = ToolPayload::Function {
            arguments: String::new(),
        };
        if output.success_for_logging()
            && let (Some(input), Some(response)) = (
                output.post_tool_use_input(&payload),
                output.post_tool_use_response(&output.event_call_id, &payload),
            )
        {
            let name = crate::tools::hook_names::HookToolName::bash();
            let outcome = crate::hook_runtime::run_post_tool_use_hooks(
                &context.session,
                context.step_context.as_ref(),
                output.event_call_id.clone(),
                name.name().to_string(),
                name.matcher_aliases().to_vec(),
                input,
                response,
            )
            .await;
            crate::hook_runtime::record_additional_contexts(
                &context.session,
                &context.step_context.turn,
                outcome.additional_contexts,
            )
            .await;
            if outcome.should_block {
                return Err(UnifiedExecError::process_failed(
                    outcome
                        .feedback_message
                        .unwrap_or_else(|| "PostToolUse hook blocked the tool result".into()),
                ));
            }
            if let Some(feedback) = outcome.feedback_message {
                output.raw_output = feedback.into_bytes();
                output.original_token_count = None;
                output.output_omitted_bytes = None;
            }
        }
        // Both explicit completion waits and automatic delivery accept exactly once.
        output.hook_command = None;
        Ok(output)
    }
}
