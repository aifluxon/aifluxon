# Quick Start

This page documents APIs that exist in 0.1.1.

## Install

```powershell
pip install aifluxon
```

Requires Windows 10/11 x86_64 and CPython 3.11–3.14. See [README](README.md) to build from source with `maturin develop`.

## 1. Import and run a prompt

```python
import asyncio
from aifluxon import Agent, ControlledProvider

async def main():
    agent = Agent(provider=ControlledProvider(["Hello from AIFLUXON."]))
    run = await agent.start("hello")
    result = await run.result()
    print(result.text)
    print(result.run_id)

asyncio.run(main())
```

`result()` waits for the run to finish. It does not reconstruct the answer by summing token events in Python.

## 2. Persistent session

```python
from aifluxon import Agent, ControlledProvider, JsonFileSessionStore

agent = Agent(
    provider=ControlledProvider(["first", "second"]),
    store=JsonFileSessionStore("./aifluxon-data"),
)
session = await agent.open_or_create_session("physics-project")
await session.run("first")
await session.run("continue")
```

A session is not a run. The same session produces a new `RunId` each time.

## 3. Listen to events

```python
from aifluxon import Agent, ControlledProvider, TextDelta

run = await agent.start("hello")
async for event in run.events():
    if isinstance(event, TextDelta):
        print(event.delta, end="")
result = await run.result()
```

Dropping the iterator does not cancel the run. Call `await run.cancel()` to cancel.

## 4. Register a Python tool

```python
from aifluxon import Agent, ControlledProvider, ToolEffect, tool

@tool(description="Double a number.", effect=ToolEffect.PURE_READ, parallel_safe=True)
def double(value: float) -> float:
    return value * 2
```

Decorated callables still go through registry, validation, policy, operations, the tool ledger, budget, and cancellation.

## 5. Handle approval

```python
from aifluxon import OperationRequested, RequireApprovalPolicy

agent = Agent(provider=provider, tools=[lookup], policy=RequireApprovalPolicy())
run = await agent.start("task")
async for event in run.events():
    if isinstance(event, OperationRequested):
        await run.resolve_operation(event.operation_id, "approve")
```

Deferred commit uses `await run.commit_operation(operation_id)` instead of flattening the decision to a boolean.

## 6. Cancel

```python
await run.cancel()
```

## 7. Codex OAuth

```python
from aifluxon import Agent, CodexAuth

auth = CodexAuth()
login = await auth.login()
print(login.authorization_url)
account = await login.wait()
agent = Agent(auth.provider("gpt-5.6-codex", account_id=account.id))
```

Python never receives access tokens. See [Codex OAuth](auth.md) and `bindings/python/examples/codex_oauth.py`.

## 8. Thinking / reasoning effort

```python
from aifluxon import Agent, OpenAI, DeepSeek, Qwen

agent = Agent(OpenAI("gpt-5.4", api_key="..."), reasoning_effort="high")
await agent.run("Inspect this project")
await agent.run("Quick check", reasoning_effort="low")

deepseek = Agent(
    DeepSeek("deepseek-v4-flash", api_key="..."),
    thinking=True,
    reasoning_effort="high",
)
qwen = Agent(Qwen("qwen-plus", api_key="..."), thinking=True, thinking_budget=8192)
```

Kimi enables thinking from the model name. See [Thinking](thinking.md) and `bindings/python/examples/thinking.py`.

## 9. System prompt

AIFLUXON does not ship a product persona. Pass your own host instructions with `system_prompt=`.

```python
from aifluxon import Agent, OpenAI

agent = Agent(
    OpenAI("gpt-5.4", api_key="..."),
    system_prompt="You are a concise laboratory reviewer. Reply in Chinese.",
)
await agent.run("Summarize the method.")
await agent.run("Now translate.", system_prompt="You are a translator.")
```

On a session, a new `system_prompt` replaces the leading system message instead of stacking another copy. See `bindings/python/examples/system_prompt.py`.

## 10. Chat Completions vs Responses

Every public constructor except `ControlledProvider` takes `api_mode="chat_completions"` or `api_mode="responses"`. `"chat"` is an alias for Chat Completions. `None` uses the family default (Codex defaults to Responses; others default to Chat Completions).

```python
from aifluxon import DeepSeek, OpenAI

OpenAI("gpt-5.4", api_key="...", api_mode="responses")
DeepSeek("deepseek-v4-pro", api_key="...", api_mode="responses")
DeepSeek("deepseek-v4-flash", api_key="...", api_mode="chat_completions")
```

If the model does not support the requested mode, AIFLUXON falls back to the supported protocol. V4 Flash and V4 Pro both support Responses.

Runnable copies of these examples live in `bindings/python/examples/`.
