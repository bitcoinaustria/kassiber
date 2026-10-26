"""Bounded model-capability metadata shared by HTTP and CLI clients."""

from __future__ import annotations

from typing import Any

from .contracts import HTTP_REASONING_EFFORTS


MODEL_SUPPORT_LIST_LIMIT = 32
MODEL_SUPPORT_STRING_LIMIT = 96
# Request parameters that mean a model takes a reasoning setting. OpenRouter
# lists `reasoning` and `include_reasoning` in `supported_parameters`; servers
# that mirror OpenAI name the Chat Completions field itself.
_REASONING_PARAMETERS = frozenset({"reasoning", "include_reasoning", "reasoning_effort"})


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

    # Fast serving mode (Codex "Fast" service tier, Claude Opus fast mode).
    supports_fast_mode = item.get("supports_fast_mode")
    if isinstance(supports_fast_mode, bool):
        metadata["supports_fast_mode"] = supports_fast_mode
        description = item.get("fast_mode_description")
        if supports_fast_mode and isinstance(description, str) and description.strip():
            metadata["fast_mode_description"] = description.strip()[
                :MODEL_SUPPORT_STRING_LIMIT
            ]

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


def _lowered(values: list[str] | None) -> list[str]:
    return [value.lower() for value in values or ()]


def http_model_capabilities(item: dict[str, Any]) -> dict[str, Any]:
    """What a model listed by an OpenAI-compatible endpoint can do over HTTP.

    Adds to the bounded metadata what Kassiber derives for HTTP chat:

    - ``supports_reasoning_effort`` and ``reasoning_efforts``: whether the model
      takes an effort, and the HTTP wire levels it accepts (some of
      ``low``/``medium``/``high``; Kassiber's higher levels are sent as
      ``high``). Omitted when the listing does not say.
    - ``supports_tools``: true or false from ``supported_parameters``, or
      null when the listing does not say (OpenAI's ``/models`` never does).
    - ``input_modalities`` / ``output_modalities`` from OpenRouter's
      ``architecture``, as bounded lowercase string lists.
    """

    metadata = safe_model_capabilities(item)
    capabilities = metadata.get("capabilities") or {}
    capability_parameters = capabilities.get("supported_parameters")
    parameters = set(_lowered(metadata.get("supported_parameters"))) | set(
        _lowered(capability_parameters if isinstance(capability_parameters, list) else None)
    )
    # Known means the listing carried the list at all: an empty list says the
    # model takes no optional parameters, tools included, not "unknown". The
    # sanitised metadata drops empty lists, so look at the raw listing.
    raw_capabilities = item.get("capabilities")
    parameters_known = isinstance(item.get("supported_parameters"), list) or (
        isinstance(raw_capabilities, dict)
        and isinstance(raw_capabilities.get("supported_parameters"), list)
    )

    advertised = _lowered(metadata.get("reasoning_efforts"))
    capability_efforts = capabilities.get("reasoning_efforts")
    if isinstance(capability_efforts, list):
        advertised += _lowered(capability_efforts)
    reasoning = item.get("reasoning")
    if isinstance(reasoning, dict):
        # OpenRouter: the effort levels this model accepts, highest first.
        advertised += _lowered(safe_string_list(reasoning.get("supported_efforts")))

    explicit = item.get("supports_reasoning_effort")
    if isinstance(explicit, bool):
        effort_capable = explicit
    else:
        effort_capable = bool(
            parameters & _REASONING_PARAMETERS
            or advertised
            or capabilities.get("reasoning_effort") in (True, "true", "supported")
        )
    if effort_capable:
        accepted = [level for level in HTTP_REASONING_EFFORTS if level in advertised]
        metadata["supports_reasoning_effort"] = True
        # A listing that names only levels Kassiber does not send over HTTP
        # still takes the wire levels; an endpoint that refuses one gets the
        # turn retried without it.
        metadata["reasoning_efforts"] = accepted or list(HTTP_REASONING_EFFORTS)

    explicit_tools = item.get("supports_tools")
    if isinstance(explicit_tools, bool):
        metadata["supports_tools"] = explicit_tools
    elif parameters_known:
        metadata["supports_tools"] = "tools" in parameters
    else:
        metadata["supports_tools"] = None

    architecture = item.get("architecture")
    for key in ("input_modalities", "output_modalities"):
        source = architecture.get(key) if isinstance(architecture, dict) else None
        if source is None:
            source = item.get(key)
        modalities = safe_string_list(source)
        if modalities is not None:
            metadata[key] = list(dict.fromkeys(_lowered(modalities)))
    return metadata
