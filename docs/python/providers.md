# Providers

Public Python construction is limited to OpenAI-compatible families plus the offline `ControlledProvider`.

ChatGPT Web and DeepSeek Web are **EasyPhy-private providers; not part of the public Python SDK.**

Credentials are passed in memory as `api_key`. They are never written to session JSON.

## OpenAI

```python
from aifluxon import OpenAI
provider = OpenAI(model="gpt-4.1", api_key="...", base_url=None, api_mode=None)
```

* Default base URL: `https://api.openai.com/v1`
* Chat Completions by default; Responses is selected by model family inside the provider
* Reasoning effort and prompt cache are provider-owned
* Limitation: requires a live key for real calls; tests use `ControlledProvider`

## DeepSeek

```python
from aifluxon import DeepSeek
DeepSeek(model="deepseek-chat", api_key="...")
```

Default base URL: `https://api.deepseek.com`. Thinking / reasoning-effort mapping stays in the DeepSeek provider.

## Qwen

```python
from aifluxon import Qwen
Qwen(model="qwen-plus", api_key="...")
```

Default base URL: `https://dashscope.aliyuncs.com/compatible-mode/v1`. Summary-only turns request generic continuation (`SummaryOnly`); Runtime does not branch on the name Qwen.

## Kimi

```python
from aifluxon import Kimi
Kimi(model="kimi-k2.5", api_key="...")
```

Default base URL: `https://api.moonshot.cn/v1`. Session cache stays provider-owned.

## Gemini

```python
from aifluxon import Gemini
Gemini(model="gemini-2.5-flash", api_key="...")
```

Default base URL: `https://generativelanguage.googleapis.com/v1beta/openai`.

## Codex

```python
from aifluxon import Codex
Codex(model="gpt-5.2-codex", api_key="...")
```

Default API mode: `responses`. Non-terminal `end_turn` is interpreted by the Codex provider as `Continue(ProviderRequested)`. Codex OAuth remains EasyPhy-owned.

## Custom

```python
from aifluxon import Custom
Custom(model="local-model", base_url="http://127.0.0.1:8080/v1", provider_id="local_gateway")
```

`base_url` is required.

## ControlledProvider

```python
from aifluxon import ControlledProvider
ControlledProvider(["hello"])
ControlledProvider(turns=[{"tool": "lookup", "id": "call-1", "arguments": {"query": "x"}}, {"text": "done"}])
```

No network. Used by tests and offline examples.

## `ProviderConfig`

Frozen value returned by the constructors above. Not a live client.
