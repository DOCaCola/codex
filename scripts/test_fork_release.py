import hashlib
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import build_fork_release as release


class ForkReleaseTest(unittest.TestCase):
    def test_native_builds_package_matching_companions_and_checksums(self):
        for platform, machine, target, suffix, archive_suffix in [
            ("win32", "AMD64", "x86_64-pc-windows-msvc", ".exe", ".zip"),
            ("darwin", "arm64", "aarch64-apple-darwin", "", ".tar.gz"),
        ]:
            with self.subTest(platform=platform), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                migration = root / "codex-rs/state/migrations/1.sql"
                migration.parent.mkdir(parents=True)
                migration.write_bytes(b"SELECT 1;\r\n")
                output = root / "dist"
                cache = root / "cache"

                def package(command, **kwargs):
                    self.assertEqual(command[command.index("--target") + 1], target)
                    companion = Path(command[command.index("--hpatch-bin") + 1])
                    self.assertEqual(companion.name, "hpatch" + suffix)
                    self.assertEqual(
                        kwargs["env"]["CARGO_TARGET_DIR"], str(cache / "cargo")
                    )
                    archive = Path(command[command.index("--archive-output") + 1])
                    self.assertEqual(
                        archive.name, f"codex-doca-{target}{archive_suffix}"
                    )
                    archive.write_bytes(b"complete package")

                with (
                    patch.object(release, "REPO", root),
                    patch.object(release.sys, "platform", platform),
                    patch.object(release.platform, "machine", return_value=machine),
                    patch.object(
                        release.sys,
                        "argv",
                        [
                            "build",
                            "--output-dir",
                            str(output),
                            "--cache-dir",
                            str(cache),
                        ],
                    ),
                    patch.object(release, "prepare"),
                    patch.object(release, "build") as build,
                    patch.object(release.subprocess, "run", side_effect=package),
                ):
                    release.main()
                self.assertEqual(build.call_args.args[1], target)
                self.assertEqual(
                    migration.read_bytes(),
                    b"SELECT 1;\n" if platform == "darwin" else b"SELECT 1;\r\n",
                )
                expected = hashlib.sha256(b"complete package").hexdigest()
                self.assertEqual(
                    (output / "SHA256SUMS").read_bytes(),
                    f"{expected}  codex-doca-{target}{archive_suffix}\n".encode(),
                )


if __name__ == "__main__":
    unittest.main()
