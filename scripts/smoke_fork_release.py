#!/usr/bin/env python3
"""Check a complete native release package before uploading it."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main() -> None:
    package = Path(sys.argv[1]).resolve()
    suffix = ".exe" if sys.platform == "win32" else ""
    cli = package / "bin" / f"codex{suffix}"
    with tempfile.TemporaryDirectory(prefix="codex-release-smoke-") as home:
        env = {**os.environ, "CODEX_HOME": home}
        for executable, arguments in [
            (cli, ["--version"]),
            (package / "bin" / f"hpatch{suffix}", ["--help"]),
            (package / "bin" / f"codex-exec-server{suffix}", ["--help"]),
            (package / "codex-path" / f"rg{suffix}", ["--version"]),
        ]:
            subprocess.run(
                [str(executable), *arguments], env=env, check=True, timeout=20
            )
        features = subprocess.check_output(
            [str(cli), "features", "list"], env=env, text=True, timeout=20
        )
        daemon = next(
            line
            for line in features.splitlines()
            if line.startswith("daemon_auto_start")
        )
        assert daemon.split()[-1] == "true", daemon
        request = {
            "id": 1,
            "method": "initialize",
            "params": {
                "clientInfo": {"name": "codexdc-release-check", "version": "1"},
                "capabilities": {"experimentalApi": True},
            },
        }
        # Send the request and close stdin, then drain stdout through process exit.
        # Keep stderr in the CI log instead of a temporary file whose inherited
        # Windows handle can outlive the app-server process.
        result = subprocess.run(
            [str(cli), "app-server"],
            env=env,
            input=json.dumps(request) + "\n",
            stdout=subprocess.PIPE,
            text=True,
            check=True,
            timeout=20,
        )
        replies = [json.loads(line) for line in result.stdout.splitlines()]
        reply = next((reply for reply in replies if reply.get("id") == 1), None)
        assert reply and reply.get("result", {}).get("userAgent"), replies
        print("Native package and CodexDC app-server handshake passed.")


if __name__ == "__main__":
    main()
