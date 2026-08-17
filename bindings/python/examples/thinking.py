"""Thinking / reasoning effort on Agent. Swap the provider to a live one to send traffic."""

from __future__ import annotations

import argparse
import asyncio

from aifluxon import (
    Agent,
    Codex,
    ControlledProvider,
    Custom,
    DeepSeek,
    Gemini,
    Kimi,
    OpenAI,
    Qwen,
)


def openai_agent(api_key: str) -> Agent:
    return Agent(OpenAI("gpt-5.4", api_key=api_key), reasoning_effort="high")


def gemini_agent(api_key: str) -> Agent:
    return Agent(Gemini("gemini-2.5-flash", api_key=api_key), reasoning_effort="low")


def deepseek_agent(api_key: str) -> Agent:
    return Agent(
        DeepSeek("deepseek-v4-flash", api_key=api_key),
        thinking=True,
        reasoning_effort="high",
    )


def qwen_agent(api_key: str) -> Agent:
    return Agent(Qwen("qwen-plus", api_key=api_key), thinking=True, thinking_budget=8192)


def kimi_agent(api_key: str) -> Agent:
    return Agent(Kimi("kimi-k2.5", api_key=api_key))


def codex_agent(api_key: str) -> Agent:
    return Agent(Codex("gpt-5.6-codex", api_key=api_key), reasoning_effort="medium")


def custom_agent(api_key: str) -> Agent:
    return Agent(
        Custom("local-model", base_url="http://127.0.0.1:8080/v1", api_key=api_key),
        reasoning_effort="high",
    )


async def demo_offline() -> None:
    agent = Agent(ControlledProvider(["ok"]), reasoning_effort="high", thinking=True)
    print(agent.thinking_settings)
    print((await agent.run("hello", reasoning_effort="low")).text)


async def main() -> None:
    parser = argparse.ArgumentParser(description="AIFLUXON thinking settings")
    parser.add_argument("--offline", action="store_true", default=True)
    args = parser.parse_args()
    if args.offline:
        await demo_offline()


if __name__ == "__main__":
    asyncio.run(main())
