# Native hpatch Integration Plan

Temporary implementation note created on July 31, 2026. Keep this file updated while the
feature is being implemented. Remove it before upstreaming unless it is intentionally converted
into permanent design documentation.

## Objective

Add `hpatch` as an experimental, feature-gated Codex editing tool backed by a separately built and
permanently bundled companion executable deployed beside the Codex executable.

The primary goal is to reduce model token usage for file edits. The integration must preserve the
existing Codex desktop bridge behavior for file-change tracking, approvals, turn diffs, history,
and partial application failures.

## Implemented State

The local experiment is implemented behind the disabled-by-default `hpatch` feature flag.

- The model-visible surface replaces `apply_patch` with a compact grammar-constrained `hpatch`
  tool. Native `apply_patch` remains registered but hidden for replay, shell interception, and the
  internal execution path.
- Base instructions receive one idempotently marked hpatch section at the system/base level. The
  section explicitly supersedes stale apply-patch wording without copying the upstream long-form
  prompt.
- Direct and `command_stack` modes both select the same patch frontend from the feature flag.
  Hpatch stack entries are serialized and retain the existing stop-on-patch-failure behavior.
- Translation runs through the selected environment's exec backend with a read-only filesystem
  sandbox. A reserved executor-local argv0 resolves to `hpatch[.exe]` beside the executor's Codex
  binary, including remote exec-server environments.
- The exec-server protocol now supports closing process stdin without terminating the process.
  Hpatch scripts are written as raw stdin bytes, stdin is closed, and stdout/stderr are collected
  with cancellation and a 30-second timeout.
- The generated apply-patch envelope is passed directly to `ApplyPatchHandler`; it is never routed
  as a nested model tool call. Existing apply-patch events, approvals, deltas, turn diffs, and
  desktop file-change tracking remain authoritative.
- Codex hooks and permission-request hooks receive the original hpatch script and the `hpatch`
  tool identity. Internal translated patch text is not exposed to those hooks.
- Local development uses the sibling `../hpatch` Git checkout at local fork commit
  `bed8e9bac069bafb4a253a79ac681a88a36571f0` on its `codex-integration` branch, based on upstream
  commit `ebf7d6fb57e091b0d55416305897884509bd18ca`. The Codex-specific change and its regression test
  live in that fork rather than being applied during every local build.
- The companion builder requires an explicit source checkout and uses Go 1.26.5, static
  cross-compilation, a C-drive cache on Windows, atomic output replacement, and per-output build
  locking. It does not download or mutate hpatch source.
- The fork makes `CODEX_HPATCH_DISABLE_USER_DATA=1` bypass upstream metrics, settings, and hooks.
  Remove this override as soon as upstream exposes an equivalent supported mode.
- Until the fork has a remotely reachable repository, release CI prepares the same pinned source
  with `scripts/prepare-hpatch-companion-source.sh` and the maintained patch under
  `third_party/hpatch`. Delete that release-only patch path once CI can check out the fork commit.
- Canonical packages, installers, Windows/Linux/macOS release bundles, signing/notarization flows,
  and local debug/release sync scripts deploy hpatch beside Codex. Local Rust and Go intermediates
  remain on C; final local artifacts are copied to the D-drive `output` directory.
- The generated config schema includes the feature key. The feature remains off by default and has
  not been added to the local user config.

Focused verification completed on July 31, 2026:

- `cargo check -p codex-core -p codex-exec-server`
- hpatch prompt, grammar, parsing, environment-header, hook payload, tool-plan, and command-stack
  tests
- exec-server sibling resolution and executor-local reserved-path tests
- canonical Windows package-layout test and Python module compilation
- concurrent companion builds targeting the same output
- a real Windows translation smoke test confirming no workspace mutation and no user-config writes
- shell syntax, Rust formatting, generated config schema, and diff whitespace checks

Full app-server history/desktop end-to-end coverage and the full workspace test suite were not run
as part of this local experiment. The implementation deliberately reuses the existing native
apply-patch lifecycle so those public protocol paths are unchanged.

## Fixed Decisions

