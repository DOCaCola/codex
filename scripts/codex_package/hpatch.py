"""Validate companion provenance and carry redistribution notices into packages."""

import hashlib
import json
from pathlib import Path
import shutil

from .targets import REPO_ROOT


def stage_hpatch_notices(binary: Path, package: Path, target: str) -> None:
    metadata = binary.with_name(binary.name + ".metadata")
    manifest = json.loads((metadata / "provenance.json").read_text(encoding="utf-8"))
    pin = json.loads((REPO_ROOT / "third_party/hpatch/source.json").read_text())
    for key, value in pin.items():
        if manifest.get(key) != value:
            raise RuntimeError(f"hpatch provenance does not match source pin: {key}")
    if manifest["target"] != target:
        raise RuntimeError("hpatch provenance target does not match package")
    if manifest["sha256"] != hashlib.sha256(binary.read_bytes()).hexdigest():
        raise RuntimeError("hpatch binary checksum does not match provenance")
    for name in ["LICENSE", "Go-LICENSE"] + [
        dep["license"] for dep in manifest["dependencies"]
    ]:
        if not (metadata / name).is_file():
            raise RuntimeError(f"Missing hpatch dependency notice: {name}")
    shutil.copytree(metadata, package / "licenses/hpatch")
    for name in ("LICENSE", "NOTICE"):
        shutil.copyfile(REPO_ROOT / name, package / name)
