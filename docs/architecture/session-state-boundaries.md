# Session and Provider State Boundaries (A9)

## Identities

- `SessionId` is the stable logical conversation identity.
- `RunId` identifies one execution. Multiple runs in one session receive distinct run IDs.
- Provider continuation keys are session-scoped and never derived from a run ID.
- Opaque provider state is keyed by the pair `(SessionId, ProviderId)`, so changing either key cannot reuse another provider's continuation state.

## Product-neutral state

AIFLUXON separates three records:

- `SessionRecord` stores canonical messages plus generic metadata and a CAS revision.
- `ProviderStateRecord` stores an uninterpreted JSON value for one session/provider pair.
- `RunCheckpoint` stores only the serializable run state needed by a host to inspect or resume a run. Tokio channels, cancellation internals, operation waiters, and the tool ledger are not serialized.

The default builder uses the three in-memory stores and performs no hidden filesystem writes. Standalone hosts may explicitly opt into `JsonFileSessionStore(root)`. Its versioned layout is:

```text
<root>/sessions/index.json
<root>/sessions/records/<session-uuid>.json
<root>/sessions/store.lock
```

Each mutation is serialized with an OS file lock, checks the caller's revision before writing, flushes and synchronizes a same-directory temporary file, and atomically replaces the destination. Record filenames come only from typed UUID session IDs. Invalid or mismatched records are quarantined rather than accepted. The index is derived from the per-session records and is never the conversation authority.

## EasyPhy product history

EasyPhy Product History remains the only EasyPhy conversation source of truth. It continues to own titles, timeline entries, tool events, patches, workspaces, Quick Access context, migrations, corruption handling, and `STALE_CHAT_SESSION_REVISION` CAS behavior.

The current EasyPhy execution path continues to pass its already-authorized product messages explicitly. It does not configure or write an AIFLUXON JSON session store, so `data/history/sessions/*.json` is not dual-written into the standalone layout. The stable EasyPhy chat ID is adapted to `SessionId` only for run/provider identity.

When the thin-host facade replaces the remaining compatibility path, its history adapter must choose one of these contracts, never both:

1. implement `SessionStore` over Product History; or
2. continue passing complete canonical messages while disabling generic conversation persistence for that host.

## Private Web provider compatibility

Provider state is deliberately opaque to Core, Runtime, stores, and public `RunRequest`/`SessionRecord` fields. Vendor fields such as `conversation_id`, `parent_message`, `context_signature`, `web_session_id`, `conduit`, `chat_session_id`, and `persistence_generation` remain private provider-adapter data.

The existing private-provider paths remain authoritative during thin-host migration:

- ChatGPT Web keeps its current process-local continuation cache, keyed by product scope and logical session, with strict history/config affinity checks. It has no existing disk continuation contract; process loss safely rebuilds from Product History rather than reusing an unverifiable remote cursor.
- DeepSeek Web keeps its existing versioned `DeepSeekWebPersistentBinding`, legacy-compatible deserialization, generation fencing, history-anchor validation, memory-cache-loss recovery, and deletion/tombstone behavior.

No old provider-session file or deserializer is removed. A later EasyPhy provider adapter may round-trip these values through `ProviderStateStore`, but it must first read the existing representation and emit only an opaque JSON value. The generic store must never interpret or rename vendor fields.

## Failure contract

- A stale session revision returns `StoreError::Conflict`; last-writer-wins is forbidden.
- A corrupt record is quarantined and is never returned as a valid session.
- Session persistence completes before the canonical run emits `Completed`. A persistence failure instead terminates the run as `Failed`.
- Cancellation and failure checkpoints retain the same session identity without turning a run ID into provider continuation state.
