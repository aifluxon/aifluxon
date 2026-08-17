# AIFLUXON Boundary Baseline (A0)

This document characterizes the boundary that exists at A0 and the ownership constraints that all later phases must preserve. It is descriptive, not a public API design commitment.

## Target dependency direction

```text
                 aifluxon-core
                 /           \
                v             v
      aifluxon-runtime    aifluxon-providers
                \             /
                 v           v
                  aifluxon-api
                       ^
                +------+------+
                |             |
             EasyPhy       Python
```

At A0, `aifluxon-api` and the Python binding do not yet exist. They must not be created before their scheduled phases.

## Current ownership snapshot

| Concern | A0 owner | Boundary requirement |
|---|---|---|
| Public legacy DTOs and provider contracts | `aifluxon-core` | A1 introduces canonical product-neutral types; A3 returns EasyPhy wire DTOs to the Host |
| Budget, tool ledger, pending store, terminal guard, context helpers | `aifluxon-runtime` | These become Kernel-owned without Tauri or product dependencies |
| Protocol helpers and provider placeholders | `aifluxon-providers` | Provider wire conversion stays outside Runtime; only functional public providers survive |
| Canonical model/tool loop | Split across `aifluxon-runtime`, `src-tauri/src/agent_runtime`, and `src-tauri/src/agent_stream.rs` | Later strangler migration must end with one AIFLUXON lifecycle owner |
| Tauri IPC and `agent-runtime-event` compatibility | EasyPhy Host | Remains a Host adapter; React wire compatibility is preserved |
| Project, Quick Access, filesystem, shell, Git, settings, MATLAB | EasyPhy Host | Registered as capabilities/tools; never moved wholesale into AIFLUXON |
| ChatGPT Web and DeepSeek Web auth/session behavior | EasyPhy Host private providers | Remains private in the first public boundary |
| Product history, migrations, revision CAS, titles, workspace association | EasyPhy Host | `STALE_CHAT_SESSION_REVISION` remains fail-closed |
| Flux and Universe semantics | EasyPhy product/integration code | Strictly out of scope; only generic compatibility shims are allowed |

## Hard crate boundaries

| Crate | Allowed | Forbidden |
|---|---|---|
| `aifluxon-core` | serde, IDs, generic domain and extension contracts | Tauri, PyO3, reqwest, OS host dependencies, EasyPhy product types |
| `aifluxon-runtime` | core, async runtime, generic orchestration/policy/state | Tauri, PyO3, EasyPhy, provider wire JSON, server frameworks |
| `aifluxon-providers` | core and provider protocol/HTTP dependencies | Tauri, PyO3, EasyPhy product dependencies |
| future `aifluxon-api` | core/runtime/providers stable facade | Tauri, PyO3, HTTP server frameworks, Kernel internals in public surface |
| future Python binding | `aifluxon-api`, PyO3, async bridge | direct Runtime internals, EasyPhy, Tauri |
| EasyPhy Host | Tauri, OS integration, product policy/private providers | becoming a second canonical Agent runtime |

## Characterization matrix

The A0 audit found executable coverage for every required behavior. Existing tests are reused wherever they already prove the invariant. Two narrow tests were added for previously indirect Settings and MATLAB approval coverage.

