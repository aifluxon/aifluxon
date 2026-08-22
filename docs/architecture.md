# Architecture

AIFLUXON is an embedded agent backend. A host process constructs it, registers providers and tools, and drives runs in-process.

```text
Host (Rust or Python)
  → aifluxon-api
      ├── aifluxon-auth          (Codex OAuth, secret stores)
      └── aifluxon-runtime
            └── aifluxon-providers / host-registered tools
```

Python uses the same path through PyO3. There is no separate Python runtime, model loop, budget, or tool ledger. The runtime crate does not depend on `aifluxon-auth`.

## Crates

| Crate | Role |
| ----- | ---- |
| `aifluxon-core` | IDs, messages, errors, and shared contracts |
| `aifluxon-runtime` | Run lifecycle, budget, tool ledger, pending operations, events |
| `aifluxon-providers` | Public model providers and HTTP transport |
| `aifluxon-auth` | Codex OAuth, secret stores, credential sources |
| `aifluxon-api` | Facade used by hosts |
| `bindings/python` | PyO3 package `aifluxon`; depends only on `aifluxon-api` |

The Python package is not a Cargo workspace member. Build it with maturin.

## What the backend owns

- Run start, continuation, cancellation, and a single terminal outcome
- Ordered events for a run
- Tool registration, validation, policy, operations, and at-most-once execution
- Session records and opaque provider state when a session store is configured

## What the host owns

- Product UI and whether to open a browser
- Secret-store namespace (`AIFLUXON`, `EasyPhy Studio`, …)
- Host-specific tools and permission products
- Any private model integrations not shipped in this repository

Codex OAuth (PKCE, callback, exchange, refresh, persistence) lives in `aifluxon-auth`, not in the host.

ChatGPT Web and DeepSeek Web are not public constructors in the Python SDK.

## Image inputs

The canonical `ContentPart::Image(ImageContent)` boundary carries provider-ready image
references. A host must resolve a local file before starting a run and pass one of:

- a supported base64 data URL;
- a public HTTP(S) URL; or
- a provider Files API id such as `file-api-...`.

OpenAI-compatible Chat Completions and Responses requests serialize these references into
their protocol-specific content blocks. Responses tool results retain image content instead
of reducing the output to text.

For DeepSeek, image input is enabled only for `deepseek-v4-flash-vision-exp`. The provider
validates the documented message-role, MIME, image-count, URL-length, inline-size, and
request-body limits before network I/O. DeepSeek Web remains a separate text-only private
integration.

## Identities

`SessionId`, `RunId`, and `ProviderSessionKey` are distinct. A session can produce many runs. Provider continuation state is scoped to `(SessionId, ProviderId)`.
