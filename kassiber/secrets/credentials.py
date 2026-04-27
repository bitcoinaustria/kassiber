"""Move backend credentials from the plaintext dotenv into the encrypted DB.

The dotenv bootstrap (`<state>/config/backends.env`) is convenient for
seeding URL / kind / network metadata, but it sits outside the SQLCipher
boundary. Anything secret-shaped that is left in there is plaintext on
disk even after `kassiber secrets init`.

This module owns the secret/non-secret split for backend env vars and the
one-shot migration that lifts the secret entries into the encrypted
`backends` table while leaving the URL-and-friends rows alone.
"""

from __future__ import annotations

import shutil
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable, Optional

from ..backends import (
    _canonicalize_backend_field_name,
    create_db_backend,
    get_db_backend,
    update_db_backend,
)
from ..errors import AppError


# Canonical secret-field set. Keys here are the *canonical* form returned
# by `_canonicalize_backend_field_name` — the dotenv scanner runs each
# `KASSIBER_BACKEND_<NAME>_<FIELD>` env var through that canonicalizer so
# aliases like `RPCPASSWORD` / `RPC_PASSWORD` collapse onto `password`
# before the comparison.
#
# URLs are deliberately not in this set — they are addresses, not
# credentials. `cookiefile` is also out: it stores a path to a cookie
# file, not the cookie itself; the file content is what carries the
# secret and should be `chmod 600` separately.
SECRET_BACKEND_FIELDS: frozenset[str] = frozenset(
    {
        "auth_header",
        "password",
        "token",
        "username",
    }
)

_BACKEND_ENV_PREFIXES: tuple[str, ...] = (
    "KASSIBER_BACKEND_",
    "SATBOOKS_BACKEND_",
)


def _split_backend_env_key(key: str) -> Optional[tuple[str, str]]:
    """Return `(backend_name, canonical_field)` for a backend env key.

    Returns `None` for any key that is not a `*_BACKEND_<NAME>_<FIELD>`
    form (so KASSIBER_DEFAULT_BACKEND, blank lines, comments, and
    unrelated variables are skipped).
    """

    for prefix in _BACKEND_ENV_PREFIXES:
        if not key.startswith(prefix):
            continue
        suffix = key[len(prefix):]
        if "_" not in suffix:
            return None
        backend_name, field_name = suffix.split("_", 1)
        backend_name = backend_name.strip().lower()
        canonical = _canonicalize_backend_field_name(field_name)
        if not backend_name or not canonical:
            return None
        return backend_name, canonical
    return None


def _strip_quotes(raw_value: str) -> str:
    value = raw_value.strip()
    if (
        len(value) >= 2
        and value[0] == value[-1]
        and value[0] in ("'", '"')
    ):
        return value[1:-1]
    return value


def _parse_dotenv_lines(lines: Iterable[str]) -> list[dict]:
    """Return one dict per non-comment, non-blank line.

    Each dict contains `lineno` (1-based), `raw` (original line including
    no trailing newline), `key`, and `value`. Lines without `=` are
    treated as opaque and only `raw` is populated.
    """

    parsed: list[dict] = []
    for index, raw_line in enumerate(lines, start=1):
        stripped = raw_line.strip()
        if not stripped or stripped.startswith("#") or "=" not in stripped:
            parsed.append({"lineno": index, "raw": raw_line, "key": None, "value": None})
            continue
        key, raw_value = stripped.split("=", 1)
        parsed.append(
            {
                "lineno": index,
                "raw": raw_line,
                "key": key.strip(),
                "value": _strip_quotes(raw_value),
            }
        )
    return parsed


def scan_dotenv_for_secrets(path: Path) -> list[dict]:
    """List every secret-shaped backend entry in the dotenv file.

    Each result row is `{backend, field, env_key, lineno}`. Returns
    `[]` for a missing or empty file.
    """

    target = Path(path).expanduser()
    if not target.exists():
        return []
    text = target.read_text(encoding="utf-8")
    findings: list[dict] = []
    for entry in _parse_dotenv_lines(text.splitlines()):
        key = entry["key"]
        if not key:
            continue
        split = _split_backend_env_key(key)
        if split is None:
            continue
        backend, field = split
        if field not in SECRET_BACKEND_FIELDS:
            continue
        findings.append(
            {
                "backend": backend,
                "field": field,
                "env_key": key,
                "lineno": entry["lineno"],
            }
        )
    return findings


