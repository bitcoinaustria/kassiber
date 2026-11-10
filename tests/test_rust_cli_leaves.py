"""The last native CLI leaves against unchanged Python, byte for byte.

`journals list`, `journals events list|get`, `journals transfers list`,
`journals quarantined`, `journals quarantine show|clear|resolve`,
`wallets identify`, `wallets analyze-file` and `wallets reveal-descriptor`
run through Python's CLI (`tests.rust_port.journal_cli_reference`) and the
Rust harness binary on the same books, plaintext and SQLCipher. Reads must
leave the book untouched; writes must leave the same book, with only the
metadata layer's generated audit ids and edit times masked. Every child runs
with redirected XDG roots and the conftest's offline preference file, on a
disposable data root.
"""
import base64
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

import pytest

from kassiber.db import open_db
from tests.test_journal_storage_format import _book
from tests.test_rust_journal_quarantine_resolve import book as masked_book, masked

ROOT = Path(__file__).resolve().parents[1]
BINARY = Path(os.environ.get("KASSIBER_RUST_BIN", ROOT / "target/rust/debug/kassiber"))
KEY = "synthetic-registration-only"  # the masked book dump opens with it
CLOCK = "2026-10-31T12:00:00Z"
DESCRIPTOR = (
    "wpkh([d34db33f/84h/0h/0h]xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz/0/*)"
)
CHANGE = DESCRIPTOR.replace("/0/*)", "/1/*)")
LISTED = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
REFERENCE = [sys.executable, "-m", "tests.rust_port.journal_cli_reference"]

pytestmark = pytest.mark.skipif(not BINARY.is_file(), reason="requires native harness binary")


@pytest.fixture
def environment(tmp_path):
    owners = tmp_path / "owners"
    owners.mkdir()
    env = {k: v for k, v in os.environ.items() if not k.startswith(("KASSIBER_OPERATOR_", "KASSIBER_RUST_TEST_"))}
    env.update(
        KASSIBER_RUST_TEST_OWNER_ROOT=str(owners),
        KASSIBER_RUST_TEST_JOURNAL_TIME=CLOCK,
        KASSIBER_RUST_TEST_JOURNAL_IDS="sequential",
        KASSIBER_TEST_NO_EGRESS="1",
        KASSIBER_DISABLE_UPDATE_CHECK="1",
        XDG_DATA_HOME=str(tmp_path / "xdg-data"),
        XDG_CONFIG_HOME=str(tmp_path / "xdg-config"),
        XDG_STATE_HOME=str(tmp_path / "xdg-state"),
    )
    # The product reads the machine-wide offline switch from this file, and
    # the conftest's PYTHONPATH arms Python children's no-egress guard (and
    # lets them import the reference from any working directory).
    assert env.get("KASSIBER_OFFLINE_PREFERENCE_FILE")
    assert str(ROOT / "tests" / "_child_hooks") in env.get("PYTHONPATH", "").split(os.pathsep)
    return env


def cli(root, env, native, argv, *, cipher=False, machine=True, cwd=ROOT):
    args = (["--machine"] if machine else []) + ["--data-root", str(root)]
    fds = ()
    if cipher:
        read_fd, write_fd = os.pipe()
        os.write(write_fd, (KEY + "\n").encode())
        os.close(write_fd)
        fds = (read_fd,)
        args += ["--db-passphrase-fd", str(read_fd)]
    prefix = [str(BINARY)] if native else REFERENCE
    try:
        return subprocess.run([*prefix, *args, *argv], env=env, pass_fds=fds, capture_output=True,
                              stdin=subprocess.DEVNULL, cwd=cwd, timeout=300)
    finally:
        for fd in fds:
            os.close(fd)


def same(reference, native, env, argv, *, cipher=False, machine=True, mask=None, cwd=ROOT):
    """Run Python on ``reference`` and Rust on ``native``; require equal bytes."""
    normalize = mask or (lambda value: value)
    expected = cli(reference, env, False, argv, cipher=cipher, machine=machine, cwd=cwd)
    actual = cli(native, env, True, argv, cipher=cipher, machine=machine, cwd=cwd)
    assert (actual.returncode, normalize(actual.stdout), normalize(actual.stderr)) == (
        expected.returncode, normalize(expected.stdout), normalize(expected.stderr)
    ), argv
    if machine and expected.stdout.startswith(b"{"):
        return json.loads(expected.stdout)
    return expected


