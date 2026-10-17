use rmcp::model::CustomNotification;
use rmcp::model::ProtocolVersion;
use rmcp::model::ServerConfig;
use rmcp::model::ServerNotification;
use rmcp::service::NotificationContext;
use rmcp::service::RoleServer;

pub(super) fn configure(mut info: ServerConfig) -> ServerConfig {
    if std::env::var_os("MCP_TEST_CHANNEL_INBOX").is_some() {
        info.protocol_version = ProtocolVersion::V_2025_06_18;
        info.capabilities
            .experimental
            .get_or_insert_default()
            .insert("claude/channel".to_string(), Default::default());
        if std::env::var_os("MCP_TEST_CHANNEL_NO_TOOLS").is_some() {
            info.capabilities.tools = None;
        }
    }
    info
}

pub(super) fn listen(context: NotificationContext<RoleServer>) {
    let Some(inbox) = std::env::var_os("MCP_TEST_CHANNEL_INBOX") else {
        return;
    };
    tokio::spawn(async move {
        let mut delivered = 0;
        while !context.peer.is_transport_closed() {
            if let Ok(text) = std::fs::read_to_string(&inbox) {
                for line in text.lines().skip(delivered) {
                    let Ok(params) = serde_json::from_str(line) else {
                        break;
                    };
                    if context
                        .peer
                        .send_notification(ServerNotification::CustomNotification(
                            CustomNotification::new("notifications/claude/channel", Some(params)),
                        ))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    delivered += 1;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    });
}
