"""Run a DeepSeek image tool using a host-selected local image.

Set DEEPSEEK_API_KEY, then run: python view_image.py ./input.png
This example makes paid provider requests; the SDK does not bundle this tool.
"""

import argparse
import asyncio
import os
from pathlib import Path

from aifluxon import Agent, DeepSeek, ImageInput, ToolEffect, tool


async def main(image_path: Path) -> None:
    # The host chooses and validates the path. The model cannot choose files.
    selected_image = image_path.resolve(strict=True)
    if not selected_image.is_file():
        raise ValueError("Select an image file.")

    @tool(description="View the selected image.", effect=ToolEffect.FS_READ)
    def view_image() -> ImageInput:
        return ImageInput.from_file(selected_image)

    agent = Agent(
        DeepSeek(
            "deepseek-flash",
            api_key=os.environ["DEEPSEEK_API_KEY"],
            api_mode="responses",
        ),
        tools=[view_image],
        max_model_rounds=4,
        max_tool_invocations=2,
    )
    result = await agent.run(
        "Use view_image to inspect the selected image, then suggest an image "
        "generation prompt that preserves its composition, lighting, and style."
    )
    print(result.text)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("image", type=Path)
    args = parser.parse_args()
    asyncio.run(main(args.image))
