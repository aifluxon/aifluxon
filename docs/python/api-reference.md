# API Reference

Inventory of the public Python surface (`aifluxon.__all__`). Internal `_native` helpers are omitted.

| Symbol | Kind | Status |
|---|---|---|
| `Agent` | class | Experimental |
| `Run` | class | Experimental |
| `RunResult` | class | Experimental |
| `Session` | class | Experimental |
| `SessionStore` | class | Experimental |
| `InMemorySessionStore` | class | Experimental |
| `JsonFileSessionStore` | class | Experimental |
| `ProviderConfig` | class | Experimental |
| `OpenAI` | constructor | Experimental |
| `DeepSeek` | constructor | Experimental |
| `Qwen` | constructor | Experimental |
| `Kimi` | constructor | Experimental |
| `Gemini` | constructor | Experimental |
| `Codex` | constructor | Experimental |
| `Custom` | constructor | Experimental |
| `ControlledProvider` | constructor | Experimental |
| `tool` | decorator | Experimental |
| `ToolEffect` | enum | Experimental |
| `AllowAllPolicy` | class | Experimental |
| `RequireApprovalPolicy` | class | Experimental |
| `Event` and event types | types | Experimental |
| Exceptions | classes | Experimental |

Import path for every symbol below is `aifluxon` unless noted.

## `Agent`

```python
class Agent:
    def __init__(
        self,
        provider: ProviderConfig,
        *,
        store: SessionStore | None = None,
        tools: Sequence[Callable] | None = None,
        policy: Any | None = None,
        max_model_rounds: int = 32,
        max_tool_invocations: int = 64,
    ) -> None: ...
```

Creates an agent around one AIFLUXON backend. The default `store` is `InMemorySessionStore` (no disk writes). `max_model_rounds` is a hard run budget and is not overridden by provider continuation hints.

### Properties

* `provider_id: str`
* `model: str`

### Methods

#### `await Agent.start(prompt: str, *, session_id: str | None = None) -> Run`

Starts one run. If `session_id` is set, stored messages and provider state for that session are restored first.

#### `await Agent.run(prompt: str, *, session_id: str | None = None) -> RunResult`

`start` plus `result()`.

#### `await Agent.create_session() -> Session`

Creates a new UUID session.

#### `await Agent.open_session(session_id: str) -> Session`

Opens an existing session. Raises `InvalidRequestError` if missing.

#### `await Agent.open_or_create_session(session_id: str) -> Session`

UUID strings are parsed as `SessionId`. Any other non-empty key is mapped with a stable hash. Never derived from `RunId`.

#### `await Agent.list_sessions() -> list[dict]`

#### `await Agent.delete_session(session_id: str) -> None`

### Raises

* `InvalidConfigurationError`
* `InvalidRequestError`
* `ProviderError`

### Example

```python
from aifluxon import Agent, ControlledProvider
agent = Agent(provider=ControlledProvider(["ok"]))
result = await agent.run("hello")
```

Related: `Run`, `Session`, `JsonFileSessionStore`.

## `Run`

One execution. `Run.id` is a `RunId` string, distinct from `Session.id`.

```python
class Run:
    id: str
    session_id: str | None
    async def events(self) -> AsyncIterator[Event]: ...
    async def result(self) -> RunResult: ...
    async def cancel(self) -> None: ...
    async def snapshot(self) -> dict: ...
    async def resolve_operation(self, operation_id: str, decision: str = "approve", *, data=None, reason: str | None = None) -> None: ...
    async def commit_operation(self, operation_id: str) -> None: ...
```

* `events()` is a bounded async iterator over Runtime events. Sequence is monotonic. Terminal events occur once.
* Dropping the iterator does **not** cancel the run.
* `result()` waits for the run to finish. It does not sum token deltas.
* `resolve_operation` is for blocking approval. `commit_operation` is for deferred prepared-effect commit.
* `cancel()` is the supported way to cancel a run.

