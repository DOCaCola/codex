import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import smoke_fork_release as smoke


class SmokeForkReleaseTest(unittest.TestCase):
    def test_handshake_keeps_stdin_open_until_reply_and_checks_exit(self):
        for exit_code, reply, expected_error in [
            (0, {"id": 1, "result": {"userAgent": "test"}}, None),
            (
                7,
                {"id": 1, "result": {"userAgent": "test"}},
                subprocess.CalledProcessError,
            ),
            (0, {"id": 1, "error": {"message": "failed"}}, AssertionError),
            (0, None, AssertionError),
        ]:
            with self.subTest(exit_code=exit_code, reply=reply):
                with tempfile.TemporaryDirectory() as directory:
                    fake = Path(directory) / "cli.py"
                    fake.write_text(
                        "import json, sys\n"
                        "if sys.argv[1:] == ['features', 'list']:\n"
                        "    print('daemon_auto_start beta true')\n"
                        "elif sys.argv[1:] == ['app-server']:\n"
                        "    request = json.loads(sys.stdin.readline())\n"
                        "    assert request['method'] == 'initialize'\n"
                        "    print(json.dumps({'method': 'notification'}), flush=True)\n"
                        + (
                            f"    print({json.dumps(reply)!r}, flush=True)\n"
                            "    assert sys.stdin.read() == ''\n"
                            if reply is not None
                            else ""
                        )
                        + f"    sys.exit({exit_code})\n"
                    )
                    popen = subprocess.Popen

                    def popen_fake(command, **kwargs):
                        return popen(
                            [sys.executable, str(fake), *command[1:]], **kwargs
                        )

                    with (
                        patch.object(smoke.subprocess, "Popen", side_effect=popen_fake),
                    ):
                        if expected_error:
                            with self.assertRaises(expected_error):
                                smoke.check_app_server(Path("codex"), os.environ.copy())
                        else:
                            smoke.check_app_server(Path("codex"), os.environ.copy())


if __name__ == "__main__":
    unittest.main()
