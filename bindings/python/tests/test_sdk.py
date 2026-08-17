from __future__ import annotations

import ast
import json
import re
from pathlib import Path

import pytest

from aifluxon import (
    Agent,
    CancelledError,
    ControlledProvider,
    DeepSeek,
    InvalidConfigurationError,
    JsonFileSessionStore,
    OperationRequested,
    RequireApprovalPolicy,
    TextDelta,
    ToolEffect,
    tool,
)
from aifluxon import __all__ as PUBLIC_API


ROOT = Path(__file__).resolve().parents[3]
DOCS = ROOT / "docs" / "python"
API_REFERENCE = DOCS / "api-reference.md"
PYTHON_CRATE = ROOT / "bindings" / "python" / "Cargo.toml"


@pytest.mark.asyncio
async def test_import_and_result() -> None:
    agent = Agent(provider=ControlledProvider(["Hello from AIFLUXON."]))
    run = await agent.start("hello")
    result = await run.result()
    assert result.text == "Hello from AIFLUXON."
    assert result.state == "completed"
    assert result.run_id == run.id


@pytest.mark.asyncio
async def test_session_creates_distinct_runs_and_restores(tmp_path: Path) -> None:
    store = JsonFileSessionStore(str(tmp_path / "data"))
    agent = Agent(
        provider=ControlledProvider(["one", "two"]),
        store=store,
    )
    session = await agent.open_or_create_session("physics-project")
    first = await session.run("first")
    second = await session.run("second")
    assert first.run_id != second.run_id
    assert first.session_id == second.session_id == session.id

    restored = Agent(
        provider=ControlledProvider(["three"]),
        store=JsonFileSessionStore(str(tmp_path / "data")),
    )
    opened = await restored.open_session(session.id)
    listed = await restored.list_sessions()
    assert opened.id == session.id
    assert listed[0]["id"] == session.id
    files = list((tmp_path / "data").rglob("*.json"))
    blob = "\n".join(path.read_text(encoding="utf-8") for path in files).lower()
    for forbidden in ("api_key", "sk-", "oauth", "cookie"):
        assert forbidden not in blob


@pytest.mark.asyncio
async def test_agent_thinking_defaults_and_per_run_override() -> None:
    agent = Agent(
        provider=ControlledProvider(["default-run", "override-run"]),
        reasoning_effort="high",
        thinking=True,
        thinking_budget=2048,
    )
    assert agent.reasoning_effort == "high"
    assert agent.thinking == "enabled"
    assert agent.thinking_budget == "2048"
    assert agent.thinking_settings.to_payload() == {
        "reasoning_effort": "high",
        "thinking_mode": "enabled",
        "thinking_budget": "2048",
    }
    first = await agent.run("one")
    second = await agent.run("two", reasoning_effort="low", thinking=False)
    assert first.text == "default-run"
    assert second.text == "override-run"
    assert agent.reasoning_effort == "high"


@pytest.mark.asyncio
async def test_agent_system_prompt_defaults_and_per_run_override(tmp_path: Path) -> None:
    store = JsonFileSessionStore(str(tmp_path / "data"))
    agent = Agent(
        provider=ControlledProvider(["one", "two", "three"]),
        store=store,
        system_prompt="You are a laboratory reviewer.",
    )
    assert agent.system_prompt == "You are a laboratory reviewer."
    session = await agent.open_or_create_session("lab")
    first = await session.run("first")
    second = await session.run("second")
    assert first.text == "one"
    assert second.text == "two"
    blob = "\n".join(
        path.read_text(encoding="utf-8") for path in (tmp_path / "data").rglob("*.json")
    )
    assert blob.count("You are a laboratory reviewer.") == 1
    third = await session.run("third", system_prompt="You are a translator.")
    assert third.text == "three"
    blob = "\n".join(
        path.read_text(encoding="utf-8") for path in (tmp_path / "data").rglob("*.json")
    )
    assert blob.count("You are a laboratory reviewer.") == 0
    assert blob.count("You are a translator.") == 1
    with pytest.raises(InvalidConfigurationError):
        Agent(provider=ControlledProvider(["x"]), system_prompt=123)


def test_thinking_settings_reject_invalid_values() -> None:
    with pytest.raises(InvalidConfigurationError):
        Agent(provider=ControlledProvider(["x"]), reasoning_effort="ultra")
    with pytest.raises(InvalidConfigurationError):
        Agent(provider=ControlledProvider(["x"]), thinking="maybe")
    with pytest.raises(InvalidConfigurationError):
        Agent(provider=ControlledProvider(["x"]), thinking_budget=0)


