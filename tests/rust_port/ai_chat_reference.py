"""Python reference for the AI chat driver, with fixed clocks.

Runs `kassiber.cli.main.main` like `python -m kassiber`, after pinning the
clocks a chat writes: `now_iso()` in the provider store, the daemon (answer
provenance and consent timestamps) and chat history (from
``KASSIBER_RUST_TEST_AI_TIME``), and the discovery cache's ``checked_at``
(from ``KASSIBER_RUST_TEST_AI_CHECKED_AT``), the values the Rust binary reads
from the same variables in test-harness builds.
"""

from __future__ import annotations

import os
import sys
from unittest import mock


def main() -> int:
    import importlib

    from kassiber import daemon
    from kassiber.ai import discovery_cache, providers
    from kassiber.core import chat_history

    cli = importlib.import_module("kassiber.cli.main")

    timestamp = os.environ["KASSIBER_RUST_TEST_AI_TIME"]
    checked_at = os.environ["KASSIBER_RUST_TEST_AI_CHECKED_AT"]
    with mock.patch.object(providers, "now_iso", return_value=timestamp), mock.patch.object(
        discovery_cache, "_checked_at", return_value=checked_at
    ), mock.patch.object(daemon, "now_iso", return_value=timestamp), mock.patch.object(
        chat_history, "now_iso", return_value=timestamp
    ):
        return cli.main(sys.argv[1:])


if __name__ == "__main__":
    raise SystemExit(main())
