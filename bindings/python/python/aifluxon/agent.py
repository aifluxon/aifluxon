from __future__ import annotations

import asyncio
import inspect
import json
from collections.abc import AsyncIterator, Callable, Mapping, Sequence
from dataclasses import dataclass
from typing import Any

from . import _native
from .auth import CodexProvider
from .errors import (
    CancelledError,
    FailedError,
    InternalError,
    InvalidConfigurationError,
    call_native,
)
from .events import Completed, Event, Failed, event_from_payload, is_terminal
from .input import ImageInput, PromptInput, normalize_prompt_input
from .messages import Message, normalize_messages
from .options import RunLimits, RunOptions
from .providers import DeepSeekWebTools, ProviderCapabilities, ProviderConfig
from .session import (
    InMemorySessionStore,
    JsonFileSessionStore,
    SessionRecord,
    SessionStore,
)
from .thinking import (
    _UNSET,
    ThinkingSettings,
    merge_thinking_settings,
    thinking_settings,
)
from .tools import AllowAllPolicy, ToolResult, descriptor_from_callable


@dataclass(frozen=True, slots=True)
class RunResult:
    run_id: str
    session_id: str | None
    state: str
    text: str
    output: list[Any]
    usage: Any = None


class Agent:
    """In-process AIFLUXON agent.

    Construction does not start a run. The default store is in-memory and
    performs no filesystem writes.
    """

    def __init__(
        self,
        provider: ProviderConfig | CodexProvider,
        *,
        store: SessionStore | None = None,
        tools: Sequence[Callable[..., Any]] | None = None,
        policy: Any | None = None,
        max_model_rounds: int = 32,
        max_tool_invocations: int = 64,
        system_prompt: str | None = None,
        reasoning_effort: str | None = None,
        thinking: bool | str | None = None,
        thinking_budget: int | str | None = None,
        options: RunOptions | None = None,
        web_tools: DeepSeekWebTools | None = None,
    ) -> None:
        legacy_limits = RunLimits(max_model_rounds, max_tool_invocations)
        if options is not None and not isinstance(options, RunOptions):
            raise InvalidConfigurationError(
                "options must be RunOptions.", kind="InvalidConfiguration"
            )
        if options is not None and legacy_limits != RunLimits():
            raise InvalidConfigurationError(
                "Use options.limits or max_model_rounds/max_tool_invocations, not both.",
                kind="InvalidConfiguration",
            )
        self._options = (
            options if options is not None else RunOptions(limits=legacy_limits)
        )
        self._callback_loop: asyncio.AbstractEventLoop | None = None
        self._system_prompt = _normalize_system_prompt(system_prompt)
        self._thinking = thinking_settings(
            reasoning_effort=reasoning_effort,
            thinking=thinking,
            thinking_budget=thinking_budget,
        )
        store = store or InMemorySessionStore()
        if not isinstance(store, (InMemorySessionStore, JsonFileSessionStore)):
            raise InvalidConfigurationError(
                "Unsupported session store.", kind="InvalidConfiguration"
            )
        store_path = store.path if isinstance(store, JsonFileSessionStore) else None
        policy_callback = (
            None
            if policy is None or isinstance(policy, AllowAllPolicy)
            else _policy_bridge(policy)
        )
        oauth_provider = (
            provider._native if isinstance(provider, CodexProvider) else None
        )
        spec = "{}" if oauth_provider is not None else json.dumps(provider.to_spec())
        self._native = call_native(
            _native.NativeAgent,
            spec,
            store_path,
            policy_callback,
            self._options.limits.max_model_rounds,
            self._options.limits.max_tool_invocations,
            oauth_provider,
        )
        for fn in tools or ():
            self._register_tool(fn)
        if web_tools is not None:
            if not isinstance(web_tools, DeepSeekWebTools):
                raise InvalidConfigurationError(
                    "web_tools must be DeepSeekWebTools.", kind="InvalidConfiguration"
                )
            call_native(
                self._native.register_web_tools, web_tools.base_url, web_tools.api_key
            )
        self._store = store

    @property
    def options(self) -> RunOptions:
        return self._options

    @property
    def capabilities(self) -> ProviderCapabilities:
        return ProviderCapabilities(
            **json.loads(call_native(self._native.capabilities))
        )

    def register_tool(self, fn: Callable[..., Any]) -> None:
        """Register a decorated tool for subsequent runs."""
        self._register_tool(fn)

    def list_tools(self) -> list[dict[str, Any]]:
        return json.loads(call_native(self._native.tool_descriptors))

    @property
    def provider_id(self) -> str:
        return self._native.provider_id()

    @property
    def model(self) -> str:
        return self._native.model()

    @property
    def system_prompt(self) -> str | None:
        return self._system_prompt

    @property
    def thinking_settings(self) -> ThinkingSettings:
        return self._thinking

    @property
    def reasoning_effort(self) -> str | None:
        return self._thinking.reasoning_effort

    @property
    def thinking(self) -> str | None:
        return self._thinking.thinking_mode

    @property
    def thinking_budget(self) -> str | None:
        return self._thinking.thinking_budget

    async def start(
        self,
        prompt: PromptInput,
        *,
        session_id: str | None = None,
        system_prompt: Any = _UNSET,
        reasoning_effort: Any = _UNSET,
        thinking: Any = _UNSET,
        thinking_budget: Any = _UNSET,
        options: RunOptions | None = None,
    ) -> Run:
        normalize_prompt_input(prompt)
        return await self.start_messages(
            [Message("user", prompt)],
            session_id=session_id,
            system_prompt=system_prompt,
            reasoning_effort=reasoning_effort,
            thinking=thinking,
            thinking_budget=thinking_budget,
            options=options,
        )

    async def start_messages(
        self,
        messages: Sequence[Message | Mapping[str, Any]],
        *,
        session_id: str | None = None,
        system_prompt: Any = _UNSET,
        reasoning_effort: Any = _UNSET,
        thinking: Any = _UNSET,
        thinking_budget: Any = _UNSET,
        options: RunOptions | None = None,
    ) -> Run:
        selected = self._options if options is None else options
        loop = asyncio.get_running_loop()
        if (
            self._callback_loop is not None
            and self._callback_loop is not loop
            and self._callback_loop.is_running()
        ):
            raise InvalidConfigurationError(
                "An Agent cannot run on different event loops concurrently.",
                kind="InvalidConfiguration",
            )
        self._callback_loop = loop
        if not isinstance(selected, RunOptions):
            raise InvalidConfigurationError(
                "options must be RunOptions.", kind="InvalidConfiguration"
            )
        settings = merge_thinking_settings(
            self._thinking,
            reasoning_effort=reasoning_effort,
            thinking=thinking,
            thinking_budget=thinking_budget,
        )
        prompt_system = (
            self._system_prompt
            if system_prompt is _UNSET
            else _normalize_system_prompt(system_prompt)
        )
        payload_messages = normalize_messages(messages)
        # Explicit message history owns its leading system messages unless the
        # caller explicitly supplies an override.
        if prompt_system and (
            system_prompt is not _UNSET or payload_messages[0]["role"] != "system"
        ):
            while payload_messages and payload_messages[0]["role"] == "system":
                payload_messages.pop(0)
            payload_messages.insert(0, Message("system", prompt_system).to_dict())
        features = selected.to_payload()["features"]
        features.update(settings.to_payload())
        run_id = await asyncio.to_thread(
            call_native,
            self._native.start_request,
            json.dumps(
                {
                    "messages": payload_messages,
                    "session_id": session_id,
                    "features": features,
                    "options": selected.to_payload(),
                }
            ),
        )
        return Run(self, run_id, session_id)

    async def run(
        self,
        prompt: PromptInput,
        *,
        session_id: str | None = None,
        system_prompt: Any = _UNSET,
        reasoning_effort: Any = _UNSET,
        thinking: Any = _UNSET,
        thinking_budget: Any = _UNSET,
        options: RunOptions | None = None,
    ) -> RunResult:
        handle = await self.start(
            prompt,
            session_id=session_id,
            system_prompt=system_prompt,
            reasoning_effort=reasoning_effort,
            thinking=thinking,
            thinking_budget=thinking_budget,
            options=options,
        )
        return await handle.result()

    async def run_messages(
        self,
        messages: Sequence[Message | Mapping[str, Any]],
        **kwargs: Any,
    ) -> RunResult:
        """start_messages() followed by result(); accepts the same keyword options."""
        return await (await self.start_messages(messages, **kwargs)).result()

    async def get_session(self, session_id: str) -> SessionRecord:
        raw = await asyncio.to_thread(
            call_native, self._native.open_session, session_id
        )
        return SessionRecord.from_dict(json.loads(raw))

    async def create_session(self) -> Session:
        session_id = await asyncio.to_thread(call_native, self._native.create_session)
        return Session(self, session_id)

    async def open_session(self, session_id: str) -> Session:
        record = json.loads(
            await asyncio.to_thread(call_native, self._native.open_session, session_id)
        )
        return Session(self, str(record["id"]), revision=int(record["revision"]))

    async def open_or_create_session(self, session_id: str) -> Session:
        record = json.loads(
            await asyncio.to_thread(
                call_native, self._native.open_or_create_session, session_id
            )
        )
        return Session(self, str(record["id"]), revision=int(record["revision"]))

    async def list_sessions(self) -> list[dict[str, Any]]:
        return json.loads(
            await asyncio.to_thread(call_native, self._native.list_sessions)
        )

    async def delete_session(self, session_id: str) -> None:
        await asyncio.to_thread(call_native, self._native.delete_session, session_id)

    def _register_tool(self, fn: Callable[..., Any]) -> None:
        descriptor = descriptor_from_callable(fn)
        call_native(
            self._native.register_tool,
            json.dumps(descriptor),
            _tool_bridge(fn, lambda: self._callback_loop),
        )


