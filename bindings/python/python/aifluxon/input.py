from __future__ import annotations

import base64
import mimetypes
from collections.abc import Sequence
from dataclasses import dataclass
from os import PathLike
from pathlib import Path
from typing import Any, TypeAlias
from urllib.parse import urlparse

from .errors import InvalidRequestError

_IMAGE_MIME_TYPES_BY_SUFFIX = {
    ".avif": "image/avif",
    ".bmp": "image/bmp",
    ".gif": "image/gif",
    ".heic": "image/heic",
    ".heif": "image/heif",
    ".ico": "image/vnd.microsoft.icon",
    ".jpeg": "image/jpeg",
    ".jpg": "image/jpeg",
    ".png": "image/png",
    ".svg": "image/svg+xml",
    ".tif": "image/tiff",
    ".tiff": "image/tiff",
    ".webp": "image/webp",
}


@dataclass(frozen=True, slots=True)
class ImageInput:
    """A provider-ready image reference used in a multimodal user prompt.

    ``reference`` may be a public HTTP(S) URL, a provider file id, or a base64
    data URL. Use :meth:`from_file` to resolve a local file into a data URL
    before it crosses the provider boundary.
    """

    reference: str
    mime_type: str

    def __post_init__(self) -> None:
        if not isinstance(self.reference, str) or not self.reference.strip():
            raise _invalid_request("ImageInput.reference must be a non-empty string.")
        if not isinstance(self.mime_type, str) or not self.mime_type.strip():
            raise _invalid_request("ImageInput.mime_type must be a non-empty string.")
        reference = self.reference.strip()
        mime_type = self.mime_type.strip().lower()
        if not mime_type.startswith("image/"):
            raise _invalid_request("ImageInput.mime_type must be an image MIME type.")
        if reference[:5].lower() == "data:":
            prefix = f"data:{mime_type};base64,"
            if reference[: len(prefix)].lower() != prefix:
                raise _invalid_request(
                    "ImageInput data URL MIME type must match "
                    "ImageInput.mime_type and use base64."
                )
        object.__setattr__(self, "reference", reference)
        object.__setattr__(self, "mime_type", mime_type)

    @classmethod
    def from_url(cls, url: str, mime_type: str) -> ImageInput:
        """Create an image input from a public HTTP(S) URL."""

        parsed = urlparse(url.strip()) if isinstance(url, str) else None
        if (
            parsed is None
            or parsed.scheme.lower() not in {"http", "https"}
            or not parsed.netloc
        ):
            raise _invalid_request(
                "ImageInput.from_url requires an absolute HTTP(S) URL."
            )
        return cls(url, mime_type)

    @classmethod
    def from_file_id(cls, file_id: str, mime_type: str) -> ImageInput:
        """Create an image input from an id issued by the selected provider."""

        return cls(file_id, mime_type)

    @classmethod
    def from_bytes(
        cls,
        data: bytes | bytearray | memoryview,
        mime_type: str,
    ) -> ImageInput:
        """Encode image bytes as a base64 data URL."""

        if not isinstance(data, (bytes, bytearray, memoryview)):
            raise _invalid_request("ImageInput.from_bytes requires a bytes-like value.")
        raw = bytes(data)
        if not raw:
            raise _invalid_request(
                "ImageInput.from_bytes requires non-empty image data."
            )
        normalized_mime = _normalize_image_mime_type(mime_type)
        encoded = base64.b64encode(raw).decode("ascii")
        return cls(f"data:{normalized_mime};base64,{encoded}", normalized_mime)

    @classmethod
    def from_file(
        cls,
        path: str | PathLike[str],
        mime_type: str | None = None,
    ) -> ImageInput:
        """Read a local image and encode it as a provider-ready data URL."""

        image_path = Path(path)
        resolved_mime = mime_type or _infer_image_mime_type(image_path)
        if resolved_mime is None:
            raise _invalid_request(
                "Could not infer the image MIME type; pass mime_type explicitly."
            )
        try:
            data = image_path.read_bytes()
        except OSError as error:
            raise _invalid_request(
                f"Could not read image file `{image_path}`: {error}"
            ) from error
        return cls.from_bytes(data, resolved_mime)

    def _to_payload(self) -> dict[str, str]:
        return {
            "type": "image",
            "reference": self.reference,
            "mime_type": self.mime_type,
        }


PromptPart: TypeAlias = str | ImageInput
PromptInput: TypeAlias = str | Sequence[PromptPart]


def normalize_prompt_input(prompt: PromptInput) -> str | list[dict[str, str]]:
    if isinstance(prompt, str):
        return prompt
    if isinstance(prompt, (bytes, bytearray, memoryview)) or not isinstance(
        prompt, Sequence
    ):
        raise _invalid_request(
            "prompt must be a string or a sequence containing strings "
            "and ImageInput values."
        )
    if not prompt:
        raise _invalid_request("A multimodal prompt must contain at least one part.")

    payload: list[dict[str, str]] = []
    for index, part in enumerate(prompt):
        if isinstance(part, str):
            payload.append({"type": "text", "text": part})
        elif isinstance(part, ImageInput):
            payload.append(part._to_payload())
        else:
            raise _invalid_request(
                f"prompt part {index} must be a string or ImageInput, "
                f"not {type(part).__name__}."
            )
    return payload


def _normalize_image_mime_type(value: Any) -> str:
    if not isinstance(value, str) or not value.strip():
        raise _invalid_request("Image MIME type must be a non-empty string.")
    mime_type = value.strip().lower()
    if not mime_type.startswith("image/"):
        raise _invalid_request("Image MIME type must start with `image/`.")
    return mime_type


def _infer_image_mime_type(path: Path) -> str | None:
    known_image_mime = _IMAGE_MIME_TYPES_BY_SUFFIX.get(path.suffix.lower())
    if known_image_mime is not None:
        return known_image_mime
    return mimetypes.guess_type(path.name)[0]


def _invalid_request(message: str) -> InvalidRequestError:
    return InvalidRequestError(message, kind="InvalidRequest")
