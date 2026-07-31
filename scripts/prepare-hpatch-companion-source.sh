#!/usr/bin/env bash
set -euo pipefail

HPATCH_REPOSITORY="https://github.com/yusing/hpatch.git"
HPATCH_COMMIT="ebf7d6fb57e091b0d55416305897884509bd18ca"

usage() {
  echo "Usage: prepare-hpatch-companion-source.sh --output <checkout-path>" >&2
}

output=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output) output="${2:?--output requires a value}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unexpected argument: $1" >&2; usage; exit 2 ;;
  esac
done
[[ -n "$output" ]] || { usage; exit 2; }

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
patch_file="$repo_root/third_party/hpatch/codex-disable-user-data.patch"
tmp_output="$output.tmp.$$"
cleanup() {
  rm -rf "$tmp_output"
}
trap cleanup EXIT

mkdir -p "$(dirname "$output")"
rm -rf "$output" "$tmp_output"
git clone --filter=blob:none --no-checkout "$HPATCH_REPOSITORY" "$tmp_output"
git -C "$tmp_output" checkout --detach "$HPATCH_COMMIT"
git -C "$tmp_output" apply "$patch_file"
mv "$tmp_output" "$output"
echo "Prepared pinned hpatch source: $output"
