from typing import Any, Callable


class NativeAgent:
    def __init__(
        self,
        provider_spec: str,
        store_path: str | None = None,
        policy_callback: Callable[..., str] | None = None,
        max_model_rounds: int = 32,
        max_tool_invocations: int = 64,
        oauth_provider: Any | None = None,
    ) -> None: ...
    def start(
        self,
        prompt: str,
        session_id: str | None = None,
        features_json: str | None = None,
        system_prompt: str | None = None,
    ) -> str: ...
    def start_with_content(
        self,
        content_json: str,
        session_id: str | None = None,
        features_json: str | None = None,
        system_prompt: str | None = None,
    ) -> str: ...

__all__ = ["NativeAgent"]
