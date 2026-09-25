#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_DIR="$(cd -- "$SCRIPT_DIR/../.." && pwd)"
WORKSPACE_DIR="$(dirname -- "$REPO_DIR")"
CODEX_RS_DIR="$REPO_DIR/codex-rs"
COPY_SCRIPT="$SCRIPT_DIR/copy_codex_release_to_output.sh"
CODEX_CARGO_TARGET_DIR="${CODEX_CARGO_TARGET_DIR:-/c/temp/codex-target}"
# Use CODEX_BUILD_PROFILE=release for the full production ThinLTO build.
CODEX_BUILD_PROFILE="${CODEX_BUILD_PROFILE:-fast-release}"

CODEX_BUILD_TEMP_DIR="${CODEX_BUILD_TEMP_DIR:-/c/temp/codex-build-tmp}"
CARGO_BIN="${CARGO_BIN:-$HOME/.cargo/bin/cargo.exe}"
HPATCH_SOURCE_DIR="${HPATCH_SOURCE_DIR:-$WORKSPACE_DIR/hpatch}"
HPATCH_BUILD_OUTPUT="${HPATCH_BUILD_OUTPUT:-/c/temp/codex-hpatch-bin/hpatch.exe}"

if [[ ! -d "$CODEX_RS_DIR" ]]; then
  echo "Could not find repo at: $CODEX_RS_DIR" >&2
  exit 1
fi

if [[ ! -x "$COPY_SCRIPT" ]]; then
  echo "Copy script missing or not executable: $COPY_SCRIPT" >&2
  exit 1
fi

if [[ ! -x "$CARGO_BIN" ]]; then
  echo "Cargo binary not found or not executable: $CARGO_BIN" >&2
  exit 1
fi

if [[ ! -f "$HPATCH_SOURCE_DIR/go.mod" ]]; then
  echo "hpatch fork not found at: $HPATCH_SOURCE_DIR" >&2
  exit 1
fi

mkdir -p "$CODEX_CARGO_TARGET_DIR" "$CODEX_BUILD_TEMP_DIR"
CODEX_BUILD_TEMP_DIR_WINDOWS="$(cygpath -w "$CODEX_BUILD_TEMP_DIR")"

"$REPO_DIR/scripts/build-hpatch-companion.sh" \
  --source "$HPATCH_SOURCE_DIR" \
  --target x86_64-pc-windows-msvc \
  --output "$HPATCH_BUILD_OUTPUT"

echo "Building Codex and package helpers ($CODEX_BUILD_PROFILE profile)..."
(
  cd "$CODEX_RS_DIR"
  CARGO_TARGET_DIR="$CODEX_CARGO_TARGET_DIR" \
    TMP="$CODEX_BUILD_TEMP_DIR_WINDOWS" \
    TEMP="$CODEX_BUILD_TEMP_DIR_WINDOWS" \
    python "$SCRIPT_DIR/build_codex_release.py" --cargo "$CARGO_BIN" --profile "$CODEX_BUILD_PROFILE"
)

CODEX_CARGO_TARGET_DIR="$CODEX_CARGO_TARGET_DIR" \
  HPATCH_BUILD_OUTPUT="$HPATCH_BUILD_OUTPUT" \
  CODEX_BUILD_PROFILE="$CODEX_BUILD_PROFILE" \
  "$COPY_SCRIPT"
