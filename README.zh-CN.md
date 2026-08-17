# AIFLUXON

[English](README.md) | [简体中文](README.zh-CN.md)

**AIFLUXON 是一个可嵌入的 Agent Runtime，用于构建 AI 应用、Coding Agent、CLI Agent、桌面 Agent 以及自定义 Agent Host。**

它提供一套统一的 canonical runtime，负责模型执行、流式事件、工具、审批、会话、取消、预算以及 Provider 状态。应用程序将 AIFLUXON 嵌入自己的进程，并自行负责 UI、凭据、产品权限策略和 Host 特有集成。

```text
                         Your Host
          ┌───────────────┼────────────────┐
          │               │                │
       Rust App        Python App      CLI / Desktop
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

## 可以用它构建什么

- Coding Agent
- CLI Agent / 终端智能助手
- 桌面 AI 应用
- 自动任务执行器
- Tool-using Assistant
- 自定义 Agent Harness
- Python Agent 应用
- Rust 原生嵌入式 Agent
- 拥有独立 UI 与权限模型的自定义 Host

未来如果需要 HTTP、WebSocket 或服务端 Adapter，也可以构建在同一个 API 之上，而不需要改变 Runtime 的执行模型。

## 核心能力

- **嵌入式 Runtime** —— 直接运行在 Host 进程内，不要求额外 daemon
- **真正的流式执行** —— 有序输出文本、reasoning、tool、usage 与 terminal events
- **Provider 抽象** —— 支持 OpenAI、DeepSeek、Qwen、Kimi、Gemini、Codex 以及自定义 OpenAI-compatible endpoint
- **Tool Runtime** —— Tool 注册、Schema 校验、Policy、审批、Deferred Operation 与 at-most-once 执行
- **持久 Session** —— Session 历史与 Provider opaque state 与单次 Run 分离
- **取消与预算** —— Run cancellation、model round limit、tool invocation limit
- **Continuation 语义** —— Provider-specific protocol 被转换为 Runtime 可理解的通用 continuation
- **Host 扩展能力** —— 产品可以注册自己的 Provider、Tool、Policy、Storage 与 Presentation Layer
- **Rust API** —— 通过 `aifluxon-api` 提供稳定的嵌入接口
- **Python SDK** —— 基于 PyO3，对接同一套 canonical Runtime

## 架构

AIFLUXON 有意将 Agent Runtime 与产品 Host 分离。

```text
Host
  ├─ UI / Terminal Presentation
  ├─ Credentials / OAuth
  ├─ Product-specific Permissions
  ├─ Host-specific Tools
  └─ Private Integrations
          │
          ▼
    aifluxon-api
          │
          ▼
    aifluxon-runtime
      ├─ Run Lifecycle
      ├─ Event Ordering
      ├─ Continuation
      ├─ Cancellation
      ├─ Budgets
      ├─ Tool Ledger
      └─ Operations
          │
      ┌───┴────┐
      ▼        ▼
 Providers    Tools
