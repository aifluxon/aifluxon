from aifluxon import (
    Agent,
    ControlledProvider,
    OperationRequested,
    RequireApprovalPolicy,
    ToolEffect,
    tool,
)


@tool(description="Look up a value.", effect=ToolEffect.EXTERNAL_SIDE_EFFECT)
def lookup(query: str) -> str:
    return f"found:{query}"


async def main() -> None:
    agent = Agent(
        provider=ControlledProvider(
            turns=[
                {"tool": "lookup", "id": "call-1", "arguments": {"query": "mass"}},
                {"text": "mass is 42"},
            ]
        ),
        tools=[lookup],
        policy=RequireApprovalPolicy(mode="blocking_approval"),
    )
    run = await agent.start("look up mass")
    async for event in run.events():
        if isinstance(event, OperationRequested):
            await run.resolve_operation(event.operation_id, "approve")
    result = await run.result()
    print(result.text)


if __name__ == "__main__":
    import asyncio

    asyncio.run(main())
