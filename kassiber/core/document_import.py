from __future__ import annotations

"""Local-AI document importer for photo/PDF transaction evidence."""

import base64
import ipaddress
import json
import mimetypes
import re
import shutil
import sqlite3
import subprocess
import tempfile
import uuid
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation
from pathlib import Path
from typing import Any, Callable, Mapping, Sequence
from urllib.parse import urlparse

from ..ai.client import ai_client_for_locator
from ..ai.providers import get_ai_provider_api_key_for_use, resolve_ai_provider
from ..errors import AppError
from ..time_utils import now_iso, parse_timestamp
from ..util import str_or_none
from . import attachments as core_attachments
from . import imports as core_imports


DEFAULT_CONFIDENCE_THRESHOLD = Decimal("0.78")
DEFAULT_MAX_PDF_PAGES = 3
MAX_RENDERED_PDF_PAGES = 8
MAX_SOURCE_BYTES = 25 * 1024 * 1024
PDF_RENDER_DPI = 180
DOCUMENT_IMPORT_FORMAT = "document_import"

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
        "amount_btc": 0.0,
        "fiat_value": 0.0,
        "counterparty": 0.0
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
- Use positive amount_btc and direction to express side.
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


def _model_base_name(model_id: str) -> str:
    return model_id.split(":", 1)[0].strip().lower()


def looks_like_vision_model(model_id: str) -> bool:
    lowered = model_id.strip().lower()
    return any(hint in lowered for hint in VISION_MODEL_NAME_HINTS)


def _model_matches(candidate: str, expected: str) -> bool:
    candidate_lower = candidate.lower()
    expected_lower = expected.lower()
    return (
        candidate_lower == expected_lower
        or _model_base_name(candidate_lower) == _model_base_name(expected_lower)
    )


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
        if installed and not any(_model_matches(candidate, model) for candidate in installed):
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
        return model, installed

    configured_default = _model_id(provider.get("default_model"))
    if configured_default and any(
        _model_matches(candidate, configured_default) for candidate in installed
    ) and looks_like_vision_model(configured_default):
        return configured_default, installed

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
    import hashlib

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


def _render_pdf_pages(path: Path, *, max_pages: int) -> tuple[list[Path], tempfile.TemporaryDirectory[str]]:
    pdftoppm = shutil.which("pdftoppm")
    if not pdftoppm:
        raise AppError(
            "PDF OCR import requires Poppler's pdftoppm",
            code="document_import_pdf_renderer_missing",
            hint="Install poppler-utils, or export the statement pages as images and import those.",
            details={"tool": "pdftoppm"},
            retryable=False,
        )
    tempdir = tempfile.TemporaryDirectory(prefix="kassiber-document-ocr-")
    prefix = Path(tempdir.name) / "page"
    pages = max(1, min(max_pages, MAX_RENDERED_PDF_PAGES))
    command = [
        pdftoppm,
        "-png",
        "-r",
        str(PDF_RENDER_DPI),
        "-f",
        "1",
        "-l",
        str(pages),
        str(path),
        str(prefix),
    ]
    completed = subprocess.run(command, text=True, capture_output=True, check=False)
    if completed.returncode != 0:
        tempdir.cleanup()
        raise AppError(
            "Could not render PDF pages for OCR",
            code="document_import_pdf_render_failed",
            hint="Open the PDF locally and export the transaction pages as PNG/JPEG, then import the images.",
            details={"stderr": (completed.stderr or "").strip()[-2048:]},
            retryable=False,
        )
    rendered = sorted(Path(tempdir.name).glob("page-*.png"))
    if not rendered:
        tempdir.cleanup()
        raise AppError(
            "PDF rendering produced no pages",
            code="document_import_pdf_render_failed",
            hint="Check whether the PDF is password-protected, then export the transaction pages as images.",
            retryable=False,
        )
    return rendered[:pages], tempdir


def _document_parts(path: Path, *, max_pages: int) -> tuple[list[dict[str, Any]], Callable[[], None]]:
    if path.suffix.lower() in IMAGE_EXTENSIONS:
        return [_image_content_part(path)], lambda: None

    rendered, tempdir = _render_pdf_pages(path, max_pages=max_pages)
    parts: list[dict[str, Any]] = []
    for index, page_path in enumerate(rendered, start=1):
        parts.append({"type": "text", "text": f"PDF page {index}:"})
        parts.append(_image_content_part(page_path))
    return parts, tempdir.cleanup


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
    text = str(value).strip().replace("\u00a0", "").replace(" ", "")
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


