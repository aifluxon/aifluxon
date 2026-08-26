# Linux headless authentication

`SystemKeyringStore` is intended for desktop Linux with an active Secret Service and DBus session. SSH sessions, systemd services, containers, and Kubernetes workloads often do not have that session. In those environments, select `EncryptedFileSecretStore` explicitly; an unavailable system keyring remains a structured `CredentialStoreUnavailableError` and never triggers a plaintext fallback.

```python
from aifluxon import CodexAuth, EncryptedFileSecretStore

store = EncryptedFileSecretStore("~/.local/share/aifluxon/credentials.vault")
await store.unlock(password)
auth = CodexAuth(secret_store=store)
```

The host owns `password`. Read it at process startup from a protected secret-manager response, an inherited file descriptor, or a read-only credential file. Do not embed it in source, command-line arguments, logs, container images, or the vault volume itself.

## Files and lifecycle

- Persist the vault on durable storage if accounts must survive restarts.
- Restrict the parent directory to the service account. AIFLUXON creates and replaces the vault as mode `0600` on Unix and fails if that permission cannot be established.
- Back up the encrypted vault and its unlock material separately. Losing either makes the stored credentials unrecoverable.
- A wrong password, invalid `AFLXCRD1` header, unsupported format version, or authentication-tag failure is reported as `CredentialCorruptedError`; the vault is never partially opened.
- `await store.lock()` zeroizes the in-memory key and prevents later reads. An HTTP request that already resolved a bearer token is not retroactively cancelled.
- Never use a plaintext `auth.json` or an automatic random vault password that disappears when the process exits.

## systemd

Run the service as a dedicated unprivileged user, persist the vault in a directory owned by that user, and inject the password with systemd credentials or another host secret manager. A service can read a credential file without putting the value in its unit file:

```ini
[Service]
User=aifluxon
StateDirectory=aifluxon
LoadCredential=aifluxon-vault-password:/run/secrets/aifluxon-vault-password
Environment=AIFLUXON_VAULT=/var/lib/aifluxon/credentials.vault
```

Application startup reads the file under `$CREDENTIALS_DIRECTORY`, unlocks the explicit vault, and discards the plaintext buffer as soon as the store is ready. Do not print the path contents or pass them through process arguments.

## Docker and Kubernetes

Use two separate mounts:

- a persistent volume for `credentials.vault`;
- a read-only secret mount or secret-manager integration for the unlock material.

Run as a fixed non-root UID/GID that owns the vault directory. Do not bake either credential into an image layer, ConfigMap, environment dump, or startup log. An ephemeral container without a persistent vault will require authentication again after replacement; that is expected and must not be hidden by a plaintext fallback.

## Operational checks

Before enabling a workload, verify that:

1. the service account can create and replace the vault in its target directory;
2. the resulting vault mode is `0600`;
3. restart with the same password restores accounts;
4. a wrong password and a modified vault both fail closed;
5. logs, crash reports, and health endpoints contain no password, access token, refresh token, or ID token.
