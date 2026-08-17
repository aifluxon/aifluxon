# Quick Start

This page documents APIs that exist in 0.1.0.

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

Runnable copies of these examples live in `bindings/python/examples/`.