def raw(root, cipher):
    """Every table, as stored; reads must leave it exactly so."""
    conn = open_db(root, passphrase=KEY if cipher else None)
    try:
        tables = [row[0] for row in conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
        return {name: sorted(map(tuple, conn.execute(f'SELECT * FROM "{name}"').fetchall()), key=repr)
                for name in tables}
    finally:
        conn.close()


def change(root, sql, params=(), cipher=False):
    conn = open_db(root, passphrase=KEY if cipher else None)
    try:
        conn.execute(sql, params)
        conn.commit()
    finally:
        conn.close()


def context(conn):
    conn.execute("INSERT OR REPLACE INTO settings VALUES('context_workspace','w'),('context_profile','p')")


def journal_book(root, cipher=False):
    """Five processed rows with compact calculations, one coded account and
    Austrian markers on some entries."""
    conn = _book(root, passphrase=KEY if cipher else None)
    try:
        context(conn)
        conn.execute("INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at)"
                     " VALUES('acct','w','p','ACC','Treasury','asset','2020'),"
                     "('dup1','w','p','D1','Same','asset','2020'),('dup2','w','p','D2','Same','asset','2020')")
        conn.execute("UPDATE journal_entries SET account_id='acct' WHERE transaction_id IN ('a','s')")
        conn.execute("UPDATE journal_entries SET at_category='neu', at_kennzahl=801 WHERE transaction_id='b'")
        # A legacy JSON calculation, a malformed compact one.
        conn.execute("UPDATE journal_entries SET calculation_json='{\"legacy\": true}', calculation_compact=NULL"
                     " WHERE transaction_id='i'")
        conn.execute("UPDATE journal_entries SET calculation_compact='[2, \"broken\"' WHERE transaction_id='t'")
        conn.commit()
    finally:
        conn.close()
    return root


def event_ids(root, cipher):
    conn = open_db(root, passphrase=KEY if cipher else None)
    try:
        return [row[0] for row in conn.execute("SELECT id FROM journal_entries ORDER BY id")]
    finally:
        conn.close()


def tamper(cursor, edit):
    payload = json.loads(base64.urlsafe_b64decode(cursor + "=" * (-len(cursor) % 4)))
    edit(payload)
    return base64.urlsafe_b64encode(json.dumps(payload).encode()).decode().rstrip("=")


@pytest.mark.parametrize("cipher", [False, True])
def test_journal_entries_and_events_match_python_byte_for_byte(tmp_path, environment, cipher):
    root = journal_book(tmp_path / "book", cipher)
    before = raw(root, cipher)

    def check(argv, **options):
        return same(root, root, environment, argv, cipher=cipher, **options)

    total = len(event_ids(root, cipher))
    listed = check(["journals", "list"])
    assert listed["kind"] == "journals.list" and len(listed["data"]) == total
    assert {"next_cursor", "has_more", "limit"} <= set(listed)
    assert any("at_category" in entry for entry in listed["data"])
    assert any("at_category" not in entry for entry in listed["data"])
    for argv in (["--limit", "0"], ["--limit", "-3"], ["--limit", "9223372036854775807"],
                 ["--cursor", "bogus"], ["--cursor", "e30"], ["--workspace", "missing"],
                 ["--profile", "missing"]):
        check(["journals", "list", *argv])
    # Page one entry at a time on Python's cursors, then tamper with them.
    cursor, seen = None, []
    while True:
        page = check(["journals", "list", "--limit", "1", *(["--cursor", cursor] if cursor else [])])
        seen += [entry["id"] for entry in page["data"]]
        cursor = page["next_cursor"]
        if cursor is None:
            break
        for broken in (cursor[:-3], tamper(cursor, lambda p: p["filters"].update(asset="BTC")),
                       tamper(cursor, lambda p: p.pop("id")), tamper(cursor, lambda p: p.update(id=None)),
                       tamper(cursor, lambda p: p.update(occurred_at=7))):
            check(["journals", "list", "--limit", "1", "--cursor", broken])
    assert len(seen) == total == len(set(seen))

    events = check(["journals", "events", "list"])
    assert events["kind"] == "journals.events.list" and len(events["data"]["events"]) == total
    for argv in (
        ["--limit", "2"], ["--limit", "0"], ["--limit", "1000"], ["--limit", "1001"],
        ["--wallet", "wallet"], ["--wallet", " WALLET "], ["--wallet", "missing"], ["--wallet", ""],
        ["--account", "ACC"], ["--account", "treasury"], ["--account", "Same"], ["--account", "missing"],
        ["--asset", "btc"], ["--asset", "eth"], ["--entry-type", "DISPOSAL"], ["--entry-type", "Income"],
        ["--start", "2024-02-01T00:00:00Z"], ["--end", "2024-03-01"],
        ["--start", "2024-02-01", "--end", "2024-03-01T23:59:59.5+01:00"],
        ["--start", "yesterday"], ["--end", "2024-13-01"], ["--start", ""],
        ["--wallet", "missing", "--start", "bogus"], ["--limit", "0", "--start", "bogus"],
    ):
        check(["journals", "events", "list", *argv])
    cursor, seen = None, []
    while True:
        page = check(["journals", "events", "list", "--asset", "BTC", "--limit", "2",
                      *(["--cursor", cursor] if cursor else [])])
        seen += [event["id"] for event in page["data"]["events"]]
        cursor = page["data"]["next_cursor"]
        if cursor is None:
            break
        # A cursor is bound to its filters.
        check(["journals", "events", "list", "--limit", "2", "--cursor", cursor])
        check(["journals", "list", "--cursor", cursor])
    assert len(seen) == total

    for event_id in event_ids(root, cipher):
        got = check(["journals", "events", "get", "--event-id", event_id])
        assert got["data"]["id"] == event_id and "calculation_compact" not in got["data"]
    for argv in (["--event-id", "missing"], ["--event-id", ""], ["--event-id", "x", "--profile", "missing"]):
        check(["journals", "events", "get", *argv])

    # Human formats print the same rows.
    first = event_ids(root, cipher)[0]
    for fmt in ("table", "plain", "csv", "json"):
        check(["--format", fmt, "journals", "list", "--limit", "2"], machine=False)
        check(["--format", fmt, "journals", "events", "list", "--limit", "2"], machine=False)
        check(["--format", fmt, "journals", "events", "get", "--event-id", first], machine=False)
    assert raw(root, cipher) == before


def transfer_book(root, cipher=False, country="generic"):
    conn = _book(root, passphrase=KEY if cipher else None)
    try:
        context(conn)
        conn.execute("UPDATE profiles SET tax_country=?", (country,))
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at)"
                     " VALUES('cold','w','p','Cold','manual','2020'),('liquid','w','p','Liquid','manual','2020')")
        for tx, wallet, direction, asset, amount, fee, rate in (
            ("o1", "wallet", "outbound", "BTC", 50_000_000, 1_000, 30000.5),
            ("i1", "cold", "inbound", "BTC", 30_000_000, 0, None),
            ("i2", "cold", "inbound", "BTC", 20_000_000, 0, None),
            ("o2", "wallet", "outbound", "BTC", 40_000_000, 0, None),
            ("l2", "liquid", "inbound", "LBTC", 39_000_000, 0, None),
            ("o3", "wallet", "outbound", "BTC", 10_000_000, 0, None),
        ):
            conn.execute(
                "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,"
                "direction,asset,amount,fee,fiat_rate,raw_json,created_at) VALUES(?,'w','p',?,?,?,"
                "'2024-05-01T00:00:00Z',?,?,?,?,?,'{}','2020')",
                (tx, wallet, "ext-" + tx, "fp-" + tx, direction, asset, amount, fee, rate))
        decision = ("INSERT INTO journal_custody_decisions(decision_id,workspace_id,profile_id,source_transaction_id,"
                    "target_transaction_id,source_observation_hash,source_start_msat,source_end_msat,"
                    "target_observation_hash,target_start_msat,target_end_msat,source_wallet_id,target_wallet_id,"
                    "source_network,target_network,source_rail,target_rail,source_asset,target_asset,state,"
                    "basis_state,reason,atomic_group_id,component_id,occurred_at,created_at)"
                    " VALUES(?,'w','p',?,?,?,?,?,?,?,?,?,?,'main','main','bitcoin','bitcoin',?,?,?,?,?,?,?,?,'2020')")

        def h(char):
            return char * 64

        conn.executemany(decision, [
            (h("1"), "o1", "i1", h("a"), 0, 30_000_000, h("b"), 0, 30_000_000, "wallet", "cold", "BTC", "BTC",
             "internal_verified", "eligible", "move", "group-1", None, "2024-05-01T00:00:00Z"),
            (h("2"), "o1", "i2", h("a"), 30_000_000, 50_000_000, h("c"), 0, 20_000_000, "wallet", None, "BTC", "BTC",
             "internal_reviewed", "blocked_by_prior_custody_basis", "reviewed", "", "component-9",
             "2024-05-01T00:00:00Z"),
            (h("3"), "o2", "l2", h("d"), 0, 39_000_000, h("e"), 0, 39_000_000, "wallet", "liquid", "BTC", "LBTC",
             "internal_verified", "eligible", "peg", "", None, "2024-05-02T00:00:00Z"),
        ])
        conn.executemany(
            "INSERT INTO journal_custody_economic_relations(relation_id,workspace_id,profile_id,relation_kind,"
            "source_transaction_id,target_transaction_id,component_id,source_asset,target_asset,source_amount_msat,"
            "target_amount_msat,basis_state,occurred_at,target_occurred_at,created_at)"
            " VALUES(?,'w','p',?,?,?,NULL,?,?,?,?,'eligible',?,?,'2020')",
            [(h("4"), "conversion", "o2", "l2", "BTC", "LBTC", 40_000_000, 39_000_000, "2024-05-02T00:00:00Z", None),
             (h("5"), "direct_payout", "o3", None, "BTC", "USDT", 10_000_000, 7_000_000_000,
              "2024-05-03T00:00:00Z", "2024-05-03T01:00:00Z")])
        conn.execute("INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,detail_json,"
                     "created_at) VALUES('o3','w','p','missing_spot_price','{}','2020')")
        conn.commit()
    finally:
        conn.close()
    return root


