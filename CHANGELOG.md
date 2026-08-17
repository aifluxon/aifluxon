# Changelog

## Unreleased

- Codex OAuth is owned by `aifluxon-auth` and exposed through `aifluxon-api::CodexAuth` and the Python `CodexAuth` SDK.
- Python `Agent` accepts `reasoning_effort`, `thinking`, and `thinking_budget` and forwards them as `ProviderFeatureRequest`.
- Python `Agent` accepts `system_prompt` and prepends it as a canonical system message. Session turns replace the leading system message instead of stacking copies.
- OpenAI-compatible providers resolve a `CredentialSource` on every remote turn. OAuth 401 retries at most once.
- System keyring, encrypted vault, and memory secret stores. No plaintext JSON fallback.

## 0.1.0

Initial experimental release.

- In-process agent backend through `aifluxon-api`
- Providers: OpenAI, DeepSeek, Qwen, Kimi, Gemini, Codex, and custom OpenAI-compatible endpoints
- Persistent sessions, async events, Python tools, approval policy, and operations
- Python SDK for Windows x86_64, CPython 3.11–3.14
- ChatGPT Web and DeepSeek Web are not part of the public Python SDK
