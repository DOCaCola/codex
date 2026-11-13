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
import time


def check_app_server(cli: Path, env: dict[str, str]) -> None:
    request = {
        "id": 1,
        "method": "initialize",
        "params": {
            "clientInfo": {"name": "codexdc-release-check", "version": "1"},
            "capabilities": {"experimentalApi": True},
        },
    }
    command = [str(cli), "app-server"]
    with subprocess.Popen(
        command, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True
    ) as process:
        lines = queue.Queue()

        def drain_stdout():
            try:
                for line in process.stdout:
                    lines.put(line)
            finally:
                lines.put(None)

        reader = threading.Thread(target=drain_stdout, daemon=True)
        reader.start()
        replies = []
        reply = None
        deadline = time.monotonic() + 20
        try:
            process.stdin.write(json.dumps(request) + "\n")
            process.stdin.flush()
            # EOF disconnects the transport and may discard a pending request.
            # Keep the client connected until initialize has actually completed.
            while reply is None:
                try:
                    line = lines.get(timeout=max(0, deadline - time.monotonic()))
                except queue.Empty:
                    raise subprocess.TimeoutExpired(command, 20) from None
                if line is None:
                    break
                message = json.loads(line)
                replies.append(message)
                if message.get("id") == 1:
                    reply = message
            process.stdin.close()
            process.wait(timeout=20)
            if process.returncode:
                raise subprocess.CalledProcessError(process.returncode, command)
            assert reply and reply.get("result", {}).get("userAgent"), replies
            reader.join(timeout=20)
            if reader.is_alive():
                raise subprocess.TimeoutExpired(command, 20)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()


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
        check_app_server(cli, env)
        print("Native package and CodexDC app-server handshake passed.")


if __name__ == "__main__":
    main()
