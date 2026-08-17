# AIFLUXON

Experimental Python SDK for the AIFLUXON in-process agent backend.

```text
Python → PyO3 → aifluxon-api → runtime → providers / tools
```

## Install

```powershell
pip install aifluxon
```

Requires **Windows 10/11 x86_64** and **CPython 3.11–3.14**. Wheels are version-specific (`cp311`–`cp314-win_amd64`), not abi3. Linux, macOS, and Windows ARM64 are not supported in 0.1.0. Installing a published wheel does not require Rust.

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

`ControlledProvider` is offline. Public providers: OpenAI, DeepSeek, Qwen, Kimi, Gemini, Codex, Custom. ChatGPT Web and DeepSeek Web are not included in this SDK.

Documentation: https://github.com/aifluxon/aifluxon/blob/main/docs/python/README.md

## Build from source

This package is the Python binding. It depends only on `aifluxon-api`.

```powershell
cd bindings/python
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install maturin pytest pytest-asyncio
maturin develop
python -c "import aifluxon; print(aifluxon.__version__)"
pytest
```
