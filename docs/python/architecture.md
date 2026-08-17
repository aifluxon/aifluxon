# Architecture

Python is the second Host of the canonical AIFLUXON embedded backend. It is not EasyPhy Remote Control.

```text
Python
  → PyO3 (`bindings/python`)
  → aifluxon-api
  → aifluxon-runtime
  → aifluxon-providers / Host-registered tools
```

EasyPhy uses the same facade:

```text
EasyPhy Host
  → aifluxon-api
  → same Runtime
```

## Dependency boundary

`bindings/python` may depend on:

* `aifluxon-api`
* PyO3 / tokio / serde_json

It must not depend on:

* `aifluxon-core`
* `aifluxon-runtime`
* `aifluxon-providers`
* EasyPhy / `src-tauri`
* Tauri

If Python needs a capability the facade lacks, add a seam to `aifluxon-api`.

## Internal concepts (not Python public API)

These exist in Runtime and may appear in architecture discussions, but they are **Internal** and are not Python constructors:

* `ToolLedger`
* `AgentCoordinator`
* `RunTable`
* `TerminalGuard`
* `PendingOperationStore`

## Invariants preserved across the Python host

* `RunId` ≠ `SessionId` ≠ `ProviderSessionKey`
* Continuation does not reset budget, session, or ledger
* Exactly one terminal event
* Event sequence is monotonic
* Tool side effects are at-most-once
* Cancellation is explicit
