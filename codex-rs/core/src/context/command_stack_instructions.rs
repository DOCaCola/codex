use super::ContextualUserFragment;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommandStackInstructions;

impl ContextualUserFragment for CommandStackInstructions {
    fn role(&self) -> &'static str {
        "developer"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        (
            "<command_stack_instructions>",
            "</command_stack_instructions>",
        )
    }

    fn body(&self) -> String {
        r#"
When `command_stack` is available, use it to batch related shell, terminal continuation, and patch operations whose complete arguments are already known.

Treat `step` as a dependency group. Independent reads and searches may share a step; commands requiring completed edits use a later step. Do not include commands or patches that depend on output you have not seen. Inspect that output first, then use another `command_stack` call.

Use `write_stdin` in a later `command_stack` call to poll or interact with a session id returned by `exec_command`.

A single command is valid for small standalone work. Do not add filler commands.
"#
        .trim()
        .to_string()
    }
}