class Session:
    """A logical conversation. One session may contain many runs."""

    def __init__(
        self, agent: Agent, session_id: str, *, revision: int | None = None
    ) -> None:
        self._agent = agent
        self.id = session_id
        self.revision = revision

    async def refresh(self) -> SessionRecord:
        record = await self._agent.get_session(self.id)
        self.revision = record.revision
        return record

    async def history(self) -> tuple[Message, ...]:
        return (await self.refresh()).messages

    async def delete(self) -> None:
        await self._agent.delete_session(self.id)

    async def start_messages(
        self,
        messages: Sequence[Message | Mapping[str, Any]],
        **kwargs: Any,
    ) -> Run:
        return await self._agent.start_messages(messages, session_id=self.id, **kwargs)

    async def run_messages(
        self,
        messages: Sequence[Message | Mapping[str, Any]],
        **kwargs: Any,
    ) -> RunResult:
        return await (await self.start_messages(messages, **kwargs)).result()

    async def start(
        self,
        prompt: PromptInput,
        *,
        system_prompt: Any = _UNSET,
        reasoning_effort: Any = _UNSET,
        thinking: Any = _UNSET,
        thinking_budget: Any = _UNSET,
        options: RunOptions | None = None,
    ) -> Run:
        return await self._agent.start(
            prompt,
            session_id=self.id,
            system_prompt=system_prompt,
            reasoning_effort=reasoning_effort,
            thinking=thinking,
            thinking_budget=thinking_budget,
            options=options,
        )

    async def run(
        self,
        prompt: PromptInput,
        *,
        system_prompt: Any = _UNSET,
        reasoning_effort: Any = _UNSET,
        thinking: Any = _UNSET,
        thinking_budget: Any = _UNSET,
        options: RunOptions | None = None,
    ) -> RunResult:
        return await self._agent.run(
            prompt,
            session_id=self.id,
            system_prompt=system_prompt,
            reasoning_effort=reasoning_effort,
            thinking=thinking,
            thinking_budget=thinking_budget,
            options=options,
        )


