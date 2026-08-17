# AIFLUXON

[English](README.md) | [简体中文](README.zh-CN.md)

**AIFLUXON is an embeddable agent runtime for building AI applications, coding agents, CLI agents, desktop agents, and custom agent hosts.**

It provides one canonical runtime for model execution, streaming, tools, approvals, sessions, cancellation, budgets, and provider state. Applications embed AIFLUXON and keep their own UI, credentials, product policy, and host-specific integrations.

```text
                         Your Host
          ┌───────────────┼────────────────┐
          │               │                │
       Rust app        Python app      CLI / Desktop
          │               │                │
          └───────────────┬┴────────────────┘
                          ▼
                    aifluxon-api
                          │
                          ▼
                       Runtime
                    ┌─────┴─────┐
                    ▼           ▼
                Providers      Tools
```

## What you can build

- Coding agents
- CLI agents and terminal assistants
- Desktop AI applications
- Autonomous task runners
- Tool-using assistants
- Custom agent harnesses
- Python agent applications
- Rust-native embedded agents
- Custom hosts with their own UI and permission model

A future HTTP, WebSocket, or service adapter can be layered above the same API without changing the runtime model.

## Core capabilities

- **Embedded runtime** — runs in the host process; no daemon required
- **Streaming execution** — ordered text, reasoning, tool, usage, and terminal events
- **Provider abstraction** — OpenAI, DeepSeek, Qwen, Kimi, Gemini, Codex, and custom OpenAI-compatible endpoints
- **Tool runtime** — registration, schema validation, policy, approvals, deferred operations, and at-most-once execution
- **Persistent sessions** — session history and opaque provider state are distinct from individual runs
- **Cancellation and budgets** — run-level cancellation, model-round limits, and tool-invocation limits
- **Continuation semantics** — provider-specific protocol details are translated into generic runtime continuation
- **Host extensibility** — products can register their own providers, tools, policy, storage, and presentation layer
- **Rust API** — stable embedding facade through `aifluxon-api`
- **Python SDK** — PyO3 binding over the same canonical runtime

## Architecture

AIFLUXON deliberately separates the runtime from the product host.

```text
Host
  ├─ UI / terminal presentation
  ├─ credentials and OAuth
  ├─ product-specific permissions
  ├─ host-specific tools
  └─ private integrations
          │
          ▼
    aifluxon-api
          │
          ▼
    aifluxon-runtime
      ├─ run lifecycle
      ├─ event ordering
      ├─ continuation
      ├─ cancellation
      ├─ budgets
      ├─ tool ledger
      └─ operations
          │
      ┌───┴────┐
      ▼        ▼
 providers    tools
```

`SessionId`, `RunId`, and `ProviderSessionKey` are separate identities. A session can produce many runs, while provider continuation state remains scoped to the logical session and provider.

See [Architecture](docs/architecture.md) for details.

## Use from Rust

AIFLUXON is Rust-first. Hosts embed the runtime through `aifluxon-api`.

```rust
use std::sync::Arc;
use aifluxon_api::{
    user_prompt_request, AllowAllToolPolicy, Aifluxon, ControlledProvider,
    NoopRunEventSink, ProviderId, ProviderRegistry, RunLimits, ToolRegistry,
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

The Rust crates are consumed as a Git dependency in `0.1.0`; they are not published to crates.io yet.

```toml
aifluxon-api = { git = "https://github.com/aifluxon/aifluxon", rev = "<commit>" }
```

Hosts that implement custom providers or lower-level extension contracts may also use the corresponding crates from the same pinned revision.

## Use from Python

The Python SDK is an official binding over the same `aifluxon-api` runtime.

```powershell
pip install aifluxon
```

```python
import asyncio
from aifluxon import Agent, ControlledProvider

async def main():
    agent = Agent(provider=ControlledProvider(["Hello from AIFLUXON."]))
    result = await agent.run("hello")
    print(result.text)

asyncio.run(main())
```

`ControlledProvider` is offline and useful for tests and examples. Public network providers are configured with their own credentials in memory.

### Python distribution support in 0.1.0

| Requirement | Support |
| --- | --- |
| Windows 10 / 11, x86_64 | Supported |
| CPython 3.11, 3.12, 3.13, 3.14 | Supported (`win_amd64` wheels) |
| Windows ARM64 | Not supported |
| Linux | Not supported by the published Python package |
| macOS | Not supported by the published Python package |
| CPython 3.10 / PyPy / other ABIs | Not supported |

Published Python wheels do not require a Rust toolchain. Each supported CPython minor version currently uses its own wheel; the package is not `abi3` in `0.1.0`.

The **Python wheel support matrix does not define the architecture boundary of AIFLUXON itself**: the runtime remains an embeddable Rust backend, while the current packaged Python distribution is intentionally Windows-first.

## Providers

Public provider families currently include:

- OpenAI
- DeepSeek API
- Qwen
- Kimi
- Gemini
- Codex
- Custom OpenAI-compatible endpoints
- `ControlledProvider` for deterministic offline tests

See [Provider documentation](docs/python/providers.md) for the current Python constructors and behavior.

## Events and streaming

Runs expose an ordered event stream. Public event categories include:

- run lifecycle
- text deltas
- reasoning deltas
- tool start / finish
- pending operations and approvals
- usage updates
- artifacts
- completed / failed / cancelled terminal events

Event sequence numbers are monotonic per run, and each run has exactly one terminal outcome.

Python example:

```python
run = await session.start("task")
async for event in run.events():
    ...
```

See [Events](docs/python/events.md).

## Tools and policy

AIFLUXON tools run through the canonical tool path:

```text
Tool descriptor
    ↓
Validation
    ↓
ToolPolicy
    ↓
Operation / approval when required
    ↓
Executor
    ↓
Tool ledger
    ↓
Budget + cancellation
```

The runtime does not impose a product-specific permission model. Hosts provide the policy appropriate for their application.

See [Tools and policy](docs/python/tools-and-policy.md).

## Documentation

### Core

- [Architecture](docs/architecture.md)
- [Contributing](CONTRIBUTING.md)
- [Changelog](CHANGELOG.md)

### Python SDK

- [Python SDK overview](docs/python/README.md)
- [Quickstart](docs/python/quickstart.md)
- [API reference](docs/python/api-reference.md)
- [Providers](docs/python/providers.md)
- [Sessions](docs/python/sessions.md)
- [Events](docs/python/events.md)
- [Tools and policy](docs/python/tools-and-policy.md)
- [Errors](docs/python/errors.md)
- [Python architecture](docs/python/architecture.md)

Examples: [`bindings/python/examples`](bindings/python/examples).

## Repository layout

```text
crates/
  aifluxon-core/       domain contracts and shared types
  aifluxon-runtime/    run lifecycle, events, tools, operations
  aifluxon-providers/  provider protocols and HTTP transport
  aifluxon-api/        stable facade for hosts

bindings/python/       PyO3 package `aifluxon`
docs/                  architecture and SDK documentation
examples/              repository-level examples
```

The Python binding depends on `aifluxon-api`; it does not implement a second runtime, model loop, budget, or tool ledger.

## Build from source

```powershell
cargo test --workspace

cd bindings/python
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install maturin pytest pytest-asyncio
maturin develop
pytest
```

The root `Cargo.lock` is tracked so release wheels rebuild against the same Rust dependency graph.

## Status

AIFLUXON `0.1.x` is **experimental**. Public APIs may still evolve before a stable release.

## License

Apache License 2.0. See [LICENSE](LICENSE).
