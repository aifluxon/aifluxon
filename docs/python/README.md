# AIFLUXON Python SDK

Python bindings for the AIFLUXON in-process agent backend. The package talks to the same Rust facade as other embedders. It is not a remote-control API.

```text
Python
  → PyO3
  → aifluxon-api
  → runtime
  → providers / tools
```

## Status

The SDK is **experimental**. Public names are listed in `aifluxon.__all__`. Helpers under `_native` are not a supported API.

## Install

```bash
python -m pip install aifluxon
```

| Platform | Status |
| -------- | ------ |
| Windows 10/11 x86_64 | Supported |
| Linux glibc x86_64 | Supported |
| Linux glibc aarch64 | Supported |
| Windows ARM64 | Not supported |
| Alpine Linux / musl | Not supported by 0.2.0 wheels |
| macOS | Not yet a Python wheel target |

Requires **CPython 3.11–3.14**. Wheels are ABI-specific (`cp3xx`), not a single abi3 wheel. Linux wheels use manylinux2014 / PEP 600 with a glibc 2.17 baseline. CPython 3.14 free-threaded builds, PyPy, Linux i686, and Windows ARM64 are outside the 0.2.0 release contract.

## Quick start

```python
import asyncio
from aifluxon import Agent, ControlledProvider

async def main():
    agent = Agent(provider=ControlledProvider(["Hello from AIFLUXON."]))
    result = await (await agent.start("hello")).result()
    print(result.text)

asyncio.run(main())
```

`ControlledProvider` is offline. It does not use the network.

## Documentation

- [Quick Start](quickstart.md)
- [API Reference](api-reference.md)
- [Providers](providers.md)
- [Sessions](sessions.md)
- [Events](events.md)
- [Tools and policy](tools-and-policy.md)
- [Errors](errors.md)
- [Codex OAuth](auth.md)
- [Thinking](thinking.md)
- [Architecture](architecture.md)

## Providers

Included:

- OpenAI
- DeepSeek API
- Qwen
- Kimi
- Gemini
- Codex (static API key or Codex OAuth via `CodexAuth`)
- Custom OpenAI-compatible endpoints
- `ControlledProvider` (offline)

ChatGPT Web and DeepSeek Web are **not** part of this SDK. There is no `ChatGptWebProvider` or `DeepSeekWebProvider` constructor.

## Out of scope

The Python package does not include a desktop UI, host-specific tools, or product-specific permission profiles.

## Stability

| Surface | Status |
| ------- | ------ |
| `aifluxon.__all__` | Experimental public API |
| `_native` | Internal |
