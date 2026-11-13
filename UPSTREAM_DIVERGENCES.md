# Fork maintenance notes

These fork-only changes should be removed once upstream provides the same or equivalent behavior.
Review every entry when merging or rebasing from upstream; prefer the upstream implementation over
maintaining parallel logic or compatibility fallbacks.

## Spawn model list truncation disclosure

- Local behavior: keep the five-entry `spawn_agent` model summary cap and append a
  notice when additional picker-visible, backend-compatible models are omitted.
  The notice tells the agent that other catalog models may be valid and to try an
  explicit override before declaring one unavailable.
- Adapted from Kevinjohn/codex commit `31ed6acb7ce68eecfe039c6230bb8d2993094587`
  linked in upstream issue openai/codex#34166. Request-level tests cover V1/V2,
  exactly five eligible models, truncation, and hidden/disabled entries.
- Remove when: upstream provides equivalent disclosure for the capped tool description.

## Fork release installation and updates

- The standalone installers retain stock paths, locks, versioned packages and
  junction/symlink switching, but use DOCaCola/codex release metadata and assets.
  The release workflow publishes both installers with the complete archives and
  SHA256SUMS; alpha releases are marked as prereleases.
- CLI update execution, notices, doctor and daemon update downloads use the fork.
  Shared release identity in install-context orders numeric `-doca.N` revisions,
  isolates the fork's version cache, and recognizes fork stable daemon packages
  while preserving explicit pins. Registry/package-manager updates are removed
  because they would replace this fork with official OpenAI packages.
- Daemon update checks compare running executable identity because the public
  server version omits the fork suffix; identical binaries do not repeatedly
  restart and fork revisions still select their matching runtime packages.
- Installer verification retains native Windows locked-executable and checksum
  coverage plus macOS package, selection, pin and running-release coverage.
  Native installer jobs gate compilation; manual runs can test only installers.
  Process-retention fixtures compile a native helper and wait for readiness,
  avoiding copied macOS platform executables and process-start races.
- Remove when: upstream supports a complete configurable release source, version
  policy and package format for forks.

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
- Without `background_jobs`, only the launching turn parks on or automatically receives a
  command. Turn completion and interruption release the turn's unfinished
  commands, so a later turn never waits on them. Parking without any
  completion for `background_command_turn_wait_ms` (default 10 minutes)
  releases them and gives the model one bounded `<commands_still_running>`
  notice. Released commands keep running and stay readable, including
  through completion waits. Added 2026-10-16 after a subagent's final report
  was withheld indefinitely by hung SSH commands from earlier work.
- With `background_jobs`, the captured session registry and completion queue
  retain their cross-turn delivery and interruption policy described below.
  The delivery-only turn wait limit does not release opted-in jobs.
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

## CodexDC app-server capability discovery

- Added: 2026-10-07.
- Local behavior: the experimental `codexdc/*` app-server namespace contains
  `codexdc/capabilities`, returning the `codexdc` fork identifier, the app-server's
  crate version, and a feature-to-integer-protocol-version map (initially empty).
  Clients must opt into `experimentalApi` during initialization.
- Reason: CodexDC must identify the backend attached to its own connection before
  enabling fork-specific desktop behavior, independently of backend configuration.
- Remove when: upstream provides equivalent attached-backend capability discovery
  and the desktop integration no longer needs the fork namespace.
- Local code: `codex-rs/app-server-protocol/src/protocol/v2/codexdc.rs`,
  `codex-rs/app-server/src/request_processors/codexdc_processor.rs`.

## Managed command-stack completion and retention contract

- Added: 2026-10-07.
- Local behavior: running stack results carry `running: true` and `success: null`;
  they are neither completed successes nor failures that block later operations.
  Child execution events keep their normal running/completed lifecycle.
- Earlier finite stack steps wait for completion without an implicit deadline.
  Explicit `timeout_ms` still applies; ordinary foreground commands retain the
  existing 300-second default. Dependency policy is internal call state.
- At the 64-result retention threshold, reservation and consumption/delivery
  sweep every eligible retired record. Live and unconsumed jobs remain protected;
  the existing admission capacity and error remain unchanged.
