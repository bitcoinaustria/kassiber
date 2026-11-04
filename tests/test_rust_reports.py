"""Byte-exact native report reads against Python-produced journals."""
from __future__ import annotations

from collections import Counter
from datetime import datetime, timedelta, timezone
import json
import os
from pathlib import Path
import sys

import pytest

from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION, open_db
from tests import cli_runner
from tests.test_rust_shell_parity import _environment

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires Rust test-harness binary")
COMMANDS = ("summary", "balance-sheet", "portfolio-summary", "capital-gains", "tax-summary", "journal-entries", "balance-history")


def _setup(tmp_path, monkeypatch, country="generic", algorithm="fifo", size=32):
    env = _environment(tmp_path / "home")
    env["KASSIBER_HYBRID_RUST_BIN"] = str(Path(RUST_BIN).resolve())
    env.setdefault("KASSIBER_HYBRID_LOG", str(tmp_path / "served.jsonl"))
    monkeypatch.setenv("KASSIBER_TEST_CLI", json.dumps([
        sys.executable, str(cli_runner.ROOT / "tests/rust_port/hybrid_cli.py")]))
    root = tmp_path / "data"
    owners = tmp_path / "owners"
    owners.mkdir(mode=0o700)
    env["KASSIBER_RUST_TEST_OWNER_ROOT"] = str(owners)
    original_prefix = cli_runner.prefix
    script = (
        "import sys; from pathlib import Path; from kassiber.operator import project; "
        "root=Path(sys.argv.pop(1)); project._owner_lock_root=lambda:root; "
        "from kassiber.cli.main import main; raise SystemExit(main(sys.argv[1:]))"
    )
    def prefix(*, reference=False, python_options=()):
        if reference:
            return (sys.executable, "-c", script, str(owners))
        return original_prefix(reference=reference, python_options=python_options)
    monkeypatch.setattr(cli_runner,"prefix",prefix)

    def call(*args, reference=False, machine=True):
        return cli_runner.run([*(["--machine"] if machine else []), "--data-root", root, *args],
                              reference=reference, env=env, capture_output=True, timeout=180)

    for args in (("init",), ("workspaces", "create", "Main"),
                 ("profiles", "create", "Book", "--workspace", "Main", "--fiat-currency", "EUR", "--tax-country", country, "--gains-algorithm", algorithm.upper())):
        result = call(*args)
        assert result.returncode == 0, result.stdout
    for index, label in enumerate(("Cold", "Hot")):
        csv = tmp_path / f"{label}.csv"
        lines = ["date,txid,direction,asset,amount,fee,fiat_rate,kind,description"]
        # Every sale consumes multiple acquisition lots, including fractional sats.
        for i in range(size):
            at = (datetime(2020 if i < 2 else 2024, 1, 1, tzinfo=timezone.utc) + timedelta(hours=i)).isoformat().replace("+00:00", "Z")
            direction = "outbound" if i % 3 == 2 else "inbound"
            amount = "0.00150000001" if direction == "outbound" else "0.00100000003"
            kind = "withdrawal" if direction == "outbound" else ("interest" if i % 11 == 0 else "deposit")
            lines.append(f"{at},{label}-{i},{direction},BTC,{amount},0,{30000+i*73+index*19},{kind},Receipt € {i}")
        csv.write_text("\n".join(lines) + "\n")
        result = call("wallets", "create", "--label", label, "--workspace", "Main", "--profile", "Book", "--kind", "custom")
        assert result.returncode == 0, result.stdout
        result = call("wallets", "import-csv", "--file", str(csv), "--wallet", label, "--workspace", "Main", "--profile", "Book")
        assert result.returncode == 0, result.stdout
    result = call("context", "set", "--workspace", "Main", "--profile", "Book")
    assert result.returncode == 0, result.stdout
    result = call("journals", "process")
    assert result.returncode == 0, result.stdout
    return root, env, call


