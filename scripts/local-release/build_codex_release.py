#!/usr/bin/env python3
"""Build the local Codex executables with Codex's matching V8 artifacts."""

import argparse
import os
from pathlib import Path
import subprocess
import sys


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cargo", required=True)
    parser.add_argument("--profile", required=True)
    args = parser.parse_args()

    repo_root = Path(__file__).resolve().parents[2]
    os.environ["CODEX_REPO_ROOT"] = str(repo_root)
    sys.path.insert(0, str(repo_root / "scripts"))
    from codex_package.targets import TARGET_SPECS
    from codex_package.v8 import resolve_codex_v8_cargo_env

    cargo_version = subprocess.check_output(
        [args.cargo, "-vV"], text=True, cwd=repo_root / "codex-rs"
    )
    host = next(
        line.removeprefix("host: ")
        for line in cargo_version.splitlines()
        if line.startswith("host: ")
    )
    print(f"Preparing Codex V8 artifacts for {host}...", flush=True)
    build_env = {**os.environ, **resolve_codex_v8_cargo_env(TARGET_SPECS[host])}
    return subprocess.run(
        [
            args.cargo,
            "build",
            "--bin",
            "codex",
            "--bin",
            "codex-code-mode-host",
            "--bin",
            "codex-command-runner",
            "--bin",
            "codex-windows-sandbox-setup",
            "--profile",
            args.profile,
        ],
        cwd=repo_root / "codex-rs",
        env=build_env,
    ).returncode


if __name__ == "__main__":
    sys.exit(main())
