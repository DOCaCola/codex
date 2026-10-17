#!/usr/bin/env python3
"""Deploy the packaged fork to the managed daemon and verify its versions."""

import json
from pathlib import Path
import subprocess
import sys


def deploy(package: Path) -> None:
    version = json.loads((package / "codex-package.json").read_text())["version"]
    executable = str(package / "bin" / "codex.exe")
    print(
        f"Deploying {version} to the managed daemon. "
        "A running daemon will restart; active work may be interrupted.",
        flush=True,
    )
    update = subprocess.run(
        [executable, "app-server", "daemon", "update", "--from-cli", "--yes"],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    installed = json.loads(update.stdout)
    if installed["status"] != "updated" or installed["installedVersion"] != version:
        raise RuntimeError(
            f"Daemon deployment version mismatch: {update.stdout.strip()}"
        )
    if installed["runningVersion"] is None:
        print(f"Daemon deployed for its next start: {version}", flush=True)
        return
    result = subprocess.run(
        [executable, "app-server", "daemon", "version"],
        check=True,
        capture_output=True,
        text=True,
    )
    state = json.loads(result.stdout)
    if state["cliVersion"] != version or state["managedCodexVersion"] != version:
        raise RuntimeError(
            f"Daemon deployment version mismatch: {result.stdout.strip()}"
        )
    # The fork strips build suffixes from the app-server user agent.
    if (
        state["status"] != "running"
        or state["appServerVersion"] != version.split("-", 1)[0]
    ):
        raise RuntimeError(f"Running daemon version mismatch: {result.stdout.strip()}")
    print(f"Daemon deployed and running: {version}", flush=True)


def main() -> int:
    package = Path(__file__).resolve().parents[3] / "output"
    try:
        deploy(package)
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Daemon deployment failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