- Do not add or require an HTTP/Responses router.
- Do not port the hpatch evaluator to Rust for the initial implementation.
- Keep hpatch as a separate process executable, but build and deploy it as a version-locked Codex
  companion artifact rather than requiring a user-managed installation.
- Place `hpatch.exe` beside `codex.exe` on Windows and `hpatch` beside `codex` on Unix in production,
  development distributions, and integration-test layouts.
- Invoke hpatch only in translation mode. It must not mutate workspace files directly.
- Keep the generated `apply_patch` envelope entirely internal to Codex.
- Do not record the generated envelope as a model tool call, nested tool call, response item, or
  rollout item.
- Apply translated changes through the existing native Codex apply-patch runtime.
- Reuse the existing patch event and turn-diff pipeline so the desktop bridge continues to receive
  canonical file-change information.
- Gate the feature behind an under-development feature flag that is disabled by default.
- When enabled, expose `hpatch` instead of `apply_patch` on the model-visible tool surface.
- Keep `apply_patch` registered dispatch-only for history, replay, shell interception, and internal
  translation compatibility.
- Do not copy the complete upstream hpatch tool description into the prompt. Token reduction is the
  feature's purpose, and the upstream description is too large to inject on every request.
- Do not add benchmarks as part of this local experiment. Use focused behavioral and prompt-size
  tests instead.

## Model-Visible Contract

The model receives a grammar-constrained freeform tool named `hpatch`.

The initial grammar should accept complete hpatch scripts only. Do not expose router-owned
correction syntax until Codex has a stateful correction protocol that can apply corrections to a
specific rejected attempt.

The tool description and feature-specific instructions must be compact. They should explain only:

- inspect all required baselines before editing;
- batch independent known edits into one script;
- use `tsel` for stable exact text and `rsel` for complete line ranges;
- use line coordinates from the unchanged baseline;
- do not run hpatch in parallel with tools that can change the same workspace;
- rely on the returned result and native file-change events rather than rereading solely to verify;
- submit a new complete script after a rejected call.

Do not expose both model-visible patch tools by default. Keeping both increases tool-schema tokens,
retains conflicting prompt guidance, and makes tool selection less predictable.

## Feature And Configuration

Add an under-development feature such as:

```text
Feature::Hpatch
key = "hpatch"
default_enabled = false
```

Introduce a resolved patch-tool mode instead of scattering feature checks:

```text
PatchToolMode::ApplyPatch
PatchToolMode::Hpatch
```

The mode should control tool planning, prompt assembly, `command_stack` schema generation, and
handler registration.

The executable should be invoked directly without a shell. Normal production and integration-test
resolution must use the companion path derived from the running Codex executable or the selected
environment's explicit runtime-resource paths. Do not rely on the user's PATH and do not silently
fall back to an unrelated globally installed hpatch.

Because Codex builds and packages the companion, compatibility is a build-time contract. Pin the
hpatch source revision or release used by the Codex build and update it deliberately. Runtime should
validate that the expected sibling artifact exists and is executable, but it should not parse
human-readable help output for capability negotiation.

If the hpatch feature is enabled and the required companion artifact is missing, fail with a short
packaging-error diagnostic. Production packaging and test setup must treat a missing companion as a
broken distribution rather than an optional feature dependency.

## apply_patch Deployment Precedent

Production Codex does not require users to install a separate `apply_patch` executable.

The `codex-apply-patch` crate provides both the reusable library implementation and a standalone
`apply_patch` Cargo binary for direct development and tests. Deployed Codex binaries link the
library through `codex-arg0`.

At startup, `codex-arg0` creates a temporary PATH entry containing:

- a Unix `apply_patch` symlink to the currently running Codex executable; or
- a Windows `apply_patch.bat` wrapper that re-executes the current Codex executable with the hidden
  `--codex-run-as-apply-patch` argument.

The Codex executable then dispatches either by argv0 (`apply_patch`) or by the hidden argv1 switch
and calls the linked `codex_apply_patch` implementation. This gives shell commands an apparent
`apply_patch` executable without shipping or installing another production binary.

The hpatch integration intentionally differs internally: hpatch remains a real companion executable
rather than an arg0 alias into Codex, because its Go evaluator is not embedded in the Rust binary.
Externally, however, it is part of the Codex distribution and must always be deployed beside Codex
in both production and test artifacts.