| Required behavior | Executable characterization | Locked observation |
|---|---|---|
| Run cancellation | `agent_runtime/runtime.rs::bug_agent_007_provider_await_honors_runtime_cancel`, `bug_agent_008_tool_await_honors_runtime_cancel`; `agent.rs::full_task_cancellation_flags_runs_and_releases_every_approval_waiter` | Cancellation exits provider/tool waits and releases approval waiters |
| Model budget | `aifluxon-runtime/coordinator.rs::bug_agent_014_budget_is_not_reset_after_provider_retry` | Model-round budget is monotonic and rejects calls beyond the limit |
| Tool budget | `aifluxon-runtime/budget.rs::bug_agent_012_budget_is_monotonic_across_retries` | Tool invocation budget stops at its exact limit and cannot be reset by retry |
| Tool at-most-once | `tools/executor.rs::bug_agent_013_provider_retry_reuses_cached_tool_side_effect`; `coordinator.rs::bug_agent_032_web_protocol_ledger_does_not_reexecute_runtime_tools` | Replayed invocation IDs reuse recorded results instead of executing again |
| Pending shell | `pending.rs::bug_agent_005_pending_operation_resolves_once`; `agent_runtime/pending.rs::bug_agent_003_pending_operation_survives_without_a_live_listener` | Shell approval persists for late observation and resolves once |
| Pending settings | `pending.rs::settings_approval_resolves_once_with_the_bound_resolution` | Ordinary settings approval preserves its bound resolution and resolves once |
| Pending MATLAB | `pending.rs::matlab_approval_resolves_once_with_the_bound_decision` | MATLAB approval preserves its decision and resolves once |
| Pending patch | `agent_runtime/pending.rs::bug_agent_001_pending_patch_is_awaiting_operation_not_terminal`; `pending.rs::bug_agent_002_reject_patch_keeps_the_prepared_effect_for_continuation` | Patch remains pending rather than terminal and apply/reject state is one-shot |
| Terminal exactly once | `terminal.rs::bug_agent_006_terminal_signal_is_emitted_once` | First terminal outcome is absorbing across completion, failure, cancellation, and budget exhaustion |
| Session/event isolation | `agent_runtime/registry.rs::cancel_one_run_does_not_flag_another_session`; `agent_events.rs::runtime_events_route_by_typed_run_id_instead_of_prefixed_names`; `agentRuntimeClient.test.ts` BUG-UI-002 suite | Cancellation and event routing do not cross run/session boundaries |
| Tool result pair integrity | `agent_stream.rs::persisted_responses_turn_requires_unique_paired_function_outputs`; `context.rs::bug_agent_028_context_pruning_keeps_tool_call_and_result_together` | Missing/duplicate outputs fail closed and pruning keeps call/result pairs together |
| Context pruning | `context.rs::bug_agent_028_context_pruning_keeps_tool_call_and_result_together`; `bug_agent_027_dynamic_runtime_metadata_stays_out_of_stable_prefix` | Pruning preserves protocol pairs and dynamic metadata stays outside the stable prefix |
| Provider continuation preservation | `context.rs::bug_agent_029_opaque_continuation_metadata_round_trips`; provider session isolation tests in `aifluxon-providers` and EasyPhy private providers | Continuation fields round-trip opaquely and provider sessions stay scoped |
| History stale revision fail-closed | `agent_history.rs::stale_session_revision_is_rejected_without_overwriting_new_messages` | A stale save returns `STALE_CHAT_SESSION_REVISION` and leaves newer messages intact |

## Runtime invariants frozen by A0

1. One invocation ID cannot produce the same side effect twice.
2. One run cannot emit or enter more than one terminal outcome.
3. Pending approval blocks the corresponding effect and remains reconstructable for a late subscriber.
4. Cancellation propagates across provider wait, tool wait, approval wait, and Host-owned process termination paths.
5. Model rounds and tool invocations are independently bounded and monotonic.
6. Provider continuation is opaque to generic Runtime code.
7. Provider wire JSON is provider-owned; generic Runtime migration must move toward canonical messages.
8. EasyPhy authority checks, approval correctness, and history CAS cannot be weakened for extraction.

## Migration constraints

- Characterize, introduce an abstraction, adapt the existing implementation, switch one caller, test, and only then remove a zero-caller legacy path.
- Do not rewrite `src-tauri/src/agent_stream.rs` as one operation.
- Do not create a second Runtime for Tauri, Python, or a future network adapter.
- Do not add EasyPhy, Flux, Universe, MATLAB, Quick Access, project-path, Tauri, or window objects to stable AIFLUXON APIs.
- Keep Flux and Universe semantics untouched. Their only allowed changes are EasyPhy-private compatibility mappings to generic AIFLUXON IDs, descriptors, executors, or policy contracts.
- Keep the existing frontend `agent-runtime-event` wire until the scheduled EasyPhy event adapter phase proves compatibility.
