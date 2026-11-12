#!/usr/bin/env python3
"""Reproduce operational redaction corrupting a valid generated component UUID.

No book, network, clock or identity source is needed. The amount pseudonym is
salted per process; the unexpected replacement of part of the UUID is stable.
This records a reference behavior issue, not an authoring atomicity failure.
"""
from pathlib import Path
import sys
import uuid

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.redaction import redact_operational_text

component_id = "00000000-cad1-4000-8000-000000000000"
assert str(uuid.UUID(component_id)) == component_id
redacted = redact_operational_text(component_id)
print("Component UUID:", component_id)
print("After operational redaction:", redacted)
assert redacted.startswith("00000000-amount#"), "Reference behavior changed; review repro"
assert redacted.endswith("-4000-8000-000000000000")
