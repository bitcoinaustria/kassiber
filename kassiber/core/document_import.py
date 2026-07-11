from __future__ import annotations

"""Local-AI document importer for photo/PDF transaction evidence."""

import base64
import hashlib
import ipaddress
import json
import math
import mimetypes
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import uuid
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation, localcontext
from pathlib import Path
from typing import Any, Callable, Mapping, Sequence
from urllib.parse import urlparse

from ..ai.client import ai_client_for_locator
from ..ai.providers import get_ai_provider_api_key_for_use, resolve_ai_provider
from ..errors import AppError
from ..msat import MSAT_PER_BTC
from ..time_utils import now_iso, parse_timestamp
from ..util import str_or_none
from . import attachments as core_attachments
from . import imports as core_imports


MAX_RENDERED_PDF_PAGES = 8
DEFAULT_CONFIDENCE_THRESHOLD = Decimal("0.78")
# A PDF is scanned completely by default up to the hard local-model budget.
# Longer documents require an explicit page range instead of being silently
# truncated to a prefix.
DEFAULT_MAX_PDF_PAGES = MAX_RENDERED_PDF_PAGES
MAX_SOURCE_BYTES = 25 * 1024 * 1024
MAX_DRAFT_ROWS = 500
MAX_PROJECTED_ATTACHMENT_BYTES = 512 * 1024 * 1024
PDF_RENDER_DPI = 180
PDF_RENDER_TIMEOUT_SECONDS = 30
PDF_RENDER_MAX_DIMENSION = 2400
MAX_RENDERED_PDF_PAGE_BYTES = 24 * 1024 * 1024
MAX_RENDERED_PDF_TOTAL_BYTES = 96 * 1024 * 1024
MAX_RENDERED_PDF_PIXELS = PDF_RENDER_MAX_DIMENSION * PDF_RENDER_MAX_DIMENSION
MAX_DECIMAL_INPUT_CHARS = 512
MAX_DECIMAL_RENDER_CHARS = 512
MAX_SQLITE_INT64 = 2**63 - 1
MIN_STORABLE_BTC = Decimal("0.00000000001")
MAX_STORABLE_BTC = Decimal("92233720.36854775807")
MAX_FIAT_FLOAT = Decimal(str(sys.float_info.max))
DOCUMENT_IMPORT_FORMAT = "document_import"
SUPPORTED_DOCUMENT_ASSETS = frozenset({"BTC", "LBTC"})
_OCR_CONFIDENCE_FIELDS = frozenset(
    {
        "occurred_at",
        "direction",
        "asset",
        "amount_btc",
        "fee_btc",
        "fiat_currency",
        "fiat_value",
        "fiat_rate",
        "counterparty",
        "description",
    }
)
_STRUCTURAL_ROW_FLAGS = frozenset(
    {
        "missing_date",
        "invalid_date",
        "missing_direction",
        "missing_amount",
        "amount_not_representable",
        "unsupported_asset",
        "non_positive_amount",
        "invalid_fee",
        "negative_fee",
        "fee_not_representable",
        "invalid_fiat_value",
        "non_positive_fiat_value",
        "fiat_value_out_of_range",
        "invalid_fiat_rate",
        "non_positive_fiat_rate",
        "fiat_rate_out_of_range",
        "missing_fiat_currency",
        "fiat_currency_mismatch",
        "invalid_source_page",
    }
)

IMAGE_EXTENSIONS = {".jpg", ".jpeg", ".png", ".webp", ".gif"}
PDF_EXTENSION = ".pdf"
SUPPORTED_EXTENSIONS = IMAGE_EXTENSIONS | {PDF_EXTENSION}

VISION_MODEL_NAME_HINTS = (
    "vision",
    "qwen3-vl",
    "qwen2.5vl",
    "qwen2-vl",
    "glm-ocr",
    "deepseek-ocr",
    "minicpm-v",
    "llava",
    "bakllava",
    "moondream",
    "gemma3",
    "ocr",
    "-vl",
)

MODEL_RECOMMENDATIONS: tuple[dict[str, str], ...] = (
    {
        "id": "glm-ocr",
        "command": "ollama pull glm-ocr",
        "use": "Fast document OCR and table extraction on modest hardware.",
    },
    {
        "id": "qwen3-vl:8b",
        "command": "ollama pull qwen3-vl:8b",
        "use": "Best default for OCR plus table reasoning when memory allows it.",
    },
    {
        "id": "qwen3-vl:4b",
        "command": "ollama pull qwen3-vl:4b",
        "use": "Smaller Qwen3-VL option for laptops.",
    },
    {
        "id": "llama3.2-vision:11b",
        "command": "ollama pull llama3.2-vision:11b",
        "use": "General image reasoning fallback with broad Ollama availability.",
    },
    {
        "id": "minicpm-v:8b",
        "command": "ollama pull minicpm-v:8b",
        "use": "Compact multimodal fallback for receipts and screenshots.",
    },
)

OCR_SYSTEM_PROMPT = """You are Kassiber's local document transaction importer.
Extract Bitcoin accounting transactions from the supplied image or PDF page text.
Return JSON only. Do not include markdown.

Schema:
{
  "rows": [
    {
      "occurred_at": "YYYY-MM-DD or RFC3339 or null",
      "direction": "inbound|outbound|null",
      "asset": "BTC|LBTC|... or null",
      "amount_btc": "decimal BTC amount or null",
      "fee_btc": "decimal BTC fee or null",
      "fiat_currency": "EUR|USD|... or null",
      "fiat_value": "decimal fiat value or null",
      "fiat_rate": "decimal fiat per BTC or null",
      "counterparty": "string or null",
      "description": "short evidence-backed string or null",
      "confidence": 0.0,
      "cell_confidences": {
        "occurred_at": 0.0,
        "direction": 0.0,
        "asset": 0.0,
        "amount_btc": 0.0,
        "fee_btc": 0.0,
        "fiat_currency": 0.0,
        "fiat_value": 0.0,
        "fiat_rate": 0.0,
        "counterparty": 0.0,
        "description": 0.0
      },
      "source_region": {
        "page": 1,
        "x": 0.0,
        "y": 0.0,
        "width": 0.0,
        "height": 0.0,
        "unit": "relative"
      },
      "evidence_text": "exact visible words/numbers supporting the row"
    }
  ]
}

Rules:
- Extract only rows that appear to be Bitcoin, Lightning, Liquid, or fiat-to-Bitcoin activity.
- Never invent missing values. Use null and low confidence when unsure.
- Include a cell_confidences entry for every non-null accounting field.
- Use positive amount_btc and direction to express side.
- Fiat values and rates must be positive and must include their visible ISO currency.
- If a row is a fiat-only bank movement without a Bitcoin amount, omit it.
- If there are no transaction rows, return {"rows":[]}.
"""


@dataclass(frozen=True)
class DocumentImportHooks:
    import_hooks: core_imports.ImportCoordinatorHooks
    attachment_hooks: core_attachments.AttachmentHooks


ClientFactory = Callable[[dict[str, Any]], Any]


def model_recommendations() -> list[dict[str, str]]:
    return [dict(row) for row in MODEL_RECOMMENDATIONS]


def _recommendations_hint() -> str:
    commands = ", ".join(row["command"] for row in MODEL_RECOMMENDATIONS[:3])
    return f"Install a local vision model with Ollama, for example: {commands}."


def _is_loopback_host(hostname: str | None) -> bool:
    if not hostname:
        return False
    host = hostname.strip().lower()
    if host == "localhost":
        return True
    try:
        return bool(ipaddress.ip_address(host).is_loopback)
    except ValueError:
        return False


