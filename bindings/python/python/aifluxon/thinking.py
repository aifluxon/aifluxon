from __future__ import annotations

from dataclasses import dataclass
from typing import Any

from .errors import InvalidConfigurationError

REASONING_EFFORTS = (
    "default",
    "none",
    "minimal",
    "low",
    "medium",
    "high",
    "xhigh",
    "max",
)

THINKING_MODES = ("enabled", "disabled", "default")

_UNSET = object()


@dataclass(frozen=True, slots=True)
class ThinkingSettings:
    """Host-selected thinking options forwarded as `ProviderFeatureRequest`.

    Unused fields are ignored by families that do not consume them. Kimi
    enables thinking from the model name and does not read these values.
    """

    reasoning_effort: str | None = None
    thinking_mode: str | None = None
    thinking_budget: str | None = None

    def to_payload(self) -> dict[str, str]:
        payload: dict[str, str] = {}
        if self.reasoning_effort is not None:
            payload["reasoning_effort"] = self.reasoning_effort
        if self.thinking_mode is not None:
            payload["thinking_mode"] = self.thinking_mode
        if self.thinking_budget is not None:
            payload["thinking_budget"] = self.thinking_budget
        return payload


def normalize_reasoning_effort(value: Any) -> str | None:
    if value is None:
        return None
    text = str(value).strip().lower()
    if not text:
        return None
    if text not in REASONING_EFFORTS:
        allowed = ", ".join(REASONING_EFFORTS)
        raise InvalidConfigurationError(
            f"reasoning_effort must be one of: {allowed}.",
            kind="InvalidConfiguration",
        )
    return text


def normalize_thinking_mode(value: Any) -> str | None:
    if value is None:
        return None
    if value is True:
        return "enabled"
    if value is False:
        return "disabled"
    text = str(value).strip().lower()
    if not text:
        return None
    aliases = {
        "on": "enabled",
        "true": "enabled",
        "enable": "enabled",
        "off": "disabled",
        "false": "disabled",
        "disable": "disabled",
        "none": "disabled",
    }
    text = aliases.get(text, text)
    if text not in THINKING_MODES:
        raise InvalidConfigurationError(
            "thinking must be True/False or enabled/disabled/default.",
            kind="InvalidConfiguration",
        )
    return text


def normalize_thinking_budget(value: Any) -> str | None:
    if value is None or value == "":
        return None
    if isinstance(value, bool):
        raise InvalidConfigurationError(
            "thinking_budget must be a positive integer.",
            kind="InvalidConfiguration",
        )
    if isinstance(value, int):
        if value <= 0:
            raise InvalidConfigurationError(
                "thinking_budget must be a positive integer.",
                kind="InvalidConfiguration",
            )
        return str(value)
    text = str(value).strip()
    if not text:
        return None
    try:
        parsed = int(text)
    except ValueError as error:
        raise InvalidConfigurationError(
            "thinking_budget must be a positive integer.",
            kind="InvalidConfiguration",
        ) from error
    if parsed <= 0:
        raise InvalidConfigurationError(
            "thinking_budget must be a positive integer.",
            kind="InvalidConfiguration",
        )
    return str(parsed)


def thinking_settings(
    *,
    reasoning_effort: Any = None,
    thinking: Any = None,
    thinking_budget: Any = None,
) -> ThinkingSettings:
    return ThinkingSettings(
        reasoning_effort=normalize_reasoning_effort(reasoning_effort),
        thinking_mode=normalize_thinking_mode(thinking),
        thinking_budget=normalize_thinking_budget(thinking_budget),
    )


def merge_thinking_settings(
    base: ThinkingSettings,
    *,
    reasoning_effort: Any = _UNSET,
    thinking: Any = _UNSET,
    thinking_budget: Any = _UNSET,
) -> ThinkingSettings:
    return ThinkingSettings(
        reasoning_effort=base.reasoning_effort
        if reasoning_effort is _UNSET
        else normalize_reasoning_effort(reasoning_effort),
        thinking_mode=base.thinking_mode
        if thinking is _UNSET
        else normalize_thinking_mode(thinking),
        thinking_budget=base.thinking_budget
        if thinking_budget is _UNSET
        else normalize_thinking_budget(thinking_budget),
    )
