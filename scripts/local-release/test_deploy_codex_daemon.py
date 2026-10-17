import contextlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import deploy_codex_daemon as deployment


class DaemonDeploymentTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="codex package ")
        self.addCleanup(temporary.cleanup)
        self.package = Path(temporary.name)
        (self.package / "codex-package.json").write_text(
            json.dumps({"version": "0.162.1-doca"})
        )
        self.update = {
            "status": "updated",
            "installedVersion": "0.162.1-doca",
            "runningVersion": "0.162.1",
        }
        self.state = {
            "status": "running",
            "cliVersion": "0.162.1-doca",
            "managedCodexVersion": "0.162.1-doca",
            "appServerVersion": "0.162.1",
        }
        self.output = io.StringIO()
        self.enterContext(contextlib.redirect_stdout(self.output))

    def test_deploys_packaged_executable_and_checks_running_daemon(self):
        with patch.object(
            deployment.subprocess,
            "run",
            side_effect=[
                subprocess.CompletedProcess([], 0, json.dumps(self.update)),
                subprocess.CompletedProcess([], 0, json.dumps(self.state)),
            ],
        ) as run:
            deployment.deploy(self.package)
        executable = str(self.package / "bin" / "codex.exe")
        self.assertEqual(
            [call.args[0] for call in run.call_args_list],
            [
                [executable, "app-server", "daemon", "update", "--from-cli", "--yes"],
                [executable, "app-server", "daemon", "version"],
            ],
        )
        self.assertIn(
            "Daemon deployed and running: 0.162.1-doca", self.output.getvalue()
        )

    def test_leaves_stopped_daemon_stopped(self):
        self.update["runningVersion"] = None
        with patch.object(
            deployment.subprocess,
            "run",
            return_value=subprocess.CompletedProcess([], 0, json.dumps(self.update)),
        ) as run:
            deployment.deploy(self.package)
        self.assertEqual(run.call_count, 1)
        self.assertIn("deployed for its next start", self.output.getvalue())

    def test_update_failure_stops_before_version_check(self):
        with patch.object(
            deployment.subprocess,
            "run",
            side_effect=subprocess.CalledProcessError(7, "codex"),
        ) as run:
            with self.assertRaises(subprocess.CalledProcessError):
                deployment.deploy(self.package)
        self.assertEqual(run.call_count, 1)
        self.assertNotIn("Daemon deployed", self.output.getvalue())

    def test_rejects_stale_selected_or_running_daemon(self):
        for field in ["cliVersion", "managedCodexVersion", "appServerVersion"]:
            with self.subTest(field=field):
                state = {**self.state, field: "0.157.0"}
                with patch.object(
                    deployment.subprocess,
                    "run",
                    side_effect=[
                        subprocess.CompletedProcess([], 0, json.dumps(self.update)),
                        subprocess.CompletedProcess([], 0, json.dumps(state)),
                    ],
                ):
                    with self.assertRaisesRegex(RuntimeError, "version mismatch"):
                        deployment.deploy(self.package)


if __name__ == "__main__":
    unittest.main()
