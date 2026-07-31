#!/usr/bin/env bash
set -euo pipefail

GO_VERSION="1.26.5"

usage() {
  echo "Usage: build-hpatch-companion.sh --source <hpatch-checkout> --target <rust-target> --output <path>" >&2
}

source_dir=""
target=""
output=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --source) source_dir="${2:?--source requires a value}"; shift 2 ;;
    --target) target="${2:?--target requires a value}"; shift 2 ;;
    --output) output="${2:?--output requires a value}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unexpected argument: $1" >&2; usage; exit 2 ;;
  esac
done
[[ -n "$source_dir" && -n "$target" && -n "$output" ]] || { usage; exit 2; }

if [[ "$source_dir" =~ ^[A-Za-z]:[\\/] ]]; then
  source_dir="$(cygpath -u "$source_dir")"
fi
source_dir="$(cd "$source_dir" && pwd)"
if [[ ! -f "$source_dir/go.mod" || ! -d "$source_dir/cmd/hpatch" ]]; then
  echo "Not an hpatch source checkout: $source_dir" >&2
  exit 1
fi
if [[ "$(sed -n 's/^module //p' "$source_dir/go.mod" | head -n 1)" != "github.com/yusing/hpatch" ]]; then
  echo "Unexpected Go module in hpatch source checkout: $source_dir" >&2
  exit 1
fi

case "$target" in
  x86_64-*-linux-*) goos=linux; goarch=amd64 ;;
  aarch64-*-linux-*) goos=linux; goarch=arm64 ;;
  x86_64-apple-darwin) goos=darwin; goarch=amd64 ;;
  aarch64-apple-darwin) goos=darwin; goarch=arm64 ;;
  x86_64-pc-windows-*) goos=windows; goarch=amd64 ;;
  aarch64-pc-windows-*) goos=windows; goarch=arm64 ;;
  *) echo "Unsupported hpatch target: $target" >&2; exit 2 ;;
esac

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) default_cache=/c/temp/codex-hpatch-build ;;
  *) default_cache="${RUNNER_TEMP:-/tmp}/codex-hpatch-build" ;;
esac
cache_dir="${HPATCH_CACHE_DIR:-$default_cache}"
mkdir -p "$cache_dir" "$(dirname "$output")"

lock_dir="$output.lock"
lock_acquired=false
for _ in {1..600}; do
  if mkdir "$lock_dir" 2>/dev/null; then
    lock_acquired=true
    break
  fi
  sleep 0.1
done
if [[ "$lock_acquired" != true ]]; then
  echo "Timed out waiting for hpatch build lock: $lock_dir" >&2
  exit 1
fi
tmp_output="$output.tmp.$$"
cleanup() {
  rm -f "$tmp_output"
  rmdir "$lock_dir" 2>/dev/null || true
}
trap cleanup EXIT

go_bin="${GO_BIN:-}"
if [[ -z "$go_bin" ]] && command -v go >/dev/null 2>&1; then
  go_bin="$(command -v go)"
fi
if [[ -z "$go_bin" ]]; then
  case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) host_os=windows; archive_ext=zip; go_exe=go.exe ;;
    Linux*) host_os=linux; archive_ext=tar.gz; go_exe=go ;;
    Darwin*) host_os=darwin; archive_ext=tar.gz; go_exe=go ;;
    *) echo "Unsupported Go bootstrap host: $(uname -s)" >&2; exit 2 ;;
  esac
  case "$(uname -m)" in
    x86_64|amd64) host_arch=amd64 ;;
    arm64|aarch64) host_arch=arm64 ;;
    *) echo "Unsupported Go bootstrap architecture: $(uname -m)" >&2; exit 2 ;;
  esac
  toolchain_dir="$cache_dir/go-$GO_VERSION-$host_os-$host_arch"
  if [[ ! -x "$toolchain_dir/go/bin/$go_exe" ]]; then
    toolchain_archive="$cache_dir/go$GO_VERSION.$host_os-$host_arch.$archive_ext"
    curl -L --fail --retry 3 "https://go.dev/dl/go$GO_VERSION.$host_os-$host_arch.$archive_ext" -o "$toolchain_archive"
    rm -rf "$toolchain_dir.tmp"
    mkdir -p "$toolchain_dir.tmp"
    if [[ "$archive_ext" == zip ]]; then
      unzip -q "$toolchain_archive" -d "$toolchain_dir.tmp"
    else
      tar -xzf "$toolchain_archive" -C "$toolchain_dir.tmp"
    fi
    mv "$toolchain_dir.tmp" "$toolchain_dir"
  fi
  go_bin="$toolchain_dir/go/bin/$go_exe"
fi

(
  cd "$source_dir"
  CGO_ENABLED=0 GOOS="$goos" GOARCH="$goarch" GOWORK=off \
    GOCACHE="$cache_dir/go-build-cache" GOMODCACHE="$cache_dir/go-mod-cache" \
    "$go_bin" build -trimpath -buildvcs=false -ldflags="-s -w -buildid=" \
      -o "$tmp_output" ./cmd/hpatch
)
chmod +x "$tmp_output" 2>/dev/null || true
mv -f "$tmp_output" "$output"
echo "Built hpatch companion from $source_dir: $output"