def _decimal_text(value: Any) -> str | None:
    number = _decimal_or_none(value)
    if number is None:
        return None
    return format(abs(number).normalize(), "f")


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
        if not isinstance(key, str):
            continue
        out[key] = float(_confidence(raw))
    return out


def _source_region(value: Any) -> dict[str, Any] | None:
    if not isinstance(value, Mapping):
        return None
    page = value.get("page")
    try:
        page_number = int(page)
    except (TypeError, ValueError):
        page_number = 1
    region: dict[str, Any] = {"page": max(1, page_number)}
    for key in ("x", "y", "width", "height"):
        number = _decimal_or_none(value.get(key))
        if number is not None:
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
    for key in ("occurred_at", "direction", "amount_btc"):
        value = cell_confidences.get(key)
        if value is not None and Decimal(str(value)) < threshold:
            flags.append(f"low_{key}_confidence")
    return flags


def _draft_row(
    raw: Mapping[str, Any],
    *,
    index: int,
    threshold: Decimal,
    source_hash: str,
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
    asset = asset_value.upper() if asset_value else None
    explicit_crypto_amount = raw.get("amount_btc")
    if explicit_crypto_amount in (None, ""):
        explicit_crypto_amount = raw.get("amount_crypto")
    amount_value = explicit_crypto_amount
    if amount_value in (None, "") and asset:
        amount_value = raw.get("amount")
    amount_btc = _decimal_text(amount_value)
    if asset is None and explicit_crypto_amount not in (None, ""):
        asset = "BTC"
    fee_value = raw.get("fee_btc")
    if fee_value in (None, ""):
        fee_value = raw.get("fee_crypto")
    fee_btc = _decimal_text(fee_value)
    fiat_currency = str_or_none(raw.get("fiat_currency"))
    fiat_value = _decimal_text(raw.get("fiat_value"))
    fiat_rate = _decimal_text(raw.get("fiat_rate"))
    confidence = _confidence(raw.get("confidence"))
    cell_confidences = _cell_confidences(raw.get("cell_confidences"))
    flags = _row_flags(
        occurred_at=occurred_at,
        direction=direction,
        amount_btc=amount_btc,
        confidence=confidence,
        cell_confidences=cell_confidences,
        threshold=threshold,
        invalid_date=invalid_date,
    )
    status = "ready" if not flags else "quarantined"
    evidence_text = str_or_none(raw.get("evidence_text"))
    counterparty = str_or_none(raw.get("counterparty"))
    description = str_or_none(raw.get("description"))
    model_row_id = str_or_none(raw.get("id") or raw.get("row_id"))
    row_id = f"docrow-{source_hash[:16]}-{index:03d}"
    import_record = None
    if occurred_at and direction and amount_btc and asset:
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
            "description": description or evidence_text,
            "raw_json": {
                "source": "document_import",
                "row_id": row_id,
                "model_row_id": model_row_id,
                "model_confidence": float(confidence),
                "cell_confidences": cell_confidences,
                "source_region": _source_region(raw.get("source_region")),
                "evidence_text": evidence_text,
            },
        }
    return {
        "id": row_id,
        "status": status,
        "flags": flags,
        "confidence": float(confidence),
        "cell_confidences": cell_confidences,
        "source_region": _source_region(raw.get("source_region")),
        "evidence_text": evidence_text,
        "record": {
            "occurred_at": occurred_at,
            "direction": direction,
            "asset": asset,
            "amount_btc": amount_btc,
            "fee_btc": fee_btc or "0",
            "fiat_currency": fiat_currency,
            "fiat_value": fiat_value,
            "fiat_rate": fiat_rate,
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
) -> list[dict[str, Any]]:
    raw_rows = payload.get("rows")
    if not isinstance(raw_rows, list):
        raise AppError(
            "Local AI OCR output must contain a rows list",
            code="document_import_ai_response_invalid",
            details={"keys": sorted(str(k) for k in payload.keys())},
            retryable=True,
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
            )
        )
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
    )


