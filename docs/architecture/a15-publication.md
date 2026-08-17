# A15 publication record

Release version: **0.1.0**

License: Apache-2.0

Maturity: Experimental

## GitHub

- Repository: https://github.com/aifluxon/aifluxon
- Visibility: public
- Owner: GitHub organization `aifluxon` (authenticated user `Ken-CUMT` is org admin)
- Main: `main`
- Release commit (packaging / EasyPhy pin): `4b18916f16e65ce83c23d03921159b948dc4e037`
- Tag: not created (production PyPI not published)
- GitHub Release: not created

## Python packaging

- Package name: `aifluxon`
- Version: `0.1.0`
- Requires-Python: `>=3.11`
- License-Expression: Apache-2.0
- maturin: 1.14.1
- PyO3: 0.26 (extension-module, not abi3)
- Local wheel: `aifluxon-0.1.0-cp314-cp314-win_amd64.whl`
- SHA256: `7A75A964DE5CE9D40A91C2F01928AAC721681B566684E52C1C8257007EA53E87`
- Platform: Windows 10/11 x86_64 only
- Intended CPython matrix: 3.11, 3.12, 3.13, 3.14 via `.github/workflows/python-release.yml`
- Locally verified ABI: `cp314-cp314-win_amd64` (this machine has CPython 3.14 only)

Root `Cargo.lock` is tracked for wheel reproducibility.

## Clean local install

- Environment: `E:\aifluxon-release-test\.venv` (outside the source tree)
- Wheel source: copied `win_amd64` wheel, not `maturin develop`
- `aifluxon.__file__`: `E:\aifluxon-release-test\.venv\Lib\site-packages\aifluxon\__init__.py`
- Smoke: `ControlledProvider` quickstart printed `Hello from AIFLUXON.`
- Uninstall/reinstall: PASS
- Rust required by the user: no
- Source checkout required: no

## PyPI

- TestPyPI: not published
- Production PyPI: not published
- Name `aifluxon` was unused on both indexes at audit time (HTTP 404)

Blocked on Trusted Publishing configuration. GitHub environments `testpypi` and `pypi` exist. Workflow: `.github/workflows/python-release.yml`.

Pending publisher fields for the user:

- Owner: `aifluxon`
- Repository: `aifluxon`
- Workflow name: `python-release.yml`
- Environment: `testpypi` then `pypi`

## EasyPhy dependency

- Type: git + rev (same repository, same SHA for api/core/runtime/providers)
- Pinned SHA: `4b18916f16e65ce83c23d03921159b948dc4e037`
- Remaining production absolute `E:\aifluxon` paths: none in manifests; plan docs may still mention the historical local tree
- EasyPhy regression: `cargo test -p easyphy-studio` 749 passed; `npm test` passed; frontend/package gates recorded in the A15 report

## Provider preservation

Public Python SDK constructors: OpenAI, DeepSeek, Qwen, Kimi, Gemini, Codex, Custom, ControlledProvider.

ChatGPT Web and DeepSeek Web: not public. Capability stubs remain crate-private and always-error in `next_turn`.