def _rewrite_dotenv_without(path: Path, drop_keys: set[str]) -> str:
    """Rewrite the dotenv file in place, dropping `drop_keys` entries.

    Returns the new file body so callers can show a diff. Comments and
    non-key lines are preserved verbatim.
    """

    target = Path(path).expanduser()
    text = target.read_text(encoding="utf-8")
    out_lines: list[str] = []
    for entry in _parse_dotenv_lines(text.splitlines()):
        key = entry["key"]
        if key and key in drop_keys:
            continue
        out_lines.append(entry["raw"])
    new_body = "\n".join(out_lines)
    if text.endswith("\n") and not new_body.endswith("\n"):
        new_body += "\n"
    target.write_text(new_body, encoding="utf-8")
    return new_body


def _now_stamp() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def migrate_dotenv_credentials(
    conn,
    dotenv_path: Path,
    *,
    create_missing_backends: bool = False,
    fallback_kind: Optional[str] = None,
    fallback_url: Optional[str] = None,
) -> dict:
    """Lift every secret-shaped dotenv entry into the encrypted DB.

    For each secret entry, the matching `backends` row is updated with
    that field. The dotenv file is rewritten without the secret entries
    after a `<file>.pre-credentials-migration-<ts>.bak` snapshot is
    saved. Non-secret entries (URL, kind, chain, network, batch_size,
    cookiefile, walletprefix, …) are left in place.

    `create_missing_backends=False` (the default) refuses to migrate a
    secret for a backend that does not yet exist in the DB, so the user
    can decide whether the entry is real or stale. `True` will create
    the missing backend using `fallback_kind` / `fallback_url` (only
    sensible during a fresh-bootstrap workflow).
    """

    target = Path(dotenv_path).expanduser()
    if not target.exists():
        return {
            "dotenv_path": str(target),
            "migrated": [],
            "skipped": [],
            "backup_path": None,
            "rewritten": False,
        }

    findings = scan_dotenv_for_secrets(target)
    if not findings:
        return {
            "dotenv_path": str(target),
            "migrated": [],
            "skipped": [],
            "backup_path": None,
            "rewritten": False,
        }

    migrated: list[dict] = []
    skipped: list[dict] = []
    drop_keys: set[str] = set()

    text = target.read_text(encoding="utf-8")
    parsed = _parse_dotenv_lines(text.splitlines())
    raw_values: dict[str, str] = {}
    for entry in parsed:
        if entry["key"]:
            raw_values[entry["key"]] = entry["value"]

    for finding in findings:
        backend_name = finding["backend"]
        field = finding["field"]
        env_key = finding["env_key"]
        value = raw_values.get(env_key)
        if value is None:
            skipped.append({**finding, "reason": "value_missing"})
            continue

        try:
            existing = get_db_backend(conn, backend_name)
        except AppError as exc:
            if exc.code != "not_found":
                raise
            existing = None

        if existing is None:
            if not create_missing_backends or not fallback_kind or not fallback_url:
                skipped.append({**finding, "reason": "backend_not_in_db"})
                continue
            create_db_backend(
                conn,
                backend_name,
                fallback_kind,
                fallback_url,
            )
            existing = get_db_backend(conn, backend_name)

        if field in {"username", "password"}:
            updates = {"config": {field: value}}
        else:
            updates = {field: value}

        update_db_backend(conn, backend_name, updates)
        migrated.append({**finding, "applied": True})
        drop_keys.add(env_key)

    backup_path: Optional[Path] = None
    rewritten = False
    if drop_keys:
        backup_path = target.with_name(
            f"{target.name}.pre-credentials-migration-{_now_stamp()}.bak"
        )
        shutil.copy2(target, backup_path)
        _rewrite_dotenv_without(target, drop_keys)
        rewritten = True

    return {
        "dotenv_path": str(target),
        "migrated": migrated,
        "skipped": skipped,
        "backup_path": str(backup_path) if backup_path else None,
        "rewritten": rewritten,
    }