## Companion Binary Packaging

Build hpatch from an explicit source checkout as part of the Codex distribution pipeline. Local
recompile scripts use `D:\Projekte\Codex Cli Fork\hpatch`; release CI prepares the pinned source in
its runner temp directory. Produce the platform-native companion name:

```text
Windows: codex.exe + hpatch.exe
Unix:    codex     + hpatch
```

The two binaries must be copied into the same final executable directory. This applies to:

- release archives and platform bundles;
- standalone installer layouts and required-file validation;
- managed desktop/daemon installations;
- local release builds copied to the repository's `output` directory;
- Cargo integration-test binary layouts;
- remote-execution helper/resource bundles when hpatch is supported remotely.

The local Windows recompile flow must copy both artifacts to:

```text
D:\Projekte\Codex Cli Fork\output\codex.exe
D:\Projekte\Codex Cli Fork\output\hpatch.exe
```

Build intermediates should remain on the configured C-drive target/temp directories. Only final
distribution artifacts should be copied to the D-drive output directory.

Package creation should stage both binaries before replacing a live distribution. If either build
or copy fails, do not report a complete Codex distribution. Prefer staging and replacing the pair
as one release unit where the platform permits it.

Do not modify PATH to find the companion during normal native hpatch tool execution. Resolve it from
Codex's stable self-executable path. On Windows, derive `hpatch.exe` from the directory containing
`codex.exe`; on Unix, derive `hpatch` from the directory containing `codex`.

For remote environments, extend the explicit exec-server runtime-resource contract with the remote
hpatch companion path. Never use the host Codex sibling to inspect a remote workspace.

Keep required third-party license and attribution material in production packages when this local
experiment becomes a distributed feature.

## Prompt Integration

Existing model base prompts contain multiple `apply_patch` references, so appending a long
developer message is not a complete solution.

Add patch-tool prompt composition to model instruction assembly. The preferred shape is a template
variable such as:

```text
{{ patch_tool_instructions }}
```

Render either compact apply-patch instructions or compact hpatch instructions according to
`PatchToolMode`.

For server-provided model prompts that do not contain the placeholder, append a concise,
idempotently marked section to the resolved base instructions at the same instruction level:

```text
<patch_tool_instructions>
...
</patch_tool_instructions>
```

The fallback section must explicitly state that any generic base-prompt reference to `apply_patch`
is obsolete for this session and that `hpatch` is the available editing tool. Keep this fallback
short because old apply-patch text will still consume context.

Apply patch-tool prompt resolution after the normal base-instruction priority resolution and before
the session configuration is persisted or startup prewarming begins. The selected mode must survive
thread resume and propagate to spawned subagents. Prompt transformation must be idempotent.

A small `HpatchInstructions` contextual fragment may be used as an operational reminder, following
the existing `CommandStackInstructions` precedent. It must not duplicate the full syntax reference.

## Native Execution Flow

1. Receive the model's hpatch freeform script.
2. Resolve the selected turn environment and its workspace root.
3. Select exactly one workspace root. Prefer the root containing the selected environment cwd;
   fail closed when root selection is absent or ambiguous.
4. Resolve the bundled hpatch companion for the selected environment and start it with the chosen
   workspace root as its process cwd.
5. Invoke translation mode directly, currently expected to be equivalent to `hpatch translate`.
6. Pipe the model script to stdin and capture stdout and stderr separately.
7. Run the translator under a read-only Codex sandbox context. Translation must not receive write
   permission merely because the eventual patch may request writes.
8. On translation failure, return a compact actionable diagnostic to the model. Do not begin a
   native file-change item because no verified changes exist yet.
9. On success, treat stdout as the internal apply-patch envelope. Keep stderr as a bounded report or
   warning channel; never echo the generated envelope to the model.
10. Parse and verify the envelope against the same selected environment filesystem and sandbox used
    by native apply_patch verification.
11. Convert it to `ApplyPatchAction` and execute it through the shared native patch execution path.
12. Return a compact success response derived from the verified/applied action, for example the
    number of changed files plus necessary selector-repair or warning notes.