def test_provider_api_mode_is_forwarded_in_spec() -> None:
    spec = DeepSeek("deepseek-v4-pro", api_key="k", api_mode="responses").to_spec()
    assert spec["kind"] == "deepseek"
    assert spec["model"] == "deepseek-v4-pro"
    assert spec["api_mode"] == "responses"


@pytest.mark.asyncio
async def test_events_are_monotonic_and_terminal_once() -> None:
    agent = Agent(provider=ControlledProvider(["visible"]))
    run = await agent.start("hello")
    events = [event async for event in run.events()]
    sequences = [event.sequence for event in events]
    assert sequences == sorted(sequences)
    terminals = [event for event in events if event.type in {"completed", "failed", "cancelled"}]
    assert len(terminals) == 1
    assert any(isinstance(event, TextDelta) for event in events)
    result = await run.result()
    assert result.text == "visible"


@pytest.mark.asyncio
async def test_cancel_and_consumer_drop_do_not_conflict() -> None:
    agent = Agent(provider=ControlledProvider(["slow"]))
    run = await agent.start("hello")
    iterator = run.events()
    await iterator.__anext__()
    await iterator.aclose()
    result = await run.result()
    assert result.state == "completed"

    hanging = Agent(provider=ControlledProvider(["will-cancel"], delay_ms=500))
    run = await hanging.start("hello")
    await run.cancel()
    with pytest.raises(CancelledError):
        await run.result()


@pytest.mark.asyncio
async def test_python_tool_uses_canonical_registry() -> None:
    executions = {"count": 0}

    @tool(description="Echo a query.", effect=ToolEffect.PURE_READ, parallel_safe=True)
    def lookup(query: str) -> str:
        executions["count"] += 1
        return f"echo:{query}"

    agent = Agent(
        provider=ControlledProvider(
            turns=[
                {"tool": "lookup", "id": "call-1", "arguments": {"query": "mass"}},
                {"tool": "lookup", "id": "call-1", "arguments": {"query": "mass"}},
                {"text": "done"},
            ]
        ),
        tools=[lookup],
    )
    result = await agent.run("lookup mass")
    assert result.text == "done"
    assert executions["count"] == 1


@pytest.mark.asyncio
async def test_approval_and_deferred_commit() -> None:
    @tool(description="Write a value.", effect=ToolEffect.EXTERNAL_SIDE_EFFECT)
    def write_value(query: str) -> str:
        return f"wrote:{query}"

    agent = Agent(
        provider=ControlledProvider(
            turns=[
                {"tool": "write_value", "id": "call-1", "arguments": {"query": "x"}},
                {"text": "approved"},
            ]
        ),
        tools=[write_value],
        policy=RequireApprovalPolicy(mode="blocking_approval"),
    )
    run = await agent.start("write")
    async for event in run.events():
        if isinstance(event, OperationRequested):
            await run.resolve_operation(event.operation_id, "approve")
    assert (await run.result()).text == "approved"

    deferred = Agent(
        provider=ControlledProvider(
            turns=[
                {"tool": "write_value", "id": "call-2", "arguments": {"query": "y"}},
                {"text": "committed"},
            ]
        ),
        tools=[write_value],
        policy=RequireApprovalPolicy(mode="deferred_commit"),
    )
    run = await deferred.start("write")
    async for event in run.events():
        if isinstance(event, OperationRequested):
            with pytest.raises(Exception):
                await run.resolve_operation(event.operation_id, "approve")
            await run.commit_operation(event.operation_id)
    assert (await run.result()).text == "committed"


@pytest.mark.asyncio
async def test_unsupported_tool_type_errors() -> None:
    with pytest.raises(TypeError):

        @tool
        def bad(value: complex) -> str:
            return str(value)


def test_public_symbols_are_documented() -> None:
    text = API_REFERENCE.read_text(encoding="utf-8")
    missing = [name for name in PUBLIC_API if name not in text]
    assert missing == [], missing


def test_python_crate_depends_only_on_aifluxon_api() -> None:
    manifest = PYTHON_CRATE.read_text(encoding="utf-8")
    for forbidden in ("aifluxon-core", "aifluxon-runtime", "aifluxon-providers", "tauri"):
        assert forbidden not in manifest


def test_documented_examples_do_not_invent_missing_imports() -> None:
    source = (ROOT / "bindings" / "python" / "python" / "aifluxon" / "__init__.py").read_text(
        encoding="utf-8"
    )
    exported = set(PUBLIC_API)
    for match in re.findall(r"from aifluxon import ([^\n]+)", (DOCS / "quickstart.md").read_text(encoding="utf-8")):
        names = [part.strip(" ,") for part in match.replace("(", " ").replace(")", " ").split() if part.strip(" ,")]
        for name in names:
            if name and name[0].isalpha():
                assert name in exported, name
