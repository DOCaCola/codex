use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use pretty_assertions::assert_eq;
use serde_json::json;

use super::*;

#[test]
fn renders_attributed_structured_notification() {
    let item = ContextualUserFragment::into(McpNotification::new(
        "vsmcp".to_string(),
        "notifications/message".to_string(),
        Some("vsmcp.visualstudio".to_string()),
        Some("event-17".to_string()),
        json!({ "kind": "debug.breakpointHit", "payload": { "line": 42 } }),
    ));

    let ResponseItem::Message { role, content, .. } = item else {
        panic!("expected message response item");
    };
    assert_eq!(role, "user");
    let [ContentItem::InputText { text }] = content.as_slice() else {
        panic!("expected one text content item");
    };
    assert!(text.starts_with("<mcp_notification>\n"));
    assert!(text.contains("\"server\":\"vsmcp\""));
    assert!(text.contains("\"kind\":\"debug.breakpointHit\""));
    assert!(text.ends_with("\n</mcp_notification>"));
}

#[test]
fn truncates_oversized_payload() {
    let fragment = McpNotification::new(
        "vsmcp".to_string(),
        "notifications/message".to_string(),
        None,
        None,
        json!({ "value": "x".repeat(MAX_NOTIFICATION_PAYLOAD_CHARS + 1) }),
    );

    let rendered = fragment.render();
    assert!(rendered.contains("\"truncated\":true"));
    assert!(rendered.contains("...[truncated]"));
}
