from aifluxon import Agent, ControlledProvider, ToolEffect, tool


@tool(description="Double a number.", effect=ToolEffect.PURE_READ, parallel_safe=True)
def double(value: float) -> float:
    return value * 2


async def main() -> None:
    agent = Agent(
        provider=ControlledProvider(
            turns=[
                {"tool": "double", "id": "call-1", "arguments": {"value": 21}},
                {"text": "The result is 42."},
            ]
        ),
        tools=[double],
    )
    result = await agent.run("double 21")
    print(result.text)


if __name__ == "__main__":
    import asyncio

    asyncio.run(main())
