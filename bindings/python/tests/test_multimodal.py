from __future__ import annotations

import json
import threading
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

import pytest
from aifluxon import (
    Agent,
    Custom,
    DeepSeek,
    ImageInput,
    InvalidRequestError,
    ToolEffect,
    tool,
)

ResponseFactory = Callable[[list[dict[str, Any]], str], str]


@contextmanager
def provider_server(
    factory: ResponseFactory,
) -> Iterator[tuple[str, list[dict[str, Any]]]]:
    requests: list[dict[str, Any]] = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:
            length = int(self.headers.get("content-length", "0"))
            payload = json.loads(self.rfile.read(length))
            requests.append(payload)
            body = factory(requests, self.path).encode()
            self.send_response(200)
            self.send_header("content-type", "text/event-stream")
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, format: str, *args: object) -> None:
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        host, port = server.server_address
        yield f"http://{host}:{port}/v1", requests
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


def chat_text(text: str = "ok") -> str:
    payload = {"choices": [{"delta": {"content": text}, "finish_reason": "stop"}]}
    return f"data: {json.dumps(payload)}\n\ndata: [DONE]\n\n"


def responses_text(text: str = "ok") -> str:
    delta = {"type": "response.output_text.delta", "sequence_number": 1, "delta": text}
    completed = {
        "type": "response.completed",
        "sequence_number": 2,
        "response": {"status": "completed", "output": []},
    }
    return f"data: {json.dumps(delta)}\n\ndata: {json.dumps(completed)}\n\n"


@pytest.mark.asyncio
async def test_python_deepseek_chat_accepts_file_id_image() -> None:
    with provider_server(lambda _requests, _path: chat_text()) as (base_url, requests):
        agent = Agent(
            DeepSeek(
                "deepseek-v4-flash-vision-exp",
                api_key="test-key",
                base_url=base_url,
                api_mode="chat_completions",
            )
        )
        result = await agent.run(
            [
                "Inspect this image.",
                ImageInput.from_file_id("file-api-image-1", "image/png"),
            ]
        )

    assert result.text == "ok"
    assert requests[0]["messages"][0]["content"] == [
        {"type": "text", "text": "Inspect this image."},
        {"type": "file", "file_id": "file-api-image-1"},
    ]


@pytest.mark.asyncio
async def test_python_deepseek_responses_accepts_url_bytes_and_local_file(
    tmp_path: Path,
) -> None:
    local_image = tmp_path / "sample.webp"
    local_image.write_bytes(b"local-image")
    with provider_server(lambda _requests, _path: responses_text()) as (
        base_url,
        requests,
    ):
        agent = Agent(
            DeepSeek(
                "deepseek-v4-flash-vision-exp",
                api_key="test-key",
                base_url=base_url,
                api_mode="responses",
            )
        )
        result = await agent.run(
            [
                "Compare the inputs.",
                ImageInput.from_url("https://example.com/input.png", "image/png"),
                ImageInput.from_bytes(b"gif-image", "image/gif"),
                ImageInput.from_file(local_image),
            ]
        )

    assert result.text == "ok"
    content = requests[0]["input"][0]["content"]
    assert content[0] == {"type": "input_text", "text": "Compare the inputs."}
    assert content[1] == {
        "type": "input_image",
        "image_url": "https://example.com/input.png",
    }
    assert content[2]["type"] == "input_image"
    assert content[2]["image_url"].startswith("data:image/gif;base64,")
    assert content[3]["image_url"].startswith("data:image/webp;base64,")


