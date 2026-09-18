# Temporary upstream divergences

These fork-only changes should be removed once upstream provides the same or equivalent behavior.
Review every entry when merging or rebasing from upstream; prefer the upstream implementation over
maintaining parallel logic or compatibility fallbacks.

## Inbound MCP notifications

- Local behavior: an opt-in path surfaces standard inbound MCP notifications to Codex sessions.
- Remove when: upstream provides an official MCP notification or session-ingress surface.

## Migration line endings matching the Windows release

- Added: 2026-09-11.
- Local behavior: `.gitattributes` pins state-crate SQL files to CRLF, matching all 67
  migration checksums embedded in the official Windows x64 `rust-v0.154.0` release.
- Reason: SQLx hashes the migration text at build time. A mixed checkout produced LF
  checksums for state migrations 19, 20, and 32 and logs migration 1, preventing the
  fork from opening databases initialized by the stock Windows executable.
- Scope: this matches the Windows release; it does not establish compatibility with
  upstream Linux/macOS builds or databases created by earlier mixed-ending fork builds.
- Remove when: upstream provides an explicit migration line-ending policy that preserves
  compatibility with existing Windows databases.
- Local code: `.gitattributes`.

## Optional Luna reserve fallback

- Added: 2026-09-11.
- Local behavior: `features.luna_reserve_fallback` defaults to `false`. Reserve usage banners
  do not switch models or hold submissions, and existing Reserve tasks retain the normal model
  picker. Set the flag to `true` to restore Reserve fallback and recovery behavior.
- Reason: local ChatGPT account usage can differ from the independent upstream credentials used
  for inference through a gateway. Actual inference errors and other usage banners still apply.
- Remove when: upstream scopes reserve fallback to the credentials used for inference or provides
  an equivalent opt-out.
- Local code: `codex-rs/features/src/lib.rs`, `codex-rs/tui/src/chatwidget/backend_banners.rs`.

## Image generation with independent provider credentials

- Added: 2026-09-11.
- Local behavior: the image tool's Free-plan exclusion applies only when requests use the
  local ChatGPT authentication path. Providers with an environment API key, explicit bearer
  token, or actor-authorized route defer entitlement to the server on actual image requests.
- Reason: a local Free account does not describe the upstream account used by a gateway such
  as codex-lb. No eligibility probes or test generations are sent; server failures remain
  recoverable tool errors. Other feature, model, and provider eligibility gates still apply.
- Remove when: upstream makes image-generation plan gating follow request authentication.
- Local code: `codex-rs/core/src/tools/spec_plan.rs`.

## Owner-provided environment configuration on repeated TUI turns

- Added: 2026-08-21.
- Local behavior: app-server omits redundant environment overrides when `turn/start` repeats the
  current attachment, for both explicit selections and default-environment requests. Task runtime
  workspace roots remain independently updated using upstream's thread-settings representation.
- Reason: the TUI sends `cwd` and runtime workspace roots on every turn. Upstream stable
  `rust-v0.149.0` can reconstruct `local` as `FromThread` and reject the turn after owner-provided
  environment configuration becomes pending, ready, or failed.
- Upstream context: PR #39278 introduced the ownership guard; PR #39597 separated thread settings
  from environment configuration.
- Remove when: upstream prevents redundant TUI environment overrides while preserving
  owner-provided configuration.
- Local code: `codex-rs/app-server/src/request_processors/turn_processor.rs`.
- Regression coverage: exercise the default-environment override builder against live core threads
  with pending, ready, and failed owner configuration; verify task roots still update and changed
  attachments remain subject to ownership validation. The v0.155.0 merge had dropped the
  default-environment check while retaining only the explicit-selection check.
