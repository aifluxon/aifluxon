from __future__ import annotations

import ast
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def read_toml(path: Path) -> dict:
    return tomllib.loads(path.read_text(encoding="utf-8"))


def read_python_version(path: Path) -> str:
    module = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    for statement in module.body:
        if not isinstance(statement, ast.Assign):
            continue
        if any(isinstance(target, ast.Name) and target.id == "__version__" for target in statement.targets):
            value = ast.literal_eval(statement.value)
            if isinstance(value, str):
                return value
    raise SystemExit(f"Missing string __version__ assignment in {path}")


def main() -> None:
    versions = {
        "workspace": read_toml(ROOT / "Cargo.toml")["workspace"]["package"]["version"],
        "binding": read_toml(ROOT / "bindings/python/Cargo.toml")["package"]["version"],
        "pyproject": read_toml(ROOT / "bindings/python/pyproject.toml")["project"]["version"],
        "python_package": read_python_version(
            ROOT / "bindings/python/python/aifluxon/__init__.py"
        ),
    }
    if len(set(versions.values())) != 1:
        raise SystemExit(f"Version mismatch: {versions}")
    print(next(iter(versions.values())))


if __name__ == "__main__":
    main()