def _same(call, *args, success=True):
    reference = call(*args, reference=True)
    rust = call(*args)
    assert (rust.returncode, rust.stdout, rust.stderr) == (reference.returncode, reference.stdout, reference.stderr)
    if success:
        assert rust.returncode == 0, rust.stdout
    return json.loads(rust.stdout)


@pytest.mark.parametrize("country,algorithm,size", [("generic", "fifo", 32), ("generic", "hifo", 32), ("at", "moving_average_at", 32), ("generic", "fifo", 1500), ("generic", "hifo", 1500), ("at", "moving_average_at", 1500)])
def test_reports_match_reference_bytes_and_run_in_rust(tmp_path, monkeypatch, country, algorithm, size):
    root, env, call = _setup(tmp_path, monkeypatch, country, algorithm, size)
    for command in COMMANDS:
        _same(call, "reports", command)
    for interval in ("hour", "day", "week", "month"):
        _same(call, "reports", "balance-history", "--interval", interval,
              "--start", "2024-01-01T12:34:56.123456Z", "--end", "2024-01-03T01:00:00Z")
    for wallet in ("Cold", "Hot"):
        _same(call, "reports", "balance-history", "--wallet", wallet,
              "--start", "2024-01-01", "--end", "2024-04-01")
    for command in ("austrian-e1kv", "austrian-tax-summary"):
        for year in (2020, 2024):
            _same(call, "reports", command, "--year", str(year), success=country=="at")
    _same(call, "reports", "summary", "--wallet", "Cold")
    _same(call, "reports", "summary", "--wallet", "absent", success=False)
    calls = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    report_calls = [c for c in calls if c["command"].startswith("reports.")]
    assert report_calls and all(c["served_by"] == "rust" for c in report_calls)
    counts = Counter(c["command"] for c in report_calls)
    assert set(counts) == {f"reports.{c}" for c in (*COMMANDS,"austrian-e1kv","austrian-tax-summary")}
    conn = open_db(root)
    assert conn.execute("SELECT COUNT(*) FROM transactions").fetchone()[0] == size * 2
    conn.close()
    _history_daemon_bytes(root, env)


@pytest.mark.parametrize("country,algorithm,size", [
    ("generic", "fifo", 32), ("generic", "hifo", 32), ("at", "moving_average_at", 32),
    ("generic", "fifo", 1500), ("generic", "hifo", 1500), ("at", "moving_average_at", 1500),
])
def test_tabular_report_csv_bytes_and_atomic_output(tmp_path, monkeypatch, country, algorithm, size):
    root, env, call = _setup(tmp_path, monkeypatch, country, algorithm, size)
    conn = open_db(root)
    conn.execute("UPDATE journal_entries SET description=? WHERE entry_type='disposal'",
                 ('Receipt €: comma, quote " and CR\r\nLF\n',))
    workspace_id, profile_id = conn.execute("SELECT workspace_id,id FROM profiles WHERE label='Book'").fetchone()
    for state in ("saved", "filed"):
        conn.execute("""INSERT INTO filed_report_snapshots(
            id,workspace_id,profile_id,report_kind,report_state,period_start_year,period_end_year,
            content_sha256,classification_summary_json,gain_summary_json,report_scope_json,created_at)
            VALUES(?,?,?,?,?,2020,2024,?,?,?,?,?)""",
            (state, workspace_id, profile_id, "full-report.csv", state, "a" * 64,
             '{"income":{"count":1,"amount_msat":1001}}', '{"gain_loss_exact":"1.2300"}',
             '{}', "2025-01-01T00:00:00Z"))
    conn.commit()
    sealed_tables = ("filed_report_snapshots", "custody_filed_report_impacts",
                     "custody_filed_report_impact_resolutions", "filed_report_chain_captures",
                     "filed_report_chain_dependencies")
    sealed_before = {table: [tuple(row) for row in conn.execute(f"SELECT * FROM {table}")]
                     for table in sealed_tables}
    commands = [(command,) for command in COMMANDS]
    commands += [(command, "--year", "2024") for command in ("austrian-e1kv", "austrian-tax-summary")]
    commands += [("filed-snapshots", "list")]
    for command in commands:
        args = ("--format", "csv", "reports", *command)
        reference = call(*args, reference=True, machine=False)
        rust = call(*args, machine=False)
        assert (rust.returncode, rust.stdout, rust.stderr) == (reference.returncode, reference.stdout, reference.stderr)
        if command[0].startswith("austrian") and country != "at":
            assert rust.returncode == 1
            continue
        assert rust.returncode == 0, rust.stderr
        assert rust.stdout.endswith(b"\r\n")
        if command[0] == "journal-entries":
            assert b'"Receipt \xe2\x82\xac: comma, quote "" and CR\r\nLF\n"' in rust.stdout
        output = tmp_path / "nested" / (command[0] + ".csv")
        file_args = ("--format", "csv", "--output", str(output), "reports", *command)
        expected = call(*file_args, reference=True, machine=False)
        csv_bytes = output.read_bytes()
        assert csv_bytes == reference.stdout
        output.write_bytes(b"previous artifact\r\n")
        actual = call(*file_args, machine=False)
        assert (actual.returncode, actual.stdout, actual.stderr) == (expected.returncode, expected.stdout, expected.stderr) == (0, b"", b"")
        assert output.read_bytes() == csv_bytes
        assert not list(output.parent.glob(".*.tmp"))
    assert sealed_before == {table: [tuple(row) for row in conn.execute(f"SELECT * FROM {table}")]
                             for table in sealed_tables}
    conn.close()
    calls = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    report_calls = [c for c in calls if c["command"].startswith("reports.")]
    assert report_calls and all(c["served_by"] == "rust" for c in report_calls)


