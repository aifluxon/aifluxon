from __future__ import annotations

import base64
import json
from dataclasses import asdict

import pytest

from aifluxon import (
    AccountSelectionRequiredError,
    Agent,
    CodexAccount,
    CodexAuth,
    CodexLoginAttempt,
    EncryptedFileSecretStore,
    MemorySecretStore,
    SystemKeyringStore,
    __all__ as PUBLIC_API,
)


def _jwt(account_id: str) -> str:
    payload = json.dumps(
        {
            "https://api.openai.com/auth": {"chatgpt_account_id": account_id},
            "email": f"{account_id}@example.com",
            "exp": 4_000_000_000,
        }
    ).encode("utf-8")
    return "aaa." + base64.urlsafe_b64encode(payload).decode("ascii").rstrip("=") + ".sig"


def test_python_exports_codex_auth() -> None:
    for name in (
        "CodexAuth",
        "CodexLoginAttempt",
        "CodexAccount",
        "CodexAuthStatus",
        "SystemKeyringStore",
        "EncryptedFileSecretStore",
        "MemorySecretStore",
    ):
        assert name in PUBLIC_API


def test_codex_account_is_safe_dataclass() -> None:
    account = CodexAccount(id="acct-1", email="a@example.com", expires_at=1)
    dumped = json.dumps(asdict(account))
    assert "access_token" not in dumped
    assert "refresh_token" not in dumped
    assert "id_token" not in dumped


def test_codex_auth_default_service_is_aifluxon() -> None:
    store = SystemKeyringStore()
    assert store.service_name == "AIFLUXON"


@pytest.mark.asyncio
async def test_python_begin_login_returns_url() -> None:
    auth = CodexAuth(secret_store=MemorySecretStore())
    login = await auth.login()
    assert isinstance(login, CodexLoginAttempt)
    assert "https://auth.openai.com/oauth/authorize" in login.authorization_url
    assert "app_EMoamEEZ73f0CkXaXp7hrann" in login.authorization_url
    await login.cancel()


@pytest.mark.asyncio
async def test_python_login_wait_returns_account() -> None:
    auth = CodexAuth(secret_store=MemorySecretStore())
    account = auth._seed_account("acct-1", "access-1", "refresh-1", _jwt("acct-1"))
    assert account.id == "acct-1"
    assert account.email == "acct-1@example.com"
    listed = await auth.accounts()
    assert listed[0].id == "acct-1"


@pytest.mark.asyncio
async def test_python_login_does_not_expose_tokens() -> None:
    auth = CodexAuth(secret_store=MemorySecretStore())
    auth._seed_account("acct-1", "access-secret-token", "refresh-secret-token", _jwt("acct-1"))
    listed = await auth.accounts()
    blob = json.dumps([asdict(item) for item in listed])
    assert "access-secret-token" not in blob
    assert "refresh-secret-token" not in blob


@pytest.mark.asyncio
async def test_python_accounts_survive_new_auth_instance() -> None:
    store = MemorySecretStore()
    first = CodexAuth(secret_store=store)
    first._seed_account("acct-1", "access-1", "refresh-1", _jwt("acct-1"))
    second = CodexAuth(secret_store=store)
    accounts = await second.accounts()
    assert accounts[0].id == "acct-1"


@pytest.mark.asyncio
async def test_python_logout_removes_account() -> None:
    auth = CodexAuth(secret_store=MemorySecretStore())
    auth._seed_account("acct-1", "access-1", "refresh-1", _jwt("acct-1"))
    await auth.logout("acct-1")
    assert await auth.accounts() == []


@pytest.mark.asyncio
async def test_python_auth_provider_creates_agent() -> None:
    auth = CodexAuth(secret_store=MemorySecretStore())
    auth._seed_account("acct-1", "access-1", "refresh-1", _jwt("acct-1"))
    provider = auth.provider("gpt-5.6-codex", account_id="acct-1")
    agent = Agent(provider)
    assert agent.provider_id == "codex"
    assert agent.model == "gpt-5.6-codex"


@pytest.mark.asyncio
async def test_python_multiple_accounts_require_explicit_selection() -> None:
    auth = CodexAuth(secret_store=MemorySecretStore())
    auth._seed_account("acct-a", "access-a", "refresh-a", _jwt("acct-a"))
    auth._seed_account("acct-b", "access-b", "refresh-b", _jwt("acct-b"))
    with pytest.raises(AccountSelectionRequiredError):
        auth.provider("gpt-5.6-codex")


@pytest.mark.asyncio
async def test_python_provider_spec_contains_no_secret() -> None:
    auth = CodexAuth(secret_store=MemorySecretStore())
    auth._seed_account("acct-1", "access-secret-token", "refresh-secret-token", _jwt("acct-1"))
    spec = auth.provider("gpt-5.6-codex", account_id="acct-1").to_spec()
    blob = json.dumps(spec)
    assert spec["kind"] == "codex_oauth"
    assert "access-secret-token" not in blob
    assert "refresh-secret-token" not in blob
    assert "api_key" not in spec


@pytest.mark.asyncio
async def test_encrypted_vault_roundtrip(tmp_path) -> None:
    path = str(tmp_path / "credentials.vault")
    store = EncryptedFileSecretStore(path)
    await store.unlock("test-password-not-a-token")
    auth = CodexAuth(secret_store=store)
    auth._seed_account("acct-1", "access-1", "refresh-1", _jwt("acct-1"))
    await store.lock()
    assert store.is_unlocked is False
    text = (tmp_path / "credentials.vault").read_bytes()
    assert b"access-1" not in text
    assert b"refresh-1" not in text
