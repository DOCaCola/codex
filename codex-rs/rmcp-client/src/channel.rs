use anyhow::Result;
use futures::future::BoxFuture;
use serde::Deserialize;
use serde::Serialize;

/// Claude channel message delivered by an explicitly enabled server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelNotification {
    pub content: String,
    #[serde(default)]
    pub meta: std::collections::BTreeMap<String, String>,
    #[serde(skip)]
    pub instructions: Option<String>,
    #[serde(skip)]
    pub connection: Option<rmcp::Peer<rmcp::RoleClient>>,
}

impl ChannelNotification {
    /// Bounds retained payloads before forwarding them to the session queue.
    pub fn is_bounded(&self) -> bool {
        self.instructions
            .as_ref()
            .is_none_or(|text| text.len() <= 2_080)
            && self.content.len()
                + self
                    .meta
                    .iter()
                    .map(|(key, value)| key.len() + value.len())
                    .sum::<usize>()
                <= 6_000
    }
}

/// Forwards typed channel messages to the owning session.
pub type SendChannelMessage =
    Box<dyn Fn(ChannelNotification) -> BoxFuture<'static, Result<()>> + Send + Sync>;
