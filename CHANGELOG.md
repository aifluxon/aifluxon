# Changelog

## Unreleased

## 0.2.0

### Added

- Linux x86_64 and Linux aarch64 Python wheels for CPython 3.11–3.14.
- manylinux2014 / glibc 2.17 compatibility with native x86_64 and ARM64 clean-install verification.
- Cross-platform source tests, concurrent-agent smoke coverage, wheel matrix validation, and native dependency audits.

### Changed

- Provider and OAuth HTTP clients now use Rustls with native certificate roots instead of native-tls.
- Python release promotion now verifies a 12-wheel matrix and SHA256 manifest, then reuses the exact TestPyPI-verified artifacts for production PyPI.

### Security

- Linux headless authentication uses an explicitly selected encrypted vault with no plaintext or implicit credential-store fallback.
- Unix encrypted vault creation enforces owner-only `0600` permissions and fails closed if permissions cannot be applied.

## 0.1.1

- Codex OAuth is owned by `aifluxon-auth` and exposed through `aifluxon-api::CodexAuth` and the Python `CodexAuth` SDK.
- Python `Agent` accepts `reasoning_effort`, `thinking`, and `thinking_budget` and forwards them as `ProviderFeatureRequest`.
- Python `Agent` accepts `system_prompt` and prepends it as a canonical system message. Session turns replace the leading system message instead of stacking copies.
- DeepSeek V4 Pro supports the Responses API in addition to Chat Completions. Python `api_mode` selects `"chat_completions"` or `"responses"`. `low` reasoning effort remains Flash-only.
- Responses tool history is serialized as `function_call` / `function_call_output` items. Codex encrypted reasoning is replayed as stored items.
- DeepSeek thinking content is replayed after tool-call turns.
- OpenAI-compatible providers resolve a `CredentialSource` on every remote turn. OAuth 401 retries at most once.
- System keyring, encrypted vault, and memory secret stores. No plaintext JSON fallback.

## 0.1.0

Initial experimental release.

- In-process agent backend through `aifluxon-api`
- Providers: OpenAI, DeepSeek, Qwen, Kimi, Gemini, Codex, and custom OpenAI-compatible endpoints
- Persistent sessions, async events, Python tools, approval policy, and operations
- Python SDK for Windows x86_64, CPython 3.11–3.14
- ChatGPT Web and DeepSeek Web are not part of the public Python SDK
