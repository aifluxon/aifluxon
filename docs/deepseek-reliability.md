# DeepSeek stream and tool-call contract

The DeepSeek provider validates each Chat Completions SSE response before returning executable tool calls: a framed `[DONE]`, a successful `finish_reason`, complete unique call IDs, names, and JSON object arguments are required. `length`, provider errors, malformed events, reasoning-only replies, and premature EOF cannot become successful answers. Responses retains its completed-terminal check and now validates tool calls before filtering them.

DeepSeek tool ID/name fields are metadata snapshots; only arguments are appended. Generic compatible providers retain their existing fragment behavior. Assistant tool-call reasoning is replayed byte-for-byte, without whitespace trimming. Current V4 models and `deepseek-flash` receive a 256,000-token output allowance; legacy model defaults are unchanged. This is a ceiling, not a target response length.

Transient failures before visible text or reasoning or provider-hosted work retry the same request at most twice, with cancellable exponential backoff and jitter (300-400 / 600-700 ms), honoring bounded Retry-After delays. Local tools are dispatched only after a validated turn, so a failed attempt never executes a partial tool call. Failures after visible output, output-limit failures, and non-transient errors are reported rather than replayed. Structured `PROVIDER_MODEL_ATTEMPT_FAILED` events contain attempt counts and retry classification, not credentials or model content.

The shared HTTP client reuses connection pools with per-request credentials. Streaming returns at the protocol terminal without waiting for the socket to close, while Chat Completions still captures trailing usage before `[DONE]`. The SSE parser scans only new bytes and drains its buffer once per chunk. Exhausting the runtime's continuation allowance reports failure instead of completion.

Reference: [DeepSeek Harness Chat Completions translator](https://github.com/deepseek-ai/deepseek-harness/blob/99f6f02fecdb7dff40c3fbc9470f5907c29f74ca/packages/llm/llm-deepseek/src/translate.ts), [SSE framing](https://github.com/deepseek-ai/deepseek-harness/blob/99f6f02fecdb7dff40c3fbc9470f5907c29f74ca/packages/llm/llm-deepseek/src/sse.ts), and [official API](https://api-docs.deepseek.com/api/create-chat-completion/). Newer Harness versions use Messages transport; this change preserves existing Chat Completions and Responses integrations.

Validation: `cargo test --workspace`. Regression cases cover byte-wise Unicode/tool argument fragmentation, repeated metadata, missing terminals, malformed JSON, length limits, bounded/cancelled retries, open sockets after terminal, connection reuse with separate credentials, and exactly one failed terminal on continuation exhaustion.

The protocol checks and retry policy now cover every public API family; see [shared API reliability](api-reliability.md).
