# Contributing

## Tests

```powershell
cargo fmt --all -- --check
cargo test --workspace
cargo check --workspace
```

Python:

```powershell
cd bindings/python
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install maturin pytest pytest-asyncio
maturin develop
pytest
```

## Python API changes

1. Add or extend the stable surface in `aifluxon-api`.
2. Wrap it in `bindings/python`.
3. Export it from `aifluxon.__all__`.
4. Update `docs/python/api-reference.md` and examples if needed.

Do not import `aifluxon-core`, `aifluxon-runtime`, or `aifluxon-providers` from the Python crate. Do not present `_native` as a supported user API.

Root `Cargo.lock` is tracked so Python wheels rebuild against the same Rust dependency graph.
