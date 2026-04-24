"""CLI output formatting and JSON envelope construction.

Every user-facing response from kassiber flows through `emit`. Based on
`--format`, `emit` renders the payload as a JSON envelope, CSV, plain
key:value text, or an ASCII table.

The JSON envelope is the machine contract — the smoke test in
`tests/test_cli_smoke.py` pins its shape. Two variants:

  success: {"kind": "<command>.<sub>...", "schema_version": 1, "data": ...}
  error:   {"kind": "error", "schema_version": 1, "error": {
               "code": ..., "message": ..., "hint": ...,
               "details": ..., "retryable": ..., "debug": ...,
           }}

`kind` is derived from the argparse namespace — the main `command` plus
whichever `<thing>_command` subcommand attribute is set — so every nested
subcommand gets a unique, grep-able identifier without the call site
having to spell it out.

`json_ready` is the universal serializer: it unwraps `sqlite3.Row`, walks
nested dicts/lists, and turns `Decimal` into float. Call sites should
never pass raw sqlite Rows or Decimals through `json.dumps`.
"""

import csv
import json
import sqlite3
import sys
from decimal import Decimal
from pathlib import Path

from .errors import AppError


SCHEMA_VERSION = 1
OUTPUT_FORMATS = ("table", "json", "plain", "csv")


def json_ready(value):
    """Recursively convert sqlite Rows / Decimals / nested collections to
    JSON-serializable Python builtins."""
    if isinstance(value, sqlite3.Row):
        return {k: json_ready(value[k]) for k in value.keys()}
    if isinstance(value, dict):
        return {k: json_ready(v) for k, v in value.items()}
    if isinstance(value, list):
        return [json_ready(v) for v in value]
    if isinstance(value, Decimal):
        return float(value)
    return value


# Attributes on the argparse namespace that identify a nested subcommand.
# Order matters: `derive_kind` walks this tuple in order and joins matches
# with `.` to form the envelope `kind`. When adding a new nested subparser
# with `dest="<ns>_command"`, add the attr name here.
_KIND_SUBCOMMAND_ATTRS = (
    "backends_command",
    "context_command",
    "workspaces_command",
    "profiles_command",
    "accounts_command",
    "wallets_command",
    "transactions_command",
    "attachments_command",
    "metadata_command",
    "notes_command",
    "tags_command",
    "bip329_command",
    "journals_command",
    "journal_transfers_command",
    "transfers_command",
    "events_command",
    "quarantine_command",
    "quarantine_resolve_command",
    "records_command",
    "records_note_command",
    "reports_command",
    "rates_command",
    "diagnostics_command",
)


def derive_kind(args, override=None):
    """Build the envelope `kind` from an argparse namespace.

    Pass `override` to force a specific kind — used by commands that want
    to group multiple argparse paths under one canonical kind.
    """
    if override:
        return override
    parts = []
    command = getattr(args, "command", None)
    if command:
        parts.append(command)
    for attr in _KIND_SUBCOMMAND_ATTRS:
        value = getattr(args, attr, None)
        if value:
            parts.append(value)
    return ".".join(parts) if parts else "response"


_RESERVED_ENVELOPE_KEYS = {"kind", "schema_version", "data", "error"}


def _normalized_envelope_meta(envelope_meta):
    if not envelope_meta:
        return {}
    normalized = json_ready(envelope_meta)
    for key in normalized:
        if key in _RESERVED_ENVELOPE_KEYS:
            raise ValueError(f"Envelope metadata cannot override reserved key '{key}'")
    return normalized


def build_envelope(kind, data, envelope_meta=None):
    envelope = {"kind": kind, "schema_version": SCHEMA_VERSION, "data": json_ready(data)}
    envelope.update(_normalized_envelope_meta(envelope_meta))
    return envelope


def build_error_envelope(code, message, details=None, hint=None, retryable=False, debug=None):
    error_body = {
        "code": code,
        "message": message,
        "hint": hint,
        "details": json_ready(details) if details is not None else None,
        "retryable": bool(retryable),
        "debug": debug,
    }
    return {"kind": "error", "schema_version": SCHEMA_VERSION, "error": error_body}


# -- output stream handling --------------------------------------------------


