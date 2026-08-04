use std::sync::Arc;

use rmcp::ClientHandler;
use rmcp::RoleClient;
use rmcp::model::CancelledNotificationParam;
use rmcp::model::ClientInfo;
use rmcp::model::CustomNotification;
use rmcp::model::ElicitRequestParams;
use rmcp::model::ElicitResult;
#[allow(deprecated)]
use rmcp::model::LoggingLevel;
#[allow(deprecated)]
use rmcp::model::LoggingMessageNotificationParam;
use rmcp::model::ProgressNotificationParam;
use rmcp::model::ResourceUpdatedNotificationParam;
use rmcp::service::NotificationContext;
use rmcp::service::RequestContext;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::warn;

use crate::rmcp_client::Elicitation;
use crate::rmcp_client::SendElicitation;
use crate::rmcp_client::SendNotification;
use crate::rmcp_client::SurfaceNotification;

#[derive(Clone)]
pub(crate) struct LoggingClientHandler {
    client_info: ClientInfo,
    send_elicitation: Arc<SendElicitation>,
    send_notification: Option<Arc<SendNotification>>,
}

impl LoggingClientHandler {
    pub(crate) fn new(
        client_info: ClientInfo,
        send_elicitation: SendElicitation,
        send_notification: Option<SendNotification>,
    ) -> Self {
        Self {
            client_info,
            send_elicitation: Arc::new(send_elicitation),
            send_notification: send_notification.map(Arc::new),
        }
    }

    async fn send_surface_notification(&self, notification: SurfaceNotification) {
        let Some(send_notification) = self.send_notification.as_ref() else {
            return;
        };
        if let Err(error) = send_notification(notification).await {
            warn!("failed to surface MCP notification: {error:#}");
        }
    }
}

impl ClientHandler for LoggingClientHandler {
    async fn on_custom_notification(
        &self,
        notification: CustomNotification,
        _context: NotificationContext<RoleClient>,
    ) {
        let CustomNotification { method, params, .. } = notification;
        let payload = params.unwrap_or(serde_json::Value::Null);
        self.send_surface_notification(surface_notification(method, None, payload))
            .await;
    }

    async fn create_elicitation(
        &self,
        request: ElicitRequestParams,
        context: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, rmcp::ErrorData> {
        (self.send_elicitation)(context.id, Elicitation::Mcp(request))
            .await
            .map(Into::into)
            .map_err(|err| rmcp::ErrorData::internal_error(err.to_string(), None))
    }

    async fn on_cancelled(
        &self,
        params: CancelledNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        info!(
            "MCP server cancelled request (request_id: {:?}, reason: {:?})",
            params.request_id, params.reason
        );
    }

    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        info!(
            "MCP server progress notification (token: {:?}, progress: {}, total: {:?}, message: {:?})",
            params.progress_token, params.progress, params.total, params.message
        );
    }

    async fn on_resource_updated(
        &self,
        params: ResourceUpdatedNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        info!("MCP server resource updated (uri: {})", params.uri);
        self.send_surface_notification(surface_notification(
            "notifications/resources/updated".to_string(),
            None,
            serde_json::json!({ "uri": params.uri }),
        ))
        .await;
    }

    async fn on_resource_list_changed(&self, _context: NotificationContext<RoleClient>) {
        info!("MCP server resource list changed");
        self.send_surface_notification(surface_notification(
            "notifications/resources/list_changed".to_string(),
            None,
            serde_json::json!({}),
        ))
        .await;
    }

    async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
        info!("MCP server tool list changed");
        self.send_surface_notification(surface_notification(
            "notifications/tools/list_changed".to_string(),
            None,
            serde_json::json!({}),
        ))
        .await;
    }

    async fn on_prompt_list_changed(&self, _context: NotificationContext<RoleClient>) {
        info!("MCP server prompt list changed");
        self.send_surface_notification(surface_notification(
            "notifications/prompts/list_changed".to_string(),
            None,
            serde_json::json!({}),
        ))
        .await;
    }

    fn get_info(&self) -> ClientInfo {
        self.client_info.clone()
    }

    #[allow(deprecated)]
    async fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        let LoggingMessageNotificationParam {
            level,
            logger,
            data,
            ..
        } = params;
        let logger = logger.as_deref();
        match &level {
            LoggingLevel::Emergency
            | LoggingLevel::Alert
            | LoggingLevel::Critical
            | LoggingLevel::Error => {
                error!(
                    "MCP server log message (level: {:?}, logger: {:?}, data: {})",
                    level, logger, data
                );
            }
            LoggingLevel::Warning => {
                warn!(
                    "MCP server log message (level: {:?}, logger: {:?}, data: {})",
                    level, logger, data
                );
            }
            LoggingLevel::Notice | LoggingLevel::Info => {
                info!(
                    "MCP server log message (level: {:?}, logger: {:?}, data: {})",
                    level, logger, data
                );
            }
            LoggingLevel::Debug => {
                debug!(
                    "MCP server log message (level: {:?}, logger: {:?}, data: {})",
                    level, logger, data
                );
            }
        }

        let mut notification = surface_notification(
            "notifications/message".to_string(),
            logger.map(str::to_string),
            data.clone(),
        );
        notification.payload = serde_json::json!({
            "level": level,
            "logger": logger,
            "data": data,
        });
        self.send_surface_notification(notification).await;
    }
}

fn surface_notification(
    method: String,
    fallback_source: Option<String>,
    payload: serde_json::Value,
) -> SurfaceNotification {
    let object = payload.as_object();
    let source = object
        .and_then(|object| value_string(object, &["source"]))
        .or(fallback_source);
    let message_id = object
        .and_then(|object| value_string(object, &["msgId", "messageId", "eventId"]))
        .or_else(|| vsmcp_message_id(object));
    SurfaceNotification {
        method,
        source,
        message_id,
        payload,
    }
}

fn value_string(
    object: &serde_json::Map<String, serde_json::Value>,
    keys: &[&str],
) -> Option<String> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    })
}

fn vsmcp_message_id(object: Option<&serde_json::Map<String, serde_json::Value>>) -> Option<String> {
    let object = object?;
    let timestamp = object.get("timestamp")?.as_str()?;
    let sequence = object.get("sequence")?.as_i64()?;
    Some(format!("{timestamp}:{sequence}"))
}

#[cfg(test)]
#[path = "logging_client_handler_tests.rs"]
mod tests;
