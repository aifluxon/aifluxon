# AIFLUXON

Experimental Python SDK for the AIFLUXON embedded Agent backend.

```text
Python → PyO3 → aifluxon-api → canonical Runtime → Providers / Tools
```

## Install

```powershell
pip install aifluxon
```

Requires **Windows 10/11 x86_64** and **CPython 3.11–3.14**. Wheels are version-specific (`cp311`–`cp314-win_amd64`), not abi3. Linux, macOS, and Windows ARM64 are not supported in 0.1.0. Installing the published wheel does not require Rust.

License: **Apache-2.0**.

## Quick start

```python
import asyncio
from aifluxon import Agent, ControlledProvider

async def main():
    agent = Agent(provider=ControlledProvider(["Hello from AIFLUXON."]))
    print((await agent.run("hello")).text)

asyncio.run(main())
```

`ControlledProvider` is offline. Public providers: OpenAI, DeepSeek, Qwen, Kimi, Gemini, Codex, Custom. ChatGPT Web and DeepSeek Web are **not** part of this SDK.

Documentation: https://github.com/aifluxon/aifluxon/blob/main/docs/python/README.md

## Develop from source

This directory is the PyO3 host. It talks only to `aifluxon-api`.

```powershell
cd bindings/python
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install maturin pytest pytest-asyncio
maturin develop
python -c "import aifluxon; print(aifluxon.__version__)"
pytest
```

Do not depend on `aifluxon-core`, `aifluxon-runtime`, `aifluxon-providers`, EasyPhy, or Tauri from this crate.
