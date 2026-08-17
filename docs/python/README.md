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

```powershell
pip install aifluxon
```

| Platform | Status |
| -------- | ------ |
| Windows 10/11 x86_64 | Supported |
| Windows ARM64 | Not supported |
| Linux | Not supported in 0.1.1 |
| macOS | Not supported in 0.1.1 |

Requires **CPython 3.11–3.14**. Wheels are ABI-specific (`cp3xx-win_amd64`), not a single abi3 wheel.

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
