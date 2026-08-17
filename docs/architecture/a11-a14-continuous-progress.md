# A11–A14 Continuous Progress

Source of truth after A13.2: standalone tree `E:\aifluxon`. EasyPhy Host worktree: `E:\easyphy-worktrees\fluxon-agent-core`, branch `refactor/fluxon-agent-core-linux`.

## A11.1 Minimal Python Binding — PASS

Code changes:
- `aifluxon-api`: `RunResult`, `RunHandle::result()`, `ControlledProvider`, `register_provider_from_json`, event JSON, session CRUD, prompt helper
- `bindings/python`: PyO3 host depending only on `aifluxon-api`

Public API added: `Agent`, `Run`, `RunResult`, provider constructors, exceptions, `ControlledProvider`

Python docs: `docs/python/*` created; `bindings/python/examples/quickstart.py`

Gate: `import aifluxon`, offline smoke, docs consistency tests PASS

### Python API Documentation Delta

Public API added: Agent, Run, RunResult, exceptions, provider constructors, ControlledProvider
Public API changed: none
Public API removed: none
Documentation updated: README, quickstart, api-reference, errors, architecture
Examples updated: quickstart.py
Documentation consistency: PASS

## A11.2 Persistent Python Sessions — PASS

Code changes:
- `JsonFileProviderStateStore` paired with `JsonFileSessionStore`
- Python `Session`, `JsonFileSessionStore`, `open_or_create_session`

Gate: process restart restore, CAS, quarantine, provider-state isolation, no secret persistence PASS

### Python API Documentation Delta

Public API added: Session, SessionStore, InMemorySessionStore, JsonFileSessionStore
Documentation updated: sessions.md, api-reference, quickstart
Examples updated: persistent_session.py
Documentation consistency: PASS

## A12.1 Python Async Events — PASS

Code changes: bounded event pump on `Run.events()`; consumer drop does not cancel

Public API added: Event types (`RunStarted`, `TextDelta`, …)

Documentation updated: events.md
Examples updated: events.py
Documentation consistency: PASS

## A12.2 Python Tools + Generic ToolPolicy — PASS

Code changes:
- Coordinator deferred-commit wait/commit path
- `Aifluxon::commit_prepared_operation`
- `@tool`, `AllowAllPolicy`, `RequireApprovalPolicy`

Documentation updated: tools-and-policy.md
Examples updated: tools.py, approval.py
Documentation consistency: PASS

## A13.1 Public Surface Cleanup — PASS

- Public always-error `ChatGptWebProvider` / `DeepSeekWebProvider` removed from `aifluxon-providers` crate root
- Capability functions remain for EasyPhy private adapters
- `ProviderKind` already absent
- Python `__all__` has no native helpers
- Docs do not advertise Web providers as SDK APIs

## A13.2 Independent Repository Local Cutover — PASS

License: Apache License 2.0 (`SPDX-License-Identifier: Apache-2.0`).

Created `E:\aifluxon` (did not previously exist). Copied AIFLUXON-owned crates, Python bindings, Python API docs, and AIFLUXON architecture docs. Did not copy EasyPhy Host, Tauri, Flux, Universe, rg/MarkItDown, or private Web protocol implementations (`src-tauri/src/chatgpt_web.rs`, `deepseek_web.rs`).

Capability stubs `chatgpt_web.rs` / `deepseek_web.rs` in `aifluxon-providers` remain crate-private: they expose `chatgpt_web_capabilities()` / `deepseek_web_capabilities()` and always-error `next_turn`. They are not public constructors and are not the EasyPhy Web protocol.

`LICENSE` at repo root and `bindings/python/LICENSE` are byte-identical Apache-2.0 text. Rust workspace `license = "Apache-2.0"`; Python `license = "Apache-2.0"` + `license-files = ["LICENSE"]`. No NOTICE (no vendored third-party attribution required).

Standalone Rust: `cargo fmt --all -- --check`, `cargo test --workspace` (18+46+68+35), `cargo check --workspace` PASS.

Standalone Python: `maturin develop`, `import aifluxon`, pytest 10 passed, six offline examples PASS.

### Python API Documentation Delta

Public API added: none
Public API changed: none
Public API removed: none
Documentation updated: moved to `E:\aifluxon\docs\python`; README notes experimental + Windows x86_64 first
Examples updated: copied with the SDK
Documentation consistency: PASS

## A13.3 EasyPhy Dependency Cutover — PASS

```text
EasyPhy src-tauri
  → E:/aifluxon/crates/aifluxon-api
  → E:/aifluxon/crates/aifluxon-core
  → E:/aifluxon/crates/aifluxon-runtime
  → E:/aifluxon/crates/aifluxon-providers
```

AIFLUXON has no EasyPhy path. EasyPhy workspace members no longer include AIFLUXON crates.

Direct core/runtime/providers deps remain as Host extension contracts (private `ModelProvider` impls, provider family construction, `RunTable` / skills / tool registry). `aifluxon-api` is the product-facing facade; collapsing the other three would invent the wrong abstraction.

After EasyPhy compile + tests + frontend/package gates: deleted in-tree `crates/aifluxon-*` and `bindings/python`. EasyPhy `docs/python` is a pointer only.

Private Web providers and Host tools remain EasyPhy-owned.

## A14.1 Transport-neutral Audit — PASS (sibling replay)

In `E:\aifluxon`:

- `start` / `start_on` / `cancel` / `snapshot` / `resolve_operation` / `commit_prepared_operation` / `result` / events / session CRUD on `aifluxon-api`
- no Tauri, Axum, Warp, Actix, HTTP/WebSocket server
- `reqwest` client only in providers
- Python depends only on `aifluxon-api`
- no `easyphy-worktrees` / `easyphy-studio` / `src-tauri` in AIFLUXON Rust/Python sources

## A14.2 Final Ownership + Python API Audit — PASS (sibling replay)

Ownership as designed. Python `__all__` (44 names) documented in `E:\aifluxon\docs\python/api-reference.md`. Default/Managed/Trusted are not Python policy.

## A14.3 Final Regression + Documentation Gate — PASS

Standalone `E:\aifluxon`: fmt, test workspace, check workspace, maturin, import, pytest, examples.

EasyPhy: `git diff --check`, `cargo fmt --all -- --check`, `cargo test -p easyphy-studio` (736 passed), `cargo check --workspace`, `npm test`, `npm run build:frontend`, `npm run verify:package-inputs`.

Remote publication: EXTERNAL ACTION DEFERRED.
