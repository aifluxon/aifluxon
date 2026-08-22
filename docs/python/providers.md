# Providers

Public Python construction covers OpenAI-compatible families plus the offline `ControlledProvider`.

ChatGPT Web and DeepSeek Web are **not** part of this SDK.

Credentials are passed in memory as `api_key`. They are never written to session JSON.

## OpenAI

```python
from aifluxon import OpenAI
provider = OpenAI(model="gpt-4.1", api_key="...", base_url=None, api_mode=None)
```

- Reasoning effort is selected on `Agent`, not on this constructor. See [Thinking](thinking.md).
- Default base URL: `https://api.openai.com/v1`

## DeepSeek

```python
from aifluxon import DeepSeek
DeepSeek(model="deepseek-chat", api_key="...")
DeepSeek(model="deepseek-v4-pro", api_key="...", api_mode="responses")
```

Default base URL: `https://api.deepseek.com`. Default API mode: `chat_completions`. `deepseek-v4-flash`, `deepseek-v4-pro`, and `deepseek-v4-flash-vision-exp` can use `api_mode="responses"`; other DeepSeek models stay on Chat Completions even if Responses is requested. The Rust embedding API supports image content for `deepseek-v4-flash-vision-exp`; the current Python `Agent.run()` prompt surface remains text-only. Enable thinking on `Agent` with `thinking=True` and `reasoning_effort="low"|"high"|"max"`. Only `deepseek-v4*` models have the toggle; `low` is valid on V4 Flash and is raised to `high` on V4 Pro. See [Thinking](thinking.md).

## Qwen

```python
from aifluxon import Qwen
Qwen(model="qwen-plus", api_key="...")
```

Default base URL: `https://dashscope.aliyuncs.com/compatible-mode/v1`. Hybrid models take `Agent(thinking=True, thinking_budget=8192)`. Summary-only turns request generic continuation (`SummaryOnly`). See [Thinking](thinking.md).

## Kimi

```python
from aifluxon import Kimi
Kimi(model="kimi-k2.5", api_key="...")
```

Default base URL: `https://api.moonshot.cn/v1`. Session cache stays provider-owned. Thinking is enabled from the model name (`kimi-k2.5` / `kimi-k2.6`); Agent thinking arguments are ignored.

## Gemini

```python
from aifluxon import Gemini
Gemini(model="gemini-2.5-flash", api_key="...")
```

Default base URL: `https://generativelanguage.googleapis.com/v1beta/openai`. Set `Agent(..., reasoning_effort="low")`. Unsupported levels are capped by the Gemini provider.

## Codex

```python
from aifluxon import Codex
Codex(model="gpt-5.2-codex", api_key="...")
```

Default API mode: `responses`. Set `Agent(..., reasoning_effort="medium")` for both the API-key constructor and `CodexAuth.provider(...)`. Non-terminal `end_turn` is interpreted as `Continue(ProviderRequested)`.

ChatGPT OAuth is a separate path: `CodexAuth.provider(...)`. See [Codex OAuth](auth.md). Do not copy an OAuth access token into `api_key`.

## Custom

```python
from aifluxon import Custom
Custom(model="local-model", base_url="http://127.0.0.1:8080/v1", provider_id="local_gateway")
```

`base_url` is required. Reasoning uses the same `Agent(reasoning_effort=...)` path as OpenAI.

## ControlledProvider

```python
from aifluxon import ControlledProvider
ControlledProvider(["hello"])
ControlledProvider(turns=[{"tool": "lookup", "id": "call-1", "arguments": {"query": "x"}}, {"text": "done"}])
```

No network. Used by tests and offline examples.

## `ProviderConfig`

Frozen value returned by the constructors above. Not a live client.
