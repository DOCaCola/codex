# Temporary upstream divergences

These fork-only changes should be removed once upstream provides the same or equivalent behavior.
Review every entry when merging or rebasing from upstream; prefer the upstream implementation over
maintaining parallel logic or compatibility fallbacks.

## Inbound MCP notifications

- Local behavior: an opt-in path surfaces standard inbound MCP notifications to Codex sessions.
- Remove when: upstream provides an official MCP notification or session-ingress surface.

## Owner-provided environment configuration on repeated TUI turns

- Added: 2026-08-21.
- Local behavior: app-server omits redundant environment overrides when `turn/start` repeats the
  current attachment.
- Reason: the TUI sends `cwd` and runtime workspace roots on every turn. Upstream stable
  `rust-v0.149.0` can reconstruct `local` as `FromThread` and reject the turn after owner-provided
  environment configuration becomes pending, ready, or failed.
- Upstream context: PR #39278 introduced the ownership guard; PR #39597 separated thread settings
  from environment configuration.
- Remove when: upstream prevents redundant TUI environment overrides while preserving
  owner-provided configuration.
- Local code: `codex-rs/app-server/src/request_processors/turn_processor.rs`.
