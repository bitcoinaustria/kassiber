"""CLI entrypoints for kassiber.

Keep this package import light: daemon helpers import ``kassiber.cli.handlers``
directly, and importing the full argparse entrypoint here creates daemon cycles.
"""

from __future__ import annotations

from typing import Any

__all__ = ["main"]


def main(*args: Any, **kwargs: Any) -> Any:
    from .main import main as _main

    return _main(*args, **kwargs)
