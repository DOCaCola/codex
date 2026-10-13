use codex_app_server_protocol::BackgroundJob;
use codex_app_server_protocol::BackgroundJobDelivery;
use codex_app_server_protocol::BackgroundJobMode;
use codex_app_server_protocol::BackgroundJobState;
use codex_app_server_protocol::ClientResponsePayload;
use codex_app_server_protocol::CodexdcBackgroundJobsListResponse;
use codex_app_server_protocol::CodexdcCapabilitiesResponse;
use codex_app_server_protocol::CodexdcFeatures;
use codex_app_server_protocol::JSONRPCErrorError;
use std::collections::BTreeMap;

#[derive(Clone)]
pub(crate) struct CodexdcRequestProcessor {
    config_manager: crate::config_manager::ConfigManager,
}

impl CodexdcRequestProcessor {
    pub(crate) async fn background_work_read(
        &self,
        params: codex_app_server_protocol::CodexdcBackgroundJobsListParams,
        threads: &super::ThreadRequestProcessor,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        let (thread_id, _) = threads.load_thread(&params.thread_id).await?;
        let state = threads.thread_state_manager.thread_state(thread_id).await;
        let sender = state.lock().await.listener_command_tx().ok_or_else(|| {
            crate::error_code::invalid_request("thread background-work listener is unavailable")
        })?;
        let (reply, result) = tokio::sync::oneshot::channel();
        sender
            .send(crate::thread_state::ThreadListenerCommand::ReadBackgroundWork { reply })
            .map_err(|_| {
                crate::error_code::internal_error("thread background-work listener closed")
            })?;
        let snapshot = result.await.map_err(|_| {
            crate::error_code::internal_error("thread background-work read cancelled")
        })?;
        Ok(Some(snapshot.into()))
    }

    pub(crate) fn new(config_manager: crate::config_manager::ConfigManager) -> Self {
        Self { config_manager }
    }

    pub(crate) async fn capabilities(
        &self,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        let config = self
            .config_manager
            .load_non_project_config()
            .await
            .map_err(|error| {
                crate::error_code::internal_error(format!(
                    "failed to load base configuration: {error}"
                ))
            })?;
        Ok(Some(
            CodexdcCapabilitiesResponse {
                fork: "codexdc".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                protocol: BTreeMap::from([
                    ("backgroundJobs".to_string(), 1),
                    ("backgroundWork".to_string(), 1),
                ]),
                features: CodexdcFeatures {
                    background_jobs: config
                        .features
                        .enabled(codex_features::Feature::BackgroundJobs),
                },
            }
            .into(),
        ))
    }

    pub(crate) async fn background_jobs_list(
        &self,
        params: codex_app_server_protocol::CodexdcBackgroundJobsListParams,
        threads: &super::ThreadRequestProcessor,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        use codex_core::background_jobs as core;
        let (_, thread) = threads.load_thread(&params.thread_id).await?;
        let enabled = thread
            .config()
            .await
            .features
            .enabled(codex_features::Feature::BackgroundJobs);
        let mut data = Vec::new();
        if enabled {
            for job in thread.list_background_jobs().await.map_err(|error| {
                crate::error_code::internal_error(format!(
                    "failed to list background jobs: {error}"
                ))
            })? {
                data.push(BackgroundJob {
                    job_id: job.job_id,
                    item_id: job.item_id,
                    process_id: job.process_id,
                    mode: match job.mode {
                        core::BackgroundJobMode::Auto => BackgroundJobMode::Auto,
                        core::BackgroundJobMode::Background => BackgroundJobMode::Background,
                        core::BackgroundJobMode::Foreground => BackgroundJobMode::Foreground,
                        core::BackgroundJobMode::Interactive => BackgroundJobMode::Interactive,
                    },
                    state: match job.state {
                        core::BackgroundJobState::Running => BackgroundJobState::Running,
                        core::BackgroundJobState::Completed => BackgroundJobState::Completed,
                        core::BackgroundJobState::Failed => BackgroundJobState::Failed,
                        core::BackgroundJobState::TimedOut => BackgroundJobState::TimedOut,
                        core::BackgroundJobState::Stopped => BackgroundJobState::Stopped,
                    },
                    delivery: match job.delivery {
                        core::BackgroundJobDelivery::Pending => BackgroundJobDelivery::Pending,
                        core::BackgroundJobDelivery::Delivered => BackgroundJobDelivery::Delivered,
                        core::BackgroundJobDelivery::None => BackgroundJobDelivery::None,
                    },
                    started_at_ms: job.started_at_ms,
                    timeout_ms: job.timeout_ms,
                    deadline_at_ms: job.deadline_at_ms,
                    completed_at_ms: job.completed_at_ms,
                    exit_code: job.exit_code,
                });
            }
        }
        Ok(Some(
            CodexdcBackgroundJobsListResponse { enabled, data }.into(),
        ))
    }
}
