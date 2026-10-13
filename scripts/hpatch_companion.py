#!/usr/bin/env python3
"""Prepare and build the pinned hpatch CLI and its redistribution metadata."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

REPO = Path(__file__).resolve().parents[1]
PIN = json.loads((REPO / "third_party/hpatch/source.json").read_text())
TARGETS = {
    "x86_64-pc-windows-msvc": ("windows", "amd64"),
    "aarch64-pc-windows-msvc": ("windows", "arm64"),
    "x86_64-apple-darwin": ("darwin", "amd64"),
    "aarch64-apple-darwin": ("darwin", "arm64"),
    "x86_64-unknown-linux-musl": ("linux", "amd64"),
    "aarch64-unknown-linux-musl": ("linux", "arm64"),
    "x86_64-unknown-linux-gnu": ("linux", "amd64"),
    "aarch64-unknown-linux-gnu": ("linux", "arm64"),
}


def verify_source(source: Path) -> None:
    commit = subprocess.check_output(
        ["git", "-C", str(source), "rev-parse", "HEAD"], text=True
    ).strip()
    dirty = subprocess.check_output(
        ["git", "-C", str(source), "status", "--porcelain"], text=True
    ).strip()
    if commit != PIN["commit"] or dirty:
        raise RuntimeError("hpatch source must be clean and match source.json")


def prepare(source: Path) -> None:
    if not source.exists():
        source.parent.mkdir(parents=True, exist_ok=True)
        # Do not overwrite or delete an existing checkout.
        subprocess.run(
            [
                "git",
                "clone",
                "--filter=blob:none",
                "--no-checkout",
                PIN["repository"],
                str(source),
            ],
            check=True,
        )
        subprocess.run(
            ["git", "-C", str(source), "checkout", "--detach", PIN["commit"]],
            check=True,
        )
    verify_source(source)


def build(source: Path, target: str, output: Path, go: str, cache: Path) -> None:
    verify_source(source)
    goos, goarch = TARGETS[target]
    cache.mkdir(parents=True, exist_ok=True)
    env = {
        **os.environ,
        "CGO_ENABLED": "0",
        "GOOS": goos,
        "GOARCH": goarch,
        "GOWORK": "off",
        "GOTOOLCHAIN": "local",
        "GOCACHE": str(cache / "go-build-cache"),
        "GOMODCACHE": str(cache / "go-mod-cache"),
    }

    def run(*args: str) -> str:
        return subprocess.check_output(
            [go, *args], cwd=source, env=env, text=True
        ).strip()

    actual_version = run("env", "GOVERSION").removeprefix("go")
    if actual_version != PIN["goVersion"]:
        raise RuntimeError(f"Install Go {PIN['goVersion']}; found {actual_version}")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="hpatch-", dir=output.parent) as temp:
        staging = Path(temp)
        binary = staging / output.name
        run(
            "build",
            "-mod=readonly",
            "-trimpath",
            "-buildvcs=false",
            "-ldflags=-s -w -buildid=",
            "-o",
            str(binary),
            "./cmd/hpatch",
        )
        metadata = staging / "metadata"
        metadata.mkdir()
        shutil.copyfile(source / "LICENSE", metadata / "LICENSE")
        shutil.copyfile(Path(run("env", "GOROOT")) / "LICENSE", metadata / "Go-LICENSE")
        modules = []
        for line in run("version", "-m", str(binary)).splitlines():
            fields = line.split()
            if not fields or fields[0] != "dep":
                continue
            module, version = fields[1:3]
            directory = Path(run("list", "-m", "-f", "{{.Dir}}", module))
            license_path = directory / "LICENSE"
            # Fail when a dependency requires a new licensing arrangement.
            name = module.replace("/", "_") + "-LICENSE"
            shutil.copyfile(license_path, metadata / name)
            modules.append({"module": module, "version": version, "license": name})
        manifest = {
            **PIN,
            "target": target,
            "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            "dependencies": modules,
        }
        (metadata / "provenance.json").write_text(
            json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
        )
        destination = output.with_name(output.name + ".metadata")
        if destination.exists():
            shutil.rmtree(destination)
        shutil.copytree(metadata, destination)
        os.replace(binary, output)
    print(f"Built {output} from {PIN['commit']}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prep = commands.add_parser("prepare")
    prep.add_argument("--output", type=Path, required=True)
    compile_parser = commands.add_parser("build")
    compile_parser.add_argument("--source", type=Path, required=True)
    compile_parser.add_argument("--target", choices=TARGETS, required=True)
    compile_parser.add_argument("--output", type=Path, required=True)
    compile_parser.add_argument("--go", default=os.environ.get("GO_BIN", "go"))
    compile_parser.add_argument(
        "--cache-dir",
        type=Path,
        default=Path(os.environ.get("HPATCH_CACHE_DIR", tempfile.gettempdir()))
        / "hpatch-cache",
    )
    args = parser.parse_args()
    if args.command == "prepare":
        prepare(args.output.resolve())
    else:
        build(
            args.source.resolve(),
            args.target,
            args.output.resolve(),
            args.go,
            args.cache_dir.resolve(),
        )


if __name__ == "__main__":
    main()
