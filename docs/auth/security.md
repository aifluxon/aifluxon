# Security contract

- Access tokens, refresh tokens, ID tokens, PKCE verifiers, and authorization codes never appear in Python public objects or JSON provider specs.
- `SecretString` and auth errors redact known secret material.
- Runtime session/checkpoint JSON must not persist OAuth secrets.
- Provider HTTP sanitizes bearer tokens out of error strings.
- Logs must not print tokens or authorization codes.
- Encrypted vaults use Argon2id and XChaCha20-Poly1305. Tampered files fail closed.
- Windows keyring chunks include a SHA-256 manifest. Missing or mismatched chunks fail closed.
- Legacy single-account Codex keyring entries are migrated only inside `aifluxon-auth`.
