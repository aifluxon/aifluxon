from aifluxon import Agent, ControlledProvider


async def main() -> None:
    agent = Agent(provider=ControlledProvider(["Hello from AIFLUXON."]))
    run = await agent.start("hello")
    result = await run.result()
    print(result.text)


if __name__ == "__main__":
    import asyncio

    asyncio.run(main())