def _open_output(args):
    """Resolve `--output PATH` to a writable stream.

    Returns `(stream, should_close)`; caller must close when `should_close`
    is True. Parent directories are created as needed.
    """
    target = getattr(args, "output", None)
    if not target or target == "-":
        return sys.stdout, False
    path = Path(target).expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)
    return path.open("w", encoding="utf-8", newline=""), True


def _write_text(args, text):
    stream, should_close = _open_output(args)
    try:
        stream.write(text)
        if not text.endswith("\n"):
            stream.write("\n")
    finally:
        if should_close:
            stream.close()


def _write_csv_rows(args, rows):
    if not isinstance(rows, list) or not rows or not isinstance(rows[0], dict):
        raise AppError(
            "CSV output requires a list of records; this command does not produce tabular output",
            code="format_unsupported",
        )
    headers = list(rows[0].keys())
    stream, should_close = _open_output(args)
    try:
        writer = csv.DictWriter(stream, fieldnames=headers, extrasaction="ignore")
        writer.writeheader()
        for row in rows:
            writer.writerow({h: _csv_cell(row.get(h)) for h in headers})
    finally:
        if should_close:
            stream.close()


def _csv_cell(value):
    if value is None:
        return ""
    if isinstance(value, (dict, list)):
        return json.dumps(json_ready(value), sort_keys=False)
    if isinstance(value, float):
        return format_table_value(value)
    return str(value)


def _plain_dict(payload):
    lines = []
    for key, value in payload.items():
        if isinstance(value, (dict, list)):
            lines.append(f"{key}: {json.dumps(json_ready(value), sort_keys=False)}")
        else:
            lines.append(f"{key}: {format_table_value(value) if isinstance(value, float) else value if value is not None else ''}")
    return "\n".join(lines)


def _plain_list(payload):
    blocks = []
    for index, item in enumerate(payload):
        if isinstance(item, dict):
            blocks.append(_plain_dict(item))
        else:
            blocks.append(str(item))
        if index < len(payload) - 1:
            blocks.append("")
    return "\n".join(blocks) if blocks else "(no rows)"


def emit(args, payload, kind=None, envelope_meta=None):
    """Render `payload` to the user-selected output format.

    Never returns anything — side-effect-only. Call sites hand over the
    raw dict/list and let this function handle JSON envelope wrapping,
    CSV serialization, plain key:value rendering, or ASCII tabulation.
    """
    fmt = getattr(args, "format", "table")
    if fmt == "json":
        envelope = build_envelope(
            derive_kind(args, override=kind),
            payload,
            envelope_meta=envelope_meta,
        )
        _write_text(args, json.dumps(envelope, indent=2, sort_keys=False))
        return
    if fmt == "csv":
        if isinstance(payload, dict):
            _write_csv_rows(args, [payload])
        else:
            _write_csv_rows(args, payload if isinstance(payload, list) else [])
        return
    if fmt == "plain":
        if isinstance(payload, list):
            _write_text(args, _plain_list(payload))
        elif isinstance(payload, dict):
            _write_text(args, _plain_dict(payload))
        else:
            _write_text(args, str(payload) if payload is not None else "")
        return
    # Default table format
    if isinstance(payload, list):
        _write_text(args, _table_text(payload))
    elif isinstance(payload, dict):
        rows = [{"field": key, "value": value} for key, value in payload.items()]
        _write_text(args, _table_text(rows))
    else:
        _write_text(args, str(payload) if payload is not None else "")


def print_table(rows):
    print(_table_text(rows))


def _table_text(rows):
    if not rows:
        return "(no rows)"
    normalized = [{key: format_table_value(value) for key, value in row.items()} for row in rows]
    headers = list(normalized[0].keys())
    widths = {header: len(header) for header in headers}
    for row in normalized:
        for header in headers:
            widths[header] = max(widths[header], len(row.get(header, "")))
    header_line = "  ".join(header.ljust(widths[header]) for header in headers)
    separator = "  ".join("-" * widths[header] for header in headers)
    lines = [header_line, separator]
    for row in normalized:
        lines.append("  ".join(row.get(header, "").ljust(widths[header]) for header in headers))
    return "\n".join(lines)


def format_table_value(value):
    if value is None:
        return ""
    if isinstance(value, float):
        return f"{value:.8f}".rstrip("0").rstrip(".")
    return str(value)