@pytest.mark.parametrize("cipher", [False, True])
def test_transfer_audit_matches_python_byte_for_byte(tmp_path, environment, cipher):
    root = transfer_book(tmp_path / "book", cipher)
    before = raw(root, cipher)
    audit = same(root, root, environment, ["journals", "transfers", "list"], cipher=cipher)
    data = audit["data"]
    assert audit["kind"] == "journals.transfers.list"
    assert data["summary"] == {"same_asset_transfers": 2, "custody_transfers": 3, "cross_asset_pairs": 1,
                               "direct_swap_payouts": 1, "quarantines": 1}
    # The source fee is booked once, on the first decision that spends it.
    assert [t["fee_msat"] for t in data["same_asset_transfers"]] == [1_000, 0]
    assert "transfer_group_id" in data["same_asset_transfers"][0]
    assert "transfer_group_id" not in data["same_asset_transfers"][1]
    for argv in (["--workspace", "missing"], ["--profile", "missing"]):
        same(root, root, environment, ["journals", "transfers", "list", *argv], cipher=cipher)
    for fmt in ("table", "plain", "csv"):
        same(root, root, environment, ["--format", fmt, "journals", "transfers", "list"], cipher=cipher,
             machine=False)
    assert raw(root, cipher) == before


@pytest.mark.parametrize("country", ["unsupported", "AT", ""])
def test_transfer_audit_tax_country_gate_matches_python(tmp_path, environment, country):
    root = transfer_book(tmp_path / "book", country=country)
    same(root, root, environment, ["journals", "transfers", "list"])


