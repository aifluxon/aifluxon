from __future__ import annotations

import argparse
import hashlib
import json
from dataclasses import asdict, dataclass
from pathlib import Path

from packaging.utils import canonicalize_name, parse_wheel_filename


EXPECTED_PYTHONS = ("cp311", "cp312", "cp313", "cp314")
EXPECTED_PLATFORMS = ("win_amd64", "manylinux_x86_64", "manylinux_aarch64")
ALLOWED_MANYLINUX_PLATFORMS = {
    "manylinux_x86_64": {"manylinux_2_17_x86_64", "manylinux2014_x86_64"},
    "manylinux_aarch64": {"manylinux_2_17_aarch64", "manylinux2014_aarch64"},
}


@dataclass(frozen=True)
class WheelRecord:
    filename: str
    sha256: str
    python: str
    platform: str


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def classify_platform(platforms: set[str]) -> str:
    if platforms == {"win_amd64"}:
        return "win_amd64"
    for label, allowed in ALLOWED_MANYLINUX_PLATFORMS.items():
        if platforms and platforms <= allowed:
            return label
    raise ValueError(f"Unsupported or non-manylinux platform tags: {sorted(platforms)}")


def inspect_wheels(dist: Path) -> tuple[str, list[WheelRecord]]:
    wheels = sorted(dist.glob("*.whl"))
    if len(wheels) != 12:
        raise ValueError(f"Expected exactly 12 wheels, found {len(wheels)} in {dist}")

    versions: set[str] = set()
    combinations: set[tuple[str, str]] = set()
    records: list[WheelRecord] = []
    for wheel in wheels:
        project, version, _build, tags = parse_wheel_filename(wheel.name)
        if canonicalize_name(project) != "aifluxon":
            raise ValueError(f"Unexpected project in wheel {wheel.name}: {project}")
        versions.add(str(version))

        interpreters = {tag.interpreter for tag in tags}
        abis = {tag.abi for tag in tags}
        platforms = {tag.platform for tag in tags}
        if len(interpreters) != 1:
            raise ValueError(f"Wheel must contain one CPython interpreter tag: {wheel.name}")
        python = next(iter(interpreters))
        if python not in EXPECTED_PYTHONS or abis != {python}:
            raise ValueError(
                f"Wheel must use an ABI-specific cp311-cp314 tag, got "
                f"interpreters={sorted(interpreters)} abis={sorted(abis)}: {wheel.name}"
            )
        platform = classify_platform(platforms)
        combination = (python, platform)
        if combination in combinations:
            raise ValueError(f"Duplicate wheel combination {combination}: {wheel.name}")
        combinations.add(combination)
        records.append(
            WheelRecord(
                filename=wheel.name,
                sha256=sha256(wheel),
                python=python,
                platform=platform,
            )
        )

    if len(versions) != 1:
        raise ValueError(f"Wheel versions do not match: {sorted(versions)}")
    expected = {(python, platform) for python in EXPECTED_PYTHONS for platform in EXPECTED_PLATFORMS}
    if combinations != expected:
        missing = sorted(expected - combinations)
        unexpected = sorted(combinations - expected)
        raise ValueError(f"Wheel matrix mismatch: missing={missing}, unexpected={unexpected}")
    records.sort(key=lambda record: (record.platform, record.python, record.filename))
    return versions.pop(), records


def manifest_payload(version: str, records: list[WheelRecord]) -> dict:
    return {
        "project": "aifluxon",
        "version": version,
        "wheels": [asdict(record) for record in records],
    }


def write_manifest(path: Path, version: str, records: list[WheelRecord]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        json.dumps(manifest_payload(version, records), indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def verify_manifest(path: Path, version: str, records: list[WheelRecord]) -> None:
    try:
        actual = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"Release manifest could not be read: {error}") from error
    expected = manifest_payload(version, records)
    if actual != expected:
        raise ValueError("Release manifest does not match the wheel matrix or SHA256 checksums")


def main() -> None:
    parser = argparse.ArgumentParser(description="Verify the AIFLUXON Python release wheel matrix")
    parser.add_argument("--dist", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--verify-manifest", action="store_true")
    args = parser.parse_args()

    version, records = inspect_wheels(args.dist)
    if args.verify_manifest:
        verify_manifest(args.manifest, version, records)
        print(f"Verified immutable AIFLUXON {version} release bundle ({len(records)} wheels)")
    else:
        write_manifest(args.manifest, version, records)
        print(f"Verified AIFLUXON {version} wheel matrix and wrote {args.manifest}")


if __name__ == "__main__":
    try:
        main()
    except ValueError as error:
        raise SystemExit(str(error)) from error
