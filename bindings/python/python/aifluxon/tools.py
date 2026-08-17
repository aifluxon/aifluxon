from __future__ import annotations

import inspect
import json
import types
from collections.abc import Callable
from enum import Enum
from typing import Any, Union, get_args, get_origin, get_type_hints


class ToolEffect(str, Enum):
    PURE_READ = "pure_read"
    FS_READ = "fs_read"
    FS_WRITE = "fs_write"
    PROCESS_SPAWN = "process_spawn"
    PROCESS_CONTROL = "process_control"
    NETWORK = "network"
    SETTINGS_WRITE = "settings_write"
    EXTERNAL_SIDE_EFFECT = "external_side_effect"
    UNKNOWN = "unknown"


class AllowAllPolicy:
    """Generic policy that allows every registered tool. This is not EasyPhy Default/Managed/Trusted."""

    def evaluate(self, name: str, arguments: dict[str, Any], effect: str) -> dict[str, Any]:
        return {"decision": "allow"}


class RequireApprovalPolicy:
    """Generic host policy that pauses the run until the caller resolves or commits the operation."""

    def __init__(
        self,
        *,
        mode: str = "blocking_approval",
        effects: frozenset[ToolEffect] | None = None,
    ) -> None:
        if mode not in {"blocking_approval", "deferred_commit"}:
            raise ValueError("mode must be 'blocking_approval' or 'deferred_commit'")
        self.mode = mode
        self.effects = effects

    def evaluate(self, name: str, arguments: dict[str, Any], effect: str) -> dict[str, Any]:
        if self.effects is not None and ToolEffect(effect) not in self.effects:
            return {"decision": "allow"}
        return {
            "decision": "require_approval",
            "mode": self.mode,
            "summary": f"Approve `{name}`",
            "payload": {"name": name, "arguments": arguments, "effect": effect},
        }


def tool(
    fn: Callable[..., Any] | None = None,
    *,
    name: str | None = None,
    description: str | None = None,
    effect: ToolEffect | str = ToolEffect.UNKNOWN,
    parallel_safe: bool = False,
    required_capabilities: list[str] | None = None,
) -> Any:
    def decorate(func: Callable[..., Any]) -> Callable[..., Any]:
        func.__aifluxon_tool__ = {  # type: ignore[attr-defined]
            "name": name or func.__name__,
            "description": description or (inspect.getdoc(func) or "").strip(),
            "effect": ToolEffect(effect).value,
            "parallel_safe": parallel_safe,
            "required_capabilities": list(required_capabilities or []),
            "input_schema": schema_from_callable(func),
        }
        return func

    if fn is None:
        return decorate
    return decorate(fn)


def schema_from_callable(fn: Callable[..., Any]) -> dict[str, Any]:
    hints = get_type_hints(fn)
    signature = inspect.signature(fn)
    properties: dict[str, Any] = {}
    required: list[str] = []
    for parameter in signature.parameters.values():
        if parameter.kind in (
            inspect.Parameter.VAR_POSITIONAL,
            inspect.Parameter.VAR_KEYWORD,
        ):
            raise TypeError(
                f"Tool `{fn.__name__}` cannot use *args/**kwargs; annotate explicit parameters."
            )
        annotation = hints.get(parameter.name, parameter.annotation)
        if annotation is inspect.Signature.empty:
            raise TypeError(
                f"Tool `{fn.__name__}` parameter `{parameter.name}` requires a type annotation."
            )
        properties[parameter.name] = annotation_to_schema(annotation, fn.__name__, parameter.name)
        if parameter.default is inspect.Parameter.empty:
            required.append(parameter.name)
    schema: dict[str, Any] = {"type": "object", "properties": properties}
    if required:
        schema["required"] = required
    return schema


def annotation_to_schema(annotation: Any, tool_name: str, field: str) -> dict[str, Any]:
    origin = get_origin(annotation)
    if origin is None:
        return _simple_schema(annotation, tool_name, field)
    args = get_args(annotation)
    if origin is list:
        if len(args) != 1:
            raise TypeError(f"Tool `{tool_name}` parameter `{field}` list annotation is unsupported.")
        return {"type": "array", "items": annotation_to_schema(args[0], tool_name, field)}
    if origin is dict:
        if len(args) != 2 or args[0] is not str:
            raise TypeError(
                f"Tool `{tool_name}` parameter `{field}` only supports dict[str, T]."
            )
        return {
            "type": "object",
            "additionalProperties": annotation_to_schema(args[1], tool_name, field),
        }
    if _is_optional(origin, args):
        non_none = [arg for arg in args if arg is not type(None)]
        if len(non_none) != 1:
            raise TypeError(f"Tool `{tool_name}` parameter `{field}` Union is unsupported.")
        schema = annotation_to_schema(non_none[0], tool_name, field)
        return schema
    raise TypeError(
        f"Tool `{tool_name}` parameter `{field}` uses unsupported type `{annotation!r}`."
    )


def _is_optional(origin: Any, args: tuple[Any, ...]) -> bool:
    return origin in (Union, types.UnionType) and type(None) in args


def _simple_schema(annotation: Any, tool_name: str, field: str) -> dict[str, Any]:
    mapping = {
        str: "string",
        int: "integer",
        float: "number",
        bool: "boolean",
    }
    if annotation in mapping:
        return {"type": mapping[annotation]}
    raise TypeError(
        f"Tool `{tool_name}` parameter `{field}` uses unsupported type `{annotation!r}`."
    )


def descriptor_from_callable(fn: Callable[..., Any]) -> dict[str, Any]:
    meta = getattr(fn, "__aifluxon_tool__", None)
    if not meta:
        raise TypeError(f"`{getattr(fn, '__name__', fn)}` is not an AIFLUXON tool.")
    return dict(meta)
