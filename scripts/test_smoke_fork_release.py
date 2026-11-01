import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import smoke_fork_release as smoke


class SmokeForkReleaseTest(unittest.TestCase):
    def test_handshake_drains_reply_after_eof_and_checks_exit(self):
        for exit_code, reply, expected_error in [
            (0, {"id": 1, "result": {"userAgent": "test"}}, None),
            (
                7,
                {"id": 1, "result": {"userAgent": "test"}},
                subprocess.CalledProcessError,
            ),
            (0, {"id": 1, "error": {"message": "failed"}}, AssertionError),
        ]:
            with self.subTest(exit_code=exit_code, reply=reply):
                with tempfile.TemporaryDirectory() as directory:
                    fake = Path(directory) / "cli.py"
                    fake.write_text(
                        "import json, sys\n"
                        "if sys.argv[1:] == ['features', 'list']:\n"
                        "    print('daemon_auto_start beta true')\n"
                        "elif sys.argv[1:] == ['app-server']:\n"
                        "    request = json.loads(sys.stdin.read())\n"
                        "    assert request['method'] == 'initialize'\n"
                        "    print(json.dumps({'method': 'notification'}))\n"
                        f"    print({json.dumps(reply)!r})\n"
                        f"    sys.exit({exit_code})\n"
                    )
                    run = subprocess.run

                    def run_fake(command, **kwargs):
                        return run([sys.executable, str(fake), *command[1:]], **kwargs)

                    with (
                        patch.object(smoke.sys, "argv", ["smoke", directory]),
                        patch.object(smoke.subprocess, "run", side_effect=run_fake),
                    ):
                        if expected_error:
                            with self.assertRaises(expected_error):
                                smoke.main()
                        else:
                            smoke.main()


if __name__ == "__main__":
    unittest.main()
