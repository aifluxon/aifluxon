from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from verify_python_wheels import inspect_wheels, verify_manifest, write_manifest


PYTHONS = ("cp311", "cp312", "cp313", "cp314")
PLATFORMS = {
    "win_amd64": "win_amd64",
    "manylinux_x86_64": "manylinux_2_17_x86_64.manylinux2014_x86_64",
    "manylinux_aarch64": "manylinux_2_17_aarch64.manylinux2014_aarch64",
}


def populate(directory: Path) -> None:
    for python in PYTHONS:
        for platform in PLATFORMS.values():
            (directory / f"aifluxon-0.2.0-{python}-{python}-{platform}.whl").write_bytes(
                f"{python}:{platform}".encode()
            )


class WheelVerifierTests(unittest.TestCase):
    def test_complete_matrix_generates_and_verifies_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            populate(root)
            version, records = inspect_wheels(root)
            manifest = root / "release-manifest.json"
            write_manifest(manifest, version, records)
            verify_manifest(manifest, version, records)

    def test_missing_wheel_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            populate(root)
            next(root.glob("*.whl")).unlink()
            with self.assertRaisesRegex(ValueError, "exactly 12"):
                inspect_wheels(root)

    def test_extra_wheel_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            populate(root)
            (root / "aifluxon-0.2.0-cp310-cp310-win_amd64.whl").write_bytes(b"extra")
            with self.assertRaisesRegex(ValueError, "exactly 12"):
                inspect_wheels(root)

    def test_wrong_tag_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            populate(root)
            wheel = next(root.glob("*cp311*win_amd64.whl"))
            wheel.rename(root / wheel.name.replace("cp311-cp311", "cp311-abi3"))
            with self.assertRaisesRegex(ValueError, "ABI-specific"):
                inspect_wheels(root)

    def test_checksum_tamper_fails(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            populate(root)
            version, records = inspect_wheels(root)
            manifest = root / "release-manifest.json"
            write_manifest(manifest, version, records)
            payload = json.loads(manifest.read_text(encoding="utf-8"))
            payload["wheels"][0]["sha256"] = "0" * 64
            manifest.write_text(json.dumps(payload), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "does not match"):
                verify_manifest(manifest, version, records)


if __name__ == "__main__":
    unittest.main()
