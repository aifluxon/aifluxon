from __future__ import annotations

import ast
import asyncio
import json
import threading
from enum import Enum
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Annotated, Any, Literal, NotRequired, TypedDict

import pytest
from aifluxon import (
    Agent,
    ControlledProvider,
    Custom,
    DeepSeekWebTools,
    ExecutionAuthority,
    FailedError,
    ImageInput,
    InternalError,
    InvalidConfigurationError,
    InvalidRequestError,
    JsonFileSessionStore,
    Message,
    OpenAI,
    OperationRequested,
    ProviderFeatures,
    Qwen,
    RequireApprovalPolicy,
    RunLimits,
    RunOptions,
    ToolCall,
    ToolEffect,
    ToolError,
    ToolResult,
    tool,
)
from aifluxon.tools import descriptor_from_callable
from test_multimodal import chat_text, provider_server, responses_text


@tool(effect=ToolEffect.PURE_READ)
def echo(value: str) -> str:
    return value


def tool_turns(count: int) -> list[dict[str, Any]]:
    return [
        {"tool": "echo", "id": f"call-{index}", "arguments": {"value": str(index)}}
        for index in range(count)
    ] + [{"text": "done"}]


@pytest.mark.asyncio
@pytest.mark.parametrize("drop_iterator", [False, True])
async def test_long_runs_complete_without_event_consumer(drop_iterator: bool) -> None:
    agent = Agent(
        ControlledProvider(turns=tool_turns(80)),
        tools=[echo],
        options=RunOptions(limits=RunLimits.unlimited()),
    )
    run = await agent.start("many tool rounds")
    if drop_iterator:
        iterator = run.events()
        await anext(iterator)
        await iterator.aclose()
    assert (await asyncio.wait_for(run.result(), 15)).text == "done"
    events = [event async for event in run.events()]
    assert len(events) > 128
    assert events[-1].type == "completed"
    assert [event async for event in run.events()] == []
    # Snapshot remains available after the native stream handle is released.
    assert (await run.snapshot())["state"] == "completed"


@pytest.mark.asyncio
async def test_event_iterators_cannot_race() -> None:
    run = await Agent(ControlledProvider(["ok"], delay_ms=50)).start("hello")
    iterator = run.events()
    await anext(iterator)
    with pytest.raises(RuntimeError, match="Only one"):
        await anext(run.events())
    await iterator.aclose()
    assert (await run.result()).text == "ok"


@pytest.mark.asyncio
async def test_premature_stream_close_is_not_success(monkeypatch) -> None:
    agent = Agent(ControlledProvider(["ok"], delay_ms=20))
    monkeypatch.setattr(type(agent._native), "next_event", lambda *_: None)
    with pytest.raises(InternalError, match="without a terminal"):
        await (await agent.start("hi")).result()


@pytest.mark.asyncio
async def test_run_limits_override_does_not_mutate_defaults() -> None:
    agent = Agent(
        ControlledProvider(turns=tool_turns(2)), tools=[echo], max_model_rounds=1
    )
    assert (
        await agent.run("hi", options=RunOptions(limits=RunLimits.unlimited()))
    ).text == "done"
    assert agent.options.limits.max_model_rounds == 1
    limited = Agent(
        ControlledProvider(turns=tool_turns(2)), tools=[echo], max_model_rounds=1
    )
    with pytest.raises(FailedError, match="budget"):
        await limited.run("hi")


@pytest.mark.asyncio
async def test_empty_prompt_parts_remain_invalid() -> None:
    agent = Agent(ControlledProvider(["unused"]))
    with pytest.raises(InvalidRequestError):
        await agent.start([])


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "cumulative, expected", [(True, "hello world"), (False, "hellohello world")]
)
async def test_custom_cumulative_stream_option_is_applied(
    cumulative: bool, expected: str
) -> None:
    chunks = [
        {"choices": [{"delta": {"content": "hello"}, "finish_reason": None}]},
        {"choices": [{"delta": {"content": "hello world"}, "finish_reason": "stop"}]},
    ]
    stream = (
        "".join(f"data: {json.dumps(chunk)}\n\n" for chunk in chunks)
        + "data: [DONE]\n\n"
    )
    with provider_server(lambda *_: stream) as (base, _):
        result = await Agent(
            Custom("alias", base_url=base, allow_cumulative_delta=cumulative)
        ).run("hi")
    assert result.text == expected