```

`SessionId`、`RunId` 和 `ProviderSessionKey` 是不同的身份。一个 Session 可以产生多个 Run，而 Provider continuation state 仍然绑定到逻辑 Session 与对应 Provider。

详细说明见 [Architecture](docs/architecture.md)。

## 在 Rust 中使用

AIFLUXON 是 **Rust-first** 的。Host 通过 `aifluxon-api` 直接嵌入 Runtime。

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

在 `0.1.0` 中，Rust crates 目前通过 Git dependency 使用，暂未发布到 crates.io：

```toml
aifluxon-api = { git = "https://github.com/aifluxon/aifluxon", rev = "<commit>" }
```

如果 Host 需要实现自定义 Provider 或更底层的扩展 Contract，也可以从同一个固定 revision 使用对应 crates。

## 在 Python 中使用

Python SDK 是建立在同一个 `aifluxon-api` Runtime 之上的官方 Binding。

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

`ControlledProvider` 完全离线，适用于测试和示例。正式网络 Provider 的凭据只在内存中传递。

### 0.1.0 Python Distribution 支持范围

| 环境 | 支持状态 |
| --- | --- |
| Windows 10 / 11, x86_64 | 支持 |
| CPython 3.11、3.12、3.13、3.14 | 支持（`win_amd64` wheels） |
| Windows ARM64 | 不支持 |
| Linux | 当前发布的 Python package 不支持 |
| macOS | 当前发布的 Python package 不支持 |
| CPython 3.10 / PyPy / 其它 ABI | 不支持 |

已发布的 Python wheel 不要求用户安装 Rust toolchain。当前每个受支持的 CPython minor version 使用独立 wheel；`0.1.0` 还不是 `abi3` package。

需要特别区分：**Python wheel 的平台支持范围不等于 AIFLUXON 本身的架构边界。** AIFLUXON 仍然是可嵌入的 Rust Agent Backend，只是当前 Python 发行版有意采用 Windows-first 策略。

## Providers

当前 Public Provider family 包括：

- OpenAI
- DeepSeek API
- Qwen
- Kimi
- Gemini
- Codex
- 自定义 OpenAI-compatible endpoint
- 用于 deterministic offline tests 的 `ControlledProvider`

当前 Python Provider 构造与行为见 [Provider documentation](docs/python/providers.md)。

## Events 与 Streaming

每个 Run 都提供有序事件流。公开事件类别包括：

- Run lifecycle
- Text delta
- Reasoning delta
- Tool start / finish
- Pending operation / approval
- Usage update
- Artifact
- Completed / Failed / Cancelled terminal event

每个 Run 的 event sequence 单调递增，并且最终只有一个 terminal outcome。

Python 示例：

```python
run = await session.start("task")
async for event in run.events():
    ...
```

详细说明见 [Events](docs/python/events.md)。

## Tools 与 Policy

AIFLUXON Tool 统一通过 canonical tool path：

```text
Tool Descriptor
    ↓
Validation
    ↓
ToolPolicy
    ↓
Operation / Approval（如需要）
    ↓
Executor
    ↓
Tool Ledger
    ↓
Budget + Cancellation
```

Runtime 不强制任何产品专属权限模式。不同 Host 可以根据自己的产品需求提供对应 ToolPolicy。

详细说明见 [Tools and policy](docs/python/tools-and-policy.md)。

## 文档

### Core

- [Architecture](docs/architecture.md)
- [Contributing](CONTRIBUTING.md)
- [Changelog](CHANGELOG.md)

### Python SDK

- [Python SDK Overview](docs/python/README.md)
- [Quickstart](docs/python/quickstart.md)
- [API Reference](docs/python/api-reference.md)
- [Providers](docs/python/providers.md)
- [Sessions](docs/python/sessions.md)
- [Events](docs/python/events.md)
- [Tools and Policy](docs/python/tools-and-policy.md)
- [Errors](docs/python/errors.md)
- [Python Architecture](docs/python/architecture.md)

示例位于 [`bindings/python/examples`](bindings/python/examples)。

## 仓库结构

```text
crates/
  aifluxon-core/       Domain Contract 与共享类型
  aifluxon-runtime/    Run lifecycle、Events、Tools、Operations
  aifluxon-providers/  Provider Protocol 与 HTTP Transport
  aifluxon-api/        提供给 Host 的稳定 Facade

bindings/python/       PyO3 package `aifluxon`
docs/                  Architecture 与 SDK 文档
examples/              仓库级示例
```

Python Binding 只依赖 `aifluxon-api`；它不会建立第二套 Runtime、Model Loop、Budget 或 Tool Ledger。

## 从源码构建

```powershell
cargo test --workspace

cd bindings/python
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install maturin pytest pytest-asyncio
maturin develop
pytest
```

根目录 `Cargo.lock` 会被跟踪，以确保 release wheel 可以基于同一套 Rust dependency graph 重建。

## 当前状态

AIFLUXON `0.1.x` 当前处于 **Experimental** 阶段，Public API 在稳定版之前仍可能继续演进。

## License

Apache License 2.0。参见 [LICENSE](LICENSE)。
