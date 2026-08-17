from __future__ import annotations

import json
from typing import Any


class AifluxonError(Exception):
    """Base error for the AIFLUXON Python SDK."""

    def __init__(self, message: str, *, kind: str = "Internal") -> None:
        super().__init__(message)
        self.message = message
        self.kind = kind

    def __str__(self) -> str:
        return self.message


class InvalidConfigurationError(AifluxonError):
    """The Agent or provider configuration is invalid."""


class InvalidRequestError(AifluxonError):
    """The request is malformed or refers to an unknown identity."""


class ProviderError(AifluxonError):
    """The model provider failed or is not registered."""


class ToolError(AifluxonError):
    """A tool was unknown, invalid, or failed during execution."""


class PolicyError(AifluxonError):
    """A tool policy denied or rejected an operation."""


class CancelledError(AifluxonError):
    """The run was cancelled."""


class BudgetExceededError(AifluxonError):
    """The run exceeded its model or tool budget."""


class StateConflictError(AifluxonError):
    """A CAS conflict or illegal operation-state transition occurred."""


class FailedError(AifluxonError):
    """The run terminated as failed."""


class InternalError(AifluxonError):
    """An unexpected internal backend failure."""


_KIND_MAP: dict[str, type[AifluxonError]] = {
    "InvalidConfiguration": InvalidConfigurationError,
    "InvalidRequest": InvalidRequestError,
    "Provider": ProviderError,
    "Tool": ToolError,
    "PolicyDenied": PolicyError,
    "OperationPending": PolicyError,
    "OperationRejected": PolicyError,
    "Cancelled": CancelledError,
    "BudgetExceeded": BudgetExceededError,
    "StateConflict": StateConflictError,
    "RuntimeUnavailable": InternalError,
    "Failed": FailedError,
    "Internal": InternalError,
}


def raise_native(error: BaseException) -> None:
    message = str(error)
    try:
        payload = json.loads(message)
        kind = str(payload.get("kind") or "Internal")
        text = str(payload.get("message") or message)
    except json.JSONDecodeError:
        kind = "Internal"
        text = message
    cls = _KIND_MAP.get(kind, InternalError)
    raise cls(text, kind=kind) from error


def call_native(fn: Any, *args: Any) -> Any:
    try:
        return fn(*args)
    except RuntimeError as error:
        raise_native(error)