@pytest.mark.parametrize("value", [-1, 2**32, True, 1.5, "32"])
def test_invalid_limits_are_sdk_errors(value) -> None:
    with pytest.raises(InvalidConfigurationError):
        RunLimits(max_model_rounds=value)


@pytest.mark.asyncio
async def test_tool_allowlist_is_enforced_at_execution() -> None:
    calls = []

    @tool
    def hidden(value: str) -> str:
        calls.append(value)
        return value

    agent = Agent(
        ControlledProvider(turns=[{"tool": "hidden", "arguments": {"value": "x"}}]),
        tools=[hidden],
    )
    with pytest.raises(FailedError, match="unregistered tool"):
        await agent.run("hi", options=RunOptions(allowed_tools=()))
    assert calls == []
    with pytest.raises(ToolError):
        await agent.run("hi", options=RunOptions(allowed_tools=("missing",)))


@pytest.mark.asyncio
async def test_tool_allowlist_reaches_provider() -> None:
    with provider_server(lambda *_: chat_text()) as (base, requests):
        agent = Agent(Custom("alias", base_url=base), tools=[echo])
        await agent.run("none", options=RunOptions(allowed_tools=()))
        await agent.run("one", options=RunOptions(allowed_tools=("echo",)))
    assert not requests[0].get("tools")
    assert requests[1]["tools"][0]["function"]["name"] == "echo"


@pytest.mark.asyncio
async def test_hosted_tools_cache_and_thinking_reach_responses() -> None:
    with provider_server(lambda *_: responses_text()) as (base, requests):
        agent = Agent(
            OpenAI("gpt-5.4", api_key="test", base_url=base, api_mode="responses"),
            reasoning_effort="high",
        )
        await agent.run(
            "hi",
            options=RunOptions(
                features=ProviderFeatures(
                    web_search=True, image_generation=True, prompt_cache_key="project"
                )
            ),
        )
    body = requests[0]
    assert body["prompt_cache_key"] == "project"
    assert {tool["type"] for tool in body["tools"]} >= {
        "web_search",
        "image_generation",
    }
    assert body["reasoning"]["effort"] == "high"


@pytest.mark.asyncio
async def test_explicit_qwen_cache_reaches_chat() -> None:
    with provider_server(lambda *_: chat_text()) as (base, requests):
        await Agent(
            Qwen("qwen-plus", api_key="test", base_url=base),
            system_prompt="static context " * 500,
        ).run(
            "hi",
            options=RunOptions(features=ProviderFeatures(explicit_cache=True)),
        )
    assert "cache_control" in json.dumps(requests[0])


@pytest.mark.asyncio
async def test_capability_grants_are_checked_before_tool_execution() -> None:
    calls = []

    @tool(required_capabilities=["fs.read"])
    def read(value: str) -> str:
        calls.append(value)
        return value

    turns = [{"tool": "read", "arguments": {"value": "allowed"}}, {"text": "ok"}]
    denied = Agent(ControlledProvider(turns=turns), tools=[read])
    result = await denied.run("hi")
    output = next(message for message in result.output if message["role"] == "tool")
    assert "capability" in json.dumps(output)
    assert calls == []
    granted = Agent(ControlledProvider(turns=turns), tools=[read])
    await granted.run(
        "hi", options=RunOptions(authority=ExecutionAuthority(capabilities={"fs.read"}))
    )
    assert calls == ["allowed"]


