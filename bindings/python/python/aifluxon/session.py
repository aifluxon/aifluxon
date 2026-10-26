from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .messages import Message


@dataclass(frozen=True, slots=True)
class SessionRecord:
    id: str
    revision: int
    created_at: int
    updated_at: int
    messages: tuple[Message, ...]

    @property
    def message_count(self) -> int:
        return len(self.messages)

    @classmethod
    def from_dict(cls, value: dict[str, Any]) -> SessionRecord:
        return cls(
            id=value["id"],
            revision=value["revision"],
            created_at=value["created_at"],
            updated_at=value["updated_at"],
            messages=tuple(Message.from_dict(message) for message in value["messages"]),
        )


class SessionStore:
    """Marker for Agent session persistence configuration."""

    path: str | None = None


class InMemorySessionStore(SessionStore):
    """Default store. Sessions exist only for the lifetime of the Agent process."""

    path = None


@dataclass(frozen=True, slots=True)
class JsonFileSessionStore(SessionStore):
    """Standalone JSON session store rooted at `path`.

    Layout:

        <path>/sessions/index.json
        <path>/sessions/records/<session-uuid>.json
        <path>/sessions/store.lock
        <path>/provider-state/<session-uuid>--<provider>.json
    """

    path: str

    def __post_init__(self) -> None:
        object.__setattr__(self, "path", str(Path(self.path)))