Do not implement the handler by issuing a synthetic nested `apply_patch` tool call. That would add
history noise, risk duplicate lifecycle events, and expose internal translation details.

## Shared Apply-Patch Execution

Refactor the current `ApplyPatchHandler` so parsing/verification is separate from execution of an
already verified `ApplyPatchAction`.

The shared execution function must continue to own:

- affected-path collection;
- effective permission calculation and permission expansion;
- Guardian and user approval behavior;
- `ApplyPatchRuntime` orchestration;
- selected local or remote `TurnEnvironment` filesystem access;
- `ToolEmitter::apply_patch_for_environment` begin/end events;
- `AppliedPatchDelta` reporting, including committed prefixes on failure;
- turn diff tracker updates;
- normal user-visible file-change approval and result behavior.

The hpatch call should have one native patch lifecycle. Translation is preparation for that
lifecycle, not a second user-visible execution.

## Desktop Bridge Requirements

The implementation must preserve the current canonical event path:

```text
ApplyPatchAction
  -> PatchApplyBegin
  -> ApplyPatchRuntime
  -> PatchApplyEnd
  -> TurnDiff
  -> app-server FileChange item / TurnDiffUpdated notification
```

The desktop bridge must continue to receive correct information for:

- added, updated, deleted, and moved files;
- unified diffs;
- approval requests and decisions;
- successful completion;
- rejection, sandbox denial, and operational failure;
- partial committed deltas when a later filesystem operation fails;
- thread history and resume reconstruction.

The bridge should not need hpatch-specific protocol types. The model tool is `hpatch`, while the
desktop-facing result remains a normal Codex file-change item.

## Model Token Policy

Only these hpatch-related values should enter model context:

- the compact tool definition and grammar;
- compact feature-specific instructions;
- the model-authored hpatch script;
- a short success result or actionable failure diagnostic.

These values must remain host-private:

- translated apply-patch envelope;
- full translator stdout;
- verbose success reports;
- internal verification data;
- desktop bridge event payloads unless they are already part of normal Codex model context.

On successful application, prefer a response shaped like:

```text
Applied changes to 3 files.
```

Include paths only when useful and inexpensive. Include selector repairs or translator warnings only
when they affect subsequent edits. Bound and truncate diagnostics using existing Codex output
policies.

Prompt-size tests should prevent accidental inclusion of the full upstream hpatch description and
should compare the feature-enabled fixed prompt/tool overhead against the normal apply-patch mode.

## Hooks

Expose one Codex `PreToolUse` and `PostToolUse` lifecycle for the model-visible `hpatch` call.

- PreToolUse receives the original hpatch script.
- Hook rewrites, if supported, rewrite the hpatch script before translation.
- PostToolUse receives the compact hpatch result and appropriate structured metadata.
- Do not fire a second user-configured apply-patch hook for the internal translated envelope.
- Continue using native patch emitter events for approvals, desktop tracking, and turn diffs.
- Disable hpatch executable-owned hooks and metrics by default when possible so Codex remains the
  lifecycle owner.

If disabling executable-owned hooks requires a data-directory argument or environment variable,
make that behavior explicit and test it.

## command_stack

When both features are enabled, `command_stack` should expose `hpatch` as the patch child instead of
`apply_patch`.

- Add a feature-dependent patch command type or generalize the existing hard-coded apply-patch
  child representation.
- A step containing hpatch runs serially.
- hpatch must not support `continue_on_failure`.
- A failed hpatch operation stops the current step and blocks later steps, matching current patch
  safety semantics.
- Do not include a hpatch operation whose script depends on output from another command in the same
  call. Inspect first and submit a later command-stack call.
- Patch children may provide an explicit `workdir`; resolve it relative to the selected environment
  cwd for hpatch translation.
- Hpatch translation emits workspace-root-relative paths, so native verification and application
  must use the exact translator root rather than the translation cwd.
- Route the child through the normal tool router so hpatch hooks and cancellation behavior remain
  consistent.

## Multi-Environment And Remote Behavior

Support the existing optional environment identifier in multi-environment turns.