def _validate_local_provider(provider: Mapping[str, Any]) -> None:
    base_url = str(provider.get("base_url") or "")
    parsed = urlparse(base_url)
    if provider.get("kind") != "local" or parsed.scheme not in {"http", "https"}:
        raise AppError(
            "Document OCR requires a local AI provider",
            code="document_import_local_ai_required",
            hint="Configure a local Ollama provider such as http://localhost:11434/v1.",
            retryable=False,
        )
    if not _is_loopback_host(parsed.hostname):
        raise AppError(
            "Document OCR is hard-disabled for off-device AI providers",
            code="document_import_remote_ai_disabled",
            hint="Use a loopback Ollama endpoint. Photos, receipts, and statements are never sent to remote providers by this importer.",
            details={"provider": provider.get("name"), "base_url_host": parsed.hostname},
            retryable=False,
        )


def _model_id(value: Any) -> str:
    return str(value or "").strip()


def looks_like_vision_model(model_id: str) -> bool:
    lowered = model_id.strip().lower()
    return any(hint in lowered for hint in VISION_MODEL_NAME_HINTS)


def _model_matches(candidate: str, expected: str) -> bool:
    return candidate.strip().lower() == expected.strip().lower()


def _installed_model_ids(models: Sequence[Mapping[str, Any]]) -> list[str]:
    out: list[str] = []
    for model in models:
        model_id = _model_id(model.get("id") if isinstance(model, Mapping) else None)
        if model_id:
            out.append(model_id)
    return out


def _choose_model(
    provider: Mapping[str, Any],
    models: Sequence[Mapping[str, Any]],
    requested_model: str | None,
) -> tuple[str, list[str]]:
    installed = _installed_model_ids(models)
    if requested_model:
        model = requested_model.strip()
        installed_match = next(
            (candidate for candidate in installed if _model_matches(candidate, model)),
            None,
        )
        if installed and installed_match is None:
            raise AppError(
                f"Local vision model '{model}' is not installed",
                code="document_import_model_missing",
                hint=_recommendations_hint(),
                details={
                    "requested_model": model,
                    "installed_models": installed,
                    "recommendations": model_recommendations(),
                },
                retryable=False,
            )
        if not looks_like_vision_model(model):
            raise AppError(
                f"Model '{model}' does not look like a vision/OCR model",
                code="document_import_model_not_vision",
                hint="Choose an installed model with vision/OCR capabilities.",
                details={"requested_model": model, "recommendations": model_recommendations()},
                retryable=False,
            )
        return installed_match or model, installed

    configured_default = _model_id(provider.get("default_model"))
    configured_match = next(
        (
            candidate
            for candidate in installed
            if configured_default and _model_matches(candidate, configured_default)
        ),
        None,
    )
    if configured_match and looks_like_vision_model(configured_match):
        return configured_match, installed

    for recommended in MODEL_RECOMMENDATIONS:
        model_id = recommended["id"]
        for candidate in installed:
            if _model_matches(candidate, model_id):
                return candidate, installed

    for candidate in installed:
        if looks_like_vision_model(candidate):
            return candidate, installed

    raise AppError(
        "No local vision model is installed",
        code="document_import_model_missing",
        hint=_recommendations_hint(),
        details={
            "installed_models": installed,
            "recommendations": model_recommendations(),
        },
        retryable=False,
    )


def _source_path(path: str | Path) -> Path:
    raw = str(path or "").strip()
    parsed = urlparse(raw)
    if parsed.scheme in {"http", "https"}:
        host = (parsed.hostname or "").lower()
        if host in {"docs.google.com", "drive.google.com"}:
            hint = (
                "Open the Google link in your logged-in browser, download the PDF/image, "
                "then import the downloaded local file. Kassiber does not read browser cookies."
            )
        else:
            hint = "Download the document locally, then import the local PDF/image file."
        raise AppError(
            "Document OCR import accepts local files, not URLs",
            code="document_import_url_not_supported",
            hint=hint,
            details={"host": host},
            retryable=False,
        )
    candidate = Path(raw).expanduser()
    if not candidate.exists():
        raise AppError(
            f"Document source not found: {candidate}",
            code="not_found",
            hint="Choose an existing image or PDF file.",
            retryable=False,
        )
    if not candidate.is_file():
        raise AppError("Document source must be a file", code="validation", retryable=False)
    suffix = candidate.suffix.lower()
    if suffix not in SUPPORTED_EXTENSIONS:
        raise AppError(
            f"Unsupported document type '{suffix or '(none)'}'",
            code="validation",
            hint="Choose a JPEG, PNG, WebP, GIF, or PDF file.",
            details={"supported_extensions": sorted(SUPPORTED_EXTENSIONS)},
            retryable=False,
        )
    size = candidate.stat().st_size
    if size > MAX_SOURCE_BYTES:
        raise AppError(
            "Document source is too large for local OCR import",
            code="validation",
            hint="Split the statement or export only the pages that contain transactions.",
            details={"size_bytes": size, "max_bytes": MAX_SOURCE_BYTES},
            retryable=False,
        )
    return candidate


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _mime_type(path: Path) -> str:
    guessed, _ = mimetypes.guess_type(path.name)
    return guessed or "application/octet-stream"


def _image_content_part(path: Path) -> dict[str, Any]:
    payload = base64.b64encode(path.read_bytes()).decode("ascii")
    return {
        "type": "image_url",
        "image_url": {"url": f"data:{_mime_type(path)};base64,{payload}"},
    }


def _rendered_png_dimensions(path: Path) -> tuple[int, int]:
    with path.open("rb") as handle:
        header = handle.read(24)
    if (
        len(header) != 24
        or header[:8] != b"\x89PNG\r\n\x1a\n"
        or header[12:16] != b"IHDR"
    ):
        raise AppError(
            "PDF renderer produced an invalid image",
            code="document_import_pdf_render_failed",
            retryable=False,
        )
    return int.from_bytes(header[16:20], "big"), int.from_bytes(header[20:24], "big")


def _validate_rendered_pdf_pages(rendered: Sequence[Path]) -> None:
    total_bytes = 0
    for page_path in rendered:
        size_bytes = page_path.stat().st_size
        width, height = _rendered_png_dimensions(page_path)
        total_bytes += size_bytes
        if (
            size_bytes < 1
            or size_bytes > MAX_RENDERED_PDF_PAGE_BYTES
            or width < 1
            or height < 1
            or width > PDF_RENDER_MAX_DIMENSION
            or height > PDF_RENDER_MAX_DIMENSION
            or width * height > MAX_RENDERED_PDF_PIXELS
            or total_bytes > MAX_RENDERED_PDF_TOTAL_BYTES
        ):
            raise AppError(
                "Rendered PDF pages exceed the local OCR safety budget",
                code="document_import_pdf_render_too_large",
                hint="Export the transaction pages as smaller PNG/JPEG images and import those.",
                details={
                    "page_bytes": size_bytes,
                    "width": width,
                    "height": height,
                    "total_bytes": total_bytes,
                    "max_dimension": PDF_RENDER_MAX_DIMENSION,
                    "max_page_bytes": MAX_RENDERED_PDF_PAGE_BYTES,
                    "max_total_bytes": MAX_RENDERED_PDF_TOTAL_BYTES,
                },
                retryable=False,
            )


