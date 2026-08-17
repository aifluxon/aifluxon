from aifluxon import Agent, ControlledProvider, TextDelta


async def main() -> None:
    agent = Agent(provider=ControlledProvider(["streamed answer"]))
    run = await agent.start("hello")
    async for event in run.events():
        if isinstance(event, TextDelta):
            print(event.delta, end="", flush=True)
        print(f"\n{event.type} seq={event.sequence}")
    result = await run.result()
    print(result.state)


if __name__ == "__main__":
    import asyncio

    asyncio.run(main())
