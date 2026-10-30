# Fork maintenance notes

These fork-only changes should be removed once upstream provides the same or equivalent behavior.
Review every entry when merging or rebasing from upstream; prefer the upstream implementation over
maintaining parallel logic or compatibility fallbacks.

## Local Windows release workflow

- Local behavior: `scripts/local-release` builds the fork's Windows executables and
  hpatch companion, then invokes the upstream package builder to assemble `../output`.
  The CLI PATH entry is `../output/bin`; complete packages support daemon startup.
  Successful packaging removes all `output.previous-*` backups, retaining only the
  latest package; a failed replacement restores the previous output.
  Both recompile scripts then deploy and pin that package through
  `app-server daemon update --from-cli --yes`, restarting a running daemon and
  verifying the selected and running versions. Packaging alone leaves daemon
  deployment to the caller.
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
- The v0.159.0 merge uses upstream's environment configuration for refreshed snapshots
  and removes the obsolete snapshot receiver from the fork's `Shell` type.
- Remove when: upstream provides equivalent command batching and persistent shell selection.
- Local code: `core/src/tools/spec_plan.rs`, `core/src/tools/parallel.rs`,
  `core/src/tools/handlers/command_stack.rs`, and `core/src/environment_selection.rs`
  under `codex-rs`.

## Claude-compatible MCP channels

- Local behavior: per-server `channel = true` opts stdio servers into the
  `claude/channel` capability and `notifications/claude/channel` contract.
  Bounded session queues start idle automatic turns and batch busy arrivals.
  Reply tools use normal MCP discovery; one-way channels also receive server
  instructions without requiring a tool catalog.
- This replaces `surface_notifications`, generic notification context, and
  VSMCP-specific parsing completely. No configuration alias or legacy ingress.
  VSMCP's server migration is deferred; its logging messages no longer enter model context.
- Permission relay and HTTP channel delivery are outside this implementation.
- Remove when: upstream provides equivalent channel ingress and scheduling.

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
- Ownership: only the launching turn parks on or automatically receives a
  command. Turn completion and interruption release the turn's unfinished
  commands, so a later turn never waits on them. Parking without any
  completion for `background_command_turn_wait_ms` (default 10 minutes)
  releases them and gives the model one bounded `<commands_still_running>`
  notice. Released commands keep running and stay readable, including
  through completion waits. Added 2026-10-16 after a subagent's final report
  was withheld indefinitely by hung SSH commands from earlier work.
- The v0.159.0 merge uses upstream's shared output buffers and sandbox
  attribution while retaining the fork's completion ownership and result cache.
- Remove when: upstream provides equivalent completion consumption, runtime
  waiting, and stack dependency semantics.
- Review stages: gated execution contract; coordinator/context/turn waiting;
  stack and hook integration; lifecycle/request-count regression tests.
- Local code: `codex-rs/core/src/unified_exec/managed*.rs`,
  `codex-rs/core/src/context/command_completion.rs`,
  `codex-rs/core/src/context/commands_still_running.rs`, turn teardown in
  `codex-rs/core/src/tasks/mod.rs`, the `background_command_turn_wait_ms` config,
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

## Stable v0.159.2 integration

- The v0.159.2 merge adopts upstream's shared background process constructor,
  Windows job containment console policy, hook launch fixes, and sandbox refresh
  suppression, alongside the updated Bedrock model catalogs.
- Fork command batching, completion delivery, and notification behavior continue
  using the upstream interfaces. The Windows daemon startup default remains
  disabled pending runtime verification of all affected launch paths.

## Stable v0.160.0 integration

- Persistent shell selection uses upstream's `start_environment` helper and shared
  pending configuration results, retaining the selected shell in the thread mutex.
  Child threads inherit upstream's pending owner configuration behavior.
- Explicit provider model catalogs use upstream's authoritative catalog policy;
  fork gateway authentication and catalog refresh behavior remain integrated.
- Windows daemon startup remains disabled by default pending the runtime checks
  documented in the local maintenance note.

## Stable v0.160.1 integration

- Remote MCP launches adopt upstream preservation of Windows system and temporary
  directory variables alongside explicit remote environment variables. The fork
  notification path and temporary Windows daemon default remain unchanged.

## Stable v0.162.0 integration

- Command batching uses upstream stable environment declarations, current shell
  invocation handling, and environment requests that bind thread capability roots.