@pytest.mark.asyncio
async def test_canonical_message_and_session_roundtrip(tmp_path: Path) -> None:
    invocation = ToolCall("echo", {"value": "old"}, provider_call_id="provider_wire_id")
    messages = [
        Message("system", "history system"),
        Message("user", "question"),
        Message(
            "assistant",
            [],
            (invocation,),
            provider_state={"signature": "unchanged+/=", "reasoning": [1, 2]},
        ),
        Message(
            "tool",
            ["result", ImageInput.from_file_id("file-1", "image/png")],
            tool_call_id=invocation.id,
        ),
        Message("user", "continue"),
    ]
    assert Message.from_dict(messages[2].to_dict()).to_dict() == messages[2].to_dict()
    agent = Agent(
        ControlledProvider(["answer"]),
        store=JsonFileSessionStore(tmp_path),
        system_prompt="agent default",
    )
    session = await agent.open_or_create_session("stable-name")
    result = await session.run_messages(messages)
    assert result.session_id == session.id
    record = await session.refresh()
    assert record.message_count == 6
    assert record.messages[0].content == ["history system"]
    assert record.messages[2].to_dict() == messages[2].to_dict()
    assert record.messages[3].tool_call_id == invocation.id
    assert record.messages[3].provider_state == {"call_id": "provider_wire_id"}
    assert record.messages[3].content == messages[3].content
    assert session.revision == record.revision
    assert await session.history() == record.messages
    await session.delete()
    with pytest.raises(InvalidRequestError):
        await agent.get_session(session.id)


@pytest.mark.asyncio
@pytest.mark.parametrize("mode", ["chat_completions", "responses"])
async def test_messages_preserve_wire_tool_ids(mode: str) -> None:
    call = ToolCall("echo", {"value": "x"}, provider_call_id="call_external_123")
    with provider_server(
        lambda *_: responses_text() if mode == "responses" else chat_text()
    ) as (base, requests):
        await Agent(Custom("alias", base_url=base, api_mode=mode)).run_messages(
            [
                Message("user", "hi"),
                Message("assistant", [], (call,)),
                Message("tool", "x", tool_call_id=call.id),
                Message("user", "continue"),
            ]
        )
    assert "call_external_123" in json.dumps(requests[0])
    if mode == "responses":
        output = next(
            item
            for item in requests[0]["input"]
            if item.get("type") == "function_call_output"
        )
        assert output["call_id"] == "call_external_123"
    else:
        output = next(
            item for item in requests[0]["messages"] if item["role"] == "tool"
        )
        assert output["tool_call_id"] == "call_external_123"


@pytest.mark.asyncio
async def test_async_tools_use_callers_event_loop() -> None:
    loop = asyncio.get_running_loop()
    future = loop.create_future()
    future.set_result("from caller")

    @tool
    async def async_lookup(value: str) -> str:
        assert asyncio.get_running_loop() is loop
        return await future

    agent = Agent(
        ControlledProvider(
            turns=[
                {"tool": "async_lookup", "arguments": {"value": "x"}},
                {"text": "done"},
            ]
        ),
        tools=[async_lookup],
    )
    run = await agent.start("hi")
    events = [event async for event in run.events()]
    assert next(event.result for event in events if event.type == "tool_finished") == {
        "result": "from caller"
    }
    await run.result()


class LookupInput(TypedDict):
    mode: Literal["quick", "full"]
    note: NotRequired[str]


@pytest.mark.asyncio
async def test_richer_schema_null_and_structured_results() -> None:
    @tool
    def inspect_record(
        record: LookupInput, note: Annotated[str | None, "Optional note"] = None
    ) -> ToolResult:
        return ToolResult(
            value={"mode": record["mode"], "note": note}, content="model-facing result"
        )

    descriptor = descriptor_from_callable(inspect_record)
    assert descriptor["input_schema"]["properties"]["record"]["required"] == ["mode"]
    assert descriptor["input_schema"]["properties"]["note"]["anyOf"][-1] == {
        "type": "null"
    }
    agent = Agent(
        ControlledProvider(
            turns=[
                {
                    "tool": "inspect_record",
                    "arguments": {"record": {"mode": "quick"}, "note": None},
                },
                {"text": "ok"},
            ]
        )
    )
    agent.register_tool(inspect_record)
    assert agent.capabilities.supports_parallel_tool_calls
    assert agent.list_tools()[0]["name"] == "inspect_record"
    result = await agent.run("hi")
    output = next(message for message in result.output if message["role"] == "tool")
    assert output["content"] == [{"text": "model-facing result"}]


