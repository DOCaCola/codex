#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"
workspace_root="${repo_root}/codex-rs"
output_dir="$(cd -- "${repo_root}/.." && pwd)/output"

profile="debug"
cargo_args=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release)
            profile="release"
            cargo_args+=("--release")
            shift
            ;;
        *)
            echo "unknown argument: $1" >&2
            exit 2
            ;;
    esac
done

target_base="${CODEX_OUTPUT_TARGET_DIR:-/c/temp}"
if [[ "${target_base}" =~ ^[A-Za-z]:\\ ]]; then
    target_base="$(cygpath -u "${target_base}")"
fi

target_dir="${target_base%/}/codex-output-target"
target_dir_win="$(cygpath -w "${target_dir}")"
binary_path="${target_dir}/${profile}/codex.exe"
hpatch_path="${target_dir}/${profile}/hpatch.exe"
hpatch_source_dir="${HPATCH_SOURCE_DIR:-${repo_root}/../hpatch}"
output_path="${output_dir}/codex.exe"
staged_output_path="${output_dir}/codex.exe.tmp"

mkdir -p "${target_dir}" "${output_dir}"

"${repo_root}/scripts/build-hpatch-companion.sh" \
    --source "${hpatch_source_dir}" \
    --target x86_64-pc-windows-msvc \
    --output "${hpatch_path}"

(
    cd "${workspace_root}"
    CARGO_TARGET_DIR="${target_dir_win}" '/c/Users/DOCa Cola/.cargo/bin/cargo.exe' \
        build -p codex-cli --bin codex "${cargo_args[@]}"
)

rm -f "${staged_output_path}"
cp "${binary_path}" "${staged_output_path}"

if ! mv -f "${staged_output_path}" "${output_path}"; then
    rm -f "${staged_output_path}"
    echo "failed to replace ${output_path}; it is likely in use by a running process" >&2
    exit 1
fi

cp -f "${hpatch_path}" "${output_dir}/hpatch.exe"

echo "synced ${binary_path} -> ${output_path}"
echo "synced ${hpatch_path} -> ${output_dir}/hpatch.exe"