def _pdf_page_count(path: Path) -> int:
    pdfinfo = shutil.which("pdfinfo")
    if not pdfinfo:
        raise AppError(
            "PDF OCR import requires Poppler's pdfinfo",
            code="document_import_pdf_renderer_missing",
            hint="Install poppler-utils, or export the statement pages as images and import those.",
            details={"tool": "pdfinfo"},
            retryable=False,
        )
    try:
        completed = subprocess.run(
            [pdfinfo, str(path)],
            text=True,
            capture_output=True,
            check=False,
            timeout=PDF_RENDER_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as exc:
        raise AppError(
            "PDF inspection timed out",
            code="document_import_pdf_render_timeout",
            hint="Export only the transaction pages as PNG/JPEG and import those images.",
            details={"timeout_seconds": PDF_RENDER_TIMEOUT_SECONDS},
            retryable=False,
        ) from exc
    if completed.returncode != 0:
        raise AppError(
            "Could not inspect the PDF page count",
            code="document_import_pdf_render_failed",
            hint="Check whether the PDF is password-protected, then export the transaction pages as images.",
            retryable=False,
        )
    match = re.search(r"^Pages:\s*(\d+)\s*$", completed.stdout or "", flags=re.MULTILINE)
    if match is None or int(match.group(1)) < 1:
        raise AppError(
            "Could not determine the PDF page count",
            code="document_import_pdf_render_failed",
            hint="Export the transaction pages as PNG/JPEG and import those images.",
            retryable=False,
        )
    return int(match.group(1))


def _selected_pdf_pages(
    value: Any,
    *,
    total_pages: int,
    max_pages: int,
) -> tuple[list[int], bool]:
    raw = str(value or "").strip()
    if not raw:
        if total_pages > max_pages:
            raise AppError(
                "This PDF needs an explicit page range before OCR",
                code="document_import_pdf_page_selection_required",
                hint=(
                    f"Choose a contiguous range of at most {max_pages} pages, for example "
                    f"1-{max_pages}, or split the statement into smaller PDFs."
                ),
                details={"total_pages": total_pages, "max_pages": max_pages},
                retryable=False,
            )
        return list(range(1, total_pages + 1)), False

    match = re.fullmatch(r"(\d+)(?:\s*-\s*(\d+))?", raw)
    if match is None:
        raise AppError(
            "PDF pages must be one page or a contiguous range such as 2-6",
            code="validation",
            details={"pages": raw, "total_pages": total_pages},
            retryable=False,
        )
    start = int(match.group(1))
    end = int(match.group(2) or start)
    if start < 1 or end < start or end > total_pages:
        raise AppError(
            "PDF page range is outside the document",
            code="validation",
            details={"pages": raw, "total_pages": total_pages},
            retryable=False,
        )
    selected = list(range(start, end + 1))
    if len(selected) > max_pages:
        raise AppError(
            f"PDF page range may contain at most {max_pages} pages",
            code="validation",
            details={"pages": raw, "selected_pages": len(selected), "max_pages": max_pages},
            retryable=False,
        )
    return selected, True


def _render_pdf_pages(
    path: Path,
    *,
    max_pages: int,
    pages: Any = None,
) -> tuple[
    list[tuple[int, Path]],
    tempfile.TemporaryDirectory[str],
    dict[str, Any],
]:
    pdftoppm = shutil.which("pdftoppm")
    if not pdftoppm:
        raise AppError(
            "PDF OCR import requires Poppler's pdftoppm",
            code="document_import_pdf_renderer_missing",
            hint="Install poppler-utils, or export the statement pages as images and import those.",
            details={"tool": "pdftoppm"},
            retryable=False,
        )
    total_pages = _pdf_page_count(path)
    selected_pages, selection_explicit = _selected_pdf_pages(
        pages,
        total_pages=total_pages,
        max_pages=max_pages,
    )
    tempdir = tempfile.TemporaryDirectory(prefix="kassiber-document-ocr-")
    prefix = Path(tempdir.name) / "page"
    command = [
        pdftoppm,
        "-png",
        "-r",
        str(PDF_RENDER_DPI),
        "-scale-to",
        str(PDF_RENDER_MAX_DIMENSION),
        "-f",
        str(selected_pages[0]),
        "-l",
        str(selected_pages[-1]),
        str(path),
        str(prefix),
    ]
    try:
        completed = subprocess.run(
            command,
            text=True,
            capture_output=True,
            check=False,
            timeout=PDF_RENDER_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired as exc:
        tempdir.cleanup()
        raise AppError(
            "PDF rendering timed out",
            code="document_import_pdf_render_timeout",
            hint="Export only the transaction pages as PNG/JPEG and import those images.",
            details={"timeout_seconds": PDF_RENDER_TIMEOUT_SECONDS},
            retryable=False,
        ) from exc
    if completed.returncode != 0:
        tempdir.cleanup()
        raise AppError(
            "Could not render PDF pages for OCR",
            code="document_import_pdf_render_failed",
            hint="Open the PDF locally and export the transaction pages as PNG/JPEG, then import the images.",
            details={"stderr": (completed.stderr or "").strip()[-2048:]},
            retryable=False,
        )
    rendered = sorted(
        Path(tempdir.name).glob("page-*.png"),
        key=lambda candidate: int(re.search(r"-(\d+)\.png$", candidate.name).group(1))
        if re.search(r"-(\d+)\.png$", candidate.name)
        else 0,
    )
    if len(rendered) != len(selected_pages):
        tempdir.cleanup()
        raise AppError(
            "PDF rendering did not produce the reviewed page range",
            code="document_import_pdf_render_failed",
            hint="Check whether the PDF is password-protected, then export the transaction pages as images.",
            details={"expected_pages": len(selected_pages), "rendered_pages": len(rendered)},
            retryable=False,
        )
    try:
        _validate_rendered_pdf_pages(rendered)
    except Exception:
        tempdir.cleanup()
        raise
    metadata = {
        "total_pages": total_pages,
        "rendered_pages": selected_pages,
        "complete": len(selected_pages) == total_pages,
        "selection_explicit": selection_explicit,
        "selection": (
            str(selected_pages[0])
            if len(selected_pages) == 1
            else f"{selected_pages[0]}-{selected_pages[-1]}"
        ),
    }
    return list(zip(selected_pages, rendered)), tempdir, metadata


def _document_parts(
    path: Path,
    *,
    max_pages: int,
    pages: Any = None,
) -> tuple[list[dict[str, Any]], Callable[[], None], dict[str, Any] | None]:
    if path.suffix.lower() in IMAGE_EXTENSIONS:
        return [_image_content_part(path)], lambda: None, None

    rendered, tempdir, metadata = _render_pdf_pages(
        path,
        max_pages=max_pages,
        pages=pages,
    )
    parts: list[dict[str, Any]] = []
    for page_number, page_path in rendered:
        parts.append({"type": "text", "text": f"PDF page {page_number}:"})
        parts.append(_image_content_part(page_path))
    return parts, tempdir.cleanup, metadata


def _extract_json_object(text: str) -> dict[str, Any]:
    raw = (text or "").strip()
    if not raw:
        raise AppError(
            "Local AI returned an empty OCR response",
            code="document_import_ai_response_invalid",
            retryable=True,
        )
    fenced = re.search(r"```(?:json)?\s*(.*?)\s*```", raw, flags=re.DOTALL)
    if fenced:
        raw = fenced.group(1)
    start = raw.find("{")
    end = raw.rfind("}")
    if start >= 0 and end > start:
        raw = raw[start : end + 1]
    try:
        payload = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise AppError(
            "Local AI returned non-JSON OCR output",
            code="document_import_ai_response_invalid",
            hint="Try a stronger local vision model or a clearer image.",
            details={"reason": str(exc), "response_length": len(raw)},
            retryable=True,
        ) from exc
    if not isinstance(payload, dict):
        raise AppError(
            "Local AI OCR output must be a JSON object",
            code="document_import_ai_response_invalid",
            retryable=True,
        )
    return payload


def _decimal_or_none(value: Any) -> Decimal | None:
    if value in (None, ""):
        return None
    text = str(value).strip()
    if len(text) > MAX_DECIMAL_INPUT_CHARS:
        return None
    text = text.replace("\u00a0", "").replace(" ", "")
    if "," in text and "." in text:
        if text.rfind(",") > text.rfind("."):
            text = text.replace(".", "").replace(",", ".")
        else:
            text = text.replace(",", "")
    elif "," in text:
        if text.count(",") != 1:
            return None
        whole, fractional = text.split(",", 1)
        # A single separator before exactly three digits is ambiguous between
        # decimal and thousands notation. Quarantine it instead of guessing.
        if len(fractional) == 3 and whole.lstrip("+-") not in {"", "0"}:
            return None
        text = f"{whole}.{fractional}"
    try:
        number = Decimal(text)
    except (InvalidOperation, ValueError):
        return None
    if not number.is_finite():
        return None
    return number


def _plain_decimal_text(number: Decimal) -> str | None:
    """Render a Decimal without context rounding or exponent amplification."""

    if number == 0:
        return "0"
    sign, digits, exponent = number.as_tuple()
    digit_count = len(digits)
    if exponent >= 0:
        rendered_length = digit_count + exponent + sign
    else:
        decimal_position = digit_count + exponent
        rendered_length = (
            digit_count + 1 + sign
            if decimal_position > 0
            else 2 + (-decimal_position) + digit_count + sign
        )
    if rendered_length > MAX_DECIMAL_RENDER_CHARS:
        return None
    rendered = format(number, "f")
    if "." in rendered:
        rendered = rendered.rstrip("0").rstrip(".")
    if rendered in {"-0", "+0"}:
        return "0"
    return rendered


def _decimal_text(value: Any) -> str | None:
    number = _decimal_or_none(value)
    if number is None:
        return None
    return _plain_decimal_text(number)


def _signed_decimal_text(value: Any) -> str | None:
    return _decimal_text(value)


def _is_exact_bounded_msat(number: Decimal) -> bool:
    """Return whether a BTC value survives the INTEGER-msat boundary exactly."""

    if number < 0 or number > MAX_STORABLE_BTC:
        return False
    if number == 0:
        return True
    if number < MIN_STORABLE_BTC:
        return False
    with localcontext() as context:
        context.prec = max(50, len(number.as_tuple().digits) + 16)
        scaled = number * MSAT_PER_BTC
        integral = scaled.to_integral_value()
    return scaled == integral and integral <= MAX_SQLITE_INT64


def _is_storable_fiat(number: Decimal) -> bool:
    """Keep legacy SQLite REAL pricing finite and non-zero when populated."""

    if number <= 0 or number > MAX_FIAT_FLOAT:
        return False
    converted = float(number)
    return math.isfinite(converted) and converted > 0


def _confidence(value: Any) -> Decimal:
    number = _decimal_or_none(value)
    if number is None:
        return Decimal("0")
    if number < 0:
        return Decimal("0")
    if number > 1:
        return Decimal("1")
    return number


def _direction(value: Any) -> str | None:
    text = str_or_none(value)
    if text is None:
        return None
    normalized = text.strip().lower()
    if normalized in {"in", "incoming", "inbound", "receive", "received", "deposit", "credit", "buy"}:
        return "inbound"
    if normalized in {"out", "outgoing", "outbound", "send", "sent", "withdrawal", "withdraw", "debit", "sell"}:
        return "outbound"
    return None


def _cell_confidences(value: Any) -> dict[str, float]:
    if not isinstance(value, Mapping):
        return {}
    out: dict[str, float] = {}
    for key, raw in value.items():
        if not isinstance(key, str) or key not in _OCR_CONFIDENCE_FIELDS:
            continue
        out[key] = float(_confidence(raw))
    return out


def _source_region(
    value: Any,
    *,
    allowed_pages: Sequence[int] | None = None,
) -> dict[str, Any] | None:
    if not isinstance(value, Mapping):
        return None
    page = _decimal_or_none(value.get("page"))
    if page is None or page != page.to_integral_value():
        return None
    page_number = int(page)
    if page_number < 1:
        return None
    if allowed_pages is not None and page_number not in set(allowed_pages):
        return None
    region: dict[str, Any] = {"page": page_number}
    for key in ("x", "y", "width", "height"):
        number = _decimal_or_none(value.get(key))
        if number is not None and abs(number) <= 1_000_000:
            region[key] = float(number)
    unit = str_or_none(value.get("unit")) or "relative"
    region["unit"] = unit[:32]
    return region


def _row_flags(
    *,
    occurred_at: str | None,
    direction: str | None,
    amount_btc: str | None,
    confidence: Decimal,
    cell_confidences: Mapping[str, float],
    populated_confidence_fields: Sequence[str],
    threshold: Decimal,
    invalid_date: bool = False,
) -> list[str]:
    flags: list[str] = []
    if not occurred_at:
        flags.append("missing_date")
    if invalid_date:
        flags.append("invalid_date")
    if not direction:
        flags.append("missing_direction")
    if not amount_btc:
        flags.append("missing_amount")
    if confidence < threshold:
        flags.append("low_row_confidence")
    for key in populated_confidence_fields:
        value = cell_confidences.get(key)
        if value is None:
            flags.append(f"missing_{key}_confidence")
        elif Decimal(str(value)) < threshold:
            flags.append(f"low_{key}_confidence")
    return flags


def _confidence_review_flags(
    row: Mapping[str, Any],
    *,
    threshold: Decimal,
) -> list[str]:
    """Recompute the confidence gate from canonical visible draft fields."""

    flags: list[str] = []
    if _confidence(row.get("confidence")) < threshold:
        flags.append("low_row_confidence")
    draft = row.get("record")
    if not isinstance(draft, Mapping):
        return [*flags, "missing_review_record"]
    populated_fields = [
        key
        for key, populated in (
            ("occurred_at", draft.get("occurred_at") not in (None, "")),
            ("direction", draft.get("direction") not in (None, "")),
            ("asset", draft.get("asset") not in (None, "")),
            ("amount_btc", draft.get("amount_btc") not in (None, "")),
            (
                "fee_btc",
                draft.get("fee_btc") not in (None, "")
                and not bool(draft.get("fee_defaulted")),
            ),
            ("fiat_currency", draft.get("fiat_currency") not in (None, "")),
            ("fiat_value", draft.get("fiat_value") not in (None, "")),
            ("fiat_rate", draft.get("fiat_rate") not in (None, "")),
            ("counterparty", draft.get("counterparty") not in (None, "")),
            ("description", draft.get("description") not in (None, "")),
        )
        if populated
    ]
    cell_confidences = _cell_confidences(row.get("cell_confidences"))
    for key in populated_fields:
        value = cell_confidences.get(key)
        if value is None:
            flags.append(f"missing_{key}_confidence")
        elif Decimal(str(value)) < threshold:
            flags.append(f"low_{key}_confidence")
    return flags


def _document_row_base_id(
    *,
    source_hash: str,
    occurred_at: str | None,
    direction: str | None,
    asset: str | None,
    amount_btc: str | None,
    fee_btc: str,
) -> str:
    """Build an order-independent identity from the reviewed economic row."""

    canonical = json.dumps(
        {
            "occurred_at": occurred_at,
            "direction": direction,
            "asset": asset,
            "amount_btc": amount_btc,
            "fee_btc": fee_btc,
        },
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode("utf-8")
    digest = hashlib.sha256(canonical).hexdigest()[:16]
    return f"docrow-{source_hash[:16]}-{digest}"


def _set_draft_row_id(row: dict[str, Any], row_id: str) -> None:
    row["id"] = row_id
    import_record = row.get("import_record")
    if not isinstance(import_record, dict):
        return
    import_record["id"] = row_id
    raw_json = import_record.get("raw_json")
    if isinstance(raw_json, dict):
        raw_json["row_id"] = row_id


def _stabilize_duplicate_row_ids(rows: list[dict[str, Any]]) -> None:
    """Assign duplicate ordinals without depending on OCR response order."""

    grouped: dict[str, list[dict[str, Any]]] = {}
    for row in rows:
        base_id = str(row.get("id") or "").rsplit("-", 1)[0]
        grouped.setdefault(base_id, []).append(row)
    for base_id, duplicates in grouped.items():
        duplicates.sort(
            key=lambda row: json.dumps(
                {
                    "source_region": row.get("source_region"),
                    "evidence_text": row.get("evidence_text"),
                    "record": row.get("record"),
                },
                sort_keys=True,
                separators=(",", ":"),
                ensure_ascii=False,
            )
        )
        for ordinal, row in enumerate(duplicates, start=1):
            _set_draft_row_id(row, f"{base_id}-{ordinal:03d}")


def _draft_row(
    raw: Mapping[str, Any],
    *,
    index: int,
    threshold: Decimal,
    source_hash: str,
    expected_fiat_currency: str | None = None,
    allowed_pages: Sequence[int] | None = None,
) -> dict[str, Any]:
    raw_occurred_at = str_or_none(raw.get("occurred_at") or raw.get("date"))
    invalid_date = False
    try:
        occurred_at = parse_timestamp(raw_occurred_at) if raw_occurred_at else None
    except AppError:
        occurred_at = None
        invalid_date = True
    direction = _direction(raw.get("direction"))
    asset_value = str_or_none(raw.get("asset"))
    asset_value = asset_value[:32] if asset_value else None
    asset = asset_value.upper() if asset_value else None
    explicit_crypto_amount = raw.get("amount_btc")
    if explicit_crypto_amount in (None, ""):
        explicit_crypto_amount = raw.get("amount_crypto")
    amount_value = explicit_crypto_amount
    if amount_value in (None, "") and asset:
        amount_value = raw.get("amount")
    amount_present = amount_value not in (None, "")
    amount_btc = _signed_decimal_text(amount_value)
    amount_number = _decimal_or_none(amount_value)
    if asset is None and explicit_crypto_amount not in (None, ""):
        asset = "BTC"
    fee_value = raw.get("fee_btc")
    if fee_value in (None, ""):
        fee_value = raw.get("fee_crypto")
    fee_present = fee_value not in (None, "")
    fee_btc = _signed_decimal_text(fee_value)
    fee_number = _decimal_or_none(fee_value)
    fiat_currency = str_or_none(raw.get("fiat_currency"))
    fiat_currency = fiat_currency.strip().upper()[:16] if fiat_currency else None
    raw_fiat_value = raw.get("fiat_value")
    raw_fiat_rate = raw.get("fiat_rate")
    fiat_value_present = raw_fiat_value not in (None, "")
    fiat_rate_present = raw_fiat_rate not in (None, "")
    fiat_value_number = _decimal_or_none(raw_fiat_value)
    fiat_rate_number = _decimal_or_none(raw_fiat_rate)
    fiat_value = _decimal_text(raw_fiat_value)
    fiat_rate = _decimal_text(raw_fiat_rate)
    amount_display = (
        amount_btc
        if amount_btc is not None
        else (str(amount_value).strip()[:128] if amount_present else None)
    )
    fee_display = (
        fee_btc
        if fee_btc is not None
        else (str(fee_value).strip()[:128] if fee_present else None)
    )
    fiat_value_display = (
        fiat_value
        if fiat_value is not None
        else (str(raw_fiat_value).strip()[:128] if fiat_value_present else None)
    )
    fiat_rate_display = (
        fiat_rate
        if fiat_rate is not None
        else (str(raw_fiat_rate).strip()[:128] if fiat_rate_present else None)
    )
    confidence = _confidence(raw.get("confidence"))
    cell_confidences = _cell_confidences(raw.get("cell_confidences"))
    counterparty = str_or_none(raw.get("counterparty"))
    description = str_or_none(raw.get("description"))
    populated_confidence_fields = [
        key
        for key, populated in (
            ("occurred_at", raw_occurred_at is not None),
            ("direction", str_or_none(raw.get("direction")) is not None),
            ("asset", asset is not None),
            ("amount_btc", amount_value not in (None, "")),
            ("fee_btc", fee_present),
            ("fiat_currency", fiat_currency is not None),
            ("fiat_value", fiat_value_present),
            ("fiat_rate", fiat_rate_present),
            ("counterparty", counterparty is not None),
            ("description", description is not None),
        )
        if populated
    ]
    flags = _row_flags(
        occurred_at=occurred_at,
        direction=direction,
        amount_btc=amount_btc,
        confidence=confidence,
        cell_confidences=cell_confidences,
        populated_confidence_fields=populated_confidence_fields,
        threshold=threshold,
        invalid_date=invalid_date,
    )
    if asset is not None and asset not in SUPPORTED_DOCUMENT_ASSETS:
        flags.append("unsupported_asset")
    if amount_number is not None and amount_number <= 0:
        flags.append("non_positive_amount")
    elif amount_number is not None and not _is_exact_bounded_msat(amount_number):
        flags.append("amount_not_representable")
    if fee_present and fee_number is None:
        flags.append("invalid_fee")
    elif fee_number is not None and fee_number < 0:
        flags.append("negative_fee")
    elif fee_number is not None and not _is_exact_bounded_msat(fee_number):
        flags.append("fee_not_representable")
    if fiat_value_present and fiat_value_number is None:
        flags.append("invalid_fiat_value")
    elif fiat_value_number is not None and fiat_value_number <= 0:
        flags.append("non_positive_fiat_value")
    elif fiat_value_number is not None and not _is_storable_fiat(fiat_value_number):
        flags.append("fiat_value_out_of_range")
    if fiat_rate_present and fiat_rate_number is None:
        flags.append("invalid_fiat_rate")
    elif fiat_rate_number is not None and fiat_rate_number <= 0:
        flags.append("non_positive_fiat_rate")
    elif fiat_rate_number is not None and not _is_storable_fiat(fiat_rate_number):
        flags.append("fiat_rate_out_of_range")
    has_fiat_fact = fiat_value_present or fiat_rate_present
    if has_fiat_fact and not fiat_currency:
        flags.append("missing_fiat_currency")
    normalized_expected_currency = str(expected_fiat_currency or "").strip().upper()
    if (
        fiat_currency
        and normalized_expected_currency
        and fiat_currency != normalized_expected_currency
    ):
        flags.append("fiat_currency_mismatch")
    raw_source_region = raw.get("source_region")
    source_region = _source_region(raw_source_region, allowed_pages=allowed_pages)
    if isinstance(raw_source_region, Mapping) and source_region is None:
        flags.append("invalid_source_page")
    status = "ready" if not flags else "quarantined"
    evidence_text = str_or_none(raw.get("evidence_text"))
    evidence_text = evidence_text[:4000] if evidence_text else None
    counterparty = counterparty[:500] if counterparty else None
    description = description[:4000] if description else None
    model_row_id = str_or_none(raw.get("id") or raw.get("row_id"))
    model_row_id = model_row_id[:256] if model_row_id else None
    row_id = (
        _document_row_base_id(
            source_hash=source_hash,
            occurred_at=occurred_at,
            direction=direction,
            asset=asset,
            amount_btc=amount_btc,
            fee_btc=fee_display or "0",
        )
        + "-001"
    )
    fiat_values_valid = (
        (
            not fiat_value_present
            or (fiat_value_number is not None and _is_storable_fiat(fiat_value_number))
        )
        and (
            not fiat_rate_present
            or (fiat_rate_number is not None and _is_storable_fiat(fiat_rate_number))
        )
        and (not has_fiat_fact or bool(fiat_currency))
        and (
            not fiat_currency
            or not normalized_expected_currency
            or fiat_currency == normalized_expected_currency
        )
    )
    import_record = None
    if (
        occurred_at
        and direction
        and amount_btc
        and amount_number is not None
        and amount_number > 0
        and _is_exact_bounded_msat(amount_number)
        and asset in SUPPORTED_DOCUMENT_ASSETS
        and (
            not fee_present
            or (
                fee_number is not None
                and fee_number >= 0
                and _is_exact_bounded_msat(fee_number)
            )
        )
        and fiat_values_valid
    ):
        import_record = {
            "id": row_id,
            "occurred_at": occurred_at,
            "direction": direction,
            "asset": asset,
            "amount": amount_btc,
            "fee": fee_btc or "0",
            "fiat_currency": fiat_currency,
            "fiat_value": fiat_value,
            "fiat_rate": fiat_rate,
            "counterparty": counterparty,
            "description": description,
            "raw_json": {
                "source": "document_import",
                "row_id": row_id,
                "model_row_id": model_row_id,
                "model_confidence": float(confidence),
                "cell_confidences": cell_confidences,
                "fee_defaulted": not fee_present,
                "source_region": source_region,
                "evidence_text": evidence_text,
            },
        }
    return {
        "id": row_id,
        "status": status,
        "flags": flags,
        "confidence": float(confidence),
        "cell_confidences": cell_confidences,
        "confidence_threshold": float(threshold),
        "source_region": source_region,
        "evidence_text": evidence_text,
        "record": {
            "occurred_at": occurred_at,
            "direction": direction,
            "asset": asset,
            "amount_btc": amount_display,
            "fee_btc": fee_display or "0",
            "fee_defaulted": not fee_present,
            "fiat_currency": fiat_currency,
            "fiat_value": fiat_value_display,
            "fiat_rate": fiat_rate_display,
            "counterparty": counterparty,
            "description": description,
        },
        "import_record": import_record,
    }


def _draft_rows(
    payload: Mapping[str, Any],
    *,
    threshold: Decimal,
    source_hash: str,
    expected_fiat_currency: str | None = None,
    allowed_pages: Sequence[int] | None = None,
) -> list[dict[str, Any]]:
    raw_rows = payload.get("rows")
    if not isinstance(raw_rows, list):
        raise AppError(
            "Local AI OCR output must contain a rows list",
            code="document_import_ai_response_invalid",
            details={"keys": sorted(str(k) for k in payload.keys())},
            retryable=True,
        )
    if len(raw_rows) > MAX_DRAFT_ROWS:
        raise AppError(
            "Local AI OCR output contains too many rows",
            code="document_import_ai_response_invalid",
            details={"rows": len(raw_rows), "max_rows": MAX_DRAFT_ROWS},
            retryable=False,
        )
    rows: list[dict[str, Any]] = []
    for index, raw in enumerate(raw_rows, start=1):
        if not isinstance(raw, Mapping):
            continue
        rows.append(
            _draft_row(
                raw,
                index=index,
                threshold=threshold,
                source_hash=source_hash,
                expected_fiat_currency=expected_fiat_currency,
                allowed_pages=allowed_pages,
            )
        )
    _stabilize_duplicate_row_ids(rows)
    return rows


def _confidence_threshold(value: Any) -> Decimal:
    if value in (None, ""):
        return DEFAULT_CONFIDENCE_THRESHOLD
    number = _decimal_or_none(value)
    if number is None or number < 0 or number > 1:
        raise AppError(
            "confidence_threshold must be a number between 0 and 1",
            code="validation",
            retryable=False,
        )
    return number


def _max_pages(value: Any) -> int:
    if value in (None, ""):
        return DEFAULT_MAX_PDF_PAGES
    try:
        pages = int(value)
    except (TypeError, ValueError) as exc:
        raise AppError("max_pages must be an integer", code="validation") from exc
    if pages < 1 or pages > MAX_RENDERED_PDF_PAGES:
        raise AppError(
            f"max_pages must be between 1 and {MAX_RENDERED_PDF_PAGES}",
            code="validation",
            details={"max_pages": MAX_RENDERED_PDF_PAGES},
            retryable=False,
        )
    return pages


def _client_for_provider(provider: dict[str, Any]) -> Any:
    return ai_client_for_locator(
        base_url=provider["base_url"],
        api_key=get_ai_provider_api_key_for_use(provider),
        direct_connection=True,
    )


def preview_document_import(
    conn: sqlite3.Connection,
    *,
    source_file: str,
    provider_name: str | None = None,
    model: str | None = None,
    confidence_threshold: Any = None,
    max_pages: Any = None,
    pages: Any = None,
    expected_fiat_currency: str | None = None,
    client_factory: ClientFactory | None = None,
) -> dict[str, Any]:
    source_path = _source_path(source_file)
    threshold = _confidence_threshold(confidence_threshold)
    page_limit = _max_pages(max_pages)
    provider = resolve_ai_provider(conn, provider_name)
    _validate_local_provider(provider)
    client = (client_factory or _client_for_provider)(provider)
    models = client.list_models(strict=True)
    selected_model, installed_models = _choose_model(provider, models, model)

    stable_dir = tempfile.TemporaryDirectory(prefix="kassiber-ocr-source-")
    stable_path = Path(stable_dir.name) / source_path.name
    shutil.copyfile(source_path, stable_path)
    source_hash = _sha256_file(stable_path)
    try:
        parts, cleanup, pdf_metadata = _document_parts(
            stable_path,
            max_pages=page_limit,
            pages=pages,
        )
    except Exception:
        stable_dir.cleanup()
        raise
    try:
        message_content: list[dict[str, Any]] = [
            {
                "type": "text",
                "text": (
                    "Extract a draft Kassiber transaction table from this local document. "
                    "Return JSON only and include per-cell confidence plus source regions."
                ),
            },
            *parts,
        ]
        response = client.chat(
            model=selected_model,
            messages=[
                {"role": "system", "content": OCR_SYSTEM_PROMPT},
                {"role": "user", "content": message_content},
            ],
            options={
                "temperature": 0,
                "response_format": {"type": "json_object"},
            },
        )
    finally:
        cleanup()
        stable_dir.cleanup()

    payload = _extract_json_object(str(response.get("content") or ""))
    if _sha256_file(source_path) != source_hash:
        raise AppError(
            "The OCR source changed while preview was running",
            code="document_import_source_changed",
            hint="Preview the document again before reviewing its rows.",
            details={"filename": source_path.name},
            retryable=False,
        )
    allowed_pages = (
        pdf_metadata["rendered_pages"] if pdf_metadata is not None else [1]
    )
    rows = _draft_rows(
        payload,
        threshold=threshold,
        source_hash=source_hash,
        expected_fiat_currency=expected_fiat_currency,
        allowed_pages=allowed_pages,
    )
    ready = sum(1 for row in rows if row["status"] == "ready")
    quarantined = sum(1 for row in rows if row["status"] != "ready")
    return {
        "source": {
            "path": str(source_path),
            "filename": source_path.name,
            "media_type": _mime_type(source_path),
            "size_bytes": source_path.stat().st_size,
            "sha256": source_hash,
            "kind": "pdf" if source_path.suffix.lower() == PDF_EXTENSION else "image",
            **({"pdf": pdf_metadata} if pdf_metadata is not None else {}),
        },
        "provider": {
            "name": provider["name"],
            "kind": provider["kind"],
        },
        "model": selected_model,
        "installed_models": installed_models,
        "recommendations": model_recommendations(),
        "confidence_threshold": float(threshold),
        "expected_fiat_currency": (
            str(expected_fiat_currency).strip().upper() if expected_fiat_currency else None
        ),
        "rows": rows,
        "summary": {
            "rows": len(rows),
            "ready": ready,
            "quarantined": quarantined,
            "has_importable_rows": ready > 0,
        },
        "created_at": now_iso(),
    }


def _import_records_from_rows(
    rows: Sequence[Mapping[str, Any]],
    *,
    include_quarantined: bool,
    selected_row_ids: Sequence[str] | None,
    source_hash: str,
    expected_fiat_currency: str | None = None,
    confidence_threshold: Any = None,
) -> tuple[list[dict[str, Any]], int]:
    if len(rows) > MAX_DRAFT_ROWS:
        raise AppError(
            "Document draft contains too many rows",
            code="validation",
            details={"rows": len(rows), "max_rows": MAX_DRAFT_ROWS},
            retryable=False,
        )
    selected = (
        {str(row_id) for row_id in selected_row_ids if str(row_id)}
        if selected_row_ids is not None
        else None
    )
    records: list[dict[str, Any]] = []
    skipped_quarantined = 0
    threshold = _confidence_threshold(confidence_threshold)
    for row in rows:
        row_id = str(row.get("id") or "")
        if selected is not None and row_id not in selected:
            continue
        confidence_flags = _confidence_review_flags(row, threshold=threshold)
        if confidence_flags and not include_quarantined:
            skipped_quarantined += 1
            continue
        record = _import_record_from_draft_row(
            row,
            source_hash=source_hash,
            expected_fiat_currency=expected_fiat_currency,
        )
        if record is None:
            skipped_quarantined += 1
            continue
        if confidence_flags:
            raw_json = record.get("raw_json")
            if isinstance(raw_json, dict):
                raw_json["confidence_override"] = True
                raw_json["review_flags"] = confidence_flags
        records.append(record)
    return records, skipped_quarantined


def _import_record_from_draft_row(
    row: Mapping[str, Any],
    *,
    source_hash: str,
    expected_fiat_currency: str | None = None,
) -> dict[str, Any] | None:
    """Rebuild an import row from validated public draft fields.

    The renderer receives ``import_record`` for display convenience, but the
    daemon never trusts that hidden object on the write path.
    """

    row_id = str(row.get("id") or "")
    if not re.fullmatch(
        rf"docrow-{re.escape(source_hash[:16])}-[0-9a-f]{{16}}-\d{{3}}",
        row_id,
    ):
        return None
    flags = row.get("flags")
    if isinstance(flags, Sequence) and not isinstance(flags, (str, bytes)):
        if any(str(flag) in _STRUCTURAL_ROW_FLAGS for flag in flags):
            return None
    draft = row.get("record")
    if not isinstance(draft, Mapping):
        return None
    try:
        occurred_at = parse_timestamp(draft.get("occurred_at"))
    except AppError:
        return None
    direction = _direction(draft.get("direction"))
    asset = (str_or_none(draft.get("asset")) or "").upper()
    raw_amount = draft.get("amount_btc")
    raw_fee = draft.get("fee_btc")
    amount = _signed_decimal_text(raw_amount)
    fee_present = raw_fee not in (None, "")
    fee = _signed_decimal_text(raw_fee) if fee_present else "0"
    amount_number = _decimal_or_none(raw_amount)
    fee_number = Decimal("0") if not fee_present else _decimal_or_none(raw_fee)
    raw_fiat_value = draft.get("fiat_value")
    raw_fiat_rate = draft.get("fiat_rate")
    fiat_value_present = raw_fiat_value not in (None, "")
    fiat_rate_present = raw_fiat_rate not in (None, "")
    fiat_value_number = _decimal_or_none(raw_fiat_value)
    fiat_rate_number = _decimal_or_none(raw_fiat_rate)
    fiat_currency = str_or_none(draft.get("fiat_currency"))
    fiat_currency = fiat_currency.strip().upper()[:16] if fiat_currency else None
    has_fiat_fact = fiat_value_present or fiat_rate_present
    normalized_expected_currency = str(expected_fiat_currency or "").strip().upper()
    if (
        direction is None
        or asset not in SUPPORTED_DOCUMENT_ASSETS
        or amount_number is None
        or amount_number <= 0
        or not _is_exact_bounded_msat(amount_number)
        or amount is None
        or fee_number is None
        or fee_number < 0
        or not _is_exact_bounded_msat(fee_number)
        or fee is None
        or (
            fiat_value_present
            and (
                fiat_value_number is None
                or not _is_storable_fiat(fiat_value_number)
                or _decimal_text(raw_fiat_value) is None
            )
        )
        or (
            fiat_rate_present
            and (
                fiat_rate_number is None
                or not _is_storable_fiat(fiat_rate_number)
                or _decimal_text(raw_fiat_rate) is None
            )
        )
        or (has_fiat_fact and not fiat_currency)
        or (
            fiat_currency
            and normalized_expected_currency
            and fiat_currency != normalized_expected_currency
        )
    ):
        return None
    confidence = _confidence(row.get("confidence"))
    cell_confidences = _cell_confidences(row.get("cell_confidences"))
    evidence_text = str_or_none(row.get("evidence_text"))
    if evidence_text:
        evidence_text = evidence_text[:4000]
    counterparty = str_or_none(draft.get("counterparty"))
    counterparty = counterparty[:500] if counterparty else None
    description = str_or_none(draft.get("description"))
    description = description[:4000] if description else None
    return {
        "id": row_id,
        "occurred_at": occurred_at,
        "direction": direction,
        "asset": asset,
        "amount": amount,
        "fee": fee,
        "fiat_currency": fiat_currency,
        "fiat_value": _decimal_text(raw_fiat_value),
        "fiat_rate": _decimal_text(raw_fiat_rate),
        "counterparty": counterparty,
        "description": description,
        "raw_json": {
            "source": "document_import",
            "row_id": row_id,
            "model_confidence": float(confidence),
            "cell_confidences": cell_confidences,
            "fee_defaulted": bool(draft.get("fee_defaulted")),
            "source_region": _source_region(row.get("source_region")),
            "evidence_text": evidence_text,
        },
    }


def import_document_draft(
    conn: sqlite3.Connection,
    *,
    source_file: str,
    data_root: str | None = None,
    wallet: Mapping[str, Any],
    profile: Mapping[str, Any],
    rows: Sequence[Mapping[str, Any]],
    hooks: DocumentImportHooks,
    include_quarantined: bool = False,
    selected_row_ids: Sequence[str] | None = None,
    expected_source_sha256: str | None = None,
    confidence_threshold: Any = None,
    attach_evidence: bool = True,
    commit: bool = True,
) -> dict[str, Any]:
    source_path = _source_path(source_file)
    stable_dir = tempfile.TemporaryDirectory(prefix="kassiber-ocr-import-")
    stable_path = Path(stable_dir.name) / source_path.name
    try:
        shutil.copyfile(source_path, stable_path)
        _source_path(str(stable_path))
        source_sha256 = _sha256_file(stable_path)
    except Exception:
        stable_dir.cleanup()
        raise
    if expected_source_sha256 is not None:
        expected = expected_source_sha256.strip().lower()
        if not re.fullmatch(r"[0-9a-f]{64}", expected):
            stable_dir.cleanup()
            raise AppError(
                "Preview source hash is invalid",
                code="validation",
                retryable=False,
            )
        if source_sha256 != expected:
            stable_dir.cleanup()
            raise AppError(
                "The OCR source changed after preview",
                code="document_import_source_changed",
                hint="Preview the document again before importing its rows.",
                details={"filename": source_path.name},
                retryable=False,
            )
    try:
        records, skipped_quarantined = _import_records_from_rows(
            rows,
            include_quarantined=include_quarantined,
            selected_row_ids=selected_row_ids,
            source_hash=source_sha256,
            expected_fiat_currency=str(profile["fiat_currency"] or ""),
            confidence_threshold=confidence_threshold,
        )
    except Exception:
        stable_dir.cleanup()
        raise
    if not records:
        stable_dir.cleanup()
        raise AppError(
            "Document draft has no importable rows",
            code="document_import_no_ready_rows",
            hint="Review the OCR draft or retry with a clearer image/local vision model.",
            details={"quarantined_skipped": skipped_quarantined},
            retryable=False,
        )
    source_bytes = stable_path.stat().st_size
    projected_attachment_bytes = source_bytes * len(records)
    if attach_evidence and projected_attachment_bytes > MAX_PROJECTED_ATTACHMENT_BYTES:
        stable_dir.cleanup()
        raise AppError(
            "Document evidence copies would exceed the import storage budget",
            code="document_import_evidence_budget_exceeded",
            hint="Import fewer rows at a time or use a smaller source image/PDF.",
            details={
                "rows": len(records),
                "source_bytes": source_bytes,
                "projected_bytes": projected_attachment_bytes,
                "max_projected_bytes": MAX_PROJECTED_ATTACHMENT_BYTES,
            },
            retryable=False,
        )
    resolved_data_root = data_root or _data_root_from_connection(conn)
    attachments_root = core_attachments._attachments_root(resolved_data_root)
    savepoint = f"document_import_{uuid.uuid4().hex}"
    try:
        conn.execute(f"SAVEPOINT {savepoint}")
    except Exception:
        stable_dir.cleanup()
        raise
    savepoint_active = True
    attached: list[dict[str, Any]] = []
    copied_paths: list[Path] = []
    try:
        outcome = core_imports.import_records_into_wallet(
            conn,
            profile,
            wallet,
            records,
            DOCUMENT_IMPORT_FORMAT,
            hooks.import_hooks,
            commit=False,
            report_updates=True,
        )
        if attach_evidence:
            changed_records = [
                *(outcome.get("inserted_records") or []),
                *(outcome.get("updated_records") or []),
            ]
            for changed in changed_records:
                tx_id = changed.get("transaction_id")
                if not isinstance(tx_id, str) or not tx_id:
                    continue
                attachment = core_attachments.add_attachment(
                    conn,
                    resolved_data_root,
                    None,
                    None,
                    tx_id,
                    hooks.attachment_hooks,
                    file_path=str(stable_path),
                    label=f"OCR source: {source_path.name}",
                    media_type=_mime_type(stable_path),
                    commit=False,
                )
                if attachment.get("sha256") != source_sha256:
                    raise AppError(
                        "Managed OCR evidence did not match the reviewed source bytes",
                        code="document_import_source_changed",
                        retryable=False,
                    )
                copied_path, valid_path = core_attachments._resolve_stored_path(
                    attachments_root,
                    attachment.get("stored_relpath"),
                )
                if copied_path is not None and valid_path:
                    copied_paths.append(copied_path)
                attached.append(
                    {
                        "transaction_id": tx_id,
                        "attachment_id": attachment["id"],
                        "stored_relpath": attachment.get("stored_relpath") or "",
                    }
                )
        conn.execute(f"RELEASE SAVEPOINT {savepoint}")
        savepoint_active = False
        if commit:
            conn.commit()
    except Exception:
        try:
            if savepoint_active:
                conn.execute(f"ROLLBACK TO SAVEPOINT {savepoint}")
                conn.execute(f"RELEASE SAVEPOINT {savepoint}")
            else:
                conn.rollback()
        finally:
            for copied_path in copied_paths:
                try:
                    copied_path.unlink()
                except OSError:
                    # Best-effort rollback cleanup: the DB savepoint is still
                    # authoritative and attachment GC can remove any orphan.
                    continue
            stable_dir.cleanup()
        raise
    outcome.update(
        {
            "source": {
                "path": str(source_path),
                "filename": source_path.name,
                "sha256": source_sha256,
            },
            "source_format": DOCUMENT_IMPORT_FORMAT,
            "draft_rows_imported": len(records),
            "quarantined_skipped": skipped_quarantined,
            "attached_evidence": attached,
        }
    )
    stable_dir.cleanup()
    return outcome


def _data_root_from_connection(conn: sqlite3.Connection) -> str:
    try:
        rows = conn.execute("PRAGMA database_list").fetchall()
    except Exception:
        return ""
    for row in rows:
        try:
            name = row["name"]
            filename = row["file"]
        except (KeyError, TypeError, IndexError):
            name = row[1] if len(row) > 1 else None
            filename = row[2] if len(row) > 2 else None
        if name == "main" and filename:
            return str(Path(str(filename)).expanduser().resolve().parent)
    return ""


def preview_then_import_document(
    conn: sqlite3.Connection,
    *,
    source_file: str,
    data_root: str | None = None,
    wallet: Mapping[str, Any],
    profile: Mapping[str, Any],
    hooks: DocumentImportHooks,
    provider_name: str | None = None,
    model: str | None = None,
    confidence_threshold: Any = None,
    max_pages: Any = None,
    pages: Any = None,
    include_quarantined: bool = False,
    client_factory: ClientFactory | None = None,
) -> dict[str, Any]:
    draft = preview_document_import(
        conn,
        source_file=source_file,
        provider_name=provider_name,
        model=model,
        confidence_threshold=confidence_threshold,
        max_pages=max_pages,
        pages=pages,
        expected_fiat_currency=str(profile["fiat_currency"] or ""),
        client_factory=client_factory,
    )
    outcome = import_document_draft(
        conn,
        source_file=source_file,
        data_root=data_root,
        wallet=wallet,
        profile=profile,
        rows=draft["rows"],
        hooks=hooks,
        include_quarantined=include_quarantined,
        expected_source_sha256=draft["source"]["sha256"],
        confidence_threshold=confidence_threshold,
        commit=True,
    )
    return {"draft": draft, "import": outcome}


__all__ = [
    "DEFAULT_CONFIDENCE_THRESHOLD",
    "DEFAULT_MAX_PDF_PAGES",
    "DOCUMENT_IMPORT_FORMAT",
    "DocumentImportHooks",
    "MAX_DRAFT_ROWS",
    "MAX_PROJECTED_ATTACHMENT_BYTES",
    "MODEL_RECOMMENDATIONS",
    "PDF_RENDER_TIMEOUT_SECONDS",
    "SUPPORTED_DOCUMENT_ASSETS",
    "looks_like_vision_model",
    "model_recommendations",
    "preview_document_import",
    "import_document_draft",
    "preview_then_import_document",
]
