# Sessions

A **session** is a logical conversation. A **run** is one execution.

```text
SessionId  ≠  RunId  ≠  ProviderSessionKey  ≠  ToolInvocationId  ≠  OperationId
```

* One session can produce many runs.
* `ProviderSessionKey` is session-scoped and is never a `RunId`.
* Provider opaque state is isolated by `(SessionId, ProviderId)`.

## Identities

| Identity | Meaning |
|---|---|
| `SessionId` | Stable conversation |
| `RunId` | One `Agent.start` / `Session.run` |
| `ProviderSessionKey` | Provider continuation / cache identity |

## APIs

```python
session = await agent.create_session()
session = await agent.open_session(session_id)
session = await agent.open_or_create_session("physics-project")
await agent.list_sessions()
await agent.delete_session(session_id)

await session.start("prompt")  # returns Run
await session.run("prompt")    # returns RunResult
```

`system_prompt` on `Agent` or `Session.start` / `Session.run` is prepended as a leading system message. A later non-empty `system_prompt` on the same session replaces that leading system message; it is not stacked.

Non-UUID keys passed to `open_or_create_session` are mapped with a stable hash. UUID strings are parsed as-is.

## `InMemorySessionStore`

Default. No disk I/O. Lost on process exit.

## `JsonFileSessionStore`

```python
from aifluxon import JsonFileSessionStore
store = JsonFileSessionStore("./aifluxon-data")
```

Layout:

```text
<root>/sessions/index.json
<root>/sessions/records/<session-uuid>.json
<root>/sessions/store.lock
<root>/provider-state/<session-uuid>--<provider-id>.json
<root>/provider-state/store.lock
```

Behavior:

* schema version 1
* per-session JSON records are the conversation authority; the index is derived
* temp write, flush, `sync_all`, atomic replace
* revision CAS; stale writes raise `StateConflictError` (not last-write-wins)
* corrupt records are quarantined and never returned as valid sessions
* OS file lock serializes mutations
* process restart restores messages and provider state from the same root
* API keys, OAuth tokens, and cookies are not session fields and are not written