class Run:
    """One agent run."""

    def __init__(self, agent: Agent, run_id: str, session_id: str | None) -> None:
        self._agent = agent
        self.id = run_id
        self.session_id = session_id
        # Result completion must never depend on whether the caller consumes events.
        self._queue: asyncio.Queue[Event | None] = asyncio.Queue()
        self._events_active = False
        self._events_finished = False
        self._result: asyncio.Future[RunResult] = (
            asyncio.get_running_loop().create_future()
        )
        self._pump = asyncio.get_running_loop().create_task(self._pump_events())

    async def events(self) -> AsyncIterator[Event]:
        if self._events_active:
            raise RuntimeError("Only one event iterator may consume a run at a time.")
        if self._events_finished:
            return
        self._events_active = True
        try:
            while True:
                item = await self._queue.get()
                if item is None:
                    self._events_finished = True
                    return
                yield item
        finally:
            self._events_active = False

    async def result(self) -> RunResult:
        return await asyncio.shield(self._result)

    async def cancel(self) -> None:
        await asyncio.to_thread(call_native, self._agent._native.cancel, self.id)

    async def snapshot(self) -> dict[str, Any]:
        return json.loads(
            await asyncio.to_thread(call_native, self._agent._native.snapshot, self.id)
        )

    async def resolve_operation(
        self,
        operation_id: str,
        decision: str = "approve",
        *,
        data: Any = None,
        reason: str | None = None,
    ) -> None:
        payload = {"type": decision, "data": data, "reason": reason}
        await asyncio.to_thread(
            call_native,
            self._agent._native.resolve_operation,
            self.id,
            operation_id,
            json.dumps(payload),
        )

    async def commit_operation(self, operation_id: str) -> None:
        await asyncio.to_thread(
            call_native, self._agent._native.commit_operation, self.id, operation_id
        )

    async def _pump_events(self) -> None:
        output: list[Any] = []
        usage = None
        text = ""
        state = "running"
        failure: str | None = None
        try:
            while True:
                raw = await asyncio.to_thread(
                    call_native, self._agent._native.next_event, self.id
                )
                if raw is None:
                    break
                event = event_from_payload(json.loads(raw))
                if event.type == "run_started":
                    self.session_id = event.session_id
                elif isinstance(event, Completed):
                    output = event.output
                    text = _assistant_text(output)
                    state = "completed"
                elif isinstance(event, Failed):
                    failure = event.message
                    state = "failed"
                elif event.type == "cancelled":
                    state = "cancelled"
                elif event.type == "usage_updated":
                    usage = event.usage  # type: ignore[attr-defined]
                await self._queue.put(event)
                if is_terminal(event):
                    break
        except Exception as error:
            if not self._result.done():
                self._result.set_exception(error)
            await self._queue.put(None)
            return
        finally:
            call_native(self._agent._native.release_run, self.id)
        await self._queue.put(None)
        if self._result.done():
            return
        if state == "cancelled":
            self._result.set_exception(
                CancelledError("The run was cancelled.", kind="Cancelled")
            )
        elif state == "failed":
            self._result.set_exception(
                FailedError(failure or "The run failed.", kind="Failed")
            )
        elif state == "completed":
            self._result.set_result(
                RunResult(
                    run_id=self.id,
                    session_id=self.session_id,
                    state=state,
                    text=text,
                    output=output,
                    usage=usage,
                )
            )
        else:
            self._result.set_exception(
                InternalError(
                    "The run event stream ended without a terminal outcome.",
                    kind="Internal",
                )
            )


