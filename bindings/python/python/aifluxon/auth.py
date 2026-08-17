from __future__ import annotations

import asyncio
import json
import webbrowser
from dataclasses import dataclass
from typing import Any

from . import _native
from .errors import call_native


@dataclass(frozen=True, slots=True)
class CodexAccount:
    id: str
    email: str | None
    expires_at: int | None


@dataclass(frozen=True, slots=True)
class CodexAuthStatus:
    account: CodexAccount
    state: str


def _account_from_payload(payload: dict[str, Any]) -> CodexAccount:
    expires = payload.get("expires_at")
    return CodexAccount(
        id=str(payload["id"]),
        email=None if payload.get("email") is None else str(payload["email"]),
        expires_at=None if expires is None else int(expires),
    )


def _status_from_payload(payload: dict[str, Any]) -> CodexAuthStatus:
    return CodexAuthStatus(
        account=_account_from_payload(payload["account"]),
        state=str(payload["state"]),
    )


class SystemKeyringStore:
    def __init__(self, service_name: str = "AIFLUXON") -> None:
        self._native = call_native(_native.NativeSystemKeyringStore, service_name)

    @property
    def service_name(self) -> str:
        return self._native.service_name


class MemorySecretStore:
    def __init__(self) -> None:
        self._native = call_native(_native.NativeMemorySecretStore)


class EncryptedFileSecretStore:
    def __init__(self, path: str) -> None:
        self._native = call_native(_native.NativeEncryptedFileSecretStore, path)

    @property
    def path(self) -> str:
        return self._native.path

    @property
    def is_unlocked(self) -> bool:
        return bool(self._native.is_unlocked)

    async def unlock(self, password: str) -> None:
        await asyncio.to_thread(call_native, self._native.unlock, password)

    async def lock(self) -> None:
        await asyncio.to_thread(self._native.lock)


class CodexProvider:
    def __init__(self, native: Any) -> None:
        self._native = native

    @property
    def provider_id(self) -> str:
        return self._native.provider_id

    @property
    def model(self) -> str:
        return self._native.model

    @property
    def account_id(self) -> str:
        return self._native.account_id

    def to_spec(self) -> dict[str, Any]:
        return json.loads(self._native.spec_json())


class CodexLoginAttempt:
    def __init__(self, native: Any) -> None:
        self._native = native

    @property
    def authorization_url(self) -> str:
        return self._native.authorization_url

    async def wait(self) -> CodexAccount:
        payload = json.loads(await asyncio.to_thread(call_native, self._native.wait))
        return _account_from_payload(payload)

    async def cancel(self) -> None:
        await asyncio.to_thread(call_native, self._native.cancel)


class CodexAuth:
    def __init__(
        self,
        secret_store: SystemKeyringStore | MemorySecretStore | EncryptedFileSecretStore | None = None,
    ) -> None:
        native_store = None if secret_store is None else secret_store._native
        self._native = call_native(_native.NativeCodexAuth, native_store)

    async def login(self) -> CodexLoginAttempt:
        return CodexLoginAttempt(await asyncio.to_thread(call_native, self._native.login))

    async def login_with_browser(self) -> CodexAccount:
        attempt = await self.login()
        webbrowser.open(attempt.authorization_url)
        return await attempt.wait()

    async def accounts(self) -> list[CodexAccount]:
        payload = json.loads(await asyncio.to_thread(call_native, self._native.accounts))
        return [_account_from_payload(item) for item in payload]

    async def status(self, account_id: str) -> CodexAuthStatus:
        payload = json.loads(await asyncio.to_thread(call_native, self._native.status, account_id))
        return _status_from_payload(payload)

    async def refresh(self, account_id: str) -> CodexAuthStatus:
        payload = json.loads(await asyncio.to_thread(call_native, self._native.refresh, account_id))
        return _status_from_payload(payload)

    async def logout(self, account_id: str) -> None:
        await asyncio.to_thread(call_native, self._native.logout, account_id)

    def provider(self, model: str, account_id: str | None = None) -> CodexProvider:
        return CodexProvider(call_native(self._native.provider, model, account_id))

    def _seed_account(
        self,
        account_id: str,
        access_token: str,
        refresh_token: str,
        id_token: str,
    ) -> CodexAccount:
        payload = json.loads(
            call_native(
                self._native.seed_account,
                account_id,
                access_token,
                refresh_token,
                id_token,
            )
        )
        return _account_from_payload(payload)
