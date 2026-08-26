from __future__ import annotations

import argparse
import subprocess
import tempfile
import zipfile
from pathlib import Path


FORBIDDEN_TLS_LIBRARIES = ("libssl.so", "libcrypto.so")


def main() -> None:
    parser = argparse.ArgumentParser(description="Audit an AIFLUXON Linux wheel native extension")
    parser.add_argument("wheel", type=Path)
    args = parser.parse_args()

    if "manylinux" not in args.wheel.name:
        raise SystemExit(f"Refusing to audit a non-manylinux wheel: {args.wheel.name}")
    with tempfile.TemporaryDirectory(prefix="aifluxon-audit-") as directory:
        root = Path(directory)
        with zipfile.ZipFile(args.wheel) as archive:
            archive.extractall(root)
        extensions = list(root.glob("aifluxon/_native*.so"))
        if len(extensions) != 1:
            raise SystemExit(
                f"Expected exactly one aifluxon/_native*.so, found {len(extensions)}"
            )
        completed = subprocess.run(
            ["ldd", str(extensions[0])],
            check=False,
            capture_output=True,
            text=True,
        )
        output = completed.stdout + completed.stderr
        print(output, end="")
        if completed.returncode != 0:
            raise SystemExit(f"ldd failed with exit code {completed.returncode}")
        lowered = output.lower()
        forbidden = [library for library in FORBIDDEN_TLS_LIBRARIES if library in lowered]
        if forbidden:
            raise SystemExit(f"Wheel links forbidden system TLS libraries: {forbidden}")
        if "not found" in lowered:
            raise SystemExit("Wheel has unresolved native dependencies")


if __name__ == "__main__":
    main()
