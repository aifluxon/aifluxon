# Shared API reliability

OpenAI, DeepSeek, Qwen, Kimi, Gemini, Codex, and custom compatible providers share the same transport and validation path. Provider-specific request decoration and replay remain in their own modules. This does not add another API family or migrate native web sessions to stateless APIs.

- Chat Completions requires both a framed `[DONE]` and a successful finish reason. A token limit, filtering/error, malformed event, missing terminal, empty reply, incomplete/duplicate tool identity, or non-object JSON arguments cannot become successful tool execution. Buffered JSON is checked too. Refusals are preserved as text.
- Responses returns immediately at its explicit terminal, validates function calls before filtering, and rejects incomplete or failed payloads even when the event label says completed. Usage, canonical output, images, sparse output indices, and Codex continuation survive. Qwen summary and Kimi thinking adapters still run before final validation.
- Three attempts maximum for recoverable generation failures, only before emitted text/reasoning and without hosted operations. Local tools run after validation. Cancellation drops pending reads and backoff; each attempt has a 15-minute deadline, with the shared HTTP client's 30-second connect and 180-second idle-read limits. Permanent errors and output limits do not retry.
- Shared HTTP helpers use exponential backoff plus jitter, and Retry-After in seconds, milliseconds, or HTTP-date form. Server delays over 60 seconds return to the caller instead of retrying early. File uploads and hosted work use connection-setup-only retries; response failures on those operations are not replayed. OAuth refresh remains separately bounded to one refresh per attempt.
- Reused HTTP pools retain credentials on individual requests. SSE scanning remains incremental, supports split UTF-8/CRLF/comments, and retains trailing Chat usage before DONE. Completed responses do not wait for socket EOF. Retryable HTTP statuses and 401 do not wait for error-body EOF; other error previews are bounded to 4 KiB.

`PROVIDER_MODEL_ATTEMPT_FAILED` and `PROVIDER_HTTP_RETRY` log provider/context, attempt, and retry status without request bodies or credentials. Public response errors now use PROVIDER_* codes rather than DEEPSEEK_*.

## References

- [OpenAI streaming lifecycle](https://developers.openai.com/api/docs/guides/streaming-responses)
- [OpenAI Chat chunk schema](https://github.com/openai/openai-python/blob/main/src/openai/types/chat/chat_completion_chunk.py)
- [OpenAI SDK retry and Retry-After handling](https://github.com/openai/openai-python/blob/main/src/openai/_base_client.py)
- [Qwen OpenAI-compatible Chat contract](https://www.alibabacloud.com/help/en/model-studio/qwen-api-via-openai-chat-completions)
- [Kimi Chat and tool-call contract](https://platform.kimi.com/docs/api/chat)
- [Gemini OpenAI compatibility](https://ai.google.dev/gemini-api/docs/openai)
- [DeepSeek Harness reference and provider differences](deepseek-reliability.md)

## Validation

`cargo test --workspace --locked` and `cargo clippy --workspace --all-targets --locked -- -D warnings`.

The regression matrix covers 11 supported family/protocol routes: truncation recovery, invalid tool calls, bounded retries, and completion on an open socket. Further tests cover malformed events, output limits, visible-output replay suppression, cancellation, hosted operations, HTTP rate limiting, retry headers, and existing family replay/usage/artifact behavior. These are deterministic protocol/transport tests, not claims of live availability for every vendor.


## Structured failures and instance HTTP settings

`ProviderError::message(text)` remains an unclassified error and preserves its display text.
Use `ProviderError::new(ProviderErrorCode::..., text)` for machine-readable failures; message
prefixes do not trigger retries. Rust consumers matching `RunEvent::Failed` should include
`..` or bind its new optional `error` field; `finish_failed` continues to emit ordinary message-only
failures. HTTP failures carry `http_status` and `retry_after`. Redaction
changes only the message. Failed events include optional structured metadata, and
`RunHandle::result()` maps cancellation, rate limits, transient failures and exhausted budgets
into `AifluxonErrorKind`. `AifluxonError::provider_error()` exposes the original metadata.
Python failed events and `FailedError` expose the code, HTTP status and retry delay; the existing
`FailedError` catch behavior is preserved.

Each configured provider owns a reusable HTTP client. Set `OpenAiCompatibleConfig::http_client_tuning`
for proxy, HTTP/1 preference, connect/read timeouts and idle-pool limits. For custom TLS roots,
provide `ReqwestOpenAiTransport::with_client(client)` through `with_transport`. Keep authorization
out of client defaults: credentials belong to individual requests. Proxy values are redacted in
configuration debug output and configuration failures.

JSON provider specifications accept an optional `http_client` object. Python's `Custom` accepts
the same dictionary (or use `ProviderConfig.http_client` for other compatible providers):

```python
Custom("local-model", base_url="http://localhost:8080/v1", http_client={
    "http1_only": False,
    "connect_timeout_ms": 30000,
    "read_timeout_ms": 180000,
    "pool_idle_timeout_ms": 90000,
    "pool_max_idle_per_host": 8,
    "proxy_url": "http://localhost:3128",
})
```

Timeouts are positive integer milliseconds; unknown fields and invalid types are rejected.
Omitted settings retain existing defaults. A supplied transport can be shared intentionally
between providers, while each request continues to supply its own credentials. `send_http`
uses the provider's configured HTTP settings.

## Runtime retention and tool replay

`RunTable::new()` retains at most 128 terminal runs, in completion order, in addition to active
runs. `with_terminal_capacity(n)` changes the limit. Event listeners pin a run until they drain;
those pinned records are temporarily additional to the limit. Per-run notifications wake only
that run's consumers, without idle timer polling. Dropping the receiver stops its forwarder.

`release_run` reclaims event history, tool results and terminal operation payloads after current
listeners drain. A lightweight snapshot (identity, state and last event sequence) remains until
capacity eviction, so inspecting a recently completed Python run still works. Terminal operation
history is empty after release. `remove_terminal_run` requests full eviction, deferred while an
event listener is active; `prune_terminal_runs` applies the configured limit. The operation store
also exposes explicit terminal removal and pruning for standalone use. Do not reuse evicted run
IDs. Active runs and executing operation commits are not evicted.

Tool claims atomically reserve an invocation ID before approval or execution. Concurrent duplicates
are rejected and completed duplicates reuse the exact result, including multimodal content.
Dropping an unstarted claim releases the reservation. Once execution begins, failures or cancellation
retain a failure marker, because an interrupted side effect may already have happened. Replaying
that ID cannot execute the side effect again. Executing new tools on a terminal run is rejected.