def test_tabular_csv_refusals_leave_existing_artifacts_and_snapshots(tmp_path, monkeypatch):
    root, env, call = _setup(tmp_path, monkeypatch, country="at", algorithm="moving_average_at")
    output = tmp_path / "report.csv"
    output.write_bytes(b"previous artifact\r\n")
    conn = open_db(root)
    conn.execute("UPDATE profiles SET last_processed_input_version=-1")
    conn.commit()
    for command in [*( (c,) for c in COMMANDS ), ("austrian-e1kv", "--year", "2024"),
                    ("austrian-tax-summary", "--year", "2024")]:
        args = ("--format", "csv", "--output", str(output), "reports", *command)
        reference = call(*args, reference=True, machine=False)
        rust = call(*args, machine=False)
        assert (rust.returncode, rust.stdout, rust.stderr) == (reference.returncode, reference.stdout, reference.stderr)
        assert rust.returncode == 1
        assert output.read_bytes() == b"previous artifact\r\n"
    conn.execute("UPDATE profiles SET last_processed_input_version=journal_input_version")
    # The marker write clears the semantics stamp; restore it to stay current.
    conn.execute("UPDATE profiles SET journal_semantics_version=?", (JOURNAL_CALCULATION_SEMANTICS_VERSION,))
    for table in ("journal_entries", "journal_tax_summary", "journal_account_holdings", "journal_wallet_holdings"):
        conn.execute(f"DELETE FROM {table}")
    conn.commit()
    for command in ("capital-gains", "journal-entries", "balance-sheet", "portfolio-summary", "tax-summary"):
        args = ("--format", "csv", "--output", str(output), "reports", command)
        reference = call(*args, reference=True, machine=False)
        rust = call(*args, machine=False)
        assert (rust.returncode, rust.stdout, rust.stderr) == (reference.returncode, reference.stdout, reference.stderr)
        assert rust.returncode == 1
        assert b"CSV output requires a list of records" in rust.stderr
        assert output.read_bytes() == b"previous artifact\r\n"
    assert conn.execute("SELECT count(*) FROM filed_report_snapshots").fetchone()[0] == 0
    conn.close()


