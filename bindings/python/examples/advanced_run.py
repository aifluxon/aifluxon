"""Offline message input, per-run options, capabilities and session history."""

import asyncio
from dataclasses import replace

from aifluxon import Agent, ControlledProvider, Message, RunLimits, RunOptions


async def main() -> None:
    options = RunOptions(limits=RunLimits.unlimited())
    agent = Agent(
        ControlledProvider(
            ["Inertia resists changes in motion.", "A moving cart keeps rolling."]
        ),
        options=options,
    )
    session = await agent.open_or_create_session("physics-demo")
    first = await session.run_messages(
        [
            Message("system", "Teach physics clearly."),
            Message("user", "Explain inertia."),
        ]
    )
    print(first.text)
    print(
        (
            await session.run(
                "Give an example.", options=replace(options, allowed_tools=())
            )
        ).text
    )
    record = await session.refresh()
    print(
        f"Session {record.id}: {record.message_count} messages, revision {record.revision}"
    )
    print(f"Provider supports images: {agent.capabilities.supports_images}")


if __name__ == "__main__":
    asyncio.run(main())
