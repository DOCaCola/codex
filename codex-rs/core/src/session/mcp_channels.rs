use std::collections::VecDeque;
use std::sync::Arc;

use anyhow::Result;
use codex_mcp::McpChannelMessage;
use codex_mcp::SendMcpChannelMessage;
use codex_protocol::turn_input::TurnInput;
use codex_protocol::turn_input::TurnInputMode;
use codex_protocol::turn_input::TurnInputRequest;
use codex_protocol::turn_input::TurnInputSubmission;
use codex_protocol::turn_input::TurnStartOptions;
use futures::FutureExt;
use tracing::warn;

use super::Session;
use crate::context::ChannelMessage;
use crate::context::ContextualUserFragment;
use crate::context::channel_message::MAX_CHANNEL_CONTEXT_BYTES;

const CHANNEL_QUEUE_CAPACITY: usize = 16;
const CHANNEL_INSTRUCTION_SOURCES: usize = 64;

/// Session-owned FIFO. Identical notifications remain distinct events.
#[derive(Debug, Default)]
pub(super) struct McpChannelState {
    closed: bool,
    pending: VecDeque<McpChannelMessage>,
    instructions: VecDeque<(String, Option<String>)>,
}

impl McpChannelState {
    fn enqueue(&mut self, message: McpChannelMessage) -> bool {
        if message.server_name.len() > 256
            || !message.notification.is_bounded()
            || ChannelMessage::new(
                &message.server_name,
                &message.notification,
                /*include_instructions*/ true,
            )
            .rendered_len()
                > MAX_CHANNEL_CONTEXT_BYTES
        {
            warn!("discarding oversized MCP channel context");
            return false;
        }
        if self.closed {
            return false;
        }
        if self.pending.len() == CHANNEL_QUEUE_CAPACITY {
            warn!("MCP channel queue full; discarding incoming event");
            return false;
        }
        self.pending.push_back(message);
        true
    }
}

impl Session {
    pub(super) fn new_mcp_channel_sender(self: &Arc<Self>) -> SendMcpChannelMessage {
        let session = Arc::downgrade(self);
        Arc::new(move |message| {
            let session = session.clone();
            async move {
                let Some(session) = session.upgrade() else {
                    return Ok(());
                };
                session.enqueue_mcp_channel_message(message).await
            }
            .boxed()
        })
    }

    pub(super) fn mcp_channel_sender(self: &Arc<Self>) -> SendMcpChannelMessage {
        self.mcp_channel_sender
            .get_or_init(|| self.new_mcp_channel_sender())
            .clone()
    }

    async fn enqueue_mcp_channel_message(&self, message: McpChannelMessage) -> Result<()> {
        if self.mcp_channel_state.lock().await.enqueue(message) {
            self.mcp_channel_wakeup.notify_one();
        }
        Ok(())
    }

    /// Runs only in the submission loop, serialized with user input and interruption.
    pub(super) async fn dispatch_mcp_channels(self: &Arc<Self>) {
        if self.active_turn.lock().await.is_some() {
            return;
        }
        let (count, context, instructions) = {
            let mut state = self.mcp_channel_state.lock().await;
            state.pending.retain(|message| {
                self.services.mcp_runtime.accepts_channel_message(message)
                    && message
                        .notification
                        .connection
                        .as_ref()
                        .is_none_or(|peer| !peer.is_transport_closed())
            });
            let mut instructions = state.instructions.clone();
            let mut context: Option<ChannelMessage> = None;
            let mut count = 0;
            for message in &state.pending {
                let entry = (
                    message.server_name.clone(),
                    message.notification.instructions.clone(),
                );
                let include_instructions = !instructions.contains(&entry);
                let fragment = ChannelMessage::new(
                    &message.server_name,
                    &message.notification,
                    include_instructions,
                );
                if let Some(context) = &mut context {
                    if context.rendered_len() + 1 + fragment.rendered_len()
                        > MAX_CHANNEL_CONTEXT_BYTES
                    {
                        break;
                    }
                    context.append(fragment);
                } else {
                    context = Some(fragment);
                }
                if include_instructions {
                    instructions.retain(|(server, _)| server != &message.server_name);
                    instructions.push_back(entry);
                    if instructions.len() > CHANNEL_INSTRUCTION_SOURCES {
                        instructions.pop_front();
                    }
                }
                count += 1;
            }
            let Some(context) = context else { return };
            (count, context, instructions)
        };
        let result = super::turn_input::handle(
            self,
            TurnInputRequest::new(TurnInput::ResponseItem(ContextualUserFragment::into(
                context,
            )))
            .on_start(TurnStartOptions {
                turn_trigger: Some("mcp_channel".to_string()),
                ..Default::default()
            }),
            TurnInputMode::StartIfIdle,
            super::new_submission_id(),
        )
        .await;
        match result {
            Ok(TurnInputSubmission::Started { .. }) => {
                let mut state = self.mcp_channel_state.lock().await;
                state.pending.drain(..count);
                state.instructions = instructions;
            }
            Ok(_) => {}
            Err(error) => {
                warn!("could not start MCP channel turn: {error}");
            }
        }
    }

    pub(super) async fn clear_mcp_channels(&self) {
        let mut state = self.mcp_channel_state.lock().await;
        state.pending.clear();
        state.instructions.clear();
    }

    pub(super) async fn close_mcp_channels(&self) {
        let mut state = self.mcp_channel_state.lock().await;
        state.closed = true;
        state.pending.clear();
    }
}

#[cfg(test)]
#[path = "mcp_channels_tests.rs"]
mod tests;
