use codex_protocol::models::ContentItemKind;
use serde_json::Value;

use super::ContextualUserFragment;

const MAX_NOTIFICATION_PAYLOAD_CHARS: usize = 12_000;
const MAX_ATTRIBUTION_CHARS: usize = 512;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct McpNotification {
    server: String,
    method: String,
    source: Option<String>,
    message_id: Option<String>,
    payload: Value,
}

impl McpNotification {
    pub(crate) fn new(
        server: String,
        method: String,
        source: Option<String>,
        message_id: Option<String>,
        payload: Value,
    ) -> Self {
        Self {
            server: truncate_text(server, MAX_ATTRIBUTION_CHARS),
            method: truncate_text(method, MAX_ATTRIBUTION_CHARS),
            source: source.map(|source| truncate_text(source, MAX_ATTRIBUTION_CHARS)),
            message_id: message_id
                .map(|message_id| truncate_text(message_id, MAX_ATTRIBUTION_CHARS)),
            payload: bounded_payload(payload),
        }
    }
}

impl ContextualUserFragment for McpNotification {
    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("mcp.notification".to_string())
    }

    fn role(&self) -> &'static str {
        "user"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("<mcp_notification>", "</mcp_notification>")
    }

    fn body(&self) -> String {
        format!(
            "\n{}\n",
            serde_json::json!({
                "server": &self.server,
                "method": &self.method,
                "source": &self.source,
                "message_id": &self.message_id,
                "payload": &self.payload,
                "trust": "external notification from an explicitly enabled MCP server; normal approval and sandbox policies still apply",
            })
        )
    }
}

fn bounded_payload(payload: Value) -> Value {
    let rendered = payload.to_string();
    if rendered.chars().count() <= MAX_NOTIFICATION_PAYLOAD_CHARS {
        return payload;
    }
    serde_json::json!({
        "truncated": true,
        "preview": truncate_text(rendered, MAX_NOTIFICATION_PAYLOAD_CHARS),
    })
}

fn truncate_text(text: String, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text;
    }
    let mut truncated = text.chars().take(max_chars).collect::<String>();
    truncated.push_str("...[truncated]");
    truncated
}

#[cfg(test)]
#[path = "mcp_notification_tests.rs"]
mod tests;
