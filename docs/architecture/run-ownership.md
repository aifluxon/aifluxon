# Canonical Run Ownership (A6)

## Identity contract

- `SessionId` is the stable logical conversation identity.
- `RunId` identifies one execution and is never reused as provider continuation identity.
- `ProviderSessionKey` is derived from the logical session (or supplied explicitly by the host), not from `RunId`.
- `ToolInvocationId` and `OperationId` remain independent at-most-once identities.
- A standalone temporary run may have no `SessionId`.

## Canonical owner

`aifluxon_runtime::RunTable` is the run-correctness owner. Each registered run owns:

- `RunContext` and state;
- cancellation token and host cancellation hooks;
- caller-provided model/tool limits and their monotonic counters;
- a typed `ToolInvocationId` ledger;
- pending generic operations;
- the per-run event sequence and canonical terminal event.

The table accepts exact `RunId` lookup only. Prefix/fuzzy lookup remains an EasyPhy compatibility concern and is not part of the public AIFLUXON contract.

## State and event invariants

- Non-terminal state transitions are limited to `Running` and `AwaitingOperation`.
- `Completed`, `Failed`, and `Cancelled` are terminal.
- A run can append one terminal event only; no later event is accepted.
- Event sequence starts at 1 for `RunStarted` and increases monotonically per run.
- A normal completion is rejected while a generic operation is non-terminal.
- Cancellation first marks canonical state, wakes the cancellation token and operation waiters, then invokes each registered host cleanup hook once.
- Host hook failure is reported but cannot reopen the run.

## EasyPhy host boundary

EasyPhy keeps only product resource bindings keyed by exact `RunId`:

- product parent metadata, including the private Flux mapping;
- the existing cache/session owner string used by current provider and process adapters;
- legacy event-prefix lookup;
- current output capture and `AtomicBool` compatibility.

The old host `AgentRunTable` no longer owns lifecycle state. EasyPhy normal Chat, Flux child runs, and direct tool runs register in the canonical table. Existing `AgentChatState.running_sessions` remains a product busy-session/process-owner registry during the compatibility period; it is not a run state machine.

## Deferred migrations

- EasyPhy pending-operation product payloads still use the A5 adapter while the canonical table owns the generic operation lifecycle. A10 will finish the command/event adapter consolidation.
- Provider protocol orchestration remains in the existing Host path until the A7 preservation harness and provider migration gates pass.
- Flux cancellation discovery stays Host-only; discovered children are cancelled through canonical `RunId` state.