@pytest.mark.asyncio
async def test_python_tool_can_return_image_content_to_deepseek_responses() -> None:
    executions = 0

    def factory(requests: list[dict[str, Any]], _path: str) -> str:
        if len(requests) <= 2:
            item = {
                "type": "function_call",
                "call_id": "call_view_image",
                "name": "view_image",
                "arguments": "{}",
            }
            added = {
                "type": "response.output_item.added",
                "sequence_number": 1,
                "output_index": 0,
                "item": item,
            }
            completed = {
                "type": "response.completed",
                "sequence_number": 2,
                "response": {"status": "completed", "output": [item]},
            }
            return f"data: {json.dumps(added)}\n\ndata: {json.dumps(completed)}\n\n"
        return responses_text("described")

    @tool(description="Return a viewed image.", effect=ToolEffect.PURE_READ)
    def view_image() -> list[str | ImageInput]:
        nonlocal executions
        executions += 1
        return [
            "Rendered image",
            ImageInput.from_url("https://example.com/tool.webp", "image/webp"),
        ]

    with provider_server(factory) as (base_url, requests):
        agent = Agent(
            DeepSeek(
                "deepseek-v4-flash-vision-exp",
                api_key="test-key",
                base_url=base_url,
                api_mode="responses",
            ),
            tools=[view_image],
        )
        result = await agent.run("View the image.")

    assert result.text == "described"
    assert executions == 1
    assert len(requests) == 3
    for request in requests[1:]:
        output = request["input"][-1]
        assert output["type"] == "function_call_output"
        assert output["call_id"] == "call_view_image"
        assert output["output"] == [
            {"type": "input_text", "text": "Rendered image"},
            {"type": "input_image", "image_url": "https://example.com/tool.webp"},
        ]


@pytest.mark.parametrize("api_mode", ["chat_completions", "responses"])
@pytest.mark.asyncio
async def test_python_custom_preserves_provider_call_id(api_mode: str) -> None:
    call_id = f"call_custom_{api_mode}"

    def factory(requests: list[dict[str, Any]], _path: str) -> str:
        if len(requests) > 1:
            return (
                responses_text("done") if api_mode == "responses" else chat_text("done")
            )
        if api_mode == "responses":
            item = {
                "type": "function_call",
                "call_id": call_id,
                "name": "echo",
                "arguments": '{"value":"x"}',
            }
            added = {
                "type": "response.output_item.added",
                "sequence_number": 1,
                "output_index": 0,
                "item": item,
            }
            completed = {
                "type": "response.completed",
                "sequence_number": 2,
                "response": {"status": "completed", "output": [item]},
            }
            return f"data: {json.dumps(added)}\n\ndata: {json.dumps(completed)}\n\n"
        payload = {
            "choices": [
                {
                    "delta": {
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": call_id,
                                "function": {
                                    "name": "echo",
                                    "arguments": '{"value":"x"}',
                                },
                            }
                        ]
                    },
                    "finish_reason": "tool_calls",
                }
            ]
        }
        return f"data: {json.dumps(payload)}\n\ndata: [DONE]\n\n"

    @tool(description="Echo a value.", effect=ToolEffect.PURE_READ)
    def echo(value: str) -> str:
        return value

    with provider_server(factory) as (base_url, requests):
        result = await Agent(
            Custom(
                "custom-model",
                base_url=base_url,
                api_key="test-key",
                api_mode=api_mode,
            ),
            tools=[echo],
        ).run("Call echo.")

    assert result.text == "done"
    if api_mode == "responses":
        output = requests[1]["input"][-1]
        assert output["type"] == "function_call_output"
        assert output["call_id"] == call_id
    else:
        assert requests[1]["messages"][-1]["tool_call_id"] == call_id


def test_image_input_validation_is_explicit(tmp_path: Path) -> None:
    with pytest.raises(InvalidRequestError):
        ImageInput("", "image/png")
    with pytest.raises(InvalidRequestError):
        ImageInput.from_url("file:///tmp/image.png", "image/png")
    with pytest.raises(InvalidRequestError):
        ImageInput.from_bytes(b"", "image/png")
    with pytest.raises(InvalidRequestError):
        ImageInput.from_file(tmp_path / "missing.png")
    with pytest.raises(InvalidRequestError):
        ImageInput("https://example.com/input.pdf", "application/pdf")
    with pytest.raises(InvalidRequestError):
        ImageInput("data:image/jpeg;base64,AAAA", "image/png")
