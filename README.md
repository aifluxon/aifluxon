# AIFLUXON

AIFLUXON is an **experimental, embedded, headless Agent backend**. Hosts embed it in-process. It is not a remote-control API and it does not own product UI.

```text
Host (Rust or Python)
  → aifluxon-api
  → aifluxon-runtime
  → aifluxon-providers / Host-registered tools
```

## Install (Python)

```powershell
pip install aifluxon
```

This is the production install path once `aifluxon` 0.1.0 is on PyPI. It requires **Windows 10/11 x86_64** and a matching **CPython** wheel. No Rust toolchain is required to install the published wheel.

| Platform             | Status                 |
| -------------------- | ---------------------- |
| Windows 10/11 x86_64 | Supported              |
| Windows ARM64        | Not supported          |
| Linux                | Not supported in 0.1.0 |
| macOS                | Not supported in 0.1.0 |

| Python | Status |
| ------ | ------ |
| CPython 3.11, 3.12, 3.13, 3.14 | Supported via version-specific `win_amd64` wheels |
| CPython 3.10 | Not supported |
| PyPy / other ABIs | Not supported |

The SDK is **experimental**. Public names live in `aifluxon.__all__`.

## What it provides

- **Rust-first** crates: `aifluxon-core`, `aifluxon-runtime`, `aifluxon-providers`, `aifluxon-api`
- **Python SDK** (`aifluxon`) as a second Host over the same facade
- **Provider registry** for OpenAI, DeepSeek, Qwen, Kimi, Gemini, Codex, and Custom OpenAI-compatible endpoints
- **Tool runtime** with descriptors, generic `ToolPolicy`, operations, ledger, budget, and cancellation
- **Sessions** distinct from runs (`SessionId` ≠ `RunId` ≠ `ProviderSessionKey`)
- **Events** as a monotonic stream with a single terminal

ChatGPT Web and DeepSeek Web are **not** part of the public Python SDK.

## Python quickstart

```python
import asyncio
from aifluxon import Agent, ControlledProvider

async def main():
    agent = Agent(provider=ControlledProvider(["Hello from AIFLUXON."]))
    print((await agent.run("hello")).text)

asyncio.run(main())
```

`ControlledProvider` is offline. It does not use the network or API keys.

## Rust quickstart

```rust
use std::sync::Arc;
use aifluxon_api::{
    user_prompt_request, AllowAllToolPolicy, Aifluxon, ControlledProvider, NoopRunEventSink,
    ProviderId, ProviderRegistry, RunLimits, ToolRegistry,
};

# async fn demo() -> Result<(), Box<dyn std::error::Error>> {
let registry = ProviderRegistry::new();
registry.register(
    ProviderId::new("controlled"),
    ControlledProvider::from_text_responses("controlled", ["hello"]),
)?;
let backend = Aifluxon::builder()
    .provider_registry(registry)
    .tool_registry(ToolRegistry::new())
    .tool_policy(Arc::new(AllowAllToolPolicy))
    .event_sink(Arc::new(NoopRunEventSink))
    .build()?;
let mut run = backend
    .start(user_prompt_request(
        ProviderId::new("controlled"),
        "controlled-model",
        "hi",
        None,
        RunLimits::default(),
    ))
    .await?;
assert_eq!(run.result().await?.text, "hello");
# Ok(())
# }
```

The Rust crates are embedded by path/git revision. They are not published to crates.io in 0.1.0.

## Documentation

- [Python SDK](docs/python/README.md)
- [Python API reference](docs/python/api-reference.md)
- [Architecture (Python host)](docs/python/architecture.md)
- [Provider extension](docs/python/providers.md)
- [Tool extension](docs/python/tools-and-policy.md)
- [Runtime architecture](docs/architecture/aifluxon-boundary.md)

Runnable examples: [`bindings/python/examples`](bindings/python/examples).

## Development

Root `Cargo.lock` is tracked so Python release wheels rebuild against the same Rust dependency graph.

```powershell
cargo test --workspace
cd bindings/python
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install maturin pytest pytest-asyncio
maturin develop
pytest
```

## License

Apache License 2.0. See [LICENSE](LICENSE).
