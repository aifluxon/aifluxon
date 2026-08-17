from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping


@dataclass(frozen=True, slots=True)
class RunStarted:
    sequence: int
    run_id: str
    session_id: str | None = None
    parent_run_id: str | None = None
    type: str = "run_started"


@dataclass(frozen=True, slots=True)
class StateChanged:
    sequence: int
    run_id: str
    state: str
    type: str = "state_changed"


@dataclass(frozen=True, slots=True)
class TextDelta:
    sequence: int
    run_id: str
    delta: str
    type: str = "text_delta"


@dataclass(frozen=True, slots=True)
class ReasoningDelta:
    sequence: int
    run_id: str
    delta: str
    type: str = "reasoning_delta"


@dataclass(frozen=True, slots=True)
class ToolStarted:
    sequence: int
    run_id: str
    invocation_id: str
    name: str
    arguments: Any = None
    type: str = "tool_started"


@dataclass(frozen=True, slots=True)
class ToolFinished:
    sequence: int
    run_id: str
    invocation_id: str
    name: str
    result: Any
    type: str = "tool_finished"


@dataclass(frozen=True, slots=True)
class OperationRequested:
    sequence: int
    run_id: str
    operation: Mapping[str, Any]
    type: str = "operation_requested"

    @property
    def operation_id(self) -> str:
        return str(self.operation["id"])

    @property
    def mode(self) -> str:
        return str(self.operation.get("mode") or "")


@dataclass(frozen=True, slots=True)
class UsageUpdated:
    sequence: int
    run_id: str
    usage: Any
    type: str = "usage_updated"


@dataclass(frozen=True, slots=True)
class ArtifactProduced:
    sequence: int
    run_id: str
    artifact: str
    type: str = "artifact_produced"


@dataclass(frozen=True, slots=True)
class Completed:
    sequence: int
    run_id: str
    output: list[Any]
    type: str = "completed"


@dataclass(frozen=True, slots=True)
class Failed:
    sequence: int
    run_id: str
    message: str
    type: str = "failed"


@dataclass(frozen=True, slots=True)
class Cancelled:
    sequence: int
    run_id: str
    type: str = "cancelled"


Event = (
    RunStarted
    | StateChanged
    | TextDelta
    | ReasoningDelta
    | ToolStarted
    | ToolFinished
    | OperationRequested
    | UsageUpdated
    | ArtifactProduced
    | Completed
    | Failed
    | Cancelled
)

_TERMINAL_TYPES = {"completed", "failed", "cancelled"}


def is_terminal(event: Event) -> bool:
    return event.type in _TERMINAL_TYPES


def event_from_payload(payload: Mapping[str, Any]) -> Event:
    sequence = int(payload["sequence"])
    run_id = str(payload["run_id"])
    event_type = str(payload.get("type") or "")
    if event_type == "run_started":
        return RunStarted(
            sequence=sequence,
            run_id=run_id,
            session_id=payload.get("session_id"),
            parent_run_id=payload.get("parent_run_id"),
        )
    if event_type == "state_changed":
        return StateChanged(sequence=sequence, run_id=run_id, state=str(payload.get("state") or ""))
    if event_type == "text_delta":
        return TextDelta(sequence=sequence, run_id=run_id, delta=str(payload.get("delta") or ""))
    if event_type == "reasoning_delta":
        return ReasoningDelta(
            sequence=sequence, run_id=run_id, delta=str(payload.get("delta") or "")
        )
    if event_type == "tool_started":
        return ToolStarted(
            sequence=sequence,
            run_id=run_id,
            invocation_id=str(payload.get("invocation_id") or ""),
            name=str(payload.get("name") or ""),
            arguments=payload.get("arguments"),
        )
    if event_type == "tool_finished":
        return ToolFinished(
            sequence=sequence,
            run_id=run_id,
            invocation_id=str(payload.get("invocation_id") or ""),
            name=str(payload.get("name") or ""),
            result=payload.get("result"),
        )
    if event_type == "operation_requested":
        return OperationRequested(
            sequence=sequence,
            run_id=run_id,
            operation=payload.get("operation") or {},
        )
    if event_type == "usage_updated":
        return UsageUpdated(sequence=sequence, run_id=run_id, usage=payload.get("usage"))
    if event_type == "artifact_produced":
        return ArtifactProduced(
            sequence=sequence, run_id=run_id, artifact=str(payload.get("artifact") or "")
        )
    if event_type == "completed":
        output = payload.get("output") or []
        return Completed(sequence=sequence, run_id=run_id, output=list(output))
    if event_type == "failed":
        return Failed(sequence=sequence, run_id=run_id, message=str(payload.get("message") or ""))
    if event_type == "cancelled":
        return Cancelled(sequence=sequence, run_id=run_id)
    raise ValueError(f"Unknown run event type `{event_type}`.")