QUARANTINES = [
    ("a", "missing_spot_price", {}),
    ("b", "transfer_pair_dependency_blocked", {"blocked_by_transaction_ids": ["a"]}),
    ("s", "custody_basis_barrier", {"root_transaction_ids": ["a", "missing"]}),
    ("i", "basis_provenance_incomplete", {}),
    ("t", "insufficient_lots", {"asset": "BTC"}),
]


def held_book(root, cipher=False):
    conn = _book(root, passphrase=KEY if cipher else None)
    try:
        context(conn)
        conn.execute("UPDATE transactions SET external_id='ext-' || id")
        conn.executemany(
            "INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,detail_json,created_at)"
            " VALUES(?,'w','p',?,?,'2024-05-01T00:00:00Z')",
            [(tx, reason, json.dumps(detail)) for tx, reason, detail in QUARANTINES])
        conn.execute("UPDATE profiles SET fiat_currency='eur'")
        conn.commit()
    finally:
        conn.close()
    return root


@pytest.mark.parametrize("cipher", [False, True])
def test_quarantine_reads_match_python_byte_for_byte(tmp_path, environment, cipher):
    root = held_book(tmp_path / "book", cipher)
    before = raw(root, cipher)
    listed = same(root, root, environment, ["journals", "quarantined"], cipher=cipher)
    assert [row["transaction_id"] for row in listed["data"]] == ["t", "i", "s", "b", "a"]
    assert all({"category", "root", "actions"} <= set(row) for row in listed["data"])
    for tx in ("a", "b", "s", "i", "t", "ext-a", "missing", ""):
        same(root, root, environment, ["journals", "quarantine", "show", "--transaction", tx], cipher=cipher)
    same(root, root, environment, ["journals", "quarantined", "--profile", "missing"], cipher=cipher)
    for fmt in ("table", "plain", "csv"):
        same(root, root, environment, ["--format", fmt, "journals", "quarantined"], cipher=cipher, machine=False)
        same(root, root, environment, ["--format", fmt, "journals", "quarantine", "show", "--transaction", "b"],
             cipher=cipher, machine=False)
    assert raw(root, cipher) == before
    change(root, "DELETE FROM journal_quarantines WHERE transaction_id='t'", cipher=cipher)
    same(root, root, environment, ["journals", "quarantine", "show", "--transaction", "t"], cipher=cipher)


@pytest.mark.parametrize("detail", ["not json", "[1, 2]", "", "7"])
def test_quarantine_reads_parse_stored_detail_as_python(tmp_path, environment, detail):
    root = held_book(tmp_path / "book")
    change(root, "UPDATE journal_quarantines SET detail_json=? WHERE transaction_id='i'", (detail,))
    same(root, root, environment, ["journals", "quarantined"])
    same(root, root, environment, ["journals", "quarantine", "show", "--transaction", "i"])


def pair(base, build, cipher=False):
    reference = build(base / "python", cipher)
    native = base / "rust"
    shutil.copytree(reference, native)
    return reference, native


def masked_bytes(value):
    return json.dumps(masked(json.loads(value))).encode() if value.startswith(b"{") else value


@pytest.mark.parametrize("cipher", [False, True])
@pytest.mark.parametrize("argv", [
    ["--transaction", "a"], ["--transaction", "ext-b"], ["--transaction", "missing"],
    ["--transaction", "a", "--profile", "missing"],
], ids=["root", "external-id", "missing", "scope"])
def test_quarantine_clear_matches_python_envelope_and_book(tmp_path, environment, cipher, argv):
    reference, native = pair(tmp_path, held_book, cipher)
    same(reference, native, environment, ["journals", "quarantine", "clear", *argv], cipher=cipher)
    assert raw(native, cipher) == raw(reference, cipher)
    # A second clear finds nothing held.
    same(reference, native, environment, ["journals", "quarantine", "clear", *argv], cipher=cipher)
    assert raw(native, cipher) == raw(reference, cipher)


def test_quarantine_clear_refuses_a_row_that_is_not_held(tmp_path, environment):
    reference, native = pair(tmp_path, held_book)
    for root in (reference, native):
        change(root, "DELETE FROM journal_quarantines WHERE transaction_id='a'")
    before = raw(native, False)
    expected = same(reference, native, environment, ["journals", "quarantine", "clear", "--transaction", "a"])
    assert expected["error"]["code"] == "not_found"
    assert raw(native, False) == before


RESOLVES = [
    ["price-override", "--transaction", "a", "--fiat-rate", "41234.5"],
    ["price-override", "--transaction", "ext-a", "--fiat-value", "4123.45678901234567890123456789"],
    ["price-override", "--transaction", "a", "--fiat-rate", "10", "--fiat-value", "2"],
    ["price-override", "--transaction", "a", "--fiat-rate", "1E+3", "--fiat-value", "0"],
    ["exclude", "--transaction", "t"],
    ["price-override", "--group-id", "root:a", "--fiat-rate", "10000"],
    ["price-override", "--group-id", "root:a", "--fiat-value", "100", "--reprocess"],
    ["exclude", "--group-id", "root:t"],
    ["exclude", "--transaction", "t", "--reprocess"],
    ["exclude", "--transaction", "i", "--reprocess"],
]

