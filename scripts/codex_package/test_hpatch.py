"""Companion packaging checks with deliberately modified build artifacts."""

import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from .hpatch import stage_hpatch_notices
from .targets import REPO_ROOT


def fixture_hpatch(path: Path, target: str) -> Path:
    path.write_bytes(b"hpatch fixture")
    path.chmod(0o755)
    metadata = path.with_name(path.name + ".metadata")
    metadata.mkdir()
    pin = json.loads((REPO_ROOT / "third_party/hpatch/source.json").read_text())
    manifest = {
        **pin,
        "target": target,
        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "dependencies": [
            {
                "module": "example.test/dependency",
                "version": "v1.0.0",
                "license": "dependency-LICENSE",
            }
        ],
    }
    for name in ("LICENSE", "Go-LICENSE", "dependency-LICENSE"):
        (metadata / name).write_text(f"{name} fixture", encoding="utf-8")
    (metadata / "provenance.json").write_text(json.dumps(manifest), encoding="utf-8")
    return path


class HpatchProvenanceTest(unittest.TestCase):
    def test_mismatched_artifacts_are_rejected(self) -> None:
        target = "x86_64-pc-windows-msvc"
        for mutation, expected in [
            ("binary", "checksum"),
            ("target", "target"),
            ("commit", "source pin"),
            ("license", "dependency notice"),
        ]:
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                binary = fixture_hpatch(root / "hpatch.exe", target)
                metadata = binary.with_name(binary.name + ".metadata")
                provenance = metadata / "provenance.json"
                manifest = json.loads(provenance.read_text())
                if mutation == "binary":
                    binary.write_bytes(b"replaced after build")
                elif mutation == "target":
                    manifest["target"] = "aarch64-pc-windows-msvc"
                elif mutation == "commit":
                    manifest["commit"] = "0" * 40
                else:
                    (metadata / "dependency-LICENSE").unlink()
                provenance.write_text(json.dumps(manifest))
                with self.assertRaisesRegex(RuntimeError, expected):
                    stage_hpatch_notices(binary, root / "package", target)


if __name__ == "__main__":
    unittest.main()
