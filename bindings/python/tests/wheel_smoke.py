from __future__ import annotations

import asyncio
import tempfile
from pathlib import Path

import aifluxon
import aifluxon._native as native
from aifluxon import (
    Agent,
    ControlledProvider,
    EncryptedFileSecretStore,
    ImageInput,
    JsonFileSessionStore,
    ToolEffect,
    tool,
)


async def smoke() -> None:
    assert native is not None
    assert aifluxon.__version__ == "0.2.0"

    result = await Agent(ControlledProvider(["wheel-ok"])).run("hello")
    assert result.state == "completed"
    assert result.text == "wheel-ok"

    image = ImageInput.from_bytes(b"wheel-image", "image/png")
    multimodal = await Agent(ControlledProvider(["multimodal-ok"])).run(
        ["inspect", image]
    )
    assert multimodal.text == "multimodal-ok"

    with tempfile.TemporaryDirectory(prefix="aifluxon-wheel-") as directory:
        root = Path(directory)
        store = JsonFileSessionStore(str(root / "sessions"))
        agent = Agent(ControlledProvider(["session-ok"]), store=store)
        session = await agent.create_session()
        session_result = await session.run("persist")
        assert session_result.session_id == session.id

        reopened = Agent(ControlledProvider(["reopened"]), store=store)
        assert (await reopened.open_session(session.id)).id == session.id

        secret_store = EncryptedFileSecretStore(str(root / "credentials.vault"))
        await secret_store.unlock("wheel-smoke-password")
        assert secret_store.is_unlocked
        await secret_store.lock()
        assert not secret_store.is_unlocked

    calls = 0

    @tool(description="Echo a value.", effect=ToolEffect.PURE_READ, parallel_safe=True)
    def echo(value: str) -> str:
        nonlocal calls
        calls += 1
        return value

    tool_agent = Agent(
        ControlledProvider(
            turns=[
                {"tool": "echo", "id": "call-1", "arguments": {"value": "ok"}},
                {"text": "tool-ok"},
            ]
        ),
        tools=[echo],
    )
    assert (await tool_agent.run("use the tool")).text == "tool-ok"
    assert calls == 1

    agents = [Agent(ControlledProvider([f"parallel-{index}"])) for index in range(8)]
    results = await asyncio.gather(*(agent.run("x") for agent in agents))
    assert [result.text for result in results] == [f"parallel-{index}" for index in range(8)]


if __name__ == "__main__":
    asyncio.run(smoke())
    print("AIFLUXON wheel smoke passed")
