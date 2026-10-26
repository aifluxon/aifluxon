from __future__ import annotations

from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from typing import Any, Literal, NoReturn
from uuid import UUID, uuid4

from .errors import InvalidRequestError
from .input import ImageInput, PromptInput, normalize_prompt_input


def _invalid(message: str) -> NoReturn:
    raise InvalidRequestError(message, kind="InvalidRequest")


def _uuid(value: str) -> str:
    try:
        return str(UUID(value))
    except (ValueError, TypeError, AttributeError):
        _invalid(
            "Tool invocation IDs must be UUID strings; use provider_call_id for wire IDs."
        )


@dataclass(frozen=True, slots=True)
class ToolCall:
    name: str
    arguments: Mapping[str, Any] = field(default_factory=dict)
    id: str = field(default_factory=lambda: str(uuid4()))
    provider_call_id: str | None = None

    def to_dict(self) -> dict[str, Any]:
        if not isinstance(self.name, str) or not self.name.strip():
            _invalid("ToolCall.name must be a non-empty string.")
        if not isinstance(self.arguments, Mapping):
            _invalid("ToolCall.arguments must be a mapping.")
        return {
            "id": _uuid(self.id),
            "name": self.name,
            "arguments": dict(self.arguments),
            "provider_call_id": self.provider_call_id,
        }


@dataclass(frozen=True, slots=True)
class Message:
    """Canonical Rust message, including opaque provider state for lossless replay."""

    role: Literal["system", "user", "assistant", "tool"]
    content: PromptInput = ""
    tool_calls: Sequence[ToolCall] = ()
    tool_call_id: str | None = None
    provider_state: Any = None

    def to_dict(self) -> dict[str, Any]:
        if self.role not in {"system", "user", "assistant", "tool"}:
            _invalid(f"Unknown message role: {self.role!r}.")
        if (
            not isinstance(self.content, str)
            and isinstance(self.content, Sequence)
            and not self.content
        ):
            parts = []
        else:
            normalized = normalize_prompt_input(self.content)
            parts = (
                [{"type": "text", "text": normalized}]
                if isinstance(normalized, str)
                else normalized
            )
        content = [
            {"text": part["text"]}
            if part["type"] == "text"
            else {
                "image": {"artifact": part["reference"], "mime_type": part["mime_type"]}
            }
            for part in parts
        ]
        return {
            "role": self.role,
            "content": content,
            "tool_calls": [call.to_dict() for call in self.tool_calls],
            "tool_call_id": _uuid(self.tool_call_id)
            if self.tool_call_id is not None
            else None,
            "provider_state": self.provider_state,
        }

    @classmethod
    def from_dict(cls, value: Mapping[str, Any]) -> Message:
        """Read a canonical message from RunResult.output or a session record."""
        try:
            parts = []
            for part in value.get("content", []):
                if "text" in part:
                    parts.append(part["text"])
                elif "image" in part:
                    image = part["image"]
                    parts.append(ImageInput(image["artifact"], image["mime_type"]))
                else:
                    _invalid("Unknown canonical content part.")
            message = cls(
                role=value["role"],
                content=parts,
                tool_calls=tuple(
                    ToolCall(**call) for call in value.get("tool_calls", [])
                ),
                tool_call_id=value.get("tool_call_id"),
                provider_state=value.get("provider_state"),
            )
            message.to_dict()
            return message
        except (KeyError, TypeError) as error:
            _invalid(f"Invalid canonical message: {error}.")


def normalize_messages(
    messages: Sequence[Message | Mapping[str, Any]],
) -> list[dict[str, Any]]:
    if (
        isinstance(messages, (str, bytes))
        or not isinstance(messages, Sequence)
        or not messages
    ):
        _invalid(
            "messages must be a non-empty sequence of Message objects or canonical mappings."
        )
    result = []
    wire_ids: dict[str, str] = {}
    for message in messages:
        if isinstance(message, Mapping):
            message = Message.from_dict(message)
        if not isinstance(message, Message):
            _invalid("Each message must be a Message or canonical mapping.")
        payload = message.to_dict()
        for call in payload["tool_calls"]:
            wire_ids[call["id"]] = call["provider_call_id"] or call["id"]
        if payload["role"] == "tool" and payload["tool_call_id"] in wire_ids:
            state = payload["provider_state"]
            if state is None or isinstance(state, dict):
                payload["provider_state"] = {
                    "call_id": wire_ids[payload["tool_call_id"]],
                    **(state or {}),
                }
        result.append(payload)
    return result
