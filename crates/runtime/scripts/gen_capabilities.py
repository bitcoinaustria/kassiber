"""Write tests/data/capabilities.json: Python's capability registries.

    .venv/bin/python crates/runtime/scripts/gen_capabilities.py
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
TARGET = Path(__file__).resolve().parents[1] / "tests" / "data" / "capabilities.json"
sys.path.insert(0, str(ROOT))

from kassiber.command_capabilities import CLI_CAPABILITIES, DAEMON_CAPABILITIES  # noqa: E402


def main() -> int:
    document = {
        "cli": {path: capability.value for path, capability in sorted(CLI_CAPABILITIES.items())},
        "daemon": {kind: capability.value for kind, capability in sorted(DAEMON_CAPABILITIES.items())},
    }
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text(json.dumps(document, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
