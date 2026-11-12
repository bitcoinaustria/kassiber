"""What the session recorder keeps and how two runs are compared.

Versioned: a recording names its `RECORDER_VERSION` and `COMPARISON_POLICY`,
and a change to either rule set below needs a new id, because accepted
baselines are compared under the policy they were recorded with.
"""

from __future__ import annotations

import os
import re
import tempfile
from pathlib import Path


RECORDER_VERSION = 1
SESSION_FORMAT = "kassiber-session/1"
COMPARISON_POLICY = "kassiber-session-compare/1"

# Environment the product reads. Anything else in the test's environment is
# neither recorded nor replayed, so a replay that depends on it fails.
ENV_PREFIXES = ("KASSIBER_", "SATBOOKS_")
ENV_NAMES = frozenset(
    {
        # State roots and their platform fallbacks (`kassiber/db.py`).
        "HOME",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_STATE_HOME",
        "XDG_RUNTIME_DIR",
        # Packaged-runtime detection (`kassiber/operator/build.py`).
        "APPIMAGE",
        "APPDIR",
        # Rendering and locale.
        "TZ",
        "LANG",
        "LANGUAGE",
        "LC_ALL",
        "LC_CTYPE",
        "COLUMNS",
        "LINES",
        "TERM",
        "NO_COLOR",
        "FORCE_COLOR",
    }
)
# The data-root layout: a data root named `data` has these siblings in its
# state root (`kassiber/db.py`).
DATA_DIRNAME = "data"
STATE_DIRNAMES = ("data", "config", "exports", "attachments")

# Test-only switches: never product configuration. The harness variables are
# about the recorder itself; the others are carried as provenance only.
HARNESS_ENV = frozenset({"KASSIBER_TEST_CLI", "KASSIBER_TEST_RECORD_SESSIONS"})
TEST_ENV_PREFIX = "KASSIBER_TEST_"
# Re-armed in a Python candidate on replay: `strict` also blocks loopback.
REPLAYED_TEST_ENV = frozenset({"KASSIBER_TEST_NO_EGRESS"})
# A test hook that deliberately alters the reference's outcome; a recording
# made under it would be a false oracle.
REFUSED_TEST_ENV = frozenset({"KASSIBER_TEST_TAX_SHADOW_INJECT_DIFFERENCE"})

_SECRET_SEGMENTS = frozenset(
    {
        "TOKEN",
        "TOKENS",
        "PASSWORD",
        "PASSWD",
        "SECRET",
        "SECRETS",
        "KEY",
        "APIKEY",
        "PASSPHRASE",
        "CREDENTIAL",
        "CREDENTIALS",
        "COOKIE",
        "PRIVATE",
        "AUTHORIZATION",
        "BEARER",
        "MACAROON",
        "RUNE",
    }
)
# Values shorter than this are too likely to occur by chance in fixtures.
SECRET_SCAN_MIN_LENGTH = 8


def is_product_env(name: str) -> bool:
    if name in HARNESS_ENV or name.startswith(TEST_ENV_PREFIX):
        return False
    return name in ENV_NAMES or name.startswith(ENV_PREFIXES)


def is_secret_name(name: str) -> bool:
    return any(segment in _SECRET_SEGMENTS for segment in name.upper().split("_"))


def temp_root() -> Path:
    """The test temp area: the only place a recorded state root may live."""
    return Path(tempfile.gettempdir()).resolve()


def inside(path: Path, parent: Path) -> bool:
    try:
        path.relative_to(parent)
    except ValueError:
        return False
    return True


def in_temp_area(path: Path) -> bool:
    root = temp_root()
    return path != root and inside(path, root)


# Identities a run generates. A value of one of these shapes in the reference
# binds, on first sight, to the candidate's value at the same position; every
# later occurrence must map the same way in both directions.
UUID4_RE = re.compile(
    r"\b[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\b",
    re.IGNORECASE,
)
# `uuid4().hex`: mapped only as a whole value, never inside longer text, so a
# hash that happens to carry the version nibbles is not mistaken for one in
# running text.
UUID4_HEX_RE = re.compile(r"[0-9a-f]{12}4[0-9a-f]{3}[89ab][0-9a-f]{15}")
# ISO 8601 date-times and dates. Only those inside the run's own wall-clock
# window count as generated; fixture dates compare exactly.
DATETIME_RE = re.compile(
    r"(?<![0-9])(\d{4})-(\d{2})-(\d{2})"
    r"(?:([T ])(\d{2}):(\d{2})(?::(\d{2})(?:\.(\d{1,9}))?)?(Z|[+-]\d{2}:?\d{2})?)?(?![0-9])"
)
# Compact stamps such as `20261026T160601Z` or `20261026-160601`, as used in
# generated file names.
COMPACT_DATETIME_RE = re.compile(r"(?<![0-9])(\d{8})([T_-]?)(\d{6})(Z?)(?![0-9])")
# Epoch numbers count as generated only under keys or columns named for time.
TIME_NAME_RE = re.compile(
    r"(?:^|_)(?:at|time|timestamp|ts|epoch|expires|expiry|deadline|until|since|started|"
    r"finished|created|updated|modified|seen|checked|heartbeat)(?:$|_)",
    re.IGNORECASE,
)
# Generated identities without a recognisable shape, declared by where they
# appear: the daemon's control-channel requests (`supervisor.*`) carry a
# request id the supervisor must echo back (`kassiber/daemon.py`,
# `secret-store-{monotonic_ns}`).
DECLARED_IDENTITIES = (("supervisor.", "request_id"),)
# Daemon request fields that carry a book passphrase. Recorded stdin is
# test-authored, so these values are synthetic; the comparison uses them,
# like `InputFd` passphrases, to open encrypted books.
PASSPHRASE_FIELDS = frozenset({"passphrase_secret", "new_passphrase_secret"})
# Seconds of slack around a run's window, for clocks read just outside a step.
WINDOW_SLACK_SECONDS = 5.0


def policy_document() -> dict:
    """The comparison policy as data, for the manifest and the report."""
    return {
        "id": COMPARISON_POLICY,
        "json": "structural; key order, array order, and value types significant; float text form not",
        "database": "schema objects plus every table's rows in primary-key order; no columns masked",
        "files": "content where text (zip archives member by member), size and sha256 otherwise; modes compared",
        "generated_identities": {
            "uuid4": UUID4_RE.pattern,
            "uuid4_hex": f"whole values matching {UUID4_HEX_RE.pattern}",
            "wall_clock": "ISO 8601 date-times, dates, and compact stamps inside each run's window; "
            "epoch numbers inside the window under time-named keys; order-preserving",
            "declared": [f"{key} of records whose kind starts with {prefix}" for prefix, key in DECLARED_IDENTITIES],
            "binding": "first occurrence binds reference to candidate at the same position; bijective",
        },
        "relocation": "recorded temp paths that are occupied at replay map to fresh ones, bijectively",
        "process_ids": "each recorded process's id maps to its replayed process's id",
    }


def env_path_kind(name: str, value: str) -> str | None:
    """How a path-valued environment variable is tracked, if it is one."""
    if not value or not os.path.isabs(value):
        return None
    if name.endswith("_FILE") or os.path.isfile(value):
        return "file"
    return "dir"
