# AIFLUXON

Experimental in-process agent backend. Applications embed AIFLUXON in their own process. It is not a remote control service and it does not ship a product UI.

```text
Your application (Rust or Python)
  → aifluxon-api
  → runtime
  → providers and tools
```

The SDK is experimental. Public Python names are listed in `aifluxon.__all__`.

## Install (Python)

```powershell
pip install aifluxon
```

| Requirement | Support in 0.1.0 |
| ----------- | ---------------- |
| Windows 10 / 11, x86_64 | Supported |
| Windows ARM64 | Not supported |
| Linux | Not supported |
| macOS | Not supported |
| CPython 3.11, 3.12, 3.13, 3.14 | Supported (`win_amd64` wheels) |
| CPython 3.10, PyPy, other ABIs | Not supported |

Published wheels do not require a Rust toolchain. Each Python minor version uses its own wheel; this is not an abi3 package.

License: [Apache License 2.0](LICENSE).

## Features

- Embedded agent runtime with a stable Rust facade (`aifluxon-api`)
- Python package `aifluxon` over the same facade
- Providers: OpenAI, DeepSeek, Qwen, Kimi, Gemini, Codex, and custom OpenAI-compatible endpoints
- Tools, a generic approval policy, operations, budgets, and cancellation
- Persistent sessions and an ordered event stream

ChatGPT Web and DeepSeek Web are not part of the public Python SDK.

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

## Rust

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

The Rust crates are consumed as a Git dependency. They are not published to crates.io in 0.1.0.

```toml
aifluxon-api = { git = "https://github.com/aifluxon/aifluxon", rev = "<commit>" }
```

## Documentation

- [Python SDK](docs/python/README.md)
- [Python API reference](docs/python/api-reference.md)
- [Architecture](docs/architecture.md)
- [Providers](docs/python/providers.md)
- [Tools and policy](docs/python/tools-and-policy.md)

Examples: [`bindings/python/examples`](bindings/python/examples).

## Build from source

Root `Cargo.lock` is tracked so release wheels rebuild against the same Rust dependency graph.

```powershell
cargo test --workspace
cd bindings/python
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install maturin pytest pytest-asyncio
maturin develop
pytest
```

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Apache License 2.0. See [LICENSE](LICENSE).
