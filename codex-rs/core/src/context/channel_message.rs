use codex_protocol::models::ContentItemKind;
use codex_rmcp_client::ChannelNotification;

use super::ContextualUserFragment;

pub(crate) const MAX_CHANNEL_CONTEXT_BYTES: usize = 8_000;

/// Externally attributed channel input, including its host-owned routing source.
#[derive(Debug, Clone)]
pub(crate) struct ChannelMessage {
    text: String,
}

impl ChannelMessage {
    pub(crate) fn new(
        server: &str,
        notification: &ChannelNotification,
        include_instructions: bool,
    ) -> Self {
        let source = escape(server);
        let mut text = String::new();
        if include_instructions && let Some(instructions) = &notification.instructions {
            text.push_str(&format!(
                "<channel_instructions source=\"{source}\">\n{}\n</channel_instructions>\n",
                escape(instructions)
            ));
        }
        text.push_str(&format!("<channel source=\"{source}\""));
        for (key, value) in &notification.meta {
            if key != "source"
                && !key.is_empty()
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                text.push_str(&format!(" {key}=\"{}\"", escape(value)));
            }
        }
        text.push_str(&format!(">\n{}\n</channel>", escape(&notification.content)));
        Self { text }
    }

    pub(crate) fn rendered_len(&self) -> usize {
        self.text.len()
    }

    pub(crate) fn append(&mut self, other: Self) {
        self.text.push('\n');
        self.text.push_str(&other.text);
    }
}

impl ContextualUserFragment for ChannelMessage {
    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("mcp.channel".to_string())
    }
    fn role(&self) -> &'static str {
        "user"
    }
    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }
    fn type_markers() -> (&'static str, &'static str) {
        ("<channel ", "</channel>")
    }
    fn matches_text(text: &str) -> bool {
        let text = text.trim();
        (text.starts_with("<channel ") || text.starts_with("<channel_instructions "))
            && text.ends_with("</channel>")
    }
    fn body(&self) -> String {
        self.text.clone()
    }
    fn render(&self) -> String {
        self.body()
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
#[path = "channel_message_tests.rs"]
mod tests;
