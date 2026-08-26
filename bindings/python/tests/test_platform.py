from __future__ import annotations

import asyncio
import os
import stat
import sys

import pytest

from aifluxon import Agent, ControlledProvider, EncryptedFileSecretStore


def test_native_extension_imports() -> None:
    import aifluxon._native as native

    assert native is not None


@pytest.mark.asyncio
async def test_offline_agent_roundtrip() -> None:
    result = await Agent(ControlledProvider(["platform-ok"])).run("hello")
    assert result.state == "completed"
    assert result.text == "platform-ok"


@pytest.mark.asyncio
async def test_encrypted_store_roundtrip(tmp_path) -> None:
    path = tmp_path / "credentials.vault"
    store = EncryptedFileSecretStore(str(path))
    await store.unlock("test-password")
    assert store.is_unlocked
    await store.lock()
    assert not store.is_unlocked


@pytest.mark.skipif(sys.platform == "win32", reason="Unix permission contract")
@pytest.mark.asyncio
async def test_encrypted_store_permissions(tmp_path) -> None:
    path = tmp_path / "credentials.vault"
    store = EncryptedFileSecretStore(str(path))
    await store.unlock("test-password")
    assert stat.S_IMODE(os.stat(path).st_mode) == 0o600


@pytest.mark.asyncio
async def test_multiple_agents_can_run_concurrently() -> None:
    agents = [Agent(ControlledProvider([f"ok-{index}"])) for index in range(8)]
    results = await asyncio.gather(*(agent.run("x") for agent in agents))
    assert [result.text for result in results] == [f"ok-{index}" for index in range(8)]
