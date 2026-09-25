#!/usr/bin/env python3
"""Assemble the local Windows build using the upstream Codex package builder."""

import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--build-dir", type=Path, required=True)
    parser.add_argument("--hpatch-bin", type=Path, required=True)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    root = repo.parent
    output = root / "output"
    env = {**os.environ, "CODEX_REPO_ROOT": str(repo)}
    with tempfile.TemporaryDirectory(prefix=".codex-package-", dir=root) as temporary:
        stage = Path(temporary) / "package"
        command = [
            sys.executable,
            str(repo / "scripts/build_codex_package.py"),
            "--target",
            "x86_64-pc-windows-msvc",
            "--package-dir",
            str(stage),
            "--hpatch-bin",
            str(args.hpatch_bin.resolve()),
        ]
        for option, executable in (
            ("--entrypoint-bin", "codex.exe"),
            ("--code-mode-host-bin", "codex-code-mode-host.exe"),
            ("--codex-command-runner-bin", "codex-command-runner.exe"),
            ("--codex-windows-sandbox-setup-bin", "codex-windows-sandbox-setup.exe"),
        ):
            command.extend([option, str((args.build_dir / executable).resolve())])
        subprocess.run(command, env=env, check=True)
        # Replace output only after the upstream builder validates the complete package.
        backup = None
        if output.exists():
            backup = Path(tempfile.mkdtemp(prefix="output.previous-", dir=root))
            backup.rmdir()
            output.rename(backup)
        try:
            stage.rename(output)
        except OSError:
            if backup is not None:
                backup.rename(output)
            raise
    print(f"Packaged CLI: {output / 'bin/codex.exe'}")
    print(f"PATH directory: {output / 'bin'}")
    if backup is not None:
        print(f"Previous output preserved: {backup}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