The hpatch executable must run in the selected environment because host-local execution cannot
correctly inspect a remote filesystem. Remote Codex/runtime bundles must therefore deploy the same
version-locked hpatch companion and expose its path through the remote runtime-resource contract.

Workspace-root selection and process cwd must use `PathUri` until crossing an environment-native
execution boundary. Do not assume host-native Windows or Unix paths for foreign environments.

## Error And Cancellation Semantics

- Translation cancellation changes no files.
- Translation rejection changes no files.
- Verification rejection changes no files.
- Once native application begins, retain current Codex apply-patch semantics, including reporting a
  known committed prefix if an I/O failure occurs after earlier mutations.
- Do not claim that the complete filesystem commit is atomic unless the native apply-patch runtime
  is separately made transactional.
- Translator stderr on a successful translation is a report/warning channel, not automatically a
  failed tool call.
- Missing executable, unsupported version, ambiguous workspace root, invalid UTF-8, malformed
  translated envelope, and translated paths outside the selected root must produce short explicit
  errors.

## Security Requirements

- Invoke the executable with direct argv, never through shell interpolation.
- Pipe scripts through stdin instead of command-line arguments.
- Run translation with read-only workspace access and no network access.
- Apply writes only through native Codex approval and sandbox handling.
- Resolve all translated paths against the selected environment root and fail closed on traversal or
  foreign-root access.
- Limit captured stdout/stderr sizes to prevent unbounded memory or model-context use.
- Never trust translator output merely because the executable exited successfully; parse and verify
  it through `codex_apply_patch`.

## Expected Code Areas

- `codex-rs/features/src/lib.rs`
- `codex-rs/core/src/tools/spec_plan.rs`
- `codex-rs/core/src/tools/handlers/apply_patch.rs`
- new hpatch handler and tool-spec modules under `codex-rs/core/src/tools/handlers/`
- `codex-rs/core/src/tools/handlers/command_stack.rs`
- `codex-rs/core/src/tools/handlers/command_stack_spec.rs`
- `codex-rs/core/src/context/`
- `codex-rs/core/src/session/mod.rs`
- `codex-rs/protocol/src/openai_models.rs`
- focused app-server and thread-history tests if event behavior changes
- generated configuration schema and feature documentation as required by repository conventions
- `.github/workflows/rust-release.yml`
- `.github/workflows/rust-release-windows.yml`
- `.github/scripts/build-codex-package-archive.sh`
- `scripts/install/install.sh`
- `scripts/install/install.ps1`
- `scripts/sync-output-exe.sh`
- `../recompile_codex_release.sh`
- `../recompile_codex_release.bat`
- `../copy_codex_release_to_output.sh`

## Concrete Test Plan

Add tests alongside each implementation layer instead of relying only on end-to-end coverage.

### Feature And Tool Planning Tests

Extend feature registry and tool-planning tests to assert:

- `hpatch` is disabled by default and is under development;
- normal mode exposes `apply_patch` and not `hpatch`;
- hpatch mode exposes `hpatch` and keeps `apply_patch` dispatch-only;
- no-environment turns expose neither editing tool;
- multi-environment mode adds the required environment selector to hpatch;
- hpatch and `command_stack` together expose hpatch as the stack patch child;
- hosted, dynamic, MCP, and unrelated core tools remain unchanged.

Likely locations:

- `codex-rs/features/src/lib.rs` tests;
- `codex-rs/core/src/tools/spec_plan_tests.rs`;
- new `codex-rs/core/src/tools/handlers/hpatch_spec_tests.rs`.

### Grammar And Prompt Tests

Snapshot or exact-match the compact hpatch tool spec and instruction fragment.

Assert that:

- representative complete scripts satisfy the grammar;
- correction-only scripts are not accepted initially;
- JSON-wrapped and empty scripts are rejected by the constrained tool surface;
- feature-disabled base instructions remain byte-for-byte unchanged;
- feature-enabled instructions contain exactly one marked hpatch section;
- applying prompt transformation twice is idempotent;
- hpatch mode does not inject the complete upstream hpatch description;
- hpatch mode contains an explicit override for stale generic apply-patch references;
- resumed sessions and model switches do not duplicate the marked section;
- compact prompt/tool metadata remains under an explicit byte or estimated-token budget.