def _history_daemon_bytes(root, env):
    requests = [{"kind":"ui.reports.balance_history", "args":args} for args in (
        {}, {"interval":"day","start":"2024-01-01","end":"2024-04-01","wallet":"Cold","limit":7},
        {"interval":"week","start":"2023-12-31","end":"2024-04-01","asset":"BTC"},
        {"interval":"hour","start":"2024-01-01","end":"2024-01-03","limit":500},
        {"interval":"month","start":"2020-01-01","end":"2024-04-01","wallet":"Hot"},
        {"start":"2024-01-01","end":"2024-04-01","limit":True},
        {"limit":0}, {"limit":False}, {"limit":None}, {"limit":-1}, {"limit":[]},
        {"limit":"bad"}, {"limit":" ５００ "}, {"limit":2.9}, {"limit":10000},
        {"limit":999999999999999999999999999}, {"z":1,"a":2,"m":3},
        {"interval":None}, {"interval":[]}, {"interval":"unsupported"},
        {"start":"bad"}, {"start":"2025-01-01","end":"2024-01-01"},
        {"wallet":"absent"}, {"account":"absent"}, [],
    )]
    requests.append({"kind":"daemon.shutdown"})
    wire = "".join(json.dumps({"request_id":i, **request}) + "\n" for i,request in enumerate(requests)).encode()
    reference = cli_runner.run(["--data-root",root,"daemon"],reference=True,env=env,input=wire,capture_output=True,timeout=180)
    rust = cli_runner.run(["--data-root",root,"daemon"],env={**env,"KASSIBER_HYBRID_RUST_DAEMON":"1"},input=wire,capture_output=True,timeout=180)
    assert (rust.returncode,rust.stderr) == (reference.returncode,reference.stderr) == (0,b"")
    ready = json.loads(rust.stdout.splitlines()[0])
    assert "ui.reports.balance_history" in ready["data"]["supported_kinds"]
    assert rust.stdout.splitlines()[1:] == reference.stdout.splitlines()[1:]


def test_readiness_and_cached_market_rates_match_bytes(tmp_path, monkeypatch):
    root, env, call = _setup(tmp_path, monkeypatch)
    result = call("rates", "set", "BTC-EUR", "2026-01-01T00:00:00Z", "97531.12345678901234567890123456789")
    assert result.returncode == 0, result.stdout
    for command in COMMANDS:
        _same(call, "reports", command, success=False)
    result = call("journals", "process")
    assert result.returncode == 0, result.stdout
    for command in COMMANDS:
        _same(call, "reports", command)
    conn = open_db(root)
    conn.execute("UPDATE profiles SET last_processed_tx_count=last_processed_tx_count+1")
    conn.commit()
    conn.close()
    for command in COMMANDS:
        payload = _same(call, "reports", command, success=False)
        assert payload["error"]["code"] == "app_error"


@pytest.mark.parametrize("country,algorithm", [("generic", "fifo"), ("generic", "hifo"), ("at", "moving_average_at")])
def test_native_daemon_holdings_match_bytes(tmp_path, monkeypatch, country, algorithm):
    root, env, call = _setup(tmp_path, monkeypatch, country=country, algorithm=algorithm)
    requests = [{"kind": kind} for kind in ("ui.reports.balance_sheet", "ui.reports.portfolio_summary", "ui.reports.summary")]
    requests.extend({"kind":"ui.reports.summary", "args":args} for args in
                    ({"wallet":" Cold "}, {"wallet":"absent"}, {"wallet":""},
                     {"wallet":False}, ["wallet"], {"a":1,"\n":2,"é":3}, {}))
    requests.extend({"kind":"ui.reports.tax_summary", "args":args} for args in
                    ({}, {"year":2024}, {"year":"2024"}, {"year":True},
                     {"year":2024.9}, {"year":[]}, {"year":"bad"}, {"unknown":1}, []))
    requests.append({"kind":"daemon.shutdown"})
    wire = "".join(json.dumps({"request_id":i, **request}) + "\n" for i,request in enumerate(requests)).encode()
    args = ["--data-root", root, "daemon"]
    reference = cli_runner.run(args, reference=True, env=env, input=wire, capture_output=True, timeout=120)
    rust = cli_runner.run(args, env={**env,"KASSIBER_HYBRID_RUST_DAEMON":"1"}, input=wire, capture_output=True, timeout=120)
    assert (rust.returncode,rust.stderr) == (reference.returncode,reference.stderr) == (0,b"")
    assert rust.stdout.splitlines()[1:] == reference.stdout.splitlines()[1:]


