#!/usr/bin/env bash
set -euo pipefail

target_base="${CODEX_OUTPUT_TARGET_DIR:-${TEMP:-${TMP:-/tmp}}}"
if [[ "${target_base}" =~ ^[A-Za-z]:\\ ]]; then
    target_base="$(cygpath -u "${target_base}")"
fi

rm -rf "${target_base%/}/codex-output-target"
