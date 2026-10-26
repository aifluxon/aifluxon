from __future__ import annotations

from collections.abc import Collection, Sequence
from dataclasses import asdict, dataclass
from typing import Any, Literal, NoReturn

from .errors import InvalidConfigurationError

_U32_MAX = 2**32 - 1


def _invalid(message: str) -> NoReturn:
    raise InvalidConfigurationError(message, kind="InvalidConfiguration")


@dataclass(frozen=True, slots=True)
class RunLimits:
    """Rust run budgets. Use unlimited() to disable all loop and time budgets."""

    max_model_rounds: int = 32
    max_tool_invocations: int = 64

    def __post_init__(self) -> None:
        for name, value in asdict(self).items():
            if type(value) is not int or not 0 <= value <= _U32_MAX:
                _invalid(f"{name} must be an integer between 0 and {_U32_MAX}.")

    @classmethod
    def unlimited(cls) -> RunLimits:
        return cls(_U32_MAX, _U32_MAX)

    @property
    def is_unlimited(self) -> bool:
        return self == self.unlimited()


@dataclass(frozen=True, slots=True)
class ProviderFeatures:
    """Provider feature flags; reasoning options remain on Agent/start/run."""

    web_search: bool = False
    image_generation: bool = False
    prompt_cache_key: str | None = None
    explicit_cache: bool = False

    def __post_init__(self) -> None:
        for name in ("web_search", "image_generation", "explicit_cache"):
            if type(getattr(self, name)) is not bool:
                _invalid(f"{name} must be a boolean.")
        if self.prompt_cache_key is not None and (
            not isinstance(self.prompt_cache_key, str)
            or not self.prompt_cache_key.strip()
        ):
            _invalid("prompt_cache_key must be a non-empty string or None.")


@dataclass(frozen=True, slots=True)
class ExecutionAuthority:
    """Explicit host grants checked by Rust before tool policy evaluation."""

    capabilities: Collection[str] = frozenset()
    permission_mode: Literal["default", "managed", "trusted"] = "default"
    filesystem_scope: Literal[
        "workspace", "isolated", "scoped_selection", "trusted"
    ] = "isolated"

    def __post_init__(self) -> None:
        if self.permission_mode not in {"default", "managed", "trusted"}:
            _invalid("Unknown permission_mode.")
        if self.filesystem_scope not in {
            "workspace",
            "isolated",
            "scoped_selection",
            "trusted",
        }:
            _invalid("Unknown filesystem_scope.")
        if isinstance(self.capabilities, str):
            _invalid("capabilities must be a collection of strings.")
        values = frozenset(self.capabilities)
        if any(not isinstance(item, str) or not item.strip() for item in values):
            _invalid("capabilities must contain non-empty strings.")
        object.__setattr__(self, "capabilities", values)

    def to_payload(self) -> dict[str, Any]:
        return {
            "capabilities": sorted(self.capabilities),
            "permission_mode": self.permission_mode,
            "filesystem_scope": self.filesystem_scope,
        }


@dataclass(frozen=True, slots=True)
class RunOptions:
    """Complete defaults for a run. Per-run options replace Agent.options."""

    limits: RunLimits = RunLimits()
    features: ProviderFeatures = ProviderFeatures()
    allowed_tools: Sequence[str] | None = None
    authority: ExecutionAuthority | None = None
    session_key: str | None = None

    def __post_init__(self) -> None:
        if not isinstance(self.limits, RunLimits):
            _invalid("limits must be RunLimits.")
        if not isinstance(self.features, ProviderFeatures):
            _invalid("features must be ProviderFeatures.")
        if self.authority is not None and not isinstance(
            self.authority, ExecutionAuthority
        ):
            _invalid("authority must be ExecutionAuthority or None.")
        if self.allowed_tools is not None:
            if isinstance(self.allowed_tools, str):
                _invalid(
                    "allowed_tools must be a collection of tool names, not a string."
                )
            names = tuple(self.allowed_tools)
            if any(not isinstance(name, str) or not name.strip() for name in names):
                _invalid("allowed_tools must contain non-empty strings.")
            object.__setattr__(self, "allowed_tools", names)
        if self.session_key is not None and (
            not isinstance(self.session_key, str) or not self.session_key.strip()
        ):
            _invalid("session_key must be a non-empty string or None.")

    def to_payload(self) -> dict[str, Any]:
        return {
            "limits": asdict(self.limits),
            "features": asdict(self.features),
            "allowed_tools": self.allowed_tools,
            "authority": self.authority.to_payload()
            if self.authority is not None
            else None,
            "session_key": self.session_key,
        }
