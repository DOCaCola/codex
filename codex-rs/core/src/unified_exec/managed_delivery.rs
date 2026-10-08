//! The single acceptance path for finite command results and their hooks.
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::Job;
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
        let queued = context
            .step_context
            .turn
            .config
            .features
            .enabled(codex_features::Feature::BackgroundJobs);
        let jobs = if queued {
            self.managed.registry.queued().await
        } else {
            self.managed.records().await
        };
        let mut items = Vec::new();
        for (id, job) in jobs {
            let job_id = if queued {
                Some(job.metadata.lock().await.job_id)
            } else {
                None
            };
            if job.revoked.is_cancelled()
                || job.initial_response_consumed.load(Ordering::Acquire)
                || job.acknowledged.load(Ordering::Acquire)
            {
                if let Some(job_id) = job_id {
                    self.managed.registry.consumed(job_id).await;
                }
                continue;
            }
            if !job.initial_response_done.is_cancelled() || !job.process.has_exited() {
                continue;
            }
            let Ok(mut state) = Arc::clone(&job.state).try_lock_owned() else {
                continue;
            };
            if job.acknowledged.load(Ordering::Acquire) {
                continue;
            }
            if let Some(job_id) = job_id
                && !self.managed.registry.contains(job_id).await
            {
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
                    self.accept_managed_output(context, &job, output).await
                }
                Err(error) => Err(error),
            };
            let mut fragment = match result {
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
            {
                let mut metadata = job.metadata.lock().await;
                metadata.finish(&job.process, job.revoked.is_cancelled());
                if queued {
                    fragment = fragment.with_metadata(&metadata);
                }
            }
            if let Some(job_id) = job_id {
                self.managed.registry.consumed(job_id).await;
            }
            if !job.revoked.is_cancelled() {
                items.push(ContextualUserFragment::into(fragment));
            }
            job.acknowledged.store(true, Ordering::Release);
            if items.len() == 4 {
                break;
            }
        }
        self.evict_retired_managed_commands().await;
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
        job: &Job,
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
                job.origin.as_ref(),
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
