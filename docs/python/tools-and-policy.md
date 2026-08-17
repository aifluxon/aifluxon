# Tools and policy

Python callables go through the same tool path as other embedders. They do not bypass validation, policy, operations, the tool ledger, budget, or cancellation.

```text
Python callable
  → descriptor
  → registry
  → validation
  → policy
  → operation
  → executor
  → ledger
  → budget
  → cancellation
```

## Define a tool

```python
from aifluxon import ToolEffect, tool

@tool(description="Look up a record.", effect=ToolEffect.PURE_READ, parallel_safe=True)
def lookup(query: str) -> str:
    return query
```

- `name` defaults to the function name
- `description` defaults to the docstring
- `effect` is required for a meaningful policy; the default is `UNKNOWN` and is never guessed from the name
- `parallel_safe` defaults to `False`
- sync and async functions are both accepted
- sync functions run on a worker thread so they do not block the Rust runtime
- async functions are scheduled on the running asyncio loop via `run_coroutine_threadsafe`

## Schema

JSON Schema is generated from type annotations. Supported: `str`, `int`, `float`, `bool`, `list[T]`, `dict[str, T]`, `T | None`. Unsupported annotations raise `TypeError`. There is no silent coercion.

## `ToolEffect`

`PURE_READ`, `FS_READ`, `FS_WRITE`, `PROCESS_SPAWN`, `PROCESS_CONTROL`, `NETWORK`, `SETTINGS_WRITE`, `EXTERNAL_SIDE_EFFECT`, `UNKNOWN`.

## Policy

- `AllowAllPolicy` — allow every registered tool
- `RequireApprovalPolicy(mode="blocking_approval"|"deferred_commit", effects=None)`

Custom objects may implement `evaluate(name, arguments, effect) -> dict`.

These are generic allow / approve policies. They are not a product permission profile.

## Approval and deferred commit

Blocking:

```python
await run.resolve_operation(event.operation_id, "approve")
await run.resolve_operation(event.operation_id, "reject", reason="no")
```

Deferred prepared effect:

```python
await run.commit_operation(event.operation_id)
```

`Approve` on a deferred operation is rejected (`StateConflictError`). Use commit for the prepared-effect path instead of flattening the decision to a boolean.

## Tool ledger

Identical `ToolInvocationId` values are executed at most once. Python tools cannot opt out.

## Exceptions

Python tool exceptions become tool failures. They must not panic Rust. Validation errors fail before the executor.
