"""Write crates/mcp/tests/data/registry.json: Python's MCP grant registry.

    uv run --locked python crates/mcp/scripts/gen_registry.py [--check]

``kassiber/mcp/tools.py`` (#686) classifies every assistant tool by the
operator broker's capability registry. The file records each exposed tool's
classification, the listing and definitions at every grant and access mode,
the server instructions per grant, the refusals, `call_capability` and the
client-name cleaning, for ``crates/mcp/tests/registry.rs``.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
TARGET = Path(__file__).resolve().parents[1] / "tests" / "data" / "registry.json"
sys.path.insert(0, str(ROOT))

GRANTS = ("read", "operator", "accounting_decisions")
ACCESS = ("full", "restricted")


def document() -> dict:
    from kassiber.mcp import tools

    def refusal(tool, grant, access):
        error = tools.refusal_for(tool, grant=grant, agent_access=access)
        if error is None:
            return None
        return {
            "code": error.code,
            "message": str(error),
            "hint": error.hint,
            "details": error.details,
        }

    rows = [
        {
            "name": tool.name,
            "entry": tool.entry.name,
            "required": tool.required.value,
            "egress": tool.egress,
            "full_access_only": tool.full_access_only,
            "destructive": tool.destructive,
            "idempotent": tool.idempotent,
        }
        for tool in tools.mcp_tools()
    ]
    # Definitions do not depend on the grant, only the selection does.
    listings = {
        f"{grant}/{access}": [
            definition["name"]
            for definition in tools.tool_definitions(grant=grant, agent_access=access)
        ]
        for grant in GRANTS
        for access in ACCESS
    }
    definitions = tools.tool_definitions(grant="accounting_decisions", agent_access="full")
    refusals = {
        f"{tool.name}/{grant}/{access}": refusal(tool, grant, access)
        for tool in tools.mcp_tools()
        for grant in GRANTS
        for access in ACCESS
        if refusal(tool, grant, access) is not None
    }
    capability_cases = []
    for name in [*(tool.name for tool in tools.mcp_tools()), "nonexistent", None]:
        for grant in (None, *GRANTS):
            capability_cases.append(
                {"tool": name, "grant": grant, "capability": tools.call_capability(name, grant).value}
            )
    labels = [
        "Claude Code", "  --codex  ", "-", "x" * 80, "aéb/c@d+e_f.g h", "<script>",
        "", 5, None, "‮\u0000tab\tname", "--grant=admin",
    ]
    return {
        "tools": rows,
        "listings": listings,
        "definitions": definitions,
        "refusals": refusals,
        "instructions": {grant: tools.instructions_for(grant) for grant in (*GRANTS, "admin")},
        "call_capability": capability_cases,
        "client_labels": [{"value": value, "label": tools.client_label(value)} for value in labels],
        "max_client_name_chars": tools.MAX_CLIENT_NAME_CHARS,
    }


def render() -> str:
    return json.dumps(document(), indent=1, ensure_ascii=True, sort_keys=False) + "\n"


def main(argv: list[str]) -> int:
    text = render()
    if "--check" in argv:
        if TARGET.read_text(encoding="utf-8") != text:
            print(f"{TARGET} is stale; rerun {Path(__file__).name}", file=sys.stderr)
            return 1
        return 0
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text(text, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
