"""Regression: a list or dict component action gets a validation result.

``summarize_tool_call`` used to look ``arguments.get("action", "create")``
up in a dict for ``ui.transfers.components.plan`` and ``.apply``, so a
model that sent ``{"action": ["a"]}`` raised ``TypeError: unhashable type:
'list'`` and the chat turn ended with ``internal_error``. Since b2c0d9447
any non-string action is refused as validation before a consent card is
shown, and the turn continues; the Rust summary and chat loop do the same.
Exits non-zero if the old behaviour comes back.

    uv run --locked python tests/rust_port/repro_component_summary_unhashable.py
"""
from __future__ import annotations

from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))


def main() -> int:
    from kassiber.ai.tools import get_tool, summarize_tool_call
    from kassiber.errors import AppError

    failures = 0
    for name in ("ui.transfers.components.plan", "ui.transfers.components.apply"):
        for action in (["a"], {"k": True}, None, 5, "revise"):
            try:
                outcome = repr(summarize_tool_call(get_tool(name), {"action": action}))
                refused = False
            except AppError as exc:
                outcome = f"{exc.code}: {exc}"
                refused = exc.code == "validation"
            except TypeError as exc:
                outcome = f"TypeError: {exc}"
                refused = False
            expected = not isinstance(action, str)
            failures += refused != expected
            print(f"{name} action={action!r}: {outcome}")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
