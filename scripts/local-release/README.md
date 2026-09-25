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
`../output`, and preserves the previous output in an `output.previous-*` directory.
These backups can be removed when no longer needed.

Add `../output/bin` to PATH, not `../output`. Daemon startup requires the complete
package, including `codex-package.json`, `codex-resources`, and `codex-path`.
Copying only `codex.exe` is insufficient.

The daemon keeps its own installed package. After rebuilding, use
`codex app-server daemon update --from-cli` to select the new local package.
This asks for confirmation and can interrupt work in a running daemon.
