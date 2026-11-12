"""Touch ID helper errors survive the broker's redaction; other `auth:` text does not.

`crates/envelope/tests/data/native_auth_errors.json` holds the allowlist and
the expected outputs. This module checks Python against it and the Rust test
`crates/envelope/tests/native_auth_errors.rs` checks Rust, so both runtimes
pass and redact exactly the same messages.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

import pytest

from kassiber import log_ring
from kassiber.errors import AppError
from kassiber.operator import server

ROOT = Path(__file__).resolve().parents[1]
VECTORS = json.loads((ROOT / "crates/envelope/tests/data/native_auth_errors.json").read_text(encoding="utf-8"))
HELPER_SOURCES = ("ui-tauri/src-tauri/src/lib.rs", "ui-tauri/src-tauri/src/secret_store.rs")


def test_python_lists_match_the_shared_vectors():
    assert log_ring.NATIVE_AUTH_ERROR_PREFIX == VECTORS["prefix"]
    assert sorted(log_ring.NATIVE_AUTH_ERROR_MESSAGES) == VECTORS["messages"]
    assert [list(item) for item in log_ring.NATIVE_AUTH_ERROR_STATUS_TEMPLATES] == VECTORS["status_templates"]


@pytest.mark.parametrize("text", VECTORS["allowed"])
def test_known_helper_errors_pass_unchanged(text):
    assert log_ring.sanitize_traceback_text(text) == text


@pytest.mark.parametrize("case", VECTORS["redacted"], ids=lambda case: case["input"][:48])
def test_anything_else_after_auth_is_still_redacted(case):
    assert log_ring.sanitize_traceback_text(case["input"]) == case["expected"]
    assert "hunter2" not in case["expected"] and "synthetic-passphrase-value" not in case["expected"]


def test_broker_error_response_shows_the_helper_message():
    known = VECTORS["allowed"][0]
    response = server._error_response(AppError(known, code="native_auth_failed", retryable=False))
    assert response["error"]["message"] == known
    leaked = server._error_response(AppError(VECTORS["prefix"] + "hunter2", code="native_auth_failed"))
    assert leaked["error"]["message"] == VECTORS["prefix"] + "[redacted]"


def _helper_source() -> str:
    text = "".join((ROOT / path).read_text(encoding="utf-8") for path in HELPER_SOURCES)
    # `"a ".to_string()\n + "b"` is one message in the helper.
    return re.sub(r'"\s*\.to_string\(\)\s*\+\s*"', "", text)


def test_every_allowlisted_message_is_one_the_helper_can_print():
    source = _helper_source()
    stale = []
    for message in VECTORS["messages"]:
        flag = re.fullmatch(r"(missing|invalid) (--[a-z-]+)", message)
        if flag:
            found = f'format!("{flag[1]} {{flag}}")' in source and f'"{flag[2]}"' in source
        else:
            # The non-macOS helper prints its one message with the prefix inline.
            found = f'"{message}"' in source or f'"{VECTORS["prefix"]}{message}"' in source
        if not found:
            stale.append(message)
    for head, tail in VECTORS["status_templates"]:
        if not re.search(re.escape(head) + r"\{\w+\}" + re.escape(tail) + '"', source):
            stale.append(head + "<status>" + tail)
    assert stale == []