REFUSED = [
    ["price-override", "--transaction", "a"],
    ["price-override", "--transaction", "a", "--fiat-rate", "abc"],
    ["price-override", "--transaction", "a", "--fiat-rate", "-1"],
    ["price-override", "--transaction", "a", "--fiat-rate", "0"],
    ["price-override", "--transaction", "a", "--fiat-rate", ""],
    ["price-override", "--transaction", "a", "--fiat-rate", "NaN"],
    ["price-override", "--transaction", "a", "--fiat-value", "Infinity"],
    ["price-override", "--transaction", "a", "--fiat-value", "-5"],
    ["price-override", "--transaction", "a", "--fiat-rate", "1", "--fiat-value", "-5"],
    ["price-override", "--transaction", "a", "--fiat-rate", "-1", "--fiat-value", "abc"],
    ["price-override", "--transaction", "a", "--fiat-rate", "abc", "--fiat-value", "-1"],
    ["price-override", "--transaction", "a", "--fiat-rate", "1", "--fiat-value", "NaN"],
    ["price-override", "--group-id", "root:b", "--fiat-rate", "1"],
    ["price-override", "--group-id", "reason:missing_spot_price", "--fiat-rate", "1"],
    ["exclude", "--group-id", ""],
    ["exclude", "--transaction", "missing"],
    ["exclude", "--transaction", "missing", "--reprocess"],
    ["exclude", "--transaction", "a", "--workspace", "missing"],
    ["exclude", "--transaction", "a", "--fiat-rate", "1"],
    ["exclude"],
    ["exclude", "--transaction", "a", "--group-id", "root:a"],
]


@pytest.mark.parametrize("cipher", [False, True])
@pytest.mark.parametrize("argv", RESOLVES, ids=lambda argv: "-".join(a.strip("-") for a in argv)[:60])
def test_quarantine_resolve_matches_python_envelope_and_book(tmp_path, environment, cipher, argv):
    reference, native = pair(tmp_path, held_book, cipher)
    got = same(reference, native, environment, ["journals", "quarantine", "resolve", *argv], cipher=cipher,
               mask=masked_bytes)
    assert got["kind"] == "journals.quarantine.resolve." + argv[0], got
    assert masked_book(native, cipher) == masked_book(reference, cipher)


def test_quarantine_resolve_refusals_match_python_without_writes(tmp_path, environment):
    reference, native = pair(tmp_path, held_book)
    before = raw(native, False)
    for argv in REFUSED:
        same(reference, native, environment, ["journals", "quarantine", "resolve", *argv])
    assert raw(native, False) == before == raw(reference, False)


def test_quarantine_resolve_human_formats_match_python(tmp_path, environment):
    for fmt in ("table", "plain", "csv"):
        reference, native = pair(tmp_path / fmt, held_book)
        same(reference, native, environment,
             ["--format", fmt, "journals", "quarantine", "resolve", "exclude", "--transaction", "t"],
             machine=False, mask=lambda value: masked(value.decode()).encode())


def wallet_book(root, cipher=False):
    conn = _book(root, passphrase=KEY if cipher else None)
    try:
        context(conn)
        conn.execute("INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at)"
                     " VALUES('acct','w','p','ACC','Treasury','asset','2020')")
        conn.executemany(
            "INSERT INTO wallets(id,workspace_id,profile_id,account_id,label,kind,config_json,created_at)"
            " VALUES(?,'w','p',?,?,?,?,'2020')",
            [("desc", "acct", "Descriptor", "descriptor",
              json.dumps({"descriptor": DESCRIPTOR, "change_descriptor": CHANGE, "network": "bitcoin",
                          "blinding_key": None, "gap_limit": 5})),
             ("list", None, "Watch list", "address", json.dumps({"addresses": [LISTED], "chain": "bitcoin"})),
             ("sp", None, "Silent", "silent-payment", json.dumps({"sp_descriptor": "  sp(raw-test)  "})),
             ("odd", None, "Odd", "manual", "[1, 2]"),
             ("twin1", None, "Twin", "manual", ""), ("twin2", None, "twin", "manual", "null")])
        conn.execute("INSERT INTO backends(name,kind,chain,network,url,created_at,updated_at)"
                     " VALUES('core','bitcoinrpc','bitcoin','main','http://127.0.0.1:1','2025','2025')")
        conn.commit()
    finally:
        conn.close()
    return root


