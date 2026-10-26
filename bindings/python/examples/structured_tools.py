"""Offline typed tool, capability grant and structured result."""

import asyncio
from typing import Annotated, Literal, TypedDict

from aifluxon import (
    Agent,
    ControlledProvider,
    ExecutionAuthority,
    RunOptions,
    ToolEffect,
    ToolResult,
    tool,
)


class Lookup(TypedDict):
    topic: str
    mode: Literal["brief", "full"]


@tool(effect=ToolEffect.PURE_READ, required_capabilities=["catalog.read"])
async def lookup(query: Annotated[Lookup, "Catalog search parameters"]) -> ToolResult:
    return ToolResult(
        value={"topic": query["topic"], "found": True},
        content="Inertia is resistance to changes in motion.",
    )


async def main() -> None:
    agent = Agent(
        ControlledProvider(
            turns=[
                {
                    "tool": "lookup",
                    "arguments": {"query": {"topic": "inertia", "mode": "brief"}},
                },
                {
                    "text": "The catalog describes inertia as resistance to changes in motion."
                },
            ]
        ),
        tools=[lookup],
        options=RunOptions(authority=ExecutionAuthority(capabilities={"catalog.read"})),
    )
    run = await agent.start("Look up inertia.")
    async for event in run.events():
        if event.type == "tool_finished":
            print(event.result)
    print((await run.result()).text)


if __name__ == "__main__":
    asyncio.run(main())