- Remove when: upstream provides equivalent stack dependency and managed result
  retention semantics.
- Local code: `core/src/tools/handlers/command_stack*`, unified-exec handlers,
  and `core/src/unified_exec/{managed,managed_delivery,oneshot,process_manager}.rs`.

## Optional background job metadata

- Added: 2026-10-07.
- `[features] background_jobs = true` enables the under-development feature
  (default false) and implies `background_command_delivery`, through feature
  dependency normalization. Setting only `background_jobs` is sufficient.
- Delivery owns stack correctness, dependency waits, cache eviction, and hook
  attribution. Running stack reporting applies only with delivery enabled.
- `background_jobs` gates richer bounded completion fragments and future job
  listing/continuation/client features. With it off, completion fragment bytes
  retain the existing delivery contract. Internal job metadata may still exist.
- Jobs use UUIDs independent of reusable process handles, and capture origin,
  command/item identity, mode, timestamps, explicit and effective deadlines, and
  terminal outcome together. The original step context stays with the job.
- Experimental `codexdc/capabilities.features.backgroundJobs` resolves the
  effective base configuration on each request; `protocol` remains empty until
  a fork job-listing API exists.
- Remove when: upstream provides equivalent configurable job metadata and
  completion context. Local code: `codex-rs/features`, managed unified exec,
  `core/src/context/command_completion.rs`, and the `codexdc` request processor.

- Experimental `codexdc/backgroundJobs/list` reads a loaded thread's existing
  managed-job records, ordered by start time, with UUID/item/terminal IDs, mode,
  state, start/completion times, explicit timeout, effective deadline, exit code,
  and delivery state. A disabled thread returns `{ enabled: false, data: [] }`.
  Unknown or unloaded threads use the upstream background-terminal lookup error.
- Delivery is independent of execution state: auto/background results are
  pending until consumed (including while running); acknowledged or initially
  consumed results are delivered; unconsumed revoked/manual jobs have none.
  Listing does not consume, finalize, or evict records. The 64-record cache
  pressure rule and protected unconsumed results remain unchanged.
- Capabilities advertise the static protocol revision `backgroundJobs: 1`;
  `features.backgroundJobs` remains the current effective base-config value.


## Optional session job completion queue

- Added: 2026-10-07.
- With `background_jobs`, the session's execution service owns UUID-keyed job
  records and a FIFO completion queue. Process IDs index explicit reads only;
  a retired UUID cannot enqueue a replacement process handle's completion.
- Exit enqueues once. The first sample of every turn and the existing
  post-sampling boundary drain at most four bounded fragments, carrying the
  remaining results. Listing and explicit reads share the registry's delivery
  state; no parallel delivery cache is introduced.
- Turn parking remains the only lifecycle mode. No idle wake, unload, shutdown,
  interrupt policy, or new configuration defaults are introduced by this stage.
  With `background_jobs` off, the prior process-keyed delivery path is retained.
- Remove when: upstream provides equivalent session job identity and queued
  completion delivery. Local code: `core/src/unified_exec/managed_registry.rs`,
  managed delivery, and the first-sample boundary in `core/src/session/turn.rs`.


## Optional background job interrupt and stop policy

- Added: 2026-10-08.
- With `background_jobs`, interrupt ends the turn without killing its auto/
  background jobs. Pending completions stay in the session queue for the next
  user turn's first sample. No idle wake is introduced in this stage.
- Queue acceptance caches output/error in the shared job record and commits
  history before acknowledgement. Cancellation before commit preserves the
  queue; an admitted history commit finishes without scheduling inference.
- Terminal termination and shutdown revoke delivery as before. Ctrl-C through
  `write_stdin` for an opted-in finite job now revokes delivery and terminates
  that exact process. The stopped command's direct tool result can be returned
  without acknowledging its revoked automatic completion or running its hooks.
- With `background_jobs` off, the accepted interrupt, delivery, Ctrl-C and
  foreground one-shot policies remain unchanged. Foreground move-to-background,
  idle continuation, host deadlines, Plan wake policy and exec continuation
  remain later-stage work under the resolved audit decisions.
- Remove when: upstream supplies equivalent interrupt/stop completion ownership.
  Local code: managed queue delivery and unified-exec process manager.


