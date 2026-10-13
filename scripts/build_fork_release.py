#!/usr/bin/env python3
"""Build a complete fork release for Windows x64 or macOS Apple Silicon."""

import argparse
import hashlib
import os
import platform
from pathlib import Path
import subprocess
import sys

REPO = Path(__file__).resolve().parents[1]
os.environ["CODEX_REPO_ROOT"] = str(REPO)
sys.path.insert(0, str(REPO / "scripts"))

from hpatch_companion import build, prepare


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--cache-dir", type=Path, required=True)
    parser.add_argument(
        "--profile", choices=("fast-release", "release"), default="fast-release"
    )
    parser.add_argument("--cargo", default="cargo")
    parser.add_argument("--go", default=os.environ.get("GO_BIN", "go"))
    args = parser.parse_args()
    if sys.platform == "win32" and platform.machine().lower() in ("amd64", "x86_64"):
        target, suffix, archive_suffix = "x86_64-pc-windows-msvc", ".exe", ".zip"
    elif sys.platform == "darwin" and platform.machine().lower() in (
        "arm64",
        "aarch64",
    ):
        target, suffix, archive_suffix = "aarch64-apple-darwin", "", ".tar.gz"
    else:
        parser.error("Build on Windows x64 or macOS Apple Silicon")
    if sys.platform == "darwin":
        # The fork pins CRLF for compatibility with official Windows databases.
        # Native macOS releases hash LF migration sources; use those bytes here.
        for migration in (REPO / "codex-rs/state").rglob("*.sql"):
            migration.write_bytes(migration.read_bytes().replace(b"\r\n", b"\n"))
    cache = args.cache_dir.resolve()
    output = args.output_dir.resolve()
    build_temp = cache / "tmp"
    build_temp.mkdir(parents=True, exist_ok=True)
    output.mkdir(parents=True, exist_ok=True)
    source = cache / "hpatch-source"
    prepare(source)
    companion = cache / f"bin/hpatch{suffix}"
    build(source, target, companion, args.go, cache / "go")
    archive = output / f"codex-doca-{target}{archive_suffix}"
    # The canonical builder resolves matching V8 artifacts and builds all missing
    # Rust companions (including both Windows sandbox helpers).
    subprocess.run(
        [
            sys.executable,
            str(REPO / "scripts/build_codex_package.py"),
            "--target",
            target,
            "--cargo",
            args.cargo,
            "--cargo-profile",
            args.profile,
            "--hpatch-bin",
            str(companion),
            "--package-dir",
            str(output / "package"),
            "--archive-output",
            str(archive),
            "--force",
        ],
        cwd=REPO,
        check=True,
        env={
            **os.environ,
            "CARGO_TARGET_DIR": str(cache / "cargo"),
            "TMP": str(build_temp),
            "TEMP": str(build_temp),
        },
    )
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    (output / "SHA256SUMS").write_text(
        f"{digest}  {archive.name}\n", encoding="utf-8", newline="\n"
    )


if __name__ == "__main__":
    main()
