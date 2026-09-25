#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd -- "$SCRIPT_DIR/../.." && pwd)"
WORKSPACE_DIR="$(dirname -- "$REPO_DIR")"
CODEX_CARGO_TARGET_DIR="${CODEX_CARGO_TARGET_DIR:-/c/temp/codex-target}"
CODEX_BUILD_PROFILE="${CODEX_BUILD_PROFILE:-fast-release}"
HPATCH_BUILD_OUTPUT="${HPATCH_BUILD_OUTPUT:-/c/temp/codex-hpatch-bin/hpatch.exe}"

python "$SCRIPT_DIR/package_codex_release.py"   --build-dir "$CODEX_CARGO_TARGET_DIR/$CODEX_BUILD_PROFILE"   --hpatch-bin "$HPATCH_BUILD_OUTPUT"
