"""Bounded model-capability metadata shared by HTTP and CLI clients."""

from __future__ import annotations

from typing import Any


MODEL_SUPPORT_LIST_LIMIT = 32
MODEL_SUPPORT_STRING_LIMIT = 96


def safe_string_list(value: Any) -> list[str] | None:
    if not isinstance(value, list):
        return None
    out: list[str] = []
    for item in value:
        if not isinstance(item, str):
            continue
        text = item.strip()
        if not text:
            continue
        out.append(text[:MODEL_SUPPORT_STRING_LIMIT])
        if len(out) >= MODEL_SUPPORT_LIST_LIMIT:
            break
    return out or None


def _safe_capability_value(value: Any) -> bool | str | list[str] | None:
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        text = value.strip()
        return text[:MODEL_SUPPORT_STRING_LIMIT] if text else None
    return safe_string_list(value)


def safe_model_capabilities(item: dict[str, Any]) -> dict[str, Any]:
    metadata: dict[str, Any] = {}
    supports_reasoning_effort = item.get("supports_reasoning_effort")
    if isinstance(supports_reasoning_effort, bool):
        metadata["supports_reasoning_effort"] = supports_reasoning_effort

    supported_parameters = safe_string_list(item.get("supported_parameters"))
    if supported_parameters is not None:
        metadata["supported_parameters"] = supported_parameters

    reasoning_efforts = safe_string_list(item.get("reasoning_efforts"))
    if reasoning_efforts is not None:
        metadata["reasoning_efforts"] = reasoning_efforts

    capabilities = item.get("capabilities")
    if isinstance(capabilities, dict):
        safe_capabilities: dict[str, Any] = {}
        for key in ("reasoning_effort", "reasoning_efforts", "supported_parameters"):
            safe = _safe_capability_value(capabilities.get(key))
            if safe is not None:
                safe_capabilities[key] = safe
        if safe_capabilities:
            metadata["capabilities"] = safe_capabilities
    return metadata
