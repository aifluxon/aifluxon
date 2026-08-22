# Thinking and reasoning effort

Thinking options are **Host-selected, provider-adapted**. Python does not implement vendor protocols. `Agent` stores defaults and forwards them on every run as AIFLUXON `ProviderFeatureRequest`. Providers that do not use a field ignore it.

ChatGPT Web and DeepSeek Web are not in this SDK.

## Agent defaults and per-run overrides

```python
from aifluxon import Agent, OpenAI

agent = Agent(
    OpenAI("gpt-5.4", api_key="..."),
    reasoning_effort="high",
)
result = await agent.run("Inspect this project")
result = await agent.run("Quick check", reasoning_effort="low")
```

| Python argument | Wire field | Values |
|---|---|---|
| `reasoning_effort` | `reasoning_effort` | `default`, `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` |
| `thinking` | `thinking_mode` | `True`/`False`, or `enabled` / `disabled` / `default` |
| `thinking_budget` | `thinking_budget` | positive integer |

`None` means “do not declare”. `reasoning_effort="default"` is also a no-op after provider normalize. Inspect with `agent.thinking_settings`.

Unsupported effort values for a **model family** are lowered by AIFLUXON (Gemini `xhigh` → `high`). Invalid tokens raise `InvalidConfigurationError` in Python before the run starts.

## OpenAI / Codex / Custom

Use `reasoning_effort` only.

```python
Agent(OpenAI("gpt-5.4", api_key="..."), reasoning_effort="high")
Agent(Codex("gpt-5.6-codex", api_key="..."), reasoning_effort="xhigh")
Agent(auth.provider("gpt-5.6-codex", account_id=account.id), reasoning_effort="medium")
```

- Chat Completions: top-level `reasoning_effort`
- Responses / Codex: `reasoning.effort`
- Allowed levels depend on the model (gpt-5.6 can use `max`; many Codex models stop at `xhigh`; non-reasoning models only accept unset/`default`)

## Gemini

Same `reasoning_effort` argument. Gemini OpenAI-compatible chat writes `reasoning_effort`. There is no separate thinking toggle.

```python
Agent(Gemini("gemini-2.5-flash", api_key="..."), reasoning_effort="low")
```

## DeepSeek

Two knobs. Effort is sent only when thinking is on, and only `deepseek-v4*` models have the toggle.

```python
Agent(
    DeepSeek("deepseek-v4-flash", api_key="..."),
    thinking=True,
    reasoning_effort="low",  # low is valid on v4-flash; other models raise it to high
)
```

- Chat: `thinking: { "type": "enabled"|"disabled" }` plus `reasoning_effort` when enabled
- Responses: `reasoning.effort` (`none` when thinking is off). Select with `DeepSeek(..., api_mode="responses")`. V4 Flash, V4 Pro, and V4 Flash Vision Exp support Responses.
- Values: `low` / `high` / `max`

## Qwen

Not low/high. Hybrid models take an on/off switch and an optional token budget.

```python
Agent(
    Qwen("qwen-plus", api_key="..."),
    thinking=True,
    thinking_budget=8192,
)
```

- Chat: `enable_thinking` and optional `thinking_budget`
- Responses: `enabled` → `reasoning.effort=medium`, `disabled` → `none`, `default` leaves the body unchanged
- `-thinking` / `qwq*` models always think; `-instruct` / some coder models ignore the switch

## Kimi

No Agent knob. `kimi-k2.5` / `kimi-k2.6` enable thinking from the model name. `reasoning_effort` / `thinking` / `thinking_budget` are ignored.

## ControlledProvider

Accepts the same Agent arguments so tests can set them. They do not change scripted text.

See `bindings/python/examples/thinking.py`.