### Raises

* `CancelledError`
* `FailedError`
* `StateConflictError`

## `RunResult`

```python
class RunResult:
    run_id: str
    session_id: str | None
    state: str
    text: str
    output: list
    usage: Any
```

`text` is the last assistant message from Runtime `Completed` output.

## `Session`

```python
class Session:
    id: str
    revision: int | None
    async def start(self, prompt: str) -> Run: ...
    async def run(self, prompt: str) -> RunResult: ...
```

A session can produce many runs. See [sessions.md](sessions.md).

## `SessionStore`

Marker base class. `path` is `None` for memory stores.

## `InMemorySessionStore`

Default. Lost when the process exits. No implicit disk writes.

## `JsonFileSessionStore`

```python
JsonFileSessionStore(path: str)
```

Standalone persistence. Credentials are never written. See [sessions.md](sessions.md).

## Provider constructors

All return `ProviderConfig`.

### `ProviderConfig`

Frozen constructor result. `to_spec()` is used by the native bridge.

### `OpenAI(model, *, api_key, base_url=None, api_mode=None)`

Default base URL: `https://api.openai.com/v1`.

### `DeepSeek(model, *, api_key, base_url=None, api_mode=None)`

Default base URL: `https://api.deepseek.com`.

### `Qwen(model, *, api_key, base_url=None, api_mode=None)`

Default base URL: `https://dashscope.aliyuncs.com/compatible-mode/v1`.

### `Kimi(model, *, api_key, base_url=None, api_mode=None)`

Default base URL: `https://api.moonshot.cn/v1`.

### `Gemini(model, *, api_key, base_url=None, api_mode=None)`

Default base URL: `https://generativelanguage.googleapis.com/v1beta/openai`.

### `Codex(model, *, api_key, base_url=None, api_mode=None)`

Default API mode: `responses`. Default base URL: `https://api.openai.com/v1`.

### `Custom(model, *, base_url, api_key="", provider_id="custom", api_mode=None)`

`base_url` is required.

### `ControlledProvider(responses=None, *, turns=None, provider_id="controlled", model="controlled-model", delay_ms=None)`

Offline deterministic provider. Use `responses=["text"]` or `turns=[{"tool": ...}, {"text": ...}]`.

ChatGPT Web and DeepSeek Web are not Python constructors.

## `tool` and `ToolEffect`

```python
@tool(description="...", effect=ToolEffect.PURE_READ, parallel_safe=True)
def name(arg: str) -> str: ...
```

`ToolEffect` values: `PURE_READ`, `FS_READ`, `FS_WRITE`, `PROCESS_SPAWN`, `PROCESS_CONTROL`, `NETWORK`, `SETTINGS_WRITE`, `EXTERNAL_SIDE_EFFECT`, `UNKNOWN`.

Unsupported annotations raise `TypeError`. Effect is never inferred from the function name.

## `AllowAllPolicy`

Allows every registered tool.

## `RequireApprovalPolicy`

```python
RequireApprovalPolicy(mode="blocking_approval", effects=None)
```

`mode` is `blocking_approval` or `deferred_commit`. Optional `effects` limits which tool effects require a host decision.

## Events

`Event` is the union of:

* `RunStarted`
* `StateChanged`
* `TextDelta`
* `ReasoningDelta`
* `ToolStarted`
* `ToolFinished`
* `OperationRequested` (`operation_id`, `mode`)
* `UsageUpdated`
* `ArtifactProduced`
* `Completed`
* `Failed`
* `Cancelled`

Every event has `sequence: int`, `run_id: str`, and `type: str`. See [events.md](events.md).

## Exceptions

* `AifluxonError` — base
* `InvalidConfigurationError`
* `InvalidRequestError`
* `ProviderError`
* `ToolError`
* `PolicyError`
* `CancelledError`
* `BudgetExceededError`
* `StateConflictError`
* `FailedError`
* `InternalError`

See [errors.md](errors.md).
