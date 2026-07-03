"""CLI entrypoints for kassiber."""

from __future__ import annotations

from typing import Any

__all__ = ["main"]


def main(*args: Any, **kwargs: Any) -> Any:
    from .main import main as _main

    return _main(*args, **kwargs)
