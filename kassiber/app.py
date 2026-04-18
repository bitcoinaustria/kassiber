"""Compatibility shim for the old kassiber.app module surface."""

from .cli.handlers import *  # noqa: F401,F403
from .cli.handlers import (
    _import_coordinator_hooks,
    _insert_records_for_sync,
    _metadata_hooks,
    _report_hooks,
    _wallet_sync_hooks,
)
from .cli.main import build_parser, command_needs_db, dispatch, main

