from __future__ import annotations

import asyncio
import inspect
import json
from collections.abc import AsyncIterator, Callable, Sequence
from dataclasses import dataclass
from typing import Any

from . import _native
from .errors import CancelledError, FailedError, InvalidConfigurationError, call_native
from .events import Completed, Event, Failed, event_from_payload, is_terminal
from .providers import ProviderConfig
from .auth import CodexProvider
from .session import InMemorySessionStore, JsonFileSessionStore, SessionStore
from .thinking import ThinkingSettings, _UNSET, merge_thinking_settings, thinking_settings
from .tools import AllowAllPolicy, descriptor_from_callable


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
    ) -> None:
        store = store or InMemorySessionStore()
        store_path = store.path if isinstance(store, JsonFileSessionStore) else None
        policy_callback = None if policy is None or isinstance(policy, AllowAllPolicy) else _policy_bridge(policy)
        oauth_provider = provider._native if isinstance(provider, CodexProvider) else None
        spec = "{}" if oauth_provider is not None else json.dumps(provider.to_spec())
        self._native = call_native(
            _native.NativeAgent,
            spec,
            store_path,
            policy_callback,
            max_model_rounds,
            max_tool_invocations,
            oauth_provider,
        )
        for fn in tools or ():
            self._register_tool(fn)
        self._store = store
        self._system_prompt = _normalize_system_prompt(system_prompt)
        self._thinking = thinking_settings(
            reasoning_effort=reasoning_effort,
            thinking=thinking,
            thinking_budget=thinking_budget,
        )

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
        prompt: str,
        *,
        session_id: str | None = None,
        system_prompt: Any = _UNSET,
        reasoning_effort: Any = _UNSET,
        thinking: Any = _UNSET,
        thinking_budget: Any = _UNSET,
    ) -> Run:
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
        run_id = await asyncio.to_thread(
            call_native,
            self._native.start,
            prompt,
            session_id,
            json.dumps(settings.to_payload()),
            prompt_system,
        )
        return Run(self, run_id, session_id)

    async def run(
        self,
        prompt: str,
        *,
        session_id: str | None = None,
        system_prompt: Any = _UNSET,
        reasoning_effort: Any = _UNSET,
        thinking: Any = _UNSET,
        thinking_budget: Any = _UNSET,
    ) -> RunResult:
        handle = await self.start(
            prompt,
            session_id=session_id,
            system_prompt=system_prompt,
            reasoning_effort=reasoning_effort,
            thinking=thinking,
            thinking_budget=thinking_budget,
        )
        return await handle.result()

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
            await asyncio.to_thread(call_native, self._native.open_or_create_session, session_id)
        )
        return Session(self, str(record["id"]), revision=int(record["revision"]))

    async def list_sessions(self) -> list[dict[str, Any]]:
        return json.loads(await asyncio.to_thread(call_native, self._native.list_sessions))

    async def delete_session(self, session_id: str) -> None:
        await asyncio.to_thread(call_native, self._native.delete_session, session_id)

    def _register_tool(self, fn: Callable[..., Any]) -> None:
        descriptor = descriptor_from_callable(fn)
        call_native(
            self._native.register_tool,
            json.dumps(descriptor),
            _tool_bridge(fn),
        )


class Session:
    """A logical conversation. One session may contain many runs."""

    def __init__(self, agent: Agent, session_id: str, *, revision: int | None = None) -> None:
        self._agent = agent
        self.id = session_id
        self.revision = revision

    async def start(
        self,
        prompt: str,
        *,
        system_prompt: Any = _UNSET,
        reasoning_effort: Any = _UNSET,
        thinking: Any = _UNSET,
        thinking_budget: Any = _UNSET,
    ) -> Run:
        return await self._agent.start(
            prompt,
            session_id=self.id,
            system_prompt=system_prompt,
            reasoning_effort=reasoning_effort,
            thinking=thinking,
            thinking_budget=thinking_budget,
        )

    async def run(
        self,
        prompt: str,
        *,
        system_prompt: Any = _UNSET,
        reasoning_effort: Any = _UNSET,
        thinking: Any = _UNSET,
        thinking_budget: Any = _UNSET,
    ) -> RunResult:
        return await self._agent.run(
            prompt,
            session_id=self.id,
            system_prompt=system_prompt,
            reasoning_effort=reasoning_effort,
            thinking=thinking,
            thinking_budget=thinking_budget,
        )


class Run:
    """One agent run."""

    def __init__(self, agent: Agent, run_id: str, session_id: str | None) -> None:
        self._agent = agent
        self.id = run_id
        self.session_id = session_id
        self._queue: asyncio.Queue[Event | None] = asyncio.Queue(maxsize=128)
        self._result: asyncio.Future[RunResult] = asyncio.get_running_loop().create_future()
        self._pump = asyncio.get_running_loop().create_task(self._pump_events())

    async def events(self) -> AsyncIterator[Event]:
        while True:
            item = await self._queue.get()
            if item is None:
                return
            yield item

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
                if isinstance(event, Completed):
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
        await self._queue.put(None)
        if self._result.done():
            return
        if state == "cancelled":
            self._result.set_exception(CancelledError("The run was cancelled.", kind="Cancelled"))
        elif state == "failed":
            self._result.set_exception(
                FailedError(failure or "The run failed.", kind="Failed")
            )
        else:
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


def _tool_bridge(fn: Callable[..., Any]) -> Callable[[str], str]:
    is_async = inspect.iscoroutinefunction(fn)

    def call(arguments_json: str) -> str:
        arguments = json.loads(arguments_json) if arguments_json else {}
        if not isinstance(arguments, dict):
            raise TypeError("Tool arguments must be a JSON object.")
        if is_async:
            try:
                loop = asyncio.get_running_loop()
            except RuntimeError:
                result = asyncio.run(fn(**arguments))
            else:
                result = asyncio.run_coroutine_threadsafe(fn(**arguments), loop).result()
        else:
            result = fn(**arguments)
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
    if result is None:
        return {"ok": True}
    if isinstance(result, (dict, list, str, int, float, bool)):
        return result if isinstance(result, dict) else {"result": result}
    return {"result": str(result)}
