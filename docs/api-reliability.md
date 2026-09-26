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
