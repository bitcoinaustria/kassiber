"""Repro: a list or dict component action crashes the consent summary.

``summarize_tool_call`` looks ``arguments.get("action", "create")`` up in a
dict for ``ui.transfers.components.plan`` and ``.apply``. The chat loop
builds the consent card before the arguments are checked against the
schema, so a model that sends ``{"action": ["a"]}`` raises
``TypeError: unhashable type: 'list'`` and the turn ends with
``internal_error``. The Rust port answers the same ``internal_error``.

    uv run --locked python tests/rust_port/repro_component_summary_unhashable.py
"""
from __future__ import annotations

from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))


def main() -> int:
    from kassiber.ai.tools import get_tool, summarize_tool_call

    for name in ("ui.transfers.components.plan", "ui.transfers.components.apply"):
        for action in (["a"], {"k": True}, "revise"):
            try:
                outcome = repr(summarize_tool_call(get_tool(name), {"action": action}))
            except TypeError as exc:
                outcome = f"TypeError: {exc}"
            print(f"{name} action={action!r}: {outcome}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
