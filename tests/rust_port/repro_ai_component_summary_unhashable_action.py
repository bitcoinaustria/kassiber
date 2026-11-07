"""Python reference repro: a list `action` crashes the component consent summary.

`summarize_tool_call` for `ui.transfers.components.plan` and `.apply` (main
#679) looks the action up in a dict, so an action that is not hashable raises
`TypeError: unhashable type: 'list'` instead of falling back to "Change
custody component". `crates/ai/scripts/gen_vectors.py` feeds malformed
arguments to every summary and stops on it, so the AI vectors cannot be
regenerated until the Rust side of #679 (S2/S6/S11, then S12) settles how this
case is represented.
"""
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))


def reproduce():
    from kassiber.ai import tools

    tool = tools.get_tool("ui.transfers.components.plan")
    try:
        summary = tools.summarize_tool_call(tool, {"action": ["revise"], "component_id": "c"})
    except TypeError as error:
        return f"TypeError: {error}"
    return f"summary: {summary}"


if __name__ == "__main__":
    print(reproduce())
