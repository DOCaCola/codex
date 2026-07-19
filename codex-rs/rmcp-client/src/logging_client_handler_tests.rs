use pretty_assertions::assert_eq;
use serde_json::json;

use super::*;

#[test]
fn vsmcp_event_preserves_attribution_and_identity() {
    let notification = surface_notification(
        "notifications/message".to_string(),
        Some("vsmcp.visualstudio.dialogs".to_string()),
        json!({
            "sequence": 17,
            "timestamp": "2026-07-19T10:00:00.0000000+00:00",
            "kind": "ui.modalDialogOpened",
            "payload": { "dialog": { "title": "Breakpoint warning" } }
        }),
    );

    assert_eq!(
        notification,
        SurfaceNotification {
            method: "notifications/message".to_string(),
            source: Some("vsmcp.visualstudio.dialogs".to_string()),
            message_id: Some("2026-07-19T10:00:00.0000000+00:00:17".to_string()),
            payload: json!({
                "sequence": 17,
                "timestamp": "2026-07-19T10:00:00.0000000+00:00",
                "kind": "ui.modalDialogOpened",
                "payload": { "dialog": { "title": "Breakpoint warning" } }
            }),
        }
    );
}

#[test]
fn generic_payload_id_is_not_treated_as_notification_identity() {
    let notification = surface_notification(
        "notifications/custom".to_string(),
        None,
        json!({ "id": "job-1", "status": "running" }),
    );

    assert_eq!(notification.message_id, None);
}