- hpatch preserves its translation working directory through upstream patch
  verification; line ending handling follows the current upstream patch engine.
- The fork owner-configuration guard compares caller requests against attached
  selections while retaining independent task-root updates.
- The latest-only local-release output change remains in place.
- On 2026-10-10, the upstream daemon startup default was restored on Windows after
  installed v0.162.0-doca runtime probes confirmed direct commands, PowerShell,
  Bash, MCP startup, and a session hook had no console when launched from a
  detached app-server. The three upstream console regressions also passed,
  including the production sandbox-refresh launcher with a disposable helper.
  Sandbox provisioning itself was not exercised. The temporary daemon startup
  fixture override and local reversion note were removed.

## Stable v0.162.1 integration

- Daemon feature compatibility checks follow upstream: only explicit CLI feature
  overrides request a shared-service restart; daemon defaults and file settings
  remain authoritative until restart, subject to managed requirements.
- Multiline async questions preserve line breaks and links using upstream rendering
  and snapshot coverage. The restored daemon startup default remains enabled on
  every platform.
- The fork release smoke check expects daemon_auto_start to be true on every
  platform, matching the restored upstream default.

## Stable v0.163.0 integration

- Read-only tool parallelism follows upstream's per-tool scheduling flags.
  `command_stack` retains its exclusive outer dispatch and bounded, independent
  child scheduling; patches and dependent steps retain their ordering.
- Persistent local shell selection now uses upstream's standard shell executor.
  The retired patched-zsh backend and packaged zsh resources are removed.
- Managed command completion adopts upstream's remote output-drain policy and
  output retention, preserving final polling after the child exits.
- Inbound MCP notifications use the current RMCP client configuration and MCP
  refresh interfaces. Sandbox metadata follows upstream's dedicated module.
- Local recompilation still deploys the complete package to the managed daemon.
  Fork releases use version `0.163.0-doca`.

## Stable v0.164.0 integration

- Persistent shell selection follows upstream's separate active and configured
  environment bindings. Selecting a local shell refreshes both views while
  preserving snapshots already captured by running commands and owner configuration.
- Command batching retains hidden child tools and managed completion semantics with
  upstream's stable tool declarations, readiness checks, and shell/login parameters.
- Claude channels retain one-way server startup without a tools capability alongside
  upstream EMA authentication and tool catalog metadata.
- Runtime completion notices and channel context use upstream typed content items;
  existing bounds, attribution, and model-visible text remain unchanged.
- hpatch resolves its companion on the executor using upstream's current process
  sandbox preparation and credential-broker handling.
- The fork release workflow follows upstream Rust 1.99.1; package builds use the
  pinned V8 152.2.0 artifacts. Daemon auto-start retains the upstream enabled default.

## Stable v0.165.0 integration

- Stdin polling and finite completion reads use upstream's terminal objects and
  live environment resolver. Retained finite results preserve the original process,
  call, and environment identity after leaving the live process store.
- Command completion retains the upstream protection against process ID reuse;
  later reads cannot redirect an existing terminal operation to a replacement.
- Claude channels coexist with upstream MCP network-policy capability metadata
  and shared tool environment selection. Persistent shell selection, hpatch,
  request-auth image eligibility, and turn-scoped completion delivery remain.
- Upstream private publishing and signing workflows remain disabled. Release
  packages use version `0.165.0-doca`; daemon auto-start stays enabled.

## Stable v0.166.0 integration

- Command batching and finite completion use upstream prepared execution requests,
  shared head/tail output capture, and direct-tool timing. Persistent shell selection
  is applied before preparation and preserves captured environment snapshots.
- hpatch keeps translator working directories and original permission-hook inputs
  through upstream prepared patch requests and the shared approval path.
- Claude channels coexist with upstream MCP startup/reconnect changes. Their
  listeners start even with cached tools, and connections remain session-owned.
- The existing 200000-byte Guardian review bound moves with upstream's extracted
  reviewer crate; the former core implementation is removed.
- Complete packages include upstream's separate codex-exec-server executable and
  the fork hpatch companion. The fork workflow follows upstream Rust 1.95.0 and
  pinned V8 source/artifacts. Windows CI enables long dependency paths. Daemon
  auto-start remains enabled on every platform. The existing Starlark LF checkout
  policy covers BUILD/MODULE/WORKSPACE files too, preserving escaped continuations.