@pytest.mark.parametrize("cipher", [False, True])
def test_reveal_descriptor_matches_python_and_keeps_its_gates(tmp_path, environment, cipher):
    root = wallet_book(tmp_path / "book", cipher)
    before = raw(root, cipher)
    for wallet in ("desc", "Descriptor", "Watch list", "sp", "wallet", "odd", "Twin", "missing", ""):
        for extra in ([], ["--material-only"]):
            same(root, root, environment, ["wallets", "reveal-descriptor", "--wallet", wallet, *extra],
                 cipher=cipher)
    revealed = same(root, root, environment, ["wallets", "reveal-descriptor", "--wallet", "desc"], cipher=cipher)
    assert revealed["data"]["descriptor"] == DESCRIPTOR
    assert revealed["data"]["wallet_material"] == DESCRIPTOR + "\n" + CHANGE
    material = same(root, root, environment,
                    ["wallets", "reveal-descriptor", "--wallet", "desc", "--material-only"], cipher=cipher)
    assert material.stdout == (DESCRIPTOR + "\n" + CHANGE + "\n").encode()
    same(root, root, environment, ["wallets", "reveal-descriptor", "--wallet", "desc", "--profile", "missing"],
         cipher=cipher)
    for fmt in ("table", "plain", "csv", "json"):
        same(root, root, environment, ["--format", fmt, "wallets", "reveal-descriptor", "--wallet", "desc"],
             cipher=cipher, machine=False)
    # --output receives the material, not stdout.
    for native in (False, True):
        target = tmp_path / f"material-{native}.txt"
        result = cli(root, environment, native, ["--output", str(target), "wallets", "reveal-descriptor",
                                                  "--wallet", "desc", "--material-only"], cipher=cipher)
        assert (result.returncode, result.stdout) == (0, b"")
        assert target.read_text() == DESCRIPTOR + "\n" + CHANGE + "\n"
    if cipher:
        # A locked book stays locked: no passphrase descriptor, no prompt.
        expected = cli(root, environment, False, ["wallets", "reveal-descriptor", "--wallet", "desc"])
        actual = cli(root, environment, True, ["wallets", "reveal-descriptor", "--wallet", "desc"])
        assert actual.returncode == expected.returncode == 1, actual
        assert (actual.stdout, actual.stderr) == (expected.stdout, expected.stderr)
        assert DESCRIPTOR.encode() not in actual.stdout + actual.stderr
    assert raw(root, cipher) == before


@pytest.mark.parametrize("cipher", [False, True])
def test_identify_matches_python_in_every_format(tmp_path, environment, cipher):
    root = wallet_book(tmp_path / "book", cipher)
    # A wallet whose stored configuration is not an object fails the scan.
    for wallet in ("odd", "twin2"):
        same(root, root, environment, ["wallets", "identify", "--address", LISTED], cipher=cipher)
        change(root, "DELETE FROM wallets WHERE id=?", (wallet,), cipher=cipher)
    owned = same(root, root, environment, ["wallets", "identify", "--address", LISTED], cipher=cipher)
    assert owned["data"]["results"][0]["status"] == "owned", owned
    before = raw(root, cipher)
    candidates = tmp_path / "candidates.txt"
    candidates.write_bytes(f"# owned?\r\n{LISTED}\r\n{'f' * 64}\rbc1qnope\n".encode())
    sheet = tmp_path / "sheet.csv"
    sheet.write_text(f"date,address,txid\n2024-01-01,{LISTED},{'e' * 64}\n2024-01-02,BC1QNOPE,\n")
    binary = tmp_path / "binary.txt"
    binary.write_bytes(b"bc1q\x00")
    latin = tmp_path / "latin.txt"
    latin.write_bytes(b"caf\xe9")
    for argv in (
        ["--address", LISTED, "--txid", "f" * 64, "--candidate", "lnbc1bogus", "--candidate", "xpub-nope"],
        ["--address", LISTED.upper(), "--wallet", "Watch list"],
        ["--address", LISTED, "--wallet", "desc"],
        ["--address", LISTED, "--wallet", "missing"],
        ["--address", LISTED, "--wallet", "Twin"],
        ["--file", str(candidates)], ["--csv", str(sheet)], ["--file", str(candidates), "--csv", str(sheet)],
        ["--file", str(tmp_path / "missing.txt")], ["--csv", str(tmp_path / "missing.csv")],
        ["--file", str(tmp_path)], ["--file", str(binary)], ["--file", str(latin)], ["--file", ""],
        ["--address", LISTED, "--scan-to-index", "0"], ["--address", LISTED, "--scan-to-index", "-1"],
        ["--address", LISTED, "--scan-to-index", "99999999999999999999"],
        [], ["--scan-to-index", "-1"], ["--address", ""],
        ["--txid", "f" * 64, "--verify-on-chain", "--verify-backend", "core"],
        ["--txid", "f" * 64, "--verify-on-chain", "--verify-backend", "missing"],
        ["--address", LISTED, "--profile", "missing"],
    ):
        same(root, root, environment, ["wallets", "identify", *argv], cipher=cipher)
    for fmt in ("table", "plain", "csv", "json"):
        same(root, root, environment, ["--format", fmt, "wallets", "identify", "--address", LISTED, "--txid",
                                       "f" * 64, "--candidate", "nonsense"], cipher=cipher, machine=False)
    assert raw(root, cipher) == before


def test_identify_verify_on_chain_without_pending_txids_reaches_no_backend(tmp_path, environment):
    """An Esplora session opens nothing; with every input settled locally the
    report is the local one, marked verified, as in Python."""
    root = wallet_book(tmp_path / "book")
    change(root, "DELETE FROM wallets WHERE id IN ('odd', 'twin2')")
    change(root, "INSERT INTO backends(name,kind,chain,network,url,created_at,updated_at)"
                 " VALUES('esp','esplora','bitcoin','main','http://127.0.0.1:9','2025','2025')")
    got = same(root, root, environment, ["wallets", "identify", "--address", LISTED, "--verify-on-chain",
                                         "--verify-backend", "esp"])
    assert got["data"]["summary"]["verified_on_chain"] is True