def _assistant_text(output: list[Any]) -> str:
    for message in reversed(output):
        if not isinstance(message, dict):
            continue
        if message.get("role") != "assistant":
            continue
        parts = []
        for part in message.get("content") or []:
            if isinstance(part, dict) and part.get("text"):
                parts.append(str(part["text"]))
            elif isinstance(part, str):
                parts.append(part)
        return "".join(parts)
    return ""


def _tool_bridge(
    fn: Callable[..., Any],
    get_loop: Callable[[], asyncio.AbstractEventLoop | None],
) -> Callable[[str], str]:
    def call(arguments_json: str) -> str:
        arguments = json.loads(arguments_json) if arguments_json else {}
        if not isinstance(arguments, dict):
            raise TypeError("Tool arguments must be a JSON object.")
        result = fn(**arguments)
        if inspect.isawaitable(result):

            async def resolve() -> Any:
                return await result

            loop = get_loop()
            if loop is not None and loop.is_running():
                result = asyncio.run_coroutine_threadsafe(resolve(), loop).result()
            else:
                result = asyncio.run(resolve())
        return json.dumps(_normalize_tool_result(result))

    return call


def _policy_bridge(policy: Any) -> Callable[[str, str, str], str]:
    def call(name: str, arguments_json: str, effect: str) -> str:
        arguments = json.loads(arguments_json) if arguments_json else {}
        decision = policy.evaluate(name, arguments, effect)
        if not isinstance(decision, dict):
            raise TypeError("ToolPolicy.evaluate must return a dict.")
        return json.dumps(decision)

    return call


def _normalize_system_prompt(value: Any) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise InvalidConfigurationError(
            "system_prompt must be a string.",
            kind="InvalidConfiguration",
        )
    text = value.strip()
    return text or None


def _normalize_tool_result(result: Any) -> Any:
    if isinstance(result, ToolResult):
        if result.content is None:
            return result.value
        content = normalize_prompt_input(result.content)
        if isinstance(content, str):
            content = [{"type": "text", "text": content}]
        return {"$aifluxon_tool_result": {"content": content, "value": result.value}}
    multimodal = _normalize_tool_content(result)
    if multimodal is not None:
        return {
            "$aifluxon_tool_result": {
                "content": multimodal,
                "value": {"content": multimodal},
            }
        }
    if result is None:
        return {"ok": True}
    if isinstance(result, (dict, list, str, int, float, bool)):
        return result if isinstance(result, dict) else {"result": result}
    return {"result": str(result)}


def _normalize_tool_content(result: Any) -> list[dict[str, str]] | None:
    if isinstance(result, ImageInput):
        return [result._to_payload()]
    if isinstance(result, Sequence) and not isinstance(
        result, (str, bytes, bytearray, memoryview, dict)
    ):
        parts = normalize_prompt_input(result)
        if isinstance(parts, list) and any(part["type"] == "image" for part in parts):
            return parts
    return None