## Optional background work guard for automatic thread unload

- Added: 2026-10-08.
- With `background_jobs`, live managed registry jobs and pending, non-revoked
  auto/background completions count as activity for automatic no-subscriber
  unload. Registry work is an internal core predicate, not a new wire status.
  It scans UUID-owned records, including pending results whose terminal ID has
  been reused; the process-ID index still serves the existing ordered paths.
- The existing subscriber/status countdown also waits on the managed `changed`
  notification, registered before checking work. Delivery/revocation resumes
  the normal idle delay; there is no extra polling timer or notification.
- Explicit client removal (archive/delete) and session/server shutdown keep their
  existing termination/revocation paths. The flag-off managed map is excluded
  from the guard, preserving the accepted unload behavior.
- No idle wake, Plan policy, host deadline or exec continuation is introduced.
  Exec-runner continuation moves to stage 6 with idle continuation; parked turns
  already keep `codex exec` waiting in the current lifecycle mode.
- Remove when: upstream supplies equivalent registry-aware unload protection.
  Local code: core background work accessors and app-server thread_unloading.rs.

## Opt-in background job idle continuation

- Added: 2026-10-08. `background_jobs_idle_continuation` defaults off and implies
  `background_jobs`, which implies delivery. Its capability is captured once
  when the session is created; runtime configuration refresh cannot switch it.
- On: final answers may end a turn while jobs run. Existing registry changes and
  task-finish scheduling feed the existing idle-only continuation admission,
  with an expected previous turn and internal `background_job_completion` input.
  Mailbox work retains priority; user input starts/steers through normal admission.
- Interrupted live/pending UUIDs and jobs from the interrupted turn cannot wake or
  join an internal turn. The next user-started turn clears suppression when its
  regular execution begins. Already admitted delivery commits keep Step 9 rules.
- Plan-mode results wait for user input. A mode change during admission uses the
  existing Automatic Plan rejection, with no retry timer/loop. Flag-off tool
  descriptions and turn parking retain the accepted behavior.
- Exec-runner continuations remain Step 11b; desktop rendering remains a later
  live check. No deadline or foreground conversion is added here.
- Remove when: upstream provides equivalent opt-in completion admission and
  session instruction capabilities. Local code: session/background_continuation.rs.

## Session background command defaults

- Added: 2026-10-08. `[background_jobs]` resolves positive millisecond values for
  unattended deadlines (1800000), managed foreground timeouts (300000), and auto
  initial waits (10000). Settings apply only with `features.background_jobs`.
- `Exec` is unattended. CLI, VSCode, Custom and Unknown are interactive. `Mcp`
  is also interactive: upstream uses that source for AppServer, so it is not an
  unattended-host indicator. Delegates/internal sessions inherit root/owner class.
- Class, feature capability and defaults are captured per session. Interactive
  auto/background has no deadline; explicit model timeout overrides all defaults.
  Enabled deadlines use the existing termination path from the adopted process
  start timestamp, including foreground and requested-mode stack dependencies.
- Only the configured default auto wait (yield omitted, flag on) bypasses the
  legacy Windows floor and yield clamp. Explicit model yields keep the legacy
  clamp on either flag setting. Off retains its defaults and foreground literal.
  Deadline source/origin
  metadata preserves explicit-only timeout labels. No maximum/default reset on
  backgrounding or new exec-runner policy is introduced.
- Remove when: upstream provides equivalent typed defaults and inherited host
  policy. Local code: config/src/background_jobs.rs, core/unified_exec/background_policy.rs.

## Exec background continuation lifetime and codexdc signal

- Added: 2026-10-08. With captured background_jobs_idle_continuation enabled,
  exec follows primary-thread continuation turns instead of exiting on its first
  completion. Flag off retains its original completion/shutdown behavior.
- Experimental codexdc/backgroundWork/read and codexdc/backgroundWork/changed
  expose captured session policy and the Step 10 predicate with active-task and
  last-started-turn fences.
  Reads run through the existing unload listener and synchronize its baseline.
  The listener registers Notify before its ordinary wait snapshot. There is no
  second watcher or polling.
