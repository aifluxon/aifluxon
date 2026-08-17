from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping, Sequence


@dataclass(frozen=True, slots=True)
class ProviderConfig:
    kind: str
    model: str
    api_key: str | None = None
    base_url: str | None = None
    api_mode: str | None = None
    provider_id: str | None = None
    responses: tuple[str, ...] | None = None
    turns: tuple[Mapping[str, Any], ...] | None = None
    allow_cumulative_delta: bool | None = None
    delay_ms: int | None = None

    def to_spec(self) -> dict[str, Any]:
        spec: dict[str, Any] = {"kind": self.kind, "model": self.model}
        if self.api_key is not None:
            spec["api_key"] = self.api_key
        if self.base_url is not None:
            spec["base_url"] = self.base_url
        if self.api_mode is not None:
            spec["api_mode"] = self.api_mode
        if self.provider_id is not None:
            spec["provider_id"] = self.provider_id
        if self.allow_cumulative_delta is not None:
            spec["allow_cumulative_delta"] = self.allow_cumulative_delta
        if self.responses is not None:
            spec["responses"] = list(self.responses)
        if self.turns is not None:
            spec["turns"] = [dict(item) for item in self.turns]
        if self.delay_ms is not None:
            spec["delay_ms"] = self.delay_ms
        return spec


def OpenAI(
    model: str,
    *,
    api_key: str,
    base_url: str | None = None,
    api_mode: str | None = None,
) -> ProviderConfig:
    return ProviderConfig(
        kind="openai",
        model=model,
        api_key=api_key,
        base_url=base_url,
        api_mode=api_mode,
    )


def DeepSeek(
    model: str,
    *,
    api_key: str,
    base_url: str | None = None,
    api_mode: str | None = None,
) -> ProviderConfig:
    return ProviderConfig(
        kind="deepseek",
        model=model,
        api_key=api_key,
        base_url=base_url,
        api_mode=api_mode,
    )


def Qwen(
    model: str,
    *,
    api_key: str,
    base_url: str | None = None,
    api_mode: str | None = None,
) -> ProviderConfig:
    return ProviderConfig(
        kind="qwen",
        model=model,
        api_key=api_key,
        base_url=base_url,
        api_mode=api_mode,
    )


def Kimi(
    model: str,
    *,
    api_key: str,
    base_url: str | None = None,
    api_mode: str | None = None,
) -> ProviderConfig:
    return ProviderConfig(
        kind="kimi",
        model=model,
        api_key=api_key,
        base_url=base_url,
        api_mode=api_mode,
    )


def Gemini(
    model: str,
    *,
    api_key: str,
    base_url: str | None = None,
    api_mode: str | None = None,
) -> ProviderConfig:
    return ProviderConfig(
        kind="gemini",
        model=model,
        api_key=api_key,
        base_url=base_url,
        api_mode=api_mode,
    )


def Codex(
    model: str,
    *,
    api_key: str,
    base_url: str | None = None,
    api_mode: str | None = None,
) -> ProviderConfig:
    return ProviderConfig(
        kind="codex",
        model=model,
        api_key=api_key,
        base_url=base_url,
        api_mode=api_mode or "responses",
    )


def Custom(
    model: str,
    *,
    base_url: str,
    api_key: str = "",
    provider_id: str = "custom",
    api_mode: str | None = None,
) -> ProviderConfig:
    return ProviderConfig(
        kind="custom",
        model=model,
        api_key=api_key,
        base_url=base_url,
        api_mode=api_mode,
        provider_id=provider_id,
    )


def ControlledProvider(
    responses: Sequence[str] | None = None,
    *,
    turns: Sequence[Mapping[str, Any]] | None = None,
    provider_id: str = "controlled",
    model: str = "controlled-model",
    delay_ms: int | None = None,
) -> ProviderConfig:
    if responses is None and turns is None:
        raise ValueError("ControlledProvider requires `responses` or `turns`.")
    return ProviderConfig(
        kind="controlled",
        model=model,
        provider_id=provider_id,
        responses=tuple(responses) if responses is not None else None,
        turns=tuple(turns) if turns is not None else None,
        delay_ms=delay_ms,
    )
