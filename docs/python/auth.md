# Codex OAuth in Python

Python does not implement OAuth. It binds to `aifluxon-api::CodexAuth`.

Credentials live in the OS secure store or an encrypted vault. Python code must not read access tokens, refresh tokens, or ID tokens.

Published 0.2.0 wheels support Windows x86_64 and Linux glibc x86_64/aarch64. The Python API and structured auth errors are identical across these platforms.

## Login

```python
import asyncio
from aifluxon import Agent, CodexAuth

async def main():
    auth = CodexAuth()
    login = await auth.login()
    print("Open this URL:")
    print(login.authorization_url)
    account = await login.wait()
    agent = Agent(auth.provider("gpt-5.6-codex", account_id=account.id))
    result = await agent.run("Inspect this project")
    print(result.text)

asyncio.run(main())
```

`await auth.login()` is the headless-friendly entry: it binds the native callback listener and returns a `CodexLoginAttempt` without opening a browser. Print `login.authorization_url`, complete ChatGPT login, then `await login.wait()`. `wait()` returns a `CodexAccount` with `id`, `email`, and `expires_at` only.

To open the browser from Python:

```python
account = await auth.login_with_browser()
```

## Restart without logging in again

```python
auth = CodexAuth()
accounts = await auth.accounts()
provider = auth.provider("gpt-5.6-codex", account_id=accounts[0].id)
```

If more than one account is stored, `account_id` is required.

## Logout

```python
await auth.logout(account.id)
```

## Stores

```python
CodexAuth()  # SystemKeyringStore(service_name="AIFLUXON")
CodexAuth(secret_store=SystemKeyringStore(service_name="My Product"))
CodexAuth(secret_store=MemorySecretStore())
```

On desktop Linux, `CodexAuth()` uses `SystemKeyringStore` and requires an available Secret Service session. If the session or backend is unavailable, the operation raises `CredentialStoreUnavailableError`; AIFLUXON does not silently fall back to a file.

For SSH, systemd services, and containers, select the encrypted store explicitly:

```python
from aifluxon import CodexAuth, EncryptedFileSecretStore

store = EncryptedFileSecretStore("~/.local/share/aifluxon/credentials.vault")
await store.unlock(password)
auth = CodexAuth(secret_store=store)
```

Supply `password` from the host's secret manager or a protected credential file, not from source code. The vault keeps the `AFLXCRD1` format, is owner-only on Unix, and fails closed for a wrong password or corrupted data. `await store.lock()` prevents later credential reads; it does not cancel an HTTP request that already resolved its bearer token.

See [Linux headless operations](../auth/linux-headless.md) for systemd and container deployment guidance.

## Static API key

```python
from aifluxon import Agent, Codex

agent = Agent(Codex("gpt-5.6-codex", api_key="..."))
```

That path does not use ChatGPT OAuth.

Runnable example: [`bindings/python/examples/codex_oauth.py`](../../bindings/python/examples/codex_oauth.py).