### Translator Process Tests

Use a deterministic fake companion executable or fake selected-environment exec backend for unit
tests. Do not depend on a developer machine's globally installed hpatch.

Cover:

- script bytes are passed through stdin without shell quoting;
- executable argv contains translation mode and no shell wrapper;
- the process cwd is the selected workspace root;
- translation runs with read-only filesystem permissions and no network permission;
- stdout and stderr are captured separately;
- nonzero exit returns bounded stderr diagnostics;
- successful stderr warnings do not fail the call;
- missing sibling executable returns the intended broken-distribution error;
- a globally installed PATH hpatch is ignored when the sibling is missing;
- sibling resolution uses the stable Codex self-executable path rather than the process cwd;
- cancellation terminates the process and does not start native patch execution;
- oversized stdout/stderr is rejected or truncated according to the selected policy;
- invalid UTF-8 and malformed apply-patch stdout fail closed;
- ambiguous or missing workspace roots fail before process launch.

Add a hermetic integration test using the same pinned hpatch companion built for the test artifact.
The test must not download hpatch or resolve it from the developer's PATH.

### Packaging And Layout Tests

Assert that every supported distribution layout contains the companion beside Codex:

- Windows release staging contains `codex.exe` and `hpatch.exe` in the same directory;
- Unix release staging contains executable `codex` and `hpatch` siblings;
- package manifests and installer required-file lists include hpatch;
- managed install validation rejects a release missing hpatch;
- the local sync/recompile scripts copy both artifacts to the D-drive `output` directory;
- integration-test setup places the pinned hpatch test binary beside the tested Codex binary;
- sibling resolution succeeds even when PATH contains a different fake hpatch;
- sibling resolution fails closed when the packaged companion is absent or not executable;
- release cleanup, replacement, and uninstall logic handles both binaries;
- remote runtime-resource fixtures include the remote hpatch path when the feature is available.

### Shared Native Patch Execution Tests

Refactor existing apply-patch handler tests so the shared verified-action executor is exercised by
both frontends.

Assert that native apply_patch behavior is unchanged and hpatch-translated actions receive:

- identical affected-path calculation;
- identical approval requests;
- identical sandbox permission expansion;
- identical add, update, delete, and move behavior;
- identical `AppliedPatchDelta` tracking;
- identical partial-commit reporting after an induced filesystem failure;
- one patch begin and one patch end event using the original hpatch call id.

### History And Token-Privacy Tests

Capture conversation and rollout history after a successful hpatch call.

Assert that history contains:

- the model-authored hpatch script;
- one compact hpatch result;

Assert that history does not contain:

- the translated `*** Begin Patch` envelope;
- a synthetic nested apply-patch tool call;
- translator stdout;
- duplicate patch results or hook items.

Also verify that prompt/token estimation counts the compact hpatch definition and result but does
not count the host-private translated envelope.

### Desktop Bridge And App-Server Tests

Add v2 app-server tests based on the existing apply-patch file-change tests.

For a translated hpatch update, assert:

- one FileChange item starts with the correct path and diff;
- an approval request uses the existing file-change approval protocol;
- accepting completes the same item successfully;
- declining completes it as declined and leaves files unchanged;
- `TurnDiffUpdated` contains the expected unified diff;
- thread resume reconstructs the completed FileChange item;
- no hpatch-specific public protocol item is required;
- no duplicate legacy patch events are emitted.

Add failure cases for translation rejection, verification rejection, cancellation, and a native
partial-application failure.

### Hook Tests

Assert that:

- hpatch PreToolUse receives the original script;
- a permitted hook rewrite is translated instead of the original script;
- hpatch PostToolUse receives the compact success or failure response;
- internal translated execution does not fire user-configured apply-patch hooks;
- translator-owned hooks and metrics are disabled when Codex invokes the executable;
- cancellation and rejected hooks do not begin a native file-change lifecycle.

### command_stack Tests

Extend command-stack schema and routing tests to cover:

- hpatch is the patch command type only when its feature is enabled;
- a step containing hpatch executes serially;
- `continue_on_failure` is rejected for hpatch;
- hpatch failure stops the current step and blocks later steps;
- prior independent reads may run in an earlier step;
- child routing preserves the parent source and cancellation token;
- the model-visible stack schema does not also advertise apply_patch in hpatch mode.

### Remote And Cross-Platform Tests

Use the exec-server test infrastructure to assert that:

- local Windows launches the configured `.exe` directly and does not require cmd or PowerShell;
- local Unix launches hpatch directly;
- remote turns start hpatch through the remote `ExecBackend`;
- scripts and translated paths use the selected environment's path semantics;
- a remote environment missing hpatch fails remotely rather than falling back to the host;
- CRLF input and output are normalized only where the hpatch/apply-patch contracts require it.

### Regression Test Commands

During implementation, prefer focused test invocations before broader suites. Expected commands
will include the relevant package tests, for example:

```text
cargo test -p codex-features
cargo test -p codex-core hpatch
cargo test -p codex-core apply_patch
cargo test -p codex-core command_stack
cargo test -p codex-app-server hpatch
cargo test -p codex-app-server thread_resume
```

Adjust filters to actual test names and package capabilities as implementation files are added.
Run formatting and `cargo test -p codex-core` after focused tests pass. Broader workspace testing is
required only if shared protocol, exec-server, or model metadata contracts change.

## Verification Checklist

- Feature disabled: tool list, prompt, behavior, and snapshots remain unchanged.
- Feature enabled: model sees hpatch and does not see apply_patch.
- The full upstream hpatch description is absent from request instructions.
- A compact hpatch update translates and modifies the expected file.
- The generated apply-patch envelope is absent from recorded conversation and rollout history.
- Desktop v2 receives one FileChange item with correct changes and status.
- TurnDiffUpdated contains the expected unified diff.
- Approval-required edits use the existing file-change approval request.
- Declined approval leaves files unchanged and completes the same file-change item as declined.
- Translation failure leaves files unchanged and creates no patch lifecycle.
- Verification failure leaves files unchanged and creates no patch lifecycle.
- Native application failure reports any known committed prefix exactly as apply_patch does today.
- Cancellation terminates the translator process and leaves files unchanged before application.
- Missing bundled hpatch companion returns a concise broken-distribution error.
- Multi-environment calls select the requested environment.
- Remote calls execute hpatch remotely rather than reading through the host filesystem.
- `command_stack` serializes hpatch and stops later work after patch failure.
- `command_stack` hpatch workdirs resolve relative to the selected environment cwd while translated
  patches apply relative to the translator root.
- Hpatch PreToolUse/PostToolUse hooks fire once; internal apply-patch hooks do not fire.
- Resumed threads and spawned subagents preserve the selected patch-tool mode.
- Prompt/tool token estimates are covered by deterministic tests without runtime benchmarks.

## Implementation Order

1. Add patch-tool mode and feature plumbing.
2. Add compact hpatch grammar, tool spec, and prompt composition.
3. Refactor verified apply-patch execution into a reusable internal path.
4. Add companion build, packaging, stable sibling resolution, and selected-environment translation
   process execution.
5. Connect translated actions to native patch events and runtime.
6. Add compact model output and Codex hook behavior.
7. Integrate hpatch with `command_stack`.
8. Add local, remote, app-server, resume, and prompt-size tests.
9. Enable the feature in local user configuration with a temporary explanatory comment only after
   the implementation passes focused tests.

## Open Decisions

- Exact feature key and public configuration name.
- Remote repository location for the local `codex-integration` hpatch fork.
- Whether developer-only tests need an explicit companion path override in addition to stable
  sibling layout construction.
- How the companion is cross-compiled and cached in each release workflow without adding avoidable
  release latency.
- Exact compact grammar subset and whether a future native correction protocol is worthwhile.
- Machine-readable handling of selector-repair notes and warnings.
- Whether successful output should include changed paths or only a file count.
- How to represent the patch-tool mode in persisted session metadata independently of the rendered
  base instruction text.
- Whether startup capability probing is reliable enough to omit the tool when hpatch is unavailable.