- Exec explicitly opts into experimental transport and makes starts/work changes
  lossless on the opted-in in-process path. Other clients retain experimental
  filtering. BackgroundWork protocol revision 1 is advertised separately.
- Each idle wait uses the resolved unattended deadline. Expiry and Ctrl-C fail
  the run and invoke normal shutdown, which terminates outstanding jobs.
- Human output marks automatic continuation; JSONL keeps existing turn/item
  events. Last-message output and final token summary occur once after the last
  turn. Automatic turns inherit the original user turn's output schema.
- Remove when: upstream supports equivalent exec lifetime and event-driven host
  work signals. Local code: exec/background_wait.rs, thread_unloading.rs, codexdc_processor.rs.

## TUI managed background job timer labels

- Added: 2026-10-08. Captured background_jobs sessions show Claude-style elapsed
  labels on existing exec previews/details, /ps, and background footer/status.
  Explicit model limits appear only on running jobs; host defaults are hidden.
  Completed durations freeze from managed process metadata; no countdown is shown.
- The existing experimental codexdc work signal carries compact timer records
  and the captured capability. Its existing registry/unload watcher emits on
  metadata changes as well as pending transitions. No API polling or watcher is
  added. TUI views accept metadata after starts; timing is finalized before ends.
- TUI frame requests and transcript cache hooks redraw live clocks. Off retains
  prior rendering and snapshots. Local code: tui/background_job_timer.rs,
  tui/chatwidget/background_job_labels.rs, core/unified_exec/managed_listing.rs.
- Remove when: upstream provides equivalent captured managed-job timing labels.

## Background integration with v0.162.1 interfaces

- The continuation request uses upstream's WithTurnExtensionData wrapper.
  Snapshot reads remain under the admission lock inside the session owner,
  without exposing private session state or changing captured capability rules.
- Command-event fixtures use upstream's aggregate output and turn attribution;
  continuation fixtures include root turn lineage. Generated schemas are the
  authority for config and experimental codexdc payloads after integration.
- Compile/artifact validation is separate from behavioral validation. Regression
  coverage verifies older-origin running-job suppression and exec parked-turn
  gates. Suppression snapshots all
  eligible registry identities; wake readiness still reads only queued results.
- Stable schema export gates notifications that share an excluded experimental
  method payload, including the intersection-wrapped notification envelope.
  Existing upstream notification contracts remain intact. Experimental generation
  includes the background-work read/changed variants and all payload definitions.
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

## Background integration with v0.163.0-doca.1

- Delivery-only commands adopt current main's turn ownership, turn-end release
  and bounded no-progress park. Released results remain explicitly readable.
  Session-owned background jobs retain UUID queue ownership, captured deadlines,
  interrupted-result retention and optional idle continuation.
- Managed release reads the existing origin and timing metadata, rather than
  maintaining duplicate owner/command/start records. Retention may evict exited
  released delivery-only records; live and unconsumed registry jobs stay protected.
- Automatic completion admission respects main's terminal-recording fence.
  Reservations cannot match a finishing turn. The existing pending-work helper
  supplies fresh continued turn IDs after active-turn cleanup.
- Claude channels replace the retired custom notification ingress. Their
  automatic `ResponseItem` turns do not mark new user authorization or clear
  background interruption suppression. Channel and background workers both
  retain the existing idle/Plan/mailbox admission checks.
- Remote output remains pollable through output closure; explicit termination
  overrides retention. Fast opted-in commands still receive timing/UUID records.
- Protocol fixtures are regenerated from Rust types using the repository generators.
  Stable exports follow the stable API; experimental exports include the background
  capability/job/work definitions alongside the current main protocol surface.
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

- Release revision `0.166.0-doca.1` applies upstream's
  `CARGO_NET_GIT_FETCH_WITH_CLI=true` CI transport setting to the fork workflow.
  It avoids Cargo's bundled macOS SecureTransport failure while fetching the
  newly pinned V8 source submodules; Rust source and dependency pins are unchanged.

## Background integration with v0.166.0-doca.3

- Background execution uses upstream prepared requests and terminal identities.
  Session-owned jobs retain process-start deadlines, fast-exit metadata, exact
  terminal reads and origin-attributed completion hooks. Delivery-only jobs follow
  main's release contract and remain explicitly readable after interruption.
