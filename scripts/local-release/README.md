# Local Windows release

Run `recompile_codex_release.sh` from MSYS2 Bash or
`recompile_codex_release.bat` from Windows to build the CLI, code-mode host,
Windows sandbox helpers, and fork hpatch companion.

The hpatch checkout defaults to `../hpatch` relative to the repository.
`HPATCH_SOURCE_DIR` overrides it. Cargo output defaults to
`C:/temp/codex-target`; use `CODEX_CARGO_TARGET_DIR` to override it.
`CODEX_BUILD_PROFILE` defaults to `fast-release`.

`copy_codex_release_to_output.sh` packages an existing build using the upstream
package builder. It stages and validates the complete package before replacing
`../output`. The previous output is kept temporarily for rollback if replacement
fails. After a successful replacement, all `output.previous-*` backups are removed,
so only the latest package is retained.

Add `../output/bin` to PATH, not `../output`. Daemon startup requires the complete
package, including `codex-package.json`, `codex-resources`, and `codex-path`.
Copying only `codex.exe` is insufficient.

After successful packaging, both recompile scripts deploy the new package with
`output/bin/codex.exe app-server daemon update --from-cli --yes`. This selects
and pins the fork package in the daemon's separate installation. A running daemon
restarts automatically, which can interrupt active or queued work. A stopped daemon
stays stopped and uses the new package on its next start. The scripts verify the
selected and running versions and fail if deployment or verification fails.

The daemon must already have an installation selected. For a first installation,
run `output/bin/codex.exe app-server daemon start` before using the recompile workflow.
Packaging alone with `copy_codex_release_to_output.sh` does not deploy the daemon.
