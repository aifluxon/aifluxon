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
| `ImageInput` | class | Experimental |
| `PromptInput` / `PromptPart` | type aliases | Experimental |
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
        system_prompt: str | None = None,
        reasoning_effort: str | None = None,
        thinking: bool | str | None = None,
        thinking_budget: int | str | None = None,
        options: RunOptions | None = None,
        web_tools: DeepSeekWebTools | None = None,
    ) -> None: ...
```

Creates an agent around one AIFLUXON backend. The default `store` is `InMemorySessionStore` (no disk writes). `max_model_rounds` is a hard run budget and is not overridden by provider continuation hints.

`system_prompt` is stored on the Agent and prepended as a `role=system` message on every run. AIFLUXON does not inject an EasyPhy (or any other product) persona. Thinking options are forwarded separately. See [Thinking](thinking.md).

### Properties

* `provider_id: str`
* `model: str`
* `system_prompt: str | None`
* `thinking_settings: ThinkingSettings`
* `reasoning_effort: str | None`
* `thinking: str | None` (`enabled` / `disabled` / `default`)
* `thinking_budget: str | None`

### Methods

#### `await Agent.start(prompt: PromptInput, *, session_id: str | None = None, system_prompt=..., reasoning_effort=..., thinking=..., thinking_budget=...) -> Run`

Starts one run. If `session_id` is set, stored messages and provider state for that session are restored first. Per-run `system_prompt` and thinking arguments override Agent defaults; pass `None` to omit a system message on that request (an existing leading system message already stored on the session is kept unless a new `system_prompt` replaces it).

#### `await Agent.run(prompt: PromptInput, *, session_id: str | None = None, system_prompt=..., reasoning_effort=..., thinking=..., thinking_budget=...) -> RunResult`

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
agent = Agent(
    provider=ControlledProvider(["ok"]),
    system_prompt="You are a focused laboratory reviewer.",
)
result = await agent.run("hello")
```

Related: `Run`, `Session`, `JsonFileSessionStore`, `ThinkingSettings`.

## `ImageInput`, `PromptPart`, and `PromptInput`

```python
PromptPart = str | ImageInput
PromptInput = str | Sequence[PromptPart]

ImageInput(reference: str, mime_type: str)
ImageInput.from_url(url: str, mime_type: str)
ImageInput.from_file_id(file_id: str, mime_type: str)
ImageInput.from_bytes(data: bytes, mime_type: str)
ImageInput.from_file(path, mime_type: str | None = None)
```

Plain string prompts remain fully compatible. A sequence preserves the order of text and image parts. URLs must be absolute HTTP(S) URLs. `from_bytes` and `from_file` create base64 data URLs; local filesystem paths never cross the provider boundary. `from_file` infers the MIME type from the extension unless one is supplied.

Image MIME support, reference formats, model eligibility, image counts, and request-size limits remain provider-owned and are checked before network I/O. For DeepSeek, use `deepseek-v4-flash-vision-exp`; supported formats are JPEG, PNG, GIF, and WebP. It accepts public URLs, base64 data URLs, and Files API IDs.

Python tools may return an `ImageInput` or an ordered sequence containing strings and `ImageInput` values. The Runtime stores and replays the complete multimodal tool result without re-executing an already recorded tool call. DeepSeek accepts image tool outputs in Responses mode.

## `ThinkingSettings`

Frozen Host thinking options forwarded on each run.

```python
ThinkingSettings(reasoning_effort=None, thinking_mode=None, thinking_budget=None)
```

Built by `Agent(...)`. `to_payload()` is the JSON the native layer sends as `ProviderFeatureRequest`. See [Thinking](thinking.md).

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

* `events()` is a lossless async iterator over Runtime events. Python buffers unread events in memory so waiting for `result()` never depends on consuming events. Only one iterator may consume a Run at a time. After exhaustion, subsequent calls finish immediately. Sequence is monotonic and terminal events occur once.
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
    async def start(self, prompt: PromptInput, *, system_prompt=..., reasoning_effort=..., thinking=..., thinking_budget=...) -> Run: ...
    async def run(self, prompt: PromptInput, *, system_prompt=..., reasoning_effort=..., thinking=..., thinking_budget=...) -> RunResult: ...
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

Default base URL: `https://api.openai.com/v1`. `api_mode` is `"chat_completions"` (also `"chat"`) or `"responses"`. `None` uses the family default.

