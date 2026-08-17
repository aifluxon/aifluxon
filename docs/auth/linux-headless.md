# Linux headless

System keyring is often unavailable over SSH or in a container.

```python
from aifluxon import CodexAuth, EncryptedFileSecretStore

store = EncryptedFileSecretStore("~/.local/share/aifluxon/credentials.vault")
await store.unlock(password)
auth = CodexAuth(secret_store=store)
```

Locking the vault prevents later credential reads. An in-flight HTTP request that already resolved a bearer token is not cancelled.

Do not pass `credentials_file="auth.json"`. If the store name is `EncryptedFileSecretStore`, it must be encrypted and require unlock material.
