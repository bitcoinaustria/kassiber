"""Byte-exact native report reads against Python-produced journals."""
from __future__ import annotations

from collections import Counter
from datetime import datetime, timedelta, timezone
import json
import os
from pathlib import Path
import sys

import pytest

from kassiber.db import open_db
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

    # These new leaves port stored reads only; CSV is an excluded export.
    with monkeypatch.context() as direct:
        direct.setenv("KASSIBER_TEST_CLI", str(Path(RUST_BIN).resolve()))
        for command in ("tax-summary", "journal-entries", "austrian-e1kv", "austrian-tax-summary", "balance-history"):
            year = ("--year", "2024") if command.startswith("austrian-") else ()
            result = call("--format", "csv", "reports", command, *year, machine=False)
            assert result.returncode == 1
            assert result.stdout == b""
            assert result.stderr == b"error: CSV report exports are not implemented in this Rust read slice\n"
