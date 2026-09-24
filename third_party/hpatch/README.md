# hpatch companion

`source.json` is the authoritative source revision and Go toolchain pin.
This fork builds [DOCaCola/hpatch](https://github.com/DOCaCola/hpatch), retaining
yusing's MIT license and original Go module identity.

Codex runs the companion in translation mode with
`CODEX_HPATCH_DISABLE_USER_DATA=1`. This bypasses user configuration and metrics;
Codex applies the resulting patch through its own permission checks and sandbox.
The source fork includes the regression test for this behavior.

## Build and distribute

Install the Go version in `source.json`, Git and Python 3.12 or newer, then:

```sh
python scripts/hpatch_companion.py prepare --output build-cache/hpatch-source
python scripts/hpatch_companion.py build --source build-cache/hpatch-source \
  --target x86_64-pc-windows-msvc --output build-cache/bin/hpatch.exe
```

The second command requires a clean checkout at the pinned commit. It creates
`hpatch.exe.metadata/` alongside the executable, containing licenses for hpatch,
Go, linked Go modules, and provenance with a SHA-256 checksum and target.
The canonical package builder requires both the executable and that directory.
Each runtime, including a remote executor, needs the matching architecture's
companion beside the Codex executable.

For a complete Windows package, use `scripts/build_fork_release.py`; users need
neither Go nor a separate hpatch installation. Other architectures are supported
by the companion builder but are not yet supported fork release targets.

## Update

Review upstream changes in the separate hpatch repository, retain the integration
contract, and run `go test ./cmd/hpatch`. Commit and push the reviewed source to
DOCaCola/hpatch before changing the full commit pin here. Update the Go version
when needed. Rebuild and inspect the generated dependency notices and provenance;
re-run package tests. Do not follow upstream `main` or an unpinned Go install:
the upstream project has since been renamed to mekugi.
