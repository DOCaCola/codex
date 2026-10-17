use super::*;
use codex_rmcp_client::ChannelNotification;
use pretty_assertions::assert_eq;

fn message(content: &str) -> McpChannelMessage {
    McpChannelMessage {
        registration: Arc::new(()),
        server_name: "channel".to_string(),
        notification: ChannelNotification {
            content: content.to_string(),
            meta: Default::default(),
            instructions: None,
            connection: Default::default(),
        },
    }
}

#[test]
fn queue_preserves_repeated_events_and_rejects_overflow() {
    let mut state = McpChannelState::default();
    for _ in 0..CHANNEL_QUEUE_CAPACITY {
        assert!(state.enqueue(message("repeat")));
    }
    assert!(!state.enqueue(message("overflow")));
    assert_eq!(
        state
            .pending
            .iter()
            .map(|event| event.notification.content.as_str())
            .collect::<Vec<_>>(),
        vec!["repeat"; CHANNEL_QUEUE_CAPACITY]
    );
}

#[test]
fn admission_bounds_escaped_context_and_metadata_before_retention() {
    let mut state = McpChannelState::default();
    assert!(!state.enqueue(message(&"<".repeat(3_000))));
    let mut event = message("hello");
    event
        .notification
        .meta
        .insert("chat_id".to_string(), "x".repeat(6_000));
    assert!(!state.enqueue(event));
    assert!(state.pending.is_empty());
    assert!(state.enqueue(message("bounded")));
}
