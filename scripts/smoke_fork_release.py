#!/usr/bin/env python3
"""Check a complete native release package before uploading it."""

import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading


def main() -> None:
    package = Path(sys.argv[1]).resolve()
    suffix = ".exe" if sys.platform == "win32" else ""
    cli = package / "bin" / f"codex{suffix}"
    with tempfile.TemporaryDirectory(prefix="codex-release-smoke-") as home:
        env = {**os.environ, "CODEX_HOME": home}
        for executable, arguments in [
            (cli, ["--version"]),
            (package / "bin" / f"hpatch{suffix}", ["--help"]),
            (package / "codex-path" / f"rg{suffix}", ["--version"]),
        ]:
            subprocess.run(
                [str(executable), *arguments], env=env, check=True, timeout=20
            )
        features = subprocess.check_output(
            [str(cli), "features", "list"], env=env, text=True, timeout=20
        )
        expected = "false" if sys.platform == "win32" else "true"
        daemon = next(
            line
            for line in features.splitlines()
            if line.startswith("daemon_auto_start")
        )
        assert daemon.split()[-1] == expected, daemon
        with (Path(home) / "stderr.log").open("w") as stderr:
            child = subprocess.Popen(
                [str(cli), "app-server"],
                env=env,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=stderr,
                text=True,
            )
            replies = queue.Queue()

            def read() -> None:
                for line in child.stdout:
                    replies.put(json.loads(line))
                replies.put(None)

            threading.Thread(target=read, daemon=True).start()
            try:
                request = {
                    "id": 1,
                    "method": "initialize",
                    "params": {
                        "clientInfo": {"name": "codexdc-release-check", "version": "1"},
                        "capabilities": {"experimentalApi": True},
                    },
                }
                child.stdin.write(json.dumps(request) + "\n")
                child.stdin.flush()
                reply = replies.get(timeout=20)
                assert (
                    reply
                    and reply.get("id") == 1
                    and reply.get("result", {}).get("userAgent")
                ), reply
            finally:
                child.stdin.close()
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=10)
        print("Native package and CodexDC app-server handshake passed.")


if __name__ == "__main__":
    main()