### `DeepSeek(model, *, api_key, base_url=None, api_mode=None)`

Default base URL: `https://api.deepseek.com`. Default API mode: `chat_completions`. `deepseek-v4-flash`, `deepseek-v4-pro`, and `deepseek-v4-flash-vision-exp` also accept `api_mode="responses"`. Python and Rust hosts can pass canonical image content to `deepseek-v4-flash-vision-exp`.

### `Qwen(model, *, api_key, base_url=None, api_mode=None)`

Default base URL: `https://dashscope.aliyuncs.com/compatible-mode/v1`.

### `Kimi(model, *, api_key, base_url=None, api_mode=None)`

Default base URL: `https://api.moonshot.cn/v1`.

### `Gemini(model, *, api_key, base_url=None, api_mode=None)`

Default base URL: `https://generativelanguage.googleapis.com/v1beta/openai`.

### `Codex(model, *, api_key, base_url=None, api_mode=None)`

Default API mode: `responses`. Default base URL: `https://api.openai.com/v1`.

### `Custom(model, *, base_url, api_key="", provider_id="custom", api_mode=None, allow_cumulative_delta=None)`

`base_url` is required.

`allow_cumulative_delta` overrides Rust's custom-provider default (`True`). Set `False` for endpoints that send ordinary incremental deltas which may legitimately repeat earlier text.

### `ControlledProvider(responses=None, *, turns=None, provider_id="controlled", model="controlled-model", delay_ms=None)`

Offline deterministic provider. Use `responses=["text"]` or `turns=[{"tool": ...}, {"text": ...}]`.

ChatGPT Web and DeepSeek Web are not Python constructors.

## Codex OAuth

See [auth.md](../auth/codex-oauth.md) and [docs/python/auth.md](auth.md).

### `CodexAuth`

```python
CodexAuth(secret_store=None)
```

Default store: `SystemKeyringStore(service_name="AIFLUXON")`.

Methods: `login()`, `login_with_browser()`, `accounts()`, `status(account_id)`, `refresh(account_id)`, `logout(account_id)`, `provider(model, account_id=None)`.

### `CodexLoginAttempt`

`authorization_url`, `wait()`, `cancel()`.

### `CodexAccount`

Frozen dataclass: `id`, `email`, `expires_at`. No tokens.

### `CodexAuthStatus`

Frozen dataclass: `account`, `state`.

### `CodexProvider`

Opaque OAuth provider handle. `to_spec()` contains only `kind`, `model`, and `account_id`.

### `SystemKeyringStore`

```python
SystemKeyringStore(service_name="AIFLUXON")
```

### `MemorySecretStore`

In-memory credentials for tests and temporary sessions.

### `EncryptedFileSecretStore`

```python
EncryptedFileSecretStore(path)
await store.unlock(password)
await store.lock()
```

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
* `AuthenticationRequiredError`
* `AccountSelectionRequiredError`
* `AccountNotFoundError`
* `CallbackTimeoutError`
* `TokenRefreshError`
* `CredentialStoreUnavailableError`
* `CredentialStoreLockedError`
* `CredentialCorruptedError`

See [errors.md](errors.md).

## `RunOptions`, `RunLimits`, `ProviderFeatures`, and `ExecutionAuthority`

```python
RunOptions(
    limits=RunLimits(max_model_rounds=32, max_tool_invocations=64),
    features=ProviderFeatures(
        web_search=False, image_generation=False,
        prompt_cache_key=None, explicit_cache=False,
    ),
    allowed_tools=None, authority=None, session_key=None,
)
ExecutionAuthority(
    capabilities=frozenset(), permission_mode="default", filesystem_scope="isolated",
)
```

`Agent(options=...)` sets defaults, exposed through `Agent.options`. `Agent.start/run`, `Session.start/run` and message methods accept `options=...`. Per-run options replace all Agent options; use `dataclasses.replace` for a partial override. Existing thinking and system-prompt arguments remain independent. Legacy constructor budgets remain supported but non-default legacy budgets cannot be combined with `options`.

`RunLimits.unlimited()` matches Rust's unlimited mode, disabling loop, continuation and wall-time budgets. `is_unlimited` reports it. Finite limits must be integers in `0..2**32-1`; zero prevents the corresponding work. Cancellation, policies, provider failures and normal completion still apply.

