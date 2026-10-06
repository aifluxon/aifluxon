# AIFLUXON

Experimental Python SDK for the AIFLUXON in-process agent backend.

```text
Python → PyO3 → aifluxon-api → runtime → providers / tools
```

## Install

```bash
python -m pip install aifluxon
```

Requires **CPython 3.11–3.14** on Windows 10/11 x86_64, Linux glibc x86_64, or Linux glibc aarch64. Wheels are ABI-specific (`cp311`–`cp314`), not abi3. Linux wheels target manylinux2014 / glibc 2.17 or newer. Alpine/musl, macOS, Windows ARM64, PyPy, and free-threaded CPython are not supported by 0.2.1 wheels. Installing a published wheel does not require Rust.

License: **Apache-2.0**.

## Quick start

```python
import asyncio
from aifluxon import Agent, ControlledProvider

async def main():
    agent = Agent(provider=ControlledProvider(["Hello from AIFLUXON."]))
    print((await agent.run("hello")).text)

asyncio.run(main())
```

Multimodal prompt:

```python
from aifluxon import Agent, DeepSeek, ImageInput

agent = Agent(DeepSeek("deepseek-flash", api_key="...", api_mode="responses"))
result = await agent.run([
    "Describe this image.",
    ImageInput.from_file("./input.png"),
])
```

`ImageInput` also accepts public URLs, provider file IDs, bytes, and data URLs. Python tools can return `ImageInput` values for multimodal tool output.

For DeepSeek image-returning tools, select `api_mode="responses"`; Chat Completions supports user image inputs but not image tool outputs. The legacy `deepseek-v4-flash-vision-exp` name remains compatible. `ImageInput.from_file_id()` uses an existing provider file ID; it does not upload a file. The SDK supplies the image protocol and `@tool` bridge, while your application supplies and authorizes a `view_image` tool. See `examples/view_image.py` for a runnable example.

`ControlledProvider` is offline. Public providers: OpenAI, DeepSeek, Qwen, Kimi, Gemini, Codex, Custom. ChatGPT Web and DeepSeek Web are not included in this SDK.

Documentation: https://github.com/aifluxon/aifluxon/blob/main/docs/python/README.md

## Build from source

This package is the Python binding. It depends only on `aifluxon-api`.

Windows PowerShell:

```powershell
cd bindings/python
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install maturin pytest pytest-asyncio
maturin develop
python -c "import aifluxon; print(aifluxon.__version__)"
pytest
```

Linux:

```bash
cd bindings/python
python3 -m venv .venv
source .venv/bin/activate
python -m pip install maturin pytest pytest-asyncio
maturin develop
python -c "import aifluxon; print(aifluxon.__version__)"
pytest
```
