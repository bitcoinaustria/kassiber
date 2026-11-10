"""Freeze the Python-seeded journal books the Rust session tests start from.

The pytest modules these tests replaced seeded their books through the Python
product (`tests.test_rust_journal_commands.seed`/`seed_auto`, the paired-fee
book, a generated custody-gap book) and processed some with the Python
reference. This writes each seeded book's rows, table by table, as SQL that
`crates/kassiber/tests/support` loads into a fresh Rust-created book with
triggers off, so the Rust tests start from exactly the rows Python left.

    uv run --locked python -m tests.rust_port.freeze_session_books OUT_DIR [BOOK ...]

Python's seeding samples the clock, so refreezing a book changes it and the
goldens that start from it must be rewritten in the same change.

A one-time generator: the frozen files are goldens once Python is gone
(docs/plan/22-python-removal.md).
"""
from contextlib import closing
import importlib.util
import os
from pathlib import Path
import sqlite3
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
CLOCK = "2026-10-31T12:00:00Z"


def environment(tmp):
    owners = tmp / "owners"
    owners.mkdir(exist_ok=True)
    env = {k: v for k, v in os.environ.items() if not k.startswith(("KASSIBER_OPERATOR_", "KASSIBER_RUST_TEST_"))}
    env.update(
        KASSIBER_RUST_TEST_OWNER_ROOT=str(owners),
        KASSIBER_RUST_TEST_JOURNAL_TIME=CLOCK,
        KASSIBER_RUST_TEST_JOURNAL_IDS="sequential",
        KASSIBER_TEST_NO_EGRESS="1",
        KASSIBER_DISABLE_UPDATE_CHECK="1",
        XDG_DATA_HOME=str(tmp / "xdg-data"),
        XDG_CONFIG_HOME=str(tmp / "xdg-config"),
        XDG_STATE_HOME=str(tmp / "xdg-state"),
        KASSIBER_OFFLINE_PREFERENCE_FILE=str(tmp / "offline.json"),
    )
    return env


def literal(value):
    if value is None:
        return "NULL"
    if isinstance(value, bool):
        return str(int(value))
    if isinstance(value, int):
        return str(value)
    if isinstance(value, float):
        text = repr(value)
        if text in ("inf", "-inf", "nan"):
            raise ValueError(f"non-finite float {text}")
        return text
    if isinstance(value, bytes):
        return "X'" + value.hex() + "'"
    return "'" + value.replace("'", "''") + "'"


def dump(database):
    """Every table's rows as INSERT statements, in rowid (or key) order."""
    conn = sqlite3.connect(database)
    try:
        lines = []
        tables = conn.execute(
            "SELECT name, sql FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
        ).fetchall()
        for name, sql in [*tables, ("sqlite_sequence", "")]:
            if name == "sqlite_sequence" and not conn.execute(
                "SELECT 1 FROM sqlite_master WHERE name='sqlite_sequence'"
            ).fetchone():
                continue
            order = "" if "WITHOUT ROWID" in (sql or "").upper() else " ORDER BY rowid"
            cursor = conn.execute(f'SELECT * FROM "{name}"{order}')
            columns = ",".join(f'"{c[0]}"' for c in cursor.description)
            for row in cursor:
                lines.append(f'INSERT INTO "{name}"({columns}) VALUES({",".join(literal(v) for v in row)});')
        return "\n".join(lines) + "\n"
    finally:
        conn.close()


def processed(root, env):
    from tests.test_rust_journal_commands import invoke
    result = invoke(root, env)
    assert result.returncode == 0, result
    return root


def gaps_book(tmp, root, seed_id):
    from tests.custody_gap_stage import suites
    spec = importlib.util.spec_from_file_location("route_campaign", ROOT / "scripts/check-journal-route-campaign.py")
    campaign = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(campaign)
    generate = dict((name, g) for name, _, g in suites())["gaps"]
    root.parent.mkdir(parents=True, exist_ok=True)
    relaxed, _ = campaign.build_book(generate(seed_id), campaign.template(tmp), root)
    assert not relaxed
    return root


def legacy_pair_fee(root):
    """`ReviewLegacyPairFeeParity.seed`: the c0a015de book, opened (and so
    migrated) by Python with the context set."""
    import base64
    import gzip
    from kassiber.db import open_db
    root.mkdir(parents=True, exist_ok=True)
    fixture = ROOT / "tests/fixtures/historical/pre_pair_fee_c0a015de.sqlite3.gz.b64"
    (root / "kassiber.sqlite3").write_bytes(gzip.decompress(base64.b64decode(fixture.read_bytes())))
    for script in ("INSERT OR REPLACE INTO settings(key,value) VALUES('context_workspace','ws'),"
                   "('context_profile','profile')", ""):
        with closing(open_db(root)) as conn:
            conn.executescript(script)
            conn.commit()
    return root


def identify_book(root):
    """`RustWalletIdentifyTest.seed`: the desktop fixture plus wallets whose
    addresses Python derives."""
    from tests.test_rust_wallet_onboarding import RustWalletIdentifyTest
    from tests.rust_desktop_parity import DesktopParity
    case = RustWalletIdentifyTest.__new__(RustWalletIdentifyTest)
    case.root, case.passphrase = root, None
    case.change = lambda sql: DesktopParity.change(case, sql)
    case.seed()
    return root


def books(tmp, env):
    from tests.test_rust_journal_commands import seed, seed_auto
    from tests.test_rust_journal_views import seed_paired_fee
    yield "auto-exact", lambda r: seed_auto(r)
    yield "auto-exact-current", lambda r: seed_auto(r, current=True)
    yield "plain-unchanged", lambda r: seed(r, shape="unchanged")
    yield "plain-blocked-processed", lambda r: processed(seed(r, shape="blocked"), env)
    yield "plain-changed-processed", lambda r: processed(seed(r, shape="changed"), env)
    yield "plain-build-refusal", lambda r: seed(r, shape="build-refusal")
    yield "plain-label-refusal", lambda r: seed(r, shape="label-refusal")
    yield "paired-fee-processed", lambda r: processed(seed_paired_fee(r), env)
    yield "paired-short-processed", lambda r: processed(seed_paired_fee(r, received=4000000000), env)
    yield "gaps-10-processed", lambda r: processed(gaps_book(tmp, r, 10), env)
    yield "legacy-pair-fee-c0a015de", legacy_pair_fee
    yield "identify-onboarding", identify_book


def main():
    out = Path(sys.argv[1])
    only = set(sys.argv[2:])
    out.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=ROOT / "target") as directory:
        tmp = Path(directory)
        env = environment(tmp)
        for name, build in books(tmp, env):
            if only and name not in only:
                continue
            root = tmp / name / "data"
            build(root)
            (out / f"{name}.sql").write_text(dump(root / "kassiber.sqlite3"))
            print(name)


if __name__ == "__main__":
    main()
