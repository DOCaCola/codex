# Fork maintenance notes

These fork-only changes should be removed once upstream provides the same or equivalent behavior.
Review every entry when merging or rebasing from upstream; prefer the upstream implementation over
maintaining parallel logic or compatibility fallbacks.

## Local Windows release workflow

- Local behavior: `scripts/local-release` builds the fork's Windows executables and
  hpatch companion, then invokes the upstream package builder to assemble `../output`.
  The CLI PATH entry is `../output/bin`; complete packages support daemon startup.
- Remove when: an upstream local build workflow covers the fork companion and local
  output installation.

## Command stack and persistent local shell selection

- Local behavior: `features.command_stack` exposes a direct custom tool through upstream's
  tool registry, with a private child router for command execution and edits. It remains
  available alongside code mode when the model supports custom tools.
- The v0.157.0 merge removes the GPT-5.6 catalog and runtime overrides that disabled
  Responses Lite and code mode. Model capabilities now follow upstream metadata.
- Local shell selection updates upstream's mutex-protected environment state and refreshes
  shell snapshots for future steps while preserving snapshots already captured by running work.
- The v0.159.0-alpha.9 merge uses upstream's environment configuration for refreshed snapshots
  and removes the obsolete snapshot receiver from the fork's `Shell` type.
- Remove when: upstream provides equivalent command batching and persistent shell selection.
- Local code: `core/src/tools/spec_plan.rs`, `core/src/tools/parallel.rs`,
  `core/src/tools/handlers/command_stack.rs`, and `core/src/environment_selection.rs`
  under `codex-rs`.

## Inbound MCP notifications

- Local behavior: an opt-in path surfaces standard inbound MCP notifications to Codex sessions.
- The v0.159.0-alpha.9 merge retains this independently of upstream's per-server
  tool input schema size limit; both settings round-trip through config.
- Remove when: upstream provides an official MCP notification or session-ingress surface.

## Optional finite command completion delivery

- Added: 2026-09-20; baseline `4ba07ce918856766bd8b36dd46a12bf580ba11be`.
- Local behavior: `features.background_command_delivery` defaults to `false`.
  When enabled, finite unified-exec commands automatically deliver bounded
  completion context; the existing logical turn waits in runtime code, without
  sampling the model merely to check status. Foreground and explicit completion
  waits remain synchronous; interactive commands remain manually supervised.
- `command_stack` waits for finite dependencies before advancing and observes
  their exit status. Final-step managed jobs can deliver asynchronously.
  Empty timed reads back off; accepted completions are retained in a bounded,
  session-local cache accessible via `write_stdin`, including remote executors.
- Lifecycle: reuse the active turn and existing cleanup/interrupt behavior.
  No desktop changes or new app-server wire events; no restart-durable jobs.
  A live turn waiting for a command is intentionally still shown as active.
- The v0.159.0-alpha.9 merge uses upstream's shared output buffers and sandbox
  attribution while retaining the fork's completion ownership and result cache.
- Remove when: upstream provides equivalent completion consumption, runtime
  waiting, and stack dependency semantics.
- Review stages: gated execution contract; coordinator/context/turn waiting;
  stack and hook integration; lifecycle/request-count regression tests.
- Local code: `codex-rs/core/src/unified_exec/managed.rs`,
  `codex-rs/core/src/context/command_completion.rs`,
  `codex-rs/core/src/tools/handlers/command_stack_managed.rs`,
  unified-exec handlers, feature registry, and `core/src/session/turn.rs`.

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

## Maintained fork features

These are intentional features to review on every upstream update, rather than
session notes or temporary implementation plans:

- Windows Git Bash shell selection and execution integration.
- `command_stack` grammar, bounded dependency scheduling and managed completion.
- Experimental hpatch translation through the native apply-patch permission path;
  its independently maintained source, toolchain and license contract is in
  `third_party/hpatch/README.md` and `source.json`.
- Windows x64 fork packaging, unsigned draft releases and the `fast-release` profile.
  Only `fork-release.yml` is enabled; do not restore upstream private-runner,
  signing, or registry workflows without adapting and validating them first.

Publication history is based on `rust-v0.156.1` plus the reviewed net fork changes.
Old SDK executables, personal Cargo cache overrides, and temporary integration
notes are excluded. The original local development branches retain that history.
