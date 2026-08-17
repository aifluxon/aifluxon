# AIFLUXON Python SDK

AIFLUXON Python is the second host of the canonical embedded agent backend. It is not a remote-control API and it does not copy host product semantics.

```text
Python
  → PyO3
  → aifluxon-api
  → canonical Runtime
  → Provider / Tool
```

## Maturity

The SDK is **experimental**. Public names are listed in `aifluxon.__all__`. Internal `_native` helpers are not a user API.

## Install

```powershell
pip install aifluxon
```

| Platform             | Status                 |
| -------------------- | ---------------------- |
| Windows 10/11 x86_64 | Supported              |
| Windows ARM64        | Not supported          |
| Linux                | Not supported in 0.1.0 |
| macOS                | Not supported in 0.1.0 |

Requires **CPython 3.11–3.14**. Wheels are ABI-specific (`cp3xx-win_amd64`), not a single abi3 wheel.

## Minimal quick start

```python
import asyncio
from aifluxon import Agent, ControlledProvider

async def main():
    agent = Agent(provider=ControlledProvider(["Hello from AIFLUXON."]))
    result = await (await agent.start("hello")).result()
    print(result.text)

asyncio.run(main())
```

`ControlledProvider` is the offline/test provider. It does not use the network.

## Develop from source

```powershell
cd bindings/python
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install maturin pytest pytest-asyncio
maturin develop
python -c "import aifluxon"
```

## Documentation

- [Quick Start](quickstart.md)
- [API Reference](api-reference.md)
- [Providers](providers.md)
- [Sessions](sessions.md)
- [Events](events.md)
- [Tools & Policy](tools-and-policy.md)
- [Errors](errors.md)
- [Architecture](architecture.md)
- [API development](api-development.md)

## Providers

Public Python providers:

- OpenAI
- DeepSeek API
- Qwen
- Kimi
- Gemini
- Codex
- Custom
- ControlledProvider (offline)

ChatGPT Web and DeepSeek Web are host-private providers. They are **not** part of the public Python SDK. There is no `ChatGptWebProvider` / `DeepSeekWebProvider` constructor here.

## Not in this SDK

- Host permission products (Default / Managed / Trusted)
- Host file tools, MATLAB, bundled converters
- Tauri windows, product history, OAuth UI

## Stability

| Surface | Status |
|---|---|
| `aifluxon.__all__` | Experimental public API |
| `_native` | Internal |
| Host product adapters | Out of scope |