`allowed_tools=None` permits the registered set; an empty tuple disables registered tools. Non-empty collections select exact names for provider advertisement and execution. Unknown names fail before launch. Hosted provider tools are controlled separately through feature flags.

Required capabilities on Python tools must be granted through `ExecutionAuthority` before the Python tool policy runs. A missing grant produces a denied tool result without executing the function. Permission modes are `default`, `managed`, `trusted`; filesystem scopes are `workspace`, `isolated`, `scoped_selection`, `trusted`. These fields describe host authority; they do not create an operating-system sandbox or grant file access by themselves.

`session_key` selects the provider session/cache identity; it does not identify or create a persisted Session. Feature support remains provider-owned. See [Run options and Rust parity](run-options.md).

## `Message` and `ToolCall`

```python
Message(role, content="", tool_calls=(), tool_call_id=None, provider_state=None)
ToolCall(name, arguments={}, id=<new UUID>, provider_call_id=None)
```

`role` is `system`, `user`, `assistant`, or `tool`. Content accepts text, ordered text/image parts, or an empty sequence for tool-call-only assistant messages. `to_dict()` and `Message.from_dict()` use canonical Rust content: `{"text": "hello"}` or `{"image": {"artifact": "...", "mime_type": "image/png"}}`.

`await Agent.start_messages(messages, **run_keywords)` / `await Agent.run_messages(messages, **run_keywords)` accept a non-empty sequence of `Message` objects or canonical dictionaries from `RunResult.output`. Keywords match `Agent.start/run`, including `options`. The corresponding Session methods bind `session_id` automatically.

An explicit leading system message takes precedence over the Agent default. An explicit non-empty `system_prompt` replaces leading system messages. Stored session history is still prepended: pass only new messages when using a Session, or omit `session_id` when replaying a full transcript.

Tool invocation `id` and `tool_call_id` are canonical UUID strings. Keep upstream IDs in `provider_call_id`. The bridge infers missing tool-message `provider_state.call_id` from a preceding assistant call. Explicit provider state, including reasoning signatures, is preserved.

## `SessionRecord`

`await Agent.get_session(session_id)` returns a frozen `SessionRecord` with `id`, `revision`, `created_at`, `updated_at`, `messages: tuple[Message, ...]`, and computed `message_count`. Timestamps are Unix milliseconds. `await Session.refresh()` returns the latest record and updates `Session.revision`; `await Session.history()` refreshes and returns messages; `await Session.delete()` removes the session.

## `ProviderCapabilities`

`Agent.capabilities` exposes all eight Rust declarations: `supports_remote_conversation`, `supports_prompt_cache_key`, `supports_reasoning_stream`, `supports_parallel_tool_calls`, `supports_hosted_tools`, `supports_images`, `supports_web_search`, and `supports_session_restore`. These are provider-level declarations, not an upstream model catalog or a model-name gate.

## `DeepSeekWebTools`

```python
DeepSeekWebTools(api_key, base_url="https://api.deepseek.com")
Agent(provider, web_tools=DeepSeekWebTools(...))
```

Registers Rust's `web_search` and `web_fetch` with any Agent provider. Both have `ToolEffect.NETWORK` and follow normal allowlist, approval, validation and replay behavior. This is independent of hosted `ProviderFeatures.web_search`. API keys are excluded from provider and web-tool configuration reprs.

## Rich tool schemas and `ToolResult`

`Agent.register_tool(fn)` registers a decorated tool for subsequent runs; `Agent.list_tools()` returns canonical descriptors. Duplicate names raise `InvalidConfigurationError`.

Schema inference supports scalar types, `Any`, `None`, nested lists and string-keyed dictionaries, nullable/general unions, `Literal`, JSON-valued `Enum`, `TypedDict` with `Required`/`NotRequired`, and `Annotated` descriptions/schema metadata. Enum arguments arrive as their JSON scalar values. Positional-only and variadic parameters are rejected because tools are called by keyword.

`@tool(input_schema={...})` supplies an explicit JSON Schema. `Annotated` metadata may be a description string or schema dictionary. Async tools run on the event loop that started the run, allowing reuse of caller-owned async clients. One Agent cannot execute concurrently on different event loops.

`ToolResult(value, content=None)` separates the JSON event/replay value from optional non-empty text/image content sent to the model. Existing plain Python and `ImageInput` returns remain supported. Required capability grants are checked before both `AllowAllPolicy` and custom Python policies.
