use codex_rmcp_client::SurfaceNotification;
use serde_json::Value;
use serde_json::json;

use super::*;

fn turn_state() -> Arc<tokio::sync::Mutex<TurnState>> {
    Arc::new(tokio::sync::Mutex::new(TurnState::default()))
}

fn notification(
    method: &str,
    source: Option<&str>,
    message_id: Option<&str>,
    payload: Value,
) -> McpServerNotification {
    McpServerNotification {
        server_name: "server".to_string(),
        notification: SurfaceNotification {
            method: method.to_string(),
            source: source.map(str::to_string),
            message_id: message_id.map(str::to_string),
            payload,
        },
    }
}

#[test]
fn buffers_notifications_until_delivery_is_enabled() {
    let mut state = McpNotificationState::default();
    let turn_state = turn_state();
    assert_eq!(
        state.enqueue(
            notification(
                "notifications/message",
                None,
                Some("event-1"),
                json!({ "value": 1 }),
            ),
            Arc::clone(&turn_state)
        ),
        McpNotificationEnqueueOutcome::Queued {
            schedule_worker: false,
            dropped_oldest: false,
        }
    );

    assert!(state.enable_delivery());
    assert_eq!(state.drain_pending_for_turn(&turn_state).len(), 1);
}

#[test]
fn dedupe_identity_is_scoped_by_method_and_source() {
    let mut state = McpNotificationState::default();
    let turn_state = turn_state();
    let first = notification(
        "notifications/message",
        Some("source-a"),
        Some("event-1"),
        json!({ "value": 1 }),
    );
    assert!(matches!(
        state.enqueue(first.clone(), Arc::clone(&turn_state)),
        McpNotificationEnqueueOutcome::Queued { .. }
    ));
    assert_eq!(
        state.enqueue(first, Arc::clone(&turn_state)),
        McpNotificationEnqueueOutcome::Duplicate
    );
    assert!(matches!(
        state.enqueue(
            notification(
                "notifications/custom",
                Some("source-a"),
                Some("event-1"),
                json!({ "value": 2 }),
            ),
            Arc::clone(&turn_state)
        ),
        McpNotificationEnqueueOutcome::Queued { .. }
    ));
    assert!(matches!(
        state.enqueue(
            notification(
                "notifications/message",
                Some("source-b"),
                Some("event-1"),
                json!({ "value": 3 }),
            ),
            Arc::clone(&turn_state)
        ),
        McpNotificationEnqueueOutcome::Queued { .. }
    ));
}

#[test]
fn coalesces_identical_idless_notifications_but_keeps_changed_payloads() {
    let mut state = McpNotificationState::default();
    let turn_state = turn_state();
    let first = notification(
        "notifications/resources/list_changed",
        None,
        None,
        json!({}),
    );
    assert!(matches!(
        state.enqueue(first.clone(), Arc::clone(&turn_state)),
        McpNotificationEnqueueOutcome::Queued { .. }
    ));
    assert_eq!(
        state.enqueue(first, Arc::clone(&turn_state)),
        McpNotificationEnqueueOutcome::Duplicate
    );
    assert!(matches!(
        state.enqueue(
            notification(
                "notifications/resources/list_changed",
                None,
                None,
                json!({ "revision": 2 }),
            ),
            Arc::clone(&turn_state)
        ),
        McpNotificationEnqueueOutcome::Queued { .. }
    ));
}

#[test]
fn pending_queue_is_bounded_and_keeps_the_newest_notifications() {
    let mut state = McpNotificationState::default();
    let turn_state = turn_state();
    for index in 0..MCP_NOTIFICATION_QUEUE_CAPACITY {
        assert!(matches!(
            state.enqueue(
                notification(
                    "notifications/message",
                    None,
                    None,
                    json!({ "index": index }),
                ),
                Arc::clone(&turn_state)
            ),
            McpNotificationEnqueueOutcome::Queued {
                dropped_oldest: false,
                ..
            }
        ));
    }
    assert!(matches!(
        state.enqueue(
            notification(
                "notifications/message",
                None,
                None,
                json!({ "index": MCP_NOTIFICATION_QUEUE_CAPACITY }),
            ),
            Arc::clone(&turn_state)
        ),
        McpNotificationEnqueueOutcome::Queued {
            dropped_oldest: true,
            ..
        }
    ));

    let pending = state.drain_pending_for_turn(&turn_state);
    assert_eq!(pending.len(), MCP_NOTIFICATION_QUEUE_CAPACITY);
    assert_eq!(pending[0].notification.payload, json!({ "index": 1 }));
}

#[test]
fn evicted_identified_notification_can_be_retried() {
    let mut state = McpNotificationState::default();
    let turn_state = turn_state();
    let first = notification(
        "notifications/message",
        None,
        Some("event-0"),
        json!({ "index": 0 }),
    );
    assert!(matches!(
        state.enqueue(first.clone(), Arc::clone(&turn_state)),
        McpNotificationEnqueueOutcome::Queued { .. }
    ));
    for index in 1..=MCP_NOTIFICATION_QUEUE_CAPACITY {
        assert!(matches!(
            state.enqueue(
                notification(
                    "notifications/message",
                    None,
                    Some(&format!("event-{index}")),
                    json!({ "index": index }),
                ),
                Arc::clone(&turn_state)
            ),
            McpNotificationEnqueueOutcome::Queued { .. }
        ));
    }

    assert!(matches!(
        state.enqueue(first, Arc::clone(&turn_state)),
        McpNotificationEnqueueOutcome::Queued { .. }
    ));
}

#[test]
fn notification_discarded_with_a_stopped_turn_can_be_retried() {
    let mut state = McpNotificationState::default();
    let stopped_turn = turn_state();
    let later_turn = turn_state();
    let notification = notification(
        "notifications/message",
        None,
        Some("retry-event"),
        json!({ "value": 1 }),
    );

    assert!(matches!(
        state.enqueue(notification.clone(), stopped_turn),
        McpNotificationEnqueueOutcome::Queued { .. }
    ));
    assert!(matches!(
        state.enqueue(notification, Arc::clone(&later_turn)),
        McpNotificationEnqueueOutcome::Queued { .. }
    ));
    assert_eq!(state.drain_pending_for_turn(&later_turn).len(), 1);
}

#[test]
fn dedupe_evicts_oldest_identity() {
    let mut state = McpNotificationState::default();
    assert!(state.mark_seen("first".to_string()));
    assert!(!state.mark_seen("first".to_string()));

    for index in 0..MCP_NOTIFICATION_DEDUPE_CAPACITY {
        assert!(state.mark_seen(format!("event-{index}")));
    }

    assert!(state.mark_seen("first".to_string()));
}
