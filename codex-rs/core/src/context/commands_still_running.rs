use std::time::Duration;

use super::ContextualUserFragment;
use codex_protocol::models::ContentItemKind;

const MAX_LISTED: usize = 8;
const MAX_COMMAND_BYTES: usize = 200;

/// Runtime-produced context telling the model that its turn stopped waiting for
/// commands it launched. Listing at most eight sessions with bounded command
/// previews keeps the fragment below 3,000 bytes.
pub(crate) struct CommandsStillRunning {
    waited: Duration,
    listed: Vec<RunningCommand>,
    unlisted: usize,
}

struct RunningCommand {
    process_id: i32,
    running_for: Duration,
    command: String,
}

impl CommandsStillRunning {
    pub(crate) fn new<'a>(
        waited: Duration,
        commands: impl ExactSizeIterator<Item = (i32, &'a str, Duration)>,
    ) -> Self {
        let unlisted = commands.len().saturating_sub(MAX_LISTED);
        let listed = commands
            .take(MAX_LISTED)
            .map(|(process_id, command, running_for)| RunningCommand {
                process_id,
                running_for,
                command: command[..command.floor_char_boundary(MAX_COMMAND_BYTES)].to_string(),
            })
            .collect();
        Self {
            waited,
            listed,
            unlisted,
        }
    }
}

impl ContextualUserFragment for CommandsStillRunning {
    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("tools.commands_still_running".into())
    }
    fn role(&self) -> &'static str {
        "user"
    }
    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }
    fn type_markers() -> (&'static str, &'static str) {
        ("<commands_still_running>", "</commands_still_running>")
    }
    fn body(&self) -> String {
        let mut sessions = String::new();
        for command in &self.listed {
            sessions.push_str(&format!(
                "- session {}, running {}s: {}\n",
                command.process_id,
                command.running_for.as_secs(),
                command.command,
            ));
        }
        if self.unlisted > 0 {
            sessions.push_str(&format!("- {} more sessions\n", self.unlisted));
        }
        format!(
            "\nNo command finished during {}s of waiting, so this turn no longer waits for these running command sessions and will not receive their results automatically:\n{sessions}If a result is still required, use write_stdin with chars=\"\" and wait_mode=\"completion\" to wait for it, or read current output with write_stdin. Otherwise continue; the commands keep running as background sessions.\n",
            self.waited.as_secs(),
        )
    }
}

#[cfg(test)]
#[path = "commands_still_running_tests.rs"]
mod tests;