def test_snapshot_listing_remains_readable_when_live_journals_are_stale(tmp_path, monkeypatch):
    root, env, call = _setup(tmp_path, monkeypatch)
    for state in ("saved", "filed"):
        result=call("reports","filed-snapshots","create","--state",state,"--report-kind","summary","--period-start-year","2024","--period-end-year","2024","--content-sha256","a"*64)
        assert result.returncode==0,result.stdout
    result=call("metadata","notes","set","--transaction","Cold-0","--note","Reviewed")
    assert result.returncode==0,result.stdout
    result=_same(call,"reports","filed-snapshots","list")
    assert len(result["data"]["snapshots"])==2
    calls=[json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    assert {"command":"reports.filed-snapshots.list","served_by":"rust"} in calls
    creates = [c for c in calls if c["command"] == "reports.filed-snapshots.create"]
    assert len(creates) == 2 and all(c["served_by"] == "rust" for c in creates)


@pytest.mark.parametrize("country,algorithm", [
    ("generic", "fifo"), ("generic", "hifo"), ("at", "moving_average_at"),
])
def test_snapshot_creation_rows_and_frozen_dependency_copy(tmp_path, monkeypatch, country, algorithm):
    """Only generated identities/times are normalized; stored JSON stays literal."""
    import uuid
    from kassiber.core import custody_filed_reports as snapshots

    root, env, call = _setup(tmp_path, monkeypatch, country, algorithm)
    conn = open_db(root)
    workspace, profile = conn.execute("SELECT workspace_id,id FROM profiles WHERE label='Book'").fetchone()
    # A phase-4 local capture is a frozen fixture, independent of current journals.
    snapshots.create_filed_report_snapshot(conn, workspace_id=workspace, profile_id=profile,
        report_kind="full-report.csv", report_state="saved", period_start_year=2020, period_end_year=2024,
        content_sha256="b" * 64, classification_summary={"income":{"count":1,"amount_msat":1001}},
        gain_summary={"gain_loss_exact":"1.2300000", "status":"final"},
        report_scope={"wallet_ids":["frozen-wallet"]}, snapshot_id="captured", created_at="2025-01-01T00:00:00Z")
    conn.execute("INSERT INTO filed_report_chain_captures(snapshot_id,profile_id,captured_at) VALUES(?,?,?)",
                 ("captured", profile, "original"))
    conn.execute("""INSERT INTO filed_report_chain_dependencies(snapshot_id,profile_id,transaction_id,wallet_id,
        chain,network,txid,observation_json) VALUES(?,?,?,?,?,?,?,?)""",
        ("captured", profile, "frozen-transaction", "frozen-wallet", "bitcoin", "main", "c"*64,
         '{"status":"observed","confirmed":true,"quantity_hash":"sealed","domain_id":"original"}'))
    conn.execute("UPDATE profiles SET last_processed_at=NULL WHERE id=?", (profile,))
    conn.commit()
    database = Path(conn.execute("PRAGMA database_list").fetchone()[2])
    conn.close()
    original = database.read_bytes()
    tables = ("filed_report_snapshots", "filed_report_chain_captures", "filed_report_chain_dependencies",
              "custody_filed_report_impacts", "custody_filed_report_impact_resolutions",
              "filed_report_chain_impacts", "filed_report_chain_resolutions")
    args = ("--report-kind", " FULL.Report-CSV ", "--period-start-year", "2020", "--period-end-year", "2024",
            "--content-sha256", " A" + "A"*63 + " ", "--classification-summary-json",
            '{"Income":{"count":2,"amount_msat":1001},"fee":{}}', "--gain-summary-json",
            '{"gain_loss_exact":"1.23000000000000000000000001","status":"FINAL","fiat_currency":"eur"}',
            "--report-scope-json", '{"wallet_ids":["hot","cold","cold"]}', "--notes", ' Reviewed € 🪙, "quote"\n')

    def sequence(reference):
        replacements = {}
        outcomes = []
        for i, extra in enumerate((
            ("--state", "saved", *args), ("--state", "filed", *args),
            ("--state", "filed", "--saved-snapshot-id", "captured"),
            ("--state", "filed", "--saved-snapshot-id", "captured"),
        )):
            result = call("reports", "filed-snapshots", "create", *extra, reference=reference)
            assert result.returncode == 0, result.stdout
            envelope = json.loads(result.stdout)
            created = envelope["data"]
            assert uuid.UUID(created["id"]).version == 4
            replacements[created["id"]] = f"action-{i}"
            replacements[created["created_at"]] = "<creation-time>"
            outcomes.append(result.stdout)
        conn = open_db(root)
        rows = {table:[dict(row) for row in conn.execute(f"SELECT * FROM {table}")] for table in tables}
        conn.close()
        def normalize(v):
            if isinstance(v, dict):return {k:normalize(item) for k,item in v.items()}
            if isinstance(v, list):return [normalize(item) for item in v]
            if isinstance(v, str):return replacements.get(v,v)
            return v
        for capture in rows["filed_report_chain_captures"]:
            if capture["snapshot_id"] != "captured":capture["captured_at"] = "<creation-time>"
        rows = normalize(rows)
        for items in rows.values():items.sort(key=lambda r: json.dumps(r,sort_keys=True))
        normalized_wire = []
        for wire in outcomes:
            for actual, normalized in replacements.items():
                wire = wire.replace(actual.encode(), normalized.encode())
            normalized_wire.append(wire.decode())
        return json.dumps({"outcomes":normalized_wire,"rows":rows}, ensure_ascii=False,
                          sort_keys=True,separators=(",", ":")).encode()

    expected = sequence(True)
    database.write_bytes(original)
    assert sequence(False) == expected
    for extra in (("--saved-snapshot-id", "missing"),
                  ("--saved-snapshot-id", "captured", "--gain-summary-json", "{}"),
                  ("--saved-snapshot-id", "captured", "--report-scope-json", "{}"),
                  ("--report-kind", "summary", "--period-start-year", "2024", "--content-sha256", "a"*64,
                   "--classification-summary-json", "[]")):
        _same(call, "reports", "filed-snapshots", "create", "--state", "filed", *extra, success=False)
    _same(call, "reports", "filed-snapshots", "list")
    calls = [json.loads(line) for line in Path(env["KASSIBER_HYBRID_LOG"]).read_text().splitlines()]
    assert all(c["served_by"] == "rust" for c in calls if c["command"].startswith("reports."))


def test_table_and_plain_reads_match_bytes(tmp_path,monkeypatch):
    root,env,call=_setup(tmp_path,monkeypatch,country="at",algorithm="moving_average_at")
    for format_ in ("table","plain"):
        for command in COMMANDS:
            args=("--format",format_,"reports",command)
            python=call(*args,reference=True,machine=False)
            rust=call(*args,machine=False)
            assert (rust.returncode,rust.stdout,rust.stderr)==(python.returncode,python.stdout,python.stderr)
            assert rust.returncode==0,rust.stderr

    for format_ in ("table","plain"):
        for command in ("austrian-e1kv","austrian-tax-summary"):
            args=("--format",format_,"reports",command,"--year","2024")
            python=call(*args,reference=True,machine=False)
            rust=call(*args,machine=False)
            assert (rust.returncode,rust.stdout,rust.stderr)==(python.returncode,python.stdout,python.stderr)
            assert rust.returncode==0,rust.stderr
