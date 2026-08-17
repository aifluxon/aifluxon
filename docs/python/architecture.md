# Architecture

AIFLUXON is an in-process backend. Python and Rust applications call the same facade.

```text
Python
  → PyO3 (`bindings/python`)
  → aifluxon-api
  → aifluxon-runtime
  → aifluxon-providers / host-registered tools
```

## Binding boundary

`bindings/python` may depend on:

- `aifluxon-api`
- PyO3 / tokio / serde_json

It must not depend on:

- `aifluxon-core`
- `aifluxon-runtime`
- `aifluxon-providers`

If Python needs a capability the facade lacks, add it to `aifluxon-api` first.

## Internal types (not Python constructors)

These exist in the runtime and may appear in architecture notes, but they are not part of the Python public API:

- `ToolLedger`
- `AgentCoordinator`
- `RunTable`
- `TerminalGuard`
- `PendingOperationStore`

## Identities and invariants

- `RunId` ≠ `SessionId` ≠ `ProviderSessionKey`
- Continuation does not reset budget, session, or the tool ledger
- Exactly one terminal event per run
- Event sequence is monotonic
- Tool side effects run at most once per invocation id
- Cancellation is explicit (`await run.cancel()`)
