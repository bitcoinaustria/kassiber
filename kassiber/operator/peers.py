"""Display names for the processes behind agent sessions.

The label says which program started a `kassiber mcp serve` process (for
example `claude` or `codex`), so the user can recognize it before allowing
it. It is best effort and not an identity: any program can name itself
anything. Pairing binds to the process the OS reports, never to this name.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

# Launchers between the agent host and the MCP server itself.
_SKIPPED_NAMES = frozenset({"kassiber", "python", "python3", "uv", "uvx", "sh", "bash", "zsh"})
_MAX_ANCESTORS = 4
_MAX_LABEL_CHARS = 32


def process_label(pid: int | None) -> str | None:
    if pid is None or pid <= 0:
        return None
    try:
        current = _parent(pid)
        for _ in range(_MAX_ANCESTORS):
            if current is None or current <= 1:
                return None
            name = _name(current)
            if name is None:
                return None
            if _normalized(name) not in _SKIPPED_NAMES:
                return _clean(name)
            current = _parent(current)
    except (OSError, ValueError, subprocess.SubprocessError):
        return None
    # Only launchers found. Windows has no introspection here, so no label;
    # the process id still shows.
    return None


def _normalized(name: str) -> str:
    base = name.rsplit("/", 1)[-1].lower()
    for suffix in (".exe", ".app"):
        if base.endswith(suffix):
            base = base[: -len(suffix)]
    return base.split(".", 1)[0] if base.startswith("python") else base


def _clean(name: str) -> str | None:
    base = name.rsplit("/", 1)[-1]
    printable = "".join(ch for ch in base if ch.isprintable()).strip()
    return printable[:_MAX_LABEL_CHARS] or None


def _parent(pid: int) -> int | None:
    if sys.platform.startswith("linux"):
        stat = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8", errors="replace")
        # The name field may contain spaces or parentheses; fields resume
        # after its last closing parenthesis.
        fields = stat[stat.rindex(")") + 2 :].split()
        return int(fields[1])
    if sys.platform == "darwin":
        output = _ps(pid, "ppid=")
        return int(output) if output else None
    return None


def _name(pid: int) -> str | None:
    if sys.platform.startswith("linux"):
        return Path(f"/proc/{pid}/comm").read_text(encoding="utf-8", errors="replace").strip() or None
    if sys.platform == "darwin":
        return _ps(pid, "comm=") or None
    return None


def _ps(pid: int, field: str) -> str:
    completed = subprocess.run(
        ["/bin/ps", "-o", field, "-p", str(pid)],
        capture_output=True,
        text=True,
        timeout=2,
        env={"LC_ALL": "C", "PATH": "/usr/bin:/bin"},
        check=False,
    )
    return completed.stdout.strip()

