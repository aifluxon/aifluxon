# Run options and Rust parity

The Python SDK calls `aifluxon-api` in process and uses the existing Rust lifecycle.

```python
from dataclasses import replace
from aifluxon import Agent, ControlledProvider, ProviderFeatures, RunLimits, RunOptions

defaults = RunOptions(
    limits=RunLimits.unlimited(),
    features=ProviderFeatures(prompt_cache_key="physics-project"),
)
agent = Agent(ControlledProvider(["ok"]), options=defaults)
result = await agent.run("hello", options=replace(defaults, allowed_tools=()))
```

A per-run options object replaces all defaults. `dataclasses.replace` preserves settings when changing selected fields. Thinking and system-prompt overrides retain their existing semantics.

## Application surface

| Rust facade | Python surface |
| --- | --- |
| `RunRequest.messages` | `Message`, `ToolCall`, `start_messages`, `run_messages` |
| `RunRequest.session_id` | `session_id=` or `Session` |
| `RunRequest.model` | Provider constructor's model and provider ID |
| `RunRequest.session_key` | `RunOptions.session_key` |
| `RunRequest.allowed_tools` | `RunOptions.allowed_tools`; enforced during execution |
| `RunRequest.limits` | `RunLimits`, including `unlimited()` |
| `RunRequest.features` | `ProviderFeatures` plus existing thinking arguments |
| `RunRequest.authority` | `ExecutionAuthority` |
| Session CRUD and records | Agent session methods, `Session.refresh/history/delete`, `SessionRecord` |
| `ModelProvider.capabilities()` | `Agent.capabilities` |
| `ToolRegistry` / `ToolResult` | `@tool`, `register_tool`, `list_tools`, `ToolResult` |
| `register_deepseek_web_tools` | `DeepSeekWebTools` |
| Run handles, snapshots and operations | `Run`, typed events, result, snapshot, cancel, resolve/commit |
| Authentication and credential stores | `CodexAuth` and secret-store classes |

The Agent's provider remains fixed after construction. Rust host embedding interfaces such as custom `ModelProvider`, `Workspace`, `RunTable`, checkpoint stores, custom store traits and `start_on` are not Python extension protocols. `start_on` needs a Host-owned shared run table that this SDK does not expose. ChatGPT Web and DeepSeek Web remain outside the facade's JSON provider factory and have no Python constructor; these differ from DeepSeek's standalone search/fetch tools.

## Provider features

`ProviderFeatures` forwards the Rust fields without substituting protocols or model aliases. Hosted search and image generation are consumed by supporting Responses providers. `explicit_cache` enables Qwen cache marking under Rust's minimum-length/message-position rules. `prompt_cache_key` is forwarded to request bodies. A flag does not guarantee endpoint/model support.

For standalone DeepSeek tools:

```python
from aifluxon import Agent, DeepSeek, DeepSeekWebTools, RequireApprovalPolicy, ToolEffect

agent = Agent(
    DeepSeek("your-model", api_key="..."),
    web_tools=DeepSeekWebTools(api_key="..."),
    policy=RequireApprovalPolicy(effects=frozenset({ToolEffect.NETWORK})),
)
```

Use `start()` and consume `OperationRequested` to resolve approvals; `run()` alone cannot answer an approval request. Tools use the configured DeepSeek URL/key. Rust owns validation, response checks, retries, error redaction and deduplication.

## Messages and history

```python
from aifluxon import Message

result = await agent.run_messages([
    Message("system", "Answer as a laboratory assistant."),
    Message("user", "What is inertia?"),
])
history = [Message.from_dict(item) for item in result.output]
# Replay complete history without session_id to avoid appending it twice.
# next_result = await agent.run_messages([*history, Message("user", "Give an example.")])
```

Canonical tool IDs remain UUIDs; upstream IDs and opaque reasoning/signature fields are preserved separately. Session histories include text, images, tool calls/results, and provider state. Credentials placed in messages or tool outputs are conversation data and may be persisted.

Executable offline examples: [advanced_run.py](../../bindings/python/examples/advanced_run.py) and [structured_tools.py](../../bindings/python/examples/structured_tools.py).
