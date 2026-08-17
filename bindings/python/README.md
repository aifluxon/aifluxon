# AIFLUXON Python bindings

This crate is the Python host for the canonical AIFLUXON runtime. It talks only to `aifluxon-api`.

## Layout

```text
bindings/python/
├── Cargo.toml          # PyO3 cdylib; depends on aifluxon-api only
├── pyproject.toml      # maturin package metadata
├── src/lib.rs          # native bridge
├── python/aifluxon/    # public Python package
├── examples/
└── tests/
```

## Requirements

- CPython 3.11–3.14
- Rust stable toolchain (development only)
- [maturin](https://www.maturin.rs/)
- Packaging target: **Windows 10/11 x86_64** (experimental SDK)

Published installs:

```powershell
pip install aifluxon
```

Wheels are version-specific (`cp311` / `cp312` / `cp313` / `cp314`), not abi3. Linux, macOS, and Windows ARM64 are not supported in 0.1.0.

ChatGPT Web and DeepSeek Web are **not** part of this SDK.

## Develop

```powershell
cd bindings/python
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install maturin pytest pytest-asyncio
maturin develop
python -c "import aifluxon; print(aifluxon.__version__)"
pytest
```

## Add a public Python API

1. Add or reuse a stable seam in `crates/aifluxon-api`.
2. Expose it from `src/lib.rs` if native work is required.
3. Wrap it in `python/aifluxon/` and add the name to `__all__`.
4. Update `docs/python/api-reference.md` and any relevant example.
5. Add a test. `test_public_symbols_are_documented` must still pass.

Do not depend on `aifluxon-core`, `aifluxon-runtime`, `aifluxon-providers`, EasyPhy, or Tauri from this crate.

User-facing documentation lives in `docs/python/`.
