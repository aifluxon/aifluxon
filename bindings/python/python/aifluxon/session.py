from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


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