ANALYZED = "Datum;Richtung;Stueck;Kurs\n2024-01-02;Kauf;0,5;40000\n2024-02-03;Verkauf;0,1;45000\n"
LEDGER = (
    "Type,Date,Received Amount,Received Asset,Sent Amount,Sent Asset,Fee Amount,Fee Asset,Fiat Value,"
    "Counterparty,Note,Tx-ID\n"
    "Buy,2026-01-15,0.05000000,BTC,3000.00,EUR,3.50,EUR,,Coinfinity,First stack,\n"
    "Sell,2026-02-10,2200.00,EUR,0.03000000,BTC,1.00,EUR,,Kraken,Took some profit,\n"
    "Mining,2026-03-10,0.00050000,BTC,,,,,32.50,Solo pool,Block reward,\n"
)


def analyze(tmp_path, environment, argv, options=("--machine",)):
    work = tmp_path / "work"
    if not work.exists():
        work.mkdir()
        (work / "export.csv").write_text(ANALYZED)
        (work / "map.json").write_text('{"date":"Datum","type":"Richtung","amount":"Stueck","price":"Kurs"}')
        # The native ledger layout, which previews its normalized rows.
        (work / "ledger.csv").write_text(LEDGER)
        (work / "scan.pdf").write_bytes(b"%PDF-1.4\n")
        (work / "notes.docx").write_bytes(b"PK")
    results = []
    for native in (False, True):
        root = tmp_path / f"root-{native}"
        prefix = [str(BINARY)] if native else REFERENCE
        result = subprocess.run([*prefix, *options, "--data-root", str(root), "wallets", "analyze-file", *argv],
                                env=environment, capture_output=True, cwd=work, stdin=subprocess.DEVNULL,
                                timeout=300)
        results.append((result.returncode, result.stdout, result.stderr))
        # A layout, never a book.
        assert not (root / "kassiber.sqlite3").exists()
    assert results[1] == results[0], (argv, options)
    return results[0]


@pytest.mark.parametrize("argv", [
    [], ["--sample-rows", "0"], ["--sample-rows", "-4"], ["--sample-rows", "1000"],
    ["--file", "ledger.csv", "--sample-rows", "-4"], ["--file", "ledger.csv", "--sample-rows", "2"],
    ["--column-map", '{"date":"Datum","type":"Richtung","amount":"Stueck"}'],
    ["--column-map", "@map.json"], ["--column-map", "@missing.json"], ["--column-map", "not json"],
    ["--column-map", "[1]"], ["--column-map", "  "],
], ids=lambda argv: "-".join(argv)[:40] or "default")
def test_analyze_file_matches_python_without_a_book(tmp_path, environment, argv):
    code, stdout, _ = analyze(tmp_path, environment, ["--file", "export.csv", *argv])
    if code == 0:
        assert json.loads(stdout)["kind"] == "wallets.file_analysis"


def test_analyze_file_refusals_and_formats_match_python(tmp_path, environment):
    for argv in (["--file", "export.csv"], ["--file", "scan.pdf"], ["--file", "notes.docx"],
                 ["--file", "missing.csv"], ["--file", "."]):
        for options in (["--machine"], ["--format", "table"], ["--format", "plain"]):
            analyze(tmp_path, environment, argv, options)


@pytest.mark.parametrize("payload", ["5", "null", '["filters", "occurred_at", "created_at", "id"]'])
@pytest.mark.parametrize("leaf", [["journals", "list"], ["journals", "events", "list"]])
def test_non_object_cursor_refuses_where_python_crashes(tmp_path, environment, payload, leaf):
    """Documented difference (rust-shell.md, final CLI leaves): Python's
    `_decode_event_cursor` catches only ValueError, so a cursor whose JSON is
    not an object escapes as an internal error (`TypeError`/`AttributeError`).
    Rust refuses it like every other invalid cursor."""
    root = journal_book(tmp_path / "book")
    cursor = base64.urlsafe_b64encode(payload.encode()).decode().rstrip("=")
    python = json.loads(cli(root, environment, False, [*leaf, "--cursor", cursor]).stdout)
    rust = json.loads(cli(root, environment, True, [*leaf, "--cursor", cursor]).stdout)
    assert python["error"]["code"] == "internal_error", python
    assert rust["error"] == {
        "code": "validation",
        "message": f"Invalid cursor: {cursor}",
        "hint": "Pass the exact next_cursor value from the previous response; do not modify it or change filters.",
        "details": None,
        "retryable": False,
        "debug": None,
    }


SHIPPED_LEDGER_TRIGGERS = json.loads(
    (ROOT / "crates/store/fixtures/legacy-ledger-triggers.json").read_text())["triggers"]


