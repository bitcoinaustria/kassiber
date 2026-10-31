"""Whole Python CLI invocation with deterministic sync sources for live parity."""
import os
from pathlib import Path
import sys

from kassiber.cli.main import main
from kassiber.operator import project
from tests.rust_sync_parity import deterministic_sources
from tests.test_rust_bdk_execution import bdk_request_clock


def run():
    project._owner_lock_root = lambda: Path(os.environ["KASSIBER_RUST_TEST_OWNER_ROOT"])
    with deterministic_sources(os.environ["KASSIBER_RUST_TEST_SYNC_PREFIX"],
                               int(os.environ["KASSIBER_RUST_TEST_SYNC_NOW"])), \
         bdk_request_clock(int(os.environ["KASSIBER_RUST_TEST_SYNC_START"])):
        return main(sys.argv[1:])


if __name__ == "__main__":
    raise SystemExit(run())