@pytest.mark.asyncio
async def test_deepseek_web_tools_use_rust_registry_and_policy() -> None:
    requests = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            requests.append(
                (
                    self.path,
                    json.loads(self.rfile.read(int(self.headers["content-length"]))),
                )
            )
            payload = (
                {"results": [{"url": "https://example.com", "title": "result"}]}
                if self.path.endswith("search")
                else {"title": "page", "content": "source"}
            )
            body = json.dumps(payload).encode()
            self.send_response(200)
            self.send_header("content-type", "application/json")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        config = DeepSeekWebTools(
            "test-secret", f"http://127.0.0.1:{server.server_port}/v1"
        )
        assert "test-secret" not in repr(config)
        agent = Agent(
            ControlledProvider(
                turns=[
                    {"tool": "web_search", "arguments": {"query": "rust"}},
                    {"tool": "web_fetch", "arguments": {"url": "https://example.com"}},
                    {"text": "web done"},
                ]
            ),
            web_tools=config,
            policy=RequireApprovalPolicy(effects=frozenset({ToolEffect.NETWORK})),
        )
        run = await agent.start("search")
        approvals = 0
        async for event in run.events():
            if isinstance(event, OperationRequested):
                assert (await run.snapshot())["state"] == "awaiting_operation"
                await run.resolve_operation(event.operation_id)
                approvals += 1
        assert (await run.result()).text == "web done"
        assert approvals == 2
        assert [path for path, _ in requests] == ["/v1/tools/search", "/v1/tools/fetch"]
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def test_native_stubs_cover_extension_public_members() -> None:
    from aifluxon import _native

    source = Path(__file__).parents[1] / "python/aifluxon/_native.pyi"
    classes = {
        node.name: node
        for node in ast.parse(source.read_text()).body
        if isinstance(node, ast.ClassDef)
    }
    for name in dir(_native):
        if not name.startswith("Native"):
            continue
        assert name in classes
        declared = {
            node.name
            for node in classes[name].body
            if isinstance(node, ast.FunctionDef)
        }
        members = {
            member
            for member in vars(getattr(_native, name))
            if not member.startswith("_")
        }
        assert members <= declared, (name, members - declared)


@pytest.mark.asyncio
async def test_explicit_schema_validates_before_running_python() -> None:
    executions = []

    @tool(
        input_schema={
            "type": "object",
            "properties": {"value": {"type": "integer", "minimum": 1}},
            "required": ["value"],
            "additionalProperties": False,
        }
    )
    def checked(value):
        executions.append(value)
        return value

    agent = Agent(
        ControlledProvider(
            turns=[
                {"tool": "checked", "id": "invalid", "arguments": {"value": 0}},
                {"tool": "checked", "id": "valid", "arguments": {"value": 2}},
                {"text": "recovered"},
            ]
        ),
        tools=[checked],
    )
    result = await agent.run("hi")
    assert executions == [2]
    assert result.text == "recovered"
    assert "validation" in json.dumps(result.output)


class MixedEnum(Enum):
    ONE = 1
    LABEL = "label"


def test_enum_and_positional_only_schema_contracts() -> None:
    @tool
    def choose(value: MixedEnum) -> str:
        return str(value)

    schema = descriptor_from_callable(choose)["input_schema"]["properties"]["value"]
    assert schema == {"enum": [1, "label"]}
    with pytest.raises(TypeError, match="keyword-compatible"):

        @tool(input_schema={"type": "object"})
        def positional(value, /):
            return value


def test_custom_http_client_configuration_is_forwarded_and_redacted() -> None:
    from aifluxon import Custom

    tuning = {"http1_only": False, "connect_timeout_ms": 1000, "read_timeout_ms": 3000,
              "proxy_url": "http://user:proxy-secret@localhost:8080"}
    provider = Custom("local", base_url="http://localhost/v1", http_client=tuning)
    assert provider.to_spec()["http_client"] == tuning
    assert "proxy-secret" not in repr(provider)
    with pytest.raises(InvalidConfigurationError, match="http_client"):
        Agent(Custom("local", base_url="http://localhost/v1", http_client={"read_timeout_ms": 0}))


def test_structured_failed_event_preserves_metadata() -> None:
    from aifluxon.events import event_from_payload

    event = event_from_payload({"sequence": 2, "run_id": "run", "type": "failed", "message": "slow down",
                                "error_code": "PROVIDER_RATE_LIMITED", "error_kind": "RateLimited",
                                "http_status": 429, "retry_after_ms": 2000})
    assert event.error_code == "PROVIDER_RATE_LIMITED"
    assert event.error_kind == "RateLimited"
    assert event.http_status == 429
    assert event.retry_after_ms == 2000
