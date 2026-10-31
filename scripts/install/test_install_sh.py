#!/usr/bin/env python3

from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import select
import shlex
import shutil
import subprocess
import sys
import tarfile
import tempfile
import textwrap
import unittest


INSTALL_SCRIPT = Path(__file__).with_name("install.sh")
VERSION = "0.142.5-doca"
MISMATCH_VERSION = "0.145.0-doca"


class InstallShTest(unittest.TestCase):
    def test_upgrade_and_failed_download_keep_complete_packages(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            current = root / "codex-home/packages/standalone/current"
            previous = None
            for version in (VERSION, "0.142.5-doca.1"):
                source = root / version
                source.mkdir()
                archive, checksum, metadata = create_package_release(
                    source, metadata_version=version, binary_version=version
                )
                result, requests = run_installer_in(
                    root,
                    "latest",
                    archive_path=archive,
                    checksum_path=checksum,
                    metadata_json=metadata,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                selected = current.resolve()
                self.assertEqual(selected.name, f"{version}-aarch64-apple-darwin")
                self.assertTrue((selected / "bin/hpatch").is_file())
                if previous:
                    self.assertTrue((previous / "bin/codex").is_file())
                previous = selected
                self.assertTrue(all("DOCaCola/codex" in url for url in requests))
            for failure in ("archive", "checksum", "version", "target"):
                source = root / failure
                source.mkdir()
                archive, checksum, metadata = create_package_release(
                    source,
                    metadata_version="0.142.5-doca.2",
                    binary_version=VERSION
                    if failure == "version"
                    else "0.142.5-doca.2",
                    manifest_target="x86_64-apple-darwin"
                    if failure == "target"
                    else None,
                )
                if failure == "archive":
                    archive.write_bytes(b"corrupt archive")
                elif failure == "checksum":
                    checksum.write_bytes(b"corrupt checksums")
                result, _ = run_installer_in(
                    root,
                    "latest",
                    archive_path=archive,
                    checksum_path=checksum,
                    metadata_json=metadata,
                )
                self.assertNotEqual(result.returncode, 0, failure)
                self.assertEqual(current.resolve(), previous)

    def test_metadata_fetch_failure_is_not_reported_as_missing_assets(self) -> None:
        result, requests = run_installer(VERSION, metadata_failure=True)

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(
            requests,
            [
                "https://api.github.com/repos/DOCaCola/codex/releases/tags/"
                f"doca-v{VERSION}"
            ],
        )
        self.assertIn(
            f"Could not fetch GitHub release metadata for Codex {VERSION}",
            result.stderr,
        )
        self.assertNotIn("Could not find Codex package", result.stderr)

    def test_exact_release_uses_github_metadata_once(self) -> None:
        result, requests = run_installer(VERSION)

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(
            requests,
            [
                "https://api.github.com/repos/DOCaCola/codex/releases/tags/"
                f"doca-v{VERSION}",
                "https://github.com/DOCaCola/codex/releases/download/"
                f"doca-v{VERSION}/SHA256SUMS",
            ],
        )
        self.assertIn(f"Resolved version: {VERSION}", result.stdout)

    def test_alpha_hotfix_release_is_valid(self) -> None:
        version = "0.145.0-alpha.23.1-doca"
        metadata = json.loads(release_metadata())
        metadata["tag_name"] = f"doca-v{version}"
        result, requests = run_installer(version, metadata_json=json.dumps(metadata))

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(
            requests,
            [
                "https://api.github.com/repos/DOCaCola/codex/releases/tags/"
                f"doca-v{version}",
                "https://github.com/DOCaCola/codex/releases/download/"
                f"doca-v{version}/SHA256SUMS",
            ],
        )
        self.assertIn(f"Resolved version: {version}", result.stdout)

    def test_latest_release_reuses_version_metadata(self) -> None:
        result, requests = run_installer("latest")

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(
            requests,
            [
                "https://api.github.com/repos/DOCaCola/codex/releases/latest",
                "https://github.com/DOCaCola/codex/releases/download/"
                f"doca-v{VERSION}/SHA256SUMS",
            ],
        )
        self.assertIn(f"Resolved version: {VERSION}", result.stdout)

    def test_compact_metadata_is_independent_of_field_order(self) -> None:
        result, requests = run_installer(
            "latest", metadata_json=release_metadata(compact=True, reorder=True)
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(
            requests,
            [
                "https://api.github.com/repos/DOCaCola/codex/releases/latest",
                "https://github.com/DOCaCola/codex/releases/download/"
                f"doca-v{VERSION}/SHA256SUMS",
            ],
        )
        self.assertIn(f"Resolved version: {VERSION}", result.stdout)

    def test_macos_install_exposes_code_mode_host_beside_codex(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive_path, checksum_path, metadata_json = create_package_release(root)

            result, _requests = run_installer_in(
                root,
                VERSION,
                metadata_json=metadata_json,
                archive_path=archive_path,
                checksum_path=checksum_path,
                force_macos=True,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            install_bin = root / "install-bin"
            current = root / "codex-home" / "packages" / "standalone" / "current"
            codex_path = install_bin / "codex"
            host_path = install_bin / "codex-code-mode-host"
            self.assertEqual(os.readlink(codex_path), str(current / "bin" / "codex"))
            self.assertEqual(
                os.readlink(host_path),
                str(current / "bin" / "codex-code-mode-host"),
            )
            self.assertTrue(os.access(host_path, os.X_OK))

    def test_daemon_install_preserves_visible_cli_and_profile(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir).resolve()
            archive, checksum, metadata = create_package_release(root)
            options = dict(
                metadata_json=metadata,
                archive_path=archive,
                checksum_path=checksum,
                force_macos=True,
            )
            pinned, _ = run_installer_in(root, VERSION, **options)
            self.assertEqual(pinned.returncode, 0, pinned.stderr)
            visible = root / "install-bin/codex"
            original = visible.resolve()
            pending = visible.with_name(".codex.12345")
            pending.symlink_to(original)
            profile = root / "home/.profile"
            profile.write_text("unchanged profile\n")

            installed, _ = run_installer_in(root, "latest", daemon_only=True, **options)
            self.assertEqual(installed.returncode, 0, installed.stderr)
            daemon = root / "codex-home/packages/app-server-daemon"
            release_name = f"{VERSION}-aarch64-apple-darwin"
            self.assertEqual(
                (daemon / "current").resolve(), daemon / "releases" / release_name
            )
            self.assertEqual((daemon / "auto-update-version").read_text(), release_name)
            history = daemon / "selection-history"
            original_history = history.read_bytes()
            self.assertEqual(history.read_text().splitlines()[:2], [release_name, ""])
            repeated, _ = run_installer_in(root, "latest", daemon_only=True, **options)
            self.assertEqual(repeated.returncode, 0, repeated.stderr)
            self.assertEqual(history.read_bytes(), original_history)
            options["daemon_only"] = True
            local = daemon / "releases/local-development"
            local.mkdir()
            (daemon / "current").unlink()
            (daemon / "current").symlink_to(local)
            marker = daemon / "auto-update-version"
            marker.unlink()

            scheduled, _ = run_installer_in(
                root, "latest", update_guard_from_release=local.name, **options
            )
            self.assertEqual(scheduled.returncode, 0, scheduled.stderr)
            self.assertEqual((daemon / "current").resolve(), local)
            self.assertFalse(marker.exists())

            raced, _ = run_installer_in(
                root,
                "latest",
                update_guard_from_release="a-different-release",
                manual_update=True,
                **options,
            )
            self.assertNotEqual(raced.returncode, 0, raced.stderr)
            self.assertEqual((daemon / "current").resolve(), local)
            self.assertFalse(marker.exists())

            binary = daemon / "releases" / release_name / "bin/codex"
            contents = binary.read_text()
            binary.write_text(
                '#!/bin/sh\n[ "$1" = "--version" ] || exit 2\n'
                f"echo codex-cli {VERSION}\n"
            )
            incompatible, _ = run_installer_in(
                root,
                "latest",
                update_guard_from_release=local.name,
                manual_update=True,
                **options,
            )
            self.assertNotEqual(incompatible.returncode, 0)
            self.assertIn("does not support daemon-owned packages", incompatible.stderr)
            self.assertEqual((daemon / "current").resolve(), local)
            self.assertFalse(marker.exists())
            binary.write_text(contents)

            restored, _ = run_installer_in(
                root,
                "latest",
                update_guard_from_release=local.name,
                manual_update=True,
                **options,
            )
            self.assertEqual(restored.returncode, 0, restored.stderr)
            self.assertEqual(
                (daemon / "current").resolve(), daemon / "releases" / release_name
            )
            self.assertEqual(marker.read_text(), release_name)
            self.assertEqual(visible.resolve(), original)
            self.assertTrue(pending.is_symlink())
            self.assertEqual(profile.read_text(), "unchanged profile\n")
            self.assertFalse(
                (root / "codex-home/packages/standalone/auto-update-version").exists()
            )

    @unittest.skipUnless(
        sys.platform in ("darwin", "linux"), "requires Unix process inspection"
    )
    def test_explicit_release_pins_even_the_current_latest_version(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            options = dict(
                metadata_json=metadata,
                archive_path=archive,
                checksum_path=checksum,
                force_macos=True,
            )
            marker = root / "codex-home/packages/standalone/auto-update-version"
            latest, _ = run_installer_in(root, "latest", **options)
            self.assertEqual(latest.returncode, 0, latest.stderr)
            release_name = f"{VERSION}-aarch64-apple-darwin"
            self.assertEqual(marker.read_text(), release_name)

            pinned, _ = run_installer_in(root, VERSION, **options)
            self.assertEqual(pinned.returncode, 0, pinned.stderr)
            self.assertFalse(marker.exists())

            updater_record = (
                root / "codex-home/app-server-daemon/app-server-updater.pid"
            )
            updater_record.parent.mkdir(parents=True)
            updater_record.write_text(
                json.dumps(
                    {"pid": os.getpid(), "processStartTime": process_start_time()}
                )
            )
            old_updater, _ = run_installer_in(
                root, "latest", old_updater_parent_pid=os.getpid(), **options
            )
            self.assertEqual(old_updater.returncode, 0, old_updater.stderr)
            self.assertFalse(marker.exists())

            skipped, _ = run_installer_in(
                root, "latest", update_guard_from_release=release_name, **options
            )
            self.assertEqual(skipped.returncode, 0, skipped.stderr)
            self.assertFalse(marker.exists())

            updater_record.write_text(
                json.dumps({"pid": os.getpid(), "processStartTime": "stale"})
            )
            latest_again, _ = run_installer_in(
                root, "latest", old_updater_parent_pid=os.getpid(), **options
            )
            self.assertEqual(latest_again.returncode, 0, latest_again.stderr)
            self.assertEqual(marker.read_text(), release_name)

            managed = (
                root
                / f"codex-home/packages/standalone/releases/{release_name}/bin/codex"
            )
            managed.unlink()
            guarded, _ = run_installer_in(
                root,
                "latest",
                update_guard_from_release=release_name,
                **options,
            )
            self.assertEqual(guarded.returncode, 0, guarded.stderr)
            self.assertTrue(managed.exists())

    def test_macos_release_inspection_preserves_lsof_matches(self) -> None:
        # Exercise the macOS branch on every platform, including lsof's absolute path.
        function = (
            INSTALL_SCRIPT.read_text()
            .split("record_standalone_releases_in_use() {", 1)[1]
            .split("\n}\n", 1)[0]
        )
        function = "record_standalone_releases_in_use() {" + function + "\n}\n"
        function = function.replace("/usr/sbin/lsof ", "mock_lsof ")
        matches = "p123\nn/releases/active/bin/codex\n"
        for status, output, error, expected in (
            (0, matches, "", 0),
            (1, "", "", 0),
            (1, matches, "", 0),
            (0, matches, "inspection failed\n", 1),
            (1, matches, "inspection failed\n", 1),
            (2, "", "", 1),
            (2, matches, "", 1),
        ):
            with self.subTest(status=status, output=output, error=error):
                with tempfile.TemporaryDirectory() as temp_dir:
                    script = (
                        "set -eu\n"
                        "os=macos\n"
                        "releases=/releases\n"
                        f"tmp_dir={shlex.quote(temp_dir)}\n"
                        # Avoid depending on other processes on the test host.
                        'ps() { printf "%s codex\\n" "$(id -u)"; }\n'
                        "mock_lsof() {\n"
                        f"  printf '%s' {shlex.quote(output)}\n"
                        f"  printf '%s' {shlex.quote(error)} >&2\n"
                        f"  return {status}\n"
                        "}\n" + function + "record_standalone_releases_in_use\n"
                    )
                    result = subprocess.run(
                        ["sh"], input=script, text=True, capture_output=True
                    )
                    self.assertEqual(
                        (result.returncode, result.stdout, result.stderr),
                        (expected, "", ""),
                    )
                    self.assertEqual((Path(temp_dir) / "in-use").read_text(), output)

    def test_standalone_retains_previous_and_running_releases_across_installs(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            # macOS lsof can escape the non-ASCII bytes in the installation path.
            root = Path(temp_dir).resolve() / "André codex install"
            root.mkdir()
            system = os.uname()
            if system.sysname != "Darwin":
                self.skipTest("requires native macOS process inspection")
            if system.sysname == "Linux":
                # The race requires pruning to proceed. A live but unreadable
                # Codex/helper (e.g. the agent's sandbox) must prevent that.
                for process in Path("/proc").glob("[0-9]*"):
                    try:
                        (process / "exe").readlink()
                    except OSError:
                        try:
                            name = (process / "comm").read_text().strip()
                            status = (process / "status").read_text()
                        except OSError:
                            continue  # The process may have exited.
                        if (
                            name.startswith("codex") or name in ("rg", "bwrap")
                        ) and not any(
                            line.startswith("State:") and "Z" in line
                            for line in status.splitlines()
                        ):
                            self.skipTest(
                                f"Cannot inspect live {name} process {process.name}"
                            )
            arch = "aarch64" if system.machine in ("arm64", "aarch64") else "x86_64"
            target = f"{arch}-{'apple-darwin' if system.sysname == 'Darwin' else 'unknown-linux-musl'}"
            packages = root / "codex-home/packages/standalone"
            releases = packages / "releases"
            versions = ("0.140.0-doca", "0.141.0-doca", VERSION)
            install_options = {}
            for version in versions:
                source = root / version
                source.mkdir()
                archive, checksum, metadata = create_package_release(
                    source,
                    metadata_version=version,
                    binary_version=version,
                    target=target,
                )
                install_options[version] = dict(
                    metadata_json=metadata, archive_path=archive, checksum_path=checksum
                )

            first, _ = run_installer_in(
                root, versions[0], **install_options[versions[0]]
            )
            self.assertEqual(first.returncode, 0, first.stderr)
            oldest = releases / f"{versions[0]}-{target}"
            helper = oldest / "bin/codex-code-mode-host"
            shutil.copy("/bin/sleep", helper)
            if system.sysname == "Darwin":
                # A copied macOS system binary cannot run with its platform signature.
                subprocess.run(
                    ["/usr/bin/codesign", "--force", "--sign", "-", str(helper)],
                    check=True,
                    capture_output=True,
                )
            # A helper can outlive its parent CLI; retain its whole release.
            with subprocess.Popen([str(helper), "300"]) as running:
                try:
                    second, _ = run_installer_in(
                        root, versions[1], **install_options[versions[1]]
                    )
                    self.assertEqual(second.returncode, 0, second.stderr)

                    unowned = releases / f"0.139.0-{target}"
                    unowned.mkdir()
                    (unowned / "codex-package.json").write_text("{}")
                    retired = releases / ".retired.old-attempt"
                    retired.mkdir()
                    (retired / "leave-alone").write_text("not retried")
                    external = root / "external"
                    external.mkdir()
                    (external / "user-file").write_text("untouched")
                    (releases / f"0.138.0-{target}").symlink_to(external)

                    # Failed installs must not advance history or prune anything.
                    history = packages / "selection-history"
                    before_failure = history.read_bytes()
                    bad_source = root / "wrong-version"
                    bad_source.mkdir()
                    archive, checksum, metadata = create_package_release(
                        bad_source, metadata_version="0.199.0-doca", target=target
                    )
                    failed, _ = run_installer_in(
                        root,
                        "0.199.0-doca",
                        metadata_json=metadata,
                        archive_path=archive,
                        checksum_path=checksum,
                    )
                    self.assertNotEqual(failed.returncode, 0, failed.stderr)
                    self.assertEqual(history.read_bytes(), before_failure)
                    self.assertTrue(oldest.is_dir())

                    latest, _ = run_installer_in(
                        root, "latest", **install_options[VERSION]
                    )
                    self.assertEqual(latest.returncode, 0, latest.stderr)
                    self.assertIsNone(running.poll())
                    self.assertTrue(oldest.is_dir(), latest.stderr)
                    previous = releases / f"{versions[1]}-{target}"
                    current = releases / f"{VERSION}-{target}"
                    self.assertEqual((packages / "current").resolve(), current)
                    self.assertTrue(previous.is_dir())
                    self.assertEqual(
                        history.read_text().splitlines()[:2],
                        [current.name, previous.name],
                    )
                    recorded = history.read_bytes()
                finally:
                    running.terminate()
                    running.wait()

            if system.sysname == "Darwin":
                # lsof sees no running helper now. ps can still see names for
                # a different user, whose open files lsof silently omits.
                fake_ps = root / "bin/ps"
                foreign_uid = 0 if os.geteuid() != 0 else 501
                try:
                    for executable in (oldest / "bin/codex", helper):
                        with self.subTest(foreign_executable=executable.name):
                            fake_ps.write_text(
                                "#!/bin/sh\n"
                                'case "$*" in\n'
                                "  '-axo uid=,comm=')\n"
                                f"    printf '%s\\n' {shlex.quote(f'{foreign_uid} {executable}')}\n"
                                "    ;;\n"
                                '  *) exec /bin/ps "$@" ;;\n'
                                "esac\n"
                            )
                            fake_ps.chmod(0o755)
                            retained, _ = run_installer_in(
                                root, "latest", **install_options[VERSION]
                            )
                            self.assertEqual(retained.returncode, 0, retained.stderr)
                            self.assertTrue(oldest.is_dir(), retained.stderr)
                            self.assertEqual(history.read_bytes(), recorded)
                finally:
                    fake_ps.unlink()

            # Start the helper after the first scan but immediately before the
            # retirement rename. The second scan must restore its release.
            fake_mv = root / "bin/mv"
            ready = root / "retirement-ready"
            resume = root / "retirement-resume"
            os.mkfifo(ready)
            os.mkfifo(resume)
            real_mv = shutil.which("mv")
            fake_mv.write_text(
                "#!/bin/sh\n"
                'case "$2" in\n'
                "  */.retired.*/release)\n"
                f'    printf "ready\\n" > {shlex.quote(str(ready))}\n'
                f"    read -r _ < {shlex.quote(str(resume))}\n"
                "    ;;\n"
                "esac\n"
                f'exec {shlex.quote(real_mv)} "$@"\n'
            )
            fake_mv.chmod(0o755)
            ready_fd = os.open(ready, os.O_RDWR | os.O_NONBLOCK)
            resume_fd = os.open(resume, os.O_RDWR | os.O_NONBLOCK)
            try:
                with ThreadPoolExecutor(max_workers=1) as executor:
                    installing = executor.submit(
                        run_installer_in, root, "latest", **install_options[VERSION]
                    )
                    try:
                        readable, _, _ = select.select([ready_fd], [], [], 30)
                        self.assertTrue(
                            readable, "installer did not attempt retirement"
                        )
                        self.assertEqual(os.read(ready_fd, 128), b"ready\n")
                        with subprocess.Popen([str(helper), "300"]) as late_helper:
                            try:
                                os.write(resume_fd, b"continue\n")
                                raced, _ = installing.result(timeout=30)
                                self.assertEqual(raced.returncode, 0, raced.stderr)
                                self.assertTrue(oldest.is_dir(), raced.stderr)
                                self.assertEqual(history.read_bytes(), recorded)
                            finally:
                                late_helper.terminate()
                                late_helper.wait()
                    finally:
                        # Release the intercepted mv even if an assertion fails.
                        os.write(resume_fd, b"continue\n")
            finally:
                os.close(ready_fd)
                os.close(resume_fd)
                fake_mv.unlink()

            if system.sysname == "Darwin":
                # An external process can hold just the release directory open.
                with subprocess.Popen(["/bin/cat"], stdin=subprocess.PIPE, cwd=oldest):
                    held, _ = run_installer_in(
                        root, "latest", **install_options[VERSION]
                    )
                    self.assertEqual(held.returncode, 0, held.stderr)
                    self.assertTrue(oldest.is_dir(), held.stderr)

            # A no-op latest install (also the codex update path) cleans up the
            # now-unused release without rotating previous to current.
            repeated, _ = run_installer_in(root, "latest", **install_options[VERSION])
            self.assertEqual(repeated.returncode, 0, repeated.stderr)
            self.assertFalse(oldest.exists(), repeated.stderr)
            self.assertTrue(previous.is_dir())
            self.assertEqual(history.read_bytes(), recorded)
            self.assertTrue(unowned.is_dir())
            self.assertEqual((retired / "leave-alone").read_text(), "not retried")
            self.assertEqual((external / "user-file").read_text(), "untouched")

            # Downgrades keep the actual previously selected version.
            pinned, _ = run_installer_in(
                root, versions[1], **install_options[versions[1]]
            )
            self.assertEqual(pinned.returncode, 0, pinned.stderr)
            self.assertEqual((packages / "current").resolve(), previous)
            self.assertEqual(
                history.read_text().splitlines()[:2], [previous.name, current.name]
            )
            self.assertTrue(current.is_dir())

    @unittest.skipUnless(
        sys.platform in ("darwin", "linux"), "requires Unix process inspection"
    )
    def test_uninspectable_legacy_updater_does_not_clear_pin(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            root = Path(temp_dir)
            archive, checksum, metadata = create_package_release(root)
            options = dict(
                metadata_json=metadata,
                archive_path=archive,
                checksum_path=checksum,
                force_macos=True,
            )
            pinned, _ = run_installer_in(root, VERSION, **options)
            self.assertEqual(pinned.returncode, 0, pinned.stderr)
            updater_record = (
                root / "codex-home/app-server-daemon/app-server-updater.pid"
            )
            updater_record.parent.mkdir(parents=True)
            updater_record.write_text(
                json.dumps(
                    {"pid": os.getpid(), "processStartTime": process_start_time()}
                )
            )

            attempted, _ = run_installer_in(
                root,
                "latest",
                old_updater_parent_pid=os.getpid(),
                fail_ps=True,
                **options,
            )
            self.assertEqual(attempted.returncode, 0, attempted.stderr)
            self.assertFalse(
                (root / "codex-home/packages/standalone/auto-update-version").exists()
            )


def run_installer(
    release: str,
    *,
    metadata_failure: bool = False,
    metadata_json: str | None = None,
) -> tuple[subprocess.CompletedProcess[str], list[str]]:
    with tempfile.TemporaryDirectory() as temp_dir:
        return run_installer_in(
            Path(temp_dir),
            release,
            metadata_failure=metadata_failure,
            metadata_json=metadata_json,
        )


def run_installer_in(
    root: Path,
    release: str,
    *,
    installer_path: Path = INSTALL_SCRIPT,
    metadata_failure: bool = False,
    metadata_json: str | None = None,
    archive_path: Path | None = None,
    checksum_path: Path | None = None,
    force_macos: bool = True,
    update_guard_from_release: str | None = None,
    old_updater_parent_pid: int | None = None,
    fail_ps: bool = False,
    daemon_only: bool = False,
    manual_update: bool = False,
) -> tuple[subprocess.CompletedProcess[str], list[str]]:
    bin_dir = root / "bin"
    bin_dir.mkdir(exist_ok=True)
    request_log = root / "requests.log"
    fake_curl = bin_dir / "curl"
    fake_curl.write_text(
        textwrap.dedent(
            r"""
            #!/bin/sh
            url=""
            output=""
            previous=""
            for arg in "$@"; do
              case "$arg" in
                https://*) url="$arg" ;;
              esac
              if [ "$previous" = "-o" ]; then
                output="$arg"
              fi
              previous="$arg"
            done
            printf '%s\n' "$url" >>"$CODEX_TEST_REQUEST_LOG"

            case "$url" in
              https://api.github.com/*)
                if [ "$CODEX_TEST_METADATA_FAILURE" = "1" ]; then
                  echo "curl: (22) The requested URL returned error: 403" >&2
                  exit 22
                fi
                printf '%s\n' "$CODEX_TEST_METADATA_JSON"
                ;;
              https://github.com/DOCaCola/codex/releases/download/*/SHA256SUMS)
                if [ -n "$CODEX_TEST_CHECKSUM_PATH" ]; then
                  cp "$CODEX_TEST_CHECKSUM_PATH" "$output"
                else
                  exit 22
                fi
                ;;
              https://github.com/DOCaCola/codex/releases/download/*/codex-doca-*.tar.gz)
                if [ -n "$CODEX_TEST_ARCHIVE_PATH" ]; then
                  cp "$CODEX_TEST_ARCHIVE_PATH" "$output"
                else
                  exit 22
                fi
                ;;
              *)
                exit 22
                ;;
            esac
            """
        ).lstrip(),
        encoding="utf-8",
    )
    fake_curl.chmod(0o755)
    if force_macos:
        fake_uname = bin_dir / "uname"
        fake_uname.write_text(
            "#!/bin/sh\n"
            'case "$1" in\n'
            "  -s) printf 'Darwin\\n' ;;\n"
            "  -m) printf 'arm64\\n' ;;\n"
            "esac\n",
            encoding="utf-8",
        )
        fake_uname.chmod(0o755)
    if old_updater_parent_pid is not None:
        fake_ps = bin_dir / "ps"
        fake_ps.write_text(
            "#!/bin/sh\nexit 1\n"
            if fail_ps
            else "#!/bin/sh\n"
            'case "$*" in\n'
            '  *lstart*) printf "S %s\\n" "$CODEX_TEST_PARENT_START" ;;\n'
            '  *) printf "%s\\n" "$CODEX_TEST_PARENT_PID" ;;\n'
            "esac\n",
            encoding="utf-8",
        )
        fake_ps.chmod(0o755)

    home = root / "home"
    home.mkdir(exist_ok=True)
    env = os.environ.copy()
    env.update(
        {
            "CODEX_HOME": str(root / "codex-home"),
            "CODEX_INSTALL_DIR": str(root / "install-bin"),
            "CODEX_NON_INTERACTIVE": "1",
            "CODEX_RELEASE": release,
            "CODEX_INSTALL_DAEMON_ONLY": "1" if daemon_only else "0",
            "CODEX_TEST_ARCHIVE_PATH": str(archive_path or ""),
            "CODEX_TEST_CHECKSUM_PATH": str(checksum_path or ""),
            "CODEX_TEST_METADATA_FAILURE": "1" if metadata_failure else "0",
            "CODEX_TEST_METADATA_JSON": (
                metadata_json if metadata_json is not None else release_metadata()
            ),
            "CODEX_TEST_REQUEST_LOG": str(request_log),
            "HOME": str(home),
            "PATH": f"{bin_dir}:/usr/bin:/bin",
            "SHELL": "/bin/sh",
        }
    )
    env["CODEX_INSTALL_IF_CURRENT"] = "1" if manual_update else "0"
    if update_guard_from_release is None:
        env.pop("CODEX_INSTALL_IF_LATEST", None)
        env.pop("CODEX_UPDATE_FROM_RELEASE", None)
    else:
        env["CODEX_INSTALL_IF_LATEST"] = "0" if manual_update else "1"
        env["CODEX_UPDATE_FROM_RELEASE"] = update_guard_from_release
    if old_updater_parent_pid is not None:
        env["CODEX_TEST_PARENT_PID"] = str(old_updater_parent_pid)
        env["CODEX_TEST_PARENT_START"] = process_start_time()
    result = subprocess.run(
        ["/bin/sh", str(installer_path)],
        capture_output=True,
        check=False,
        env=env,
        text=True,
    )
    requests = (
        request_log.read_text(encoding="utf-8").splitlines()
        if request_log.exists()
        else []
    )
    return result, requests


def process_start_time() -> str:
    details = subprocess.check_output(
        ["ps", "-p", str(os.getpid()), "-o", "stat=", "-o", "lstart="], text=True
    ).strip()
    return details.split(maxsplit=1)[1]


def create_package_release(
    root: Path,
    *,
    metadata_version: str = VERSION,
    binary_version: str = VERSION,
    target: str = "aarch64-apple-darwin",
    include_exec_server: bool = False,
    manifest_target: str | None = None,
) -> tuple[Path, Path, str]:
    package_dir = root / "package"
    (package_dir / "bin").mkdir(parents=True)
    (package_dir / "codex-path").mkdir()
    metadata = {
        "version": binary_version,
        "target": manifest_target or target,
        "entrypoint": "bin/codex",
    }
    if include_exec_server:
        metadata["execServer"] = "bin/codex-exec-server"
        write_executable(package_dir / "bin/codex-exec-server", "#!/bin/sh\nexit 0\n")
    (package_dir / "codex-package.json").write_text(
        json.dumps(metadata),
        encoding="utf-8",
    )
    write_executable(
        package_dir / "bin" / "codex",
        f"#!/bin/sh\nprintf 'codex-cli {binary_version}\\n'\n",
    )
    write_executable(
        package_dir / "bin" / "codex-code-mode-host",
        "#!/bin/sh\nexit 0\n",
    )
    write_executable(package_dir / "bin" / "hpatch", "#!/bin/sh\nexit 0\n")
    write_executable(package_dir / "codex-path" / "rg", "#!/bin/sh\nexit 0\n")
    if "linux" in target:
        (package_dir / "codex-resources").mkdir()
        write_executable(package_dir / "codex-resources/bwrap", "#!/bin/sh\nexit 0\n")

    asset = f"codex-doca-{target}.tar.gz"
    archive_path = root / asset
    with tarfile.open(archive_path, "w:gz") as archive:
        for path in package_dir.iterdir():
            archive.add(path, arcname=path.name)

    archive_digest = hashlib.sha256(archive_path.read_bytes()).hexdigest()
    checksum_path = root / "SHA256SUMS"
    checksum_path.write_text(f"{archive_digest}  {asset}\n", encoding="utf-8")
    checksum_digest = hashlib.sha256(checksum_path.read_bytes()).hexdigest()
    metadata_json = json.dumps(
        {
            "assets": [
                {"name": asset, "digest": f"sha256:{archive_digest}"},
                {
                    "name": "SHA256SUMS",
                    "digest": f"sha256:{checksum_digest}",
                },
            ],
            "tag_name": f"doca-v{metadata_version}",
        },
        indent=2,
    )
    return archive_path, checksum_path, metadata_json


def write_executable(path: Path, contents: str) -> None:
    path.write_text(contents, encoding="utf-8")
    path.chmod(0o755)


def release_metadata(*, compact: bool = False, reorder: bool = False) -> str:
    assets = [
        asset_metadata(
            f"codex-doca-{target}.tar.gz",
            f"sha256:{'a' * 64}",
            reorder=reorder,
        )
        for target in (
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "aarch64-unknown-linux-musl",
            "x86_64-unknown-linux-musl",
        )
    ]
    assets.append(
        asset_metadata(
            "SHA256SUMS",
            f"sha256:{'b' * 64}",
            reorder=reorder,
        )
    )
    separators = (",", ":") if compact else None
    return json.dumps(
        {"assets": assets, "body": "braces: { } [ ]", "tag_name": f"doca-v{VERSION}"},
        indent=None if compact else 2,
        separators=separators,
    )


def asset_metadata(name: str, digest: str, *, reorder: bool) -> dict[str, str]:
    if reorder:
        return {"digest": digest, "name": name}
    return {"name": name, "digest": digest}


if __name__ == "__main__":
    unittest.main()