def preview_document_import(
    conn: sqlite3.Connection,
    *,
    source_file: str,
    provider_name: str | None = None,
    model: str | None = None,
    confidence_threshold: Any = None,
    max_pages: Any = None,
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
        parts, cleanup = _document_parts(stable_path, max_pages=page_limit)
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
    rows = _draft_rows(payload, threshold=threshold, source_hash=source_hash)
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
        },
        "provider": {
            "name": provider["name"],
            "kind": provider["kind"],
        },
        "model": selected_model,
        "installed_models": installed_models,
        "recommendations": model_recommendations(),
        "confidence_threshold": float(threshold),
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
) -> tuple[list[dict[str, Any]], int]:
    selected = (
        {str(row_id) for row_id in selected_row_ids if str(row_id)}
        if selected_row_ids is not None
        else None
    )
    records: list[dict[str, Any]] = []
    skipped_quarantined = 0
    for row in rows:
        row_id = str(row.get("id") or "")
        if selected is not None and row_id not in selected:
            continue
        status = str(row.get("status") or "")
        if status != "ready" and not include_quarantined:
            skipped_quarantined += 1
            continue
        record = _import_record_from_draft_row(row, source_hash=source_hash)
        if record is None:
            skipped_quarantined += 1
            continue
        records.append(record)
    return records, skipped_quarantined


def _import_record_from_draft_row(
    row: Mapping[str, Any],
    *,
    source_hash: str,
) -> dict[str, Any] | None:
    """Rebuild an import row from validated public draft fields.

    The renderer receives ``import_record`` for display convenience, but the
    daemon never trusts that hidden object on the write path.
    """

    row_id = str(row.get("id") or "")
    if not re.fullmatch(rf"docrow-{re.escape(source_hash[:16])}-\d{{3}}", row_id):
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
    amount = _decimal_text(draft.get("amount_btc"))
    fee = _decimal_text(draft.get("fee_btc")) or "0"
    if direction is None or asset not in {"BTC", "LBTC"} or amount is None:
        return None
    confidence = _confidence(row.get("confidence"))
    cell_confidences = _cell_confidences(row.get("cell_confidences"))
    evidence_text = str_or_none(row.get("evidence_text"))
    if evidence_text:
        evidence_text = evidence_text[:4000]
    return {
        "id": row_id,
        "occurred_at": occurred_at,
        "direction": direction,
        "asset": asset,
        "amount": amount,
        "fee": fee,
        "fiat_currency": str_or_none(draft.get("fiat_currency")),
        "fiat_value": _decimal_text(draft.get("fiat_value")),
        "fiat_rate": _decimal_text(draft.get("fiat_rate")),
        "counterparty": str_or_none(draft.get("counterparty")),
        "description": str_or_none(draft.get("description")) or evidence_text,
        "raw_json": {
            "source": "document_import",
            "row_id": row_id,
            "model_confidence": float(confidence),
            "cell_confidences": cell_confidences,
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
    attach_evidence: bool = True,
    commit: bool = True,
) -> dict[str, Any]:
    source_path = _source_path(source_file)
    source_sha256 = _sha256_file(source_path)
    if expected_source_sha256 is not None:
        expected = expected_source_sha256.strip().lower()
        if not re.fullmatch(r"[0-9a-f]{64}", expected):
            raise AppError(
                "Preview source hash is invalid",
                code="validation",
                retryable=False,
            )
        if source_sha256 != expected:
            raise AppError(
                "The OCR source changed after preview",
                code="document_import_source_changed",
                hint="Preview the document again before importing its rows.",
                details={"filename": source_path.name},
                retryable=False,
            )
    records, skipped_quarantined = _import_records_from_rows(
        rows,
        include_quarantined=include_quarantined,
        selected_row_ids=selected_row_ids,
        source_hash=source_sha256,
    )
    if not records:
        raise AppError(
            "Document draft has no importable rows",
            code="document_import_no_ready_rows",
            hint="Review the OCR draft or retry with a clearer image/local vision model.",
            details={"quarantined_skipped": skipped_quarantined},
            retryable=False,
        )
    resolved_data_root = data_root or _data_root_from_connection(conn)
    attachments_root = core_attachments._attachments_root(resolved_data_root)
    savepoint = f"document_import_{uuid.uuid4().hex}"
    conn.execute(f"SAVEPOINT {savepoint}")
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
                    file_path=str(source_path),
                    label=f"OCR source: {source_path.name}",
                    media_type=_mime_type(source_path),
                    commit=False,
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
        commit=True,
    )
    return {"draft": draft, "import": outcome}


__all__ = [
    "DEFAULT_CONFIDENCE_THRESHOLD",
    "DEFAULT_MAX_PDF_PAGES",
    "DOCUMENT_IMPORT_FORMAT",
    "DocumentImportHooks",
    "MODEL_RECOMMENDATIONS",
    "looks_like_vision_model",
    "model_recommendations",
    "preview_document_import",
    "import_document_draft",
    "preview_then_import_document",
]
