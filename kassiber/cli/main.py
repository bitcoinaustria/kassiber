from __future__ import annotations

import sys
import traceback

from ..core.runtime import bootstrap_runtime, close_runtime, emit_error, resolve_output_format
from ..errors import AppError


def _legacy_app():
    from .. import app as legacy_app

    return legacy_app


def build_parser():
    return _legacy_app().build_parser()


def dispatch(conn, args):
    return _legacy_app().dispatch(conn, args)


def command_needs_db(args):
    return _legacy_app().command_needs_db(args)


def main(argv=None):
    parser = build_parser()
    try:
        args = parser.parse_args(argv)
    except SystemExit:
        raise

    runtime = None
    try:
        args.format = resolve_output_format(args)
    except AppError as exc:
        args.format = "table"
        emit_error(args, exc)
        return 1

    try:
        runtime = bootstrap_runtime(args, needs_db=command_needs_db(args))
        dispatch(runtime.conn, args)
        return 0
    except AppError as exc:
        debug_text = None
        if args.debug:
            debug_text = traceback.format_exc()
            sys.stderr.write(debug_text)
        emit_error(args, exc, debug_text=debug_text)
        return 1
    except Exception as exc:
        debug_text = traceback.format_exc()
        if args.debug:
            sys.stderr.write(debug_text)
        wrapped = AppError(str(exc) or exc.__class__.__name__, code="internal_error")
        emit_error(args, wrapped, debug_text=debug_text if args.debug else None)
        return 1
    finally:
        if runtime is not None:
            close_runtime(runtime)
