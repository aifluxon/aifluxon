# Errors

All public exceptions subclass `AifluxonError`.

```text
AifluxonError
├── InvalidConfigurationError
├── InvalidRequestError
├── ProviderError
├── ToolError
├── PolicyError
├── CancelledError
├── BudgetExceededError
├── StateConflictError
├── FailedError
└── InternalError
```

| Exception | When | User error? | Retry? |
|---|---|---|---|
| `InvalidConfigurationError` | Missing provider fields, bad builder input | Yes | No, fix config |
| `InvalidRequestError` | Unknown session/run/tool name in the request | Yes | No, fix request |
| `ProviderError` | Unregistered provider or provider `next_turn` failure | Usually provider | Maybe, depends on the message |
| `ToolError` | Unknown or invalid tool on the request | Yes | No |
| `PolicyError` | Policy denied / rejected an operation | Depends | No unless the host retries with a new decision |
| `CancelledError` | `run.cancel()` or cancellation while waiting | No | No |
| `BudgetExceededError` | Model or tool budget exhausted | Maybe | No without higher limits |
| `StateConflictError` | Session CAS conflict or illegal operation transition | Maybe concurrent writer | Reload and retry the mutation |
| `FailedError` | Run terminated `Failed` | Depends | Inspect `message` |
| `InternalError` | Unexpected backend failure | No | No |

Do not log API keys, cookies, or tokens from exception messages into public artifacts. Provider errors are sanitized on the HTTP path; still treat messages as untrusted.