- Listener startup uses upstream atomic subscriber installation and cold-resume
  admission while retaining registry-aware unload protection and job timers.
  In-process continuation notifications use upstream serialized pending delivery.
- Background wake reservations retain the finishing-token and turn-state identity
  fences. Pending work uses fresh turn IDs and current mailbox wake policy; Claude
  channel startup, dispatch and shutdown retain upstream behavior.


## Stable v0.167.0 integration

- Command batching, persistent shell selection, finite delivery and opted-in
  background jobs use upstream's native-call evidence and extracted PreToolUse
  dispatch. Synthetic stack child calls have no independent native-call evidence;
  translated hpatch calls retain the original invocation and permission hooks.
- Direct Guardian review of recorded commands uses the captured environment's
  shell without changing the session's persistent shell selection. Upstream
  native evidence, hook rewrites, human-approval requirements and elastic context
  budgeting remain authoritative; the fork's aggregate action review bound stays.
- Claude channels retain session-owned callbacks and one-way server startup with
  upstream MCP connection leases and early Apps discovery. Channel callbacks are
  not pooled, and cached channel catalogs do not defer listener startup.
- Config reads retain upstream snapshot identity. The fork no longer clones a
  config during reads to stamp the background host; startup and child/delegate
  creation already capture and inherit it. This preserves refresh admission and
  live managed Guardian policy.
- Background continuation retains upstream turn admission, recording errors and
  audio rejection, plus the fork's captured job policy, interruption fences,
  unload protection, timers and experimental CodexDC protocol. Protocol exports
  are regenerated from the merged Rust definitions.
- Fork installation, release ordering and executable-identity daemon updates stay
  integrated. Rust remains 1.95.0; external dependencies match upstream stable.
  Daemon auto-start remains enabled on every platform. Only the fork release
  workflow is enabled. Workspace version is `0.167.0-doca`.


## Stable v0.167.1 integration

- Adopt upstream route-specific incremental-tool selection: Responses Lite uses
  `incremental_tools`; regular Bedrock Responses uses the opt-in
  `incremental_tools_legacy`. Other regular Responses routes retain their normal
  tool layout. Hosted web search remains in top-level request configuration.
- World-state and remote compaction follow upstream's shared route selection.
  Command-stack declarations and session-owned background completion continue
  through the existing tool router and sampling boundaries.
- New regular-Responses scenario snapshots use the existing fork tool-description
  hashes (persistent shell guidance and spawn-model truncation disclosure), matching
  the already-covered incremental-tool scenarios.
- Retain the existing fork features and defaults. Daemon auto-start remains
  enabled; external dependency pins and Rust 1.95.0 are unchanged.
  Workspace version is `0.167.1-doca`.

## Stable v0.169.0 integration

- Adopt upstream environment attachments and detach ownership, sandbox host
  preparation, outcome handoffs, and turn identity checks. Persistent local shell
  changes update active and configured attachments without altering captured turns.
  hpatch resolves and exposes its companion through the new sandbox runtime paths.
- Background completion keeps upstream task removal and final-input admission.
  Stale targeted aborts cannot suppress completion wakes. The optional continuation
  branch uses a boxed future at the scheduler boundary to bound its future layout.
  Existing delivery, registry, interruption, idle continuation, and timer behavior
  remain opt-in.
- Custom API-key catalogs use upstream's discovery capability and live feature
  policy instead of impersonating first-party authentication. Catalog commands
  retain table, JSON, bundled, and direct remote output; active catalog construction
  uses upstream runtime authentication.
- Claude channels retain session-owned callbacks and eager listener startup under
  both upstream deferred-start policies. Upstream connection sharing, runtime auth,
  and outcome reporting remain authoritative.
- Exec final output uses upstream bound output handles, retaining final-message
  selection across automatic background turns. TUI timers coexist with upstream
  terminal restoration, side chats, and grouped execution labels.
- External dependency pins, SQLite migration contents, and Rust 1.95.0 follow
  upstream. Daemon auto-start remains enabled on every platform. Only the fork
  release workflow is enabled. Workspace version is `0.169.0-doca`.
