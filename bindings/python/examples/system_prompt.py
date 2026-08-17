"""Host system prompt on Agent. Swap the provider to a live one to send traffic."""

from __future__ import annotations

import argparse
import asyncio

from aifluxon import Agent, ControlledProvider, OpenAI


def openai_agent(api_key: str) -> Agent:
    return Agent(
        OpenAI("gpt-5.4", api_key=api_key),
        system_prompt="You are a concise laboratory reviewer. Reply in Chinese.",
    )


async def demo_offline() -> None:
    agent = Agent(
        ControlledProvider(["ok", "still ok"]),
        system_prompt="You are a focused laboratory reviewer.",
    )
    print(agent.system_prompt)
    print((await agent.run("Summarize the method.")).text)
    print((await agent.run("Now translate.", system_prompt="You are a translator.")).text)


async def main() -> None:
    parser = argparse.ArgumentParser(description="AIFLUXON system_prompt")
    parser.add_argument("--offline", action="store_true", default=True)
    args = parser.parse_args()
    if args.offline:
        await demo_offline()


if __name__ == "__main__":
    asyncio.run(main())
