# Events

```python
run = await session.start("task")
async for event in run.events():
    ...
```

Events are bridged from the Rust stream by a background task. Python does not install a per-token GIL callback. Python buffers unread events in memory without blocking result completion.

## Ordering

* Every event has `sequence: int` and `run_id: str`.
* Sequence is monotonic for a run.
* Exactly one terminal event: `Completed`, `Failed`, or `Cancelled`.

## Slow consumers

The native forwarder retains its bounded channel. Python continuously drains it, so a slow or absent Python iterator does not stop the run. Unread events consume memory until consumed or the Run is released. Consume events during long runs to avoid retaining the full unread stream.

Only one iterator may consume a Run at a time; a second concurrent iterator raises `RuntimeError`. After exhaustion, another `events()` call finishes immediately. Events may be consumed after `result()` completes.

## Consumer drop

Closing or dropping `run.events()` does **not** cancel the run. `await run.result()` still completes. Cancel with `await run.cancel()`.

## Public event types

| Type | Class | Fields |
|---|---|---|
| `run_started` | `RunStarted` | `session_id`, `parent_run_id` |
| `state_changed` | `StateChanged` | `state` |
| `text_delta` | `TextDelta` | `delta` |
| `reasoning_delta` | `ReasoningDelta` | `delta` |
| `tool_started` | `ToolStarted` | `invocation_id`, `name` |
| `tool_finished` | `ToolFinished` | `invocation_id`, `name`, `result` |
| `operation_requested` | `OperationRequested` | `operation`, `operation_id`, `mode` |
| `usage_updated` | `UsageUpdated` | `usage` |
| `artifact_produced` | `ArtifactProduced` | `artifact` |
| `completed` | `Completed` | `output` |
| `failed` | `Failed` | `message` |
| `cancelled` | `Cancelled` | |

`Event` is the union of those classes.