def ledger_book(root, cipher=False, extra_sql=""):
    """Populated archives with the guards Kassiber shipped for them (from
    `crates/store/fixtures/legacy-ledger-triggers.json`), three of them on
    `external_documents`, plus one empty archive table."""
    conn = _book(root, passphrase=KEY if cipher else None)
    try:
        context(conn)
        conn.executescript(
            "CREATE TABLE gl_books(profile_id TEXT PRIMARY KEY REFERENCES profiles(id) ON DELETE RESTRICT,"
            " currency TEXT NOT NULL, created_at TEXT NOT NULL);"
            "CREATE TABLE gl_entries(id TEXT PRIMARY KEY, profile_id TEXT NOT NULL REFERENCES profiles(id)"
            " ON DELETE RESTRICT, description TEXT NOT NULL, UNIQUE(profile_id, id));"
            "CREATE TABLE gl_lines(entry_id TEXT NOT NULL, profile_id TEXT NOT NULL, ordinal INTEGER NOT NULL,"
            " PRIMARY KEY(entry_id, ordinal));"
            "CREATE TABLE gl_evidence(id TEXT PRIMARY KEY, profile_id TEXT NOT NULL, content BLOB NOT NULL,"
            " source_document_id TEXT REFERENCES external_documents(id) ON DELETE RESTRICT);"
            "CREATE TABLE gl_empty(id TEXT);")
        conn.execute(
            "INSERT INTO external_documents(id, workspace_id, profile_id, document_type, label, created_at,"
            " updated_at) VALUES ('doc-1', 'w', 'p', 'invoice', 'Invoice', '2026-01-01T00:00:00Z',"
            " '2026-01-01T00:00:00Z')")
        conn.execute("INSERT INTO gl_books VALUES ('p', 'EUR', '2026-01-01T00:00:00Z')")
        conn.execute("INSERT INTO gl_entries VALUES ('entry-1', 'p', 'Opening'), ('entry-2', 'p', 'Close')")
        conn.execute("INSERT INTO gl_lines VALUES ('entry-1', 'p', 1)")
        conn.execute("INSERT INTO gl_evidence VALUES ('ev-1', 'p', x'00', 'doc-1')")
        tables = {"gl_books", "gl_entries", "gl_lines", "gl_evidence", "external_documents"}
        for trigger in SHIPPED_LEDGER_TRIGGERS:
            if trigger["table"] in tables:
                conn.execute(trigger["sql"])
        if extra_sql:
            conn.executescript(extra_sql)
        conn.commit()
    finally:
        conn.close()
    return root


def ledger_triggers():
    return sum(1 for t in SHIPPED_LEDGER_TRIGGERS
               if t["table"] in {"gl_books", "gl_entries", "gl_lines", "gl_evidence", "external_documents"})


@pytest.mark.parametrize("cipher", [False, True])
def test_purge_legacy_ledger_matches_python_envelope_and_book(tmp_path, environment, cipher):
    reference, native = pair(tmp_path, ledger_book, cipher)
    before = raw(native, cipher)
    plan = same(reference, native, environment, ["maintenance", "purge-legacy-ledger"], cipher=cipher)
    assert plan["kind"] == "maintenance.purge-legacy-ledger.plan"
    assert plan["data"]["tables"] == [
        {"table": "gl_books", "rows": 1}, {"table": "gl_entries", "rows": 2},
        {"table": "gl_evidence", "rows": 1}, {"table": "gl_lines", "rows": 1},
    ]
    assert plan["data"]["applied"] is False and plan["data"]["table_count"] == 5
    assert plan["data"]["trigger_count"] == ledger_triggers() > 3
    assert raw(native, cipher) == before
    for fmt in ("table", "plain"):
        same(reference, native, environment, ["--format", fmt, "maintenance", "purge-legacy-ledger"],
             cipher=cipher, machine=False)
    done = same(reference, native, environment, ["maintenance", "purge-legacy-ledger", "--confirm"], cipher=cipher)
    assert done["kind"] == "maintenance.purge-legacy-ledger" and done["data"]["applied"] is True
    assert raw(native, cipher) == raw(reference, cipher)
    assert not any(name.startswith("gl_") for name in raw(native, cipher))
    # Nothing is left to plan or purge.
    same(reference, native, environment, ["maintenance", "purge-legacy-ledger"], cipher=cipher)
    same(reference, native, environment, ["maintenance", "purge-legacy-ledger", "--confirm"], cipher=cipher)


@pytest.mark.parametrize("trigger", [
    "CREATE TRIGGER gl_evidence_document_mirror AFTER INSERT ON external_documents"
    " BEGIN UPDATE profiles SET label = NEW.label; END;",
    "CREATE TRIGGER gl_evidence_guard BEFORE DELETE ON gl_evidence BEGIN SELECT 1; END;",
], ids=["after-insert-on-documents", "unshipped-body"])
def test_unshipped_ledger_trigger_refuses_where_python_opens(tmp_path, environment, trigger):
    """Documented difference (rust-shell.md, final CLI leaves): Python's open
    path keeps every `gl_*` object of a populated archive; Rust accepts only
    the triggers Kassiber shipped and refuses any other as schema drift
    before reading or writing the book."""
    root = ledger_book(tmp_path / "book", extra_sql=trigger)
    before = raw(root, False)
    python = json.loads(cli(root, environment, False, ["maintenance", "purge-legacy-ledger"]).stdout)
    assert python["kind"] == "maintenance.purge-legacy-ledger.plan", python
    rust = cli(root, environment, True, ["maintenance", "purge-legacy-ledger", "--confirm"])
    assert rust.returncode == 1
    assert json.loads(rust.stdout)["error"]["code"] == "requires_python_migration"
    assert raw(root, False) == before
