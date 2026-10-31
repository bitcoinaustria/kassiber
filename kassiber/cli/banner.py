"""Terminal rendering of the wordmark; keep its grid in sync with the desktop."""

from __future__ import annotations

import os
import shutil
import sys


# Checked against ui-tauri/src/components/kb/kassiberWordmarkGrid.ts by tests.
WORDMARK_GRID = (
    "..#...#......#######....#######....#######....#..#######......#######....######.",
    ".##...##....########...########...########...##..########....########...########",
    "###...###..###...###..###...###..###...###..###..###...###..###...###..###...###",
    "###...###..###...###..###...###..###...###..###..###...###..###...###..###...###",
    "###...###..###...###..###...##...###...##...###..###...###..###...##...###...###",
    "###..###...###...###..###...#....###...#....###..###...###..###...#....###...###",
    "###.###....###...###..###........###........###..###...##...###........###...###",
    "######....##########..########...########..####..########...#######...#########.",
    "######....##########...########...########.####..########...######....#######...",
    "###.###....###...###........###........###..###..###...##...###........###.###..",
    "###..###...###...###....#...###....#...###..###..###...###..###...#....###..###.",
    "###...###..###...###...##...###...##...###..###..###...###..###...##...###...###",
    "###...###..###...###..###...###..###...###..###..###...###..###...###..###...###",
    "###...###..###...###..###...###..###...###..###..###...###..###...###..###...###",
    "###...##...###...##...########...########...##...########...########...###...###",
    "###...#....###...#....#######....#######....#....#######....#######....###...###",
    ".............................................................................##.",
    ".............................................................................#..",
)


def render_banner() -> str:
    """Condense pairs of pixel rows into Unicode half-blocks without ANSI."""
    blocks = {
        (False, False): " ",
        (True, False): "▀",
        (False, True): "▄",
        (True, True): "█",
    }
    return "\n".join(
        "".join(
            blocks[(top == "#", bottom == "#")]
            for top, bottom in zip(WORDMARK_GRID[y], WORDMARK_GRID[y + 1])
        ).rstrip()
        for y in range(0, len(WORDMARK_GRID), 2)
    )


def interactive_banner() -> str | None:
    """Offer the banner only to an interactive, capable terminal."""
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        return None
    if os.environ.get("TERM") == "dumb":
        return None
    if shutil.get_terminal_size().columns < len(WORDMARK_GRID[0]):
        return None
    banner = render_banner()
    try:
        banner.encode(sys.stdout.encoding or "ascii")
    except (LookupError, UnicodeEncodeError):
        return None
    return banner
