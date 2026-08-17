# Events

```python
run = await session.start("task")
async for event in run.events():
    ...
```

Events are bridged from the Rust run stream through a bounded queue (capacity 128). Python does not install a per-token GIL callback.

## Ordering

* Every event has `sequence: int` and `run_id: str`.
* Sequence is monotonic for a run.
* Exactly one terminal event: `Completed`, `Failed`, or `Cancelled`.

## Slow consumers

The native forwarder applies backpressure on the bounded channel. A slow iterator delays event delivery; it does not reset sequence numbers.

## Consumer drop

Closing or dropping `run.events()` does **not** cancel the run. `await run.result()` still completes. Canonical cancel is `await run.cancel()`.

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
