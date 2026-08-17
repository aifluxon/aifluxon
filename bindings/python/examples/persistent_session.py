from pathlib import Path

from aifluxon import Agent, ControlledProvider, JsonFileSessionStore


async def main() -> None:
    root = Path("./aifluxon-data")
    store = JsonFileSessionStore(str(root))
    agent = Agent(
        provider=ControlledProvider(["first answer", "second answer"]),
        store=store,
    )
    session = await agent.open_or_create_session("physics-project")
    first = await session.run("analyze the data")
    second = await session.run("continue the analysis")
    print(first.run_id)
    print(second.run_id)
    print(first.run_id != second.run_id)
    print(second.text)


if __name__ == "__main__":
    import asyncio

    asyncio.run(main())
