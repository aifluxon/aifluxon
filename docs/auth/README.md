# Authentication

AIFLUXON owns Codex OAuth. Hosts own UI, browser opening, and the secret-store namespace.

```text
Host (Rust / Python / Desktop)
        ↓
   aifluxon-api::CodexAuth
        ↓
     aifluxon-auth
        ↓
  OS keyring or encrypted vault
```

The runtime crate does not import auth. Providers receive a `CredentialSource` and resolve a bearer token on every remote turn.

## Documents

- [Codex OAuth](codex-oauth.md)
- [Credential storage](credential-storage.md)
- [Linux headless](linux-headless.md)
- [Security](security.md)

## Two Codex modes

1. **Static API key** — `Codex(model, api_key=...)` talks to `https://api.openai.com/v1`.
2. **OAuth** — `CodexAuth.provider(model, account_id=...)` talks to `https://chatgpt.com/backend-api/codex` with ChatGPT account headers.

Do not copy an OAuth access token into `api_key`.
