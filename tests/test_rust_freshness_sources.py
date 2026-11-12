"""Freshness jobs beside on-chain history, against unchanged Python's daemon.

BTCPay wallet sources, BTCPay provenance (wallet and account routes) and
market-rate coverage run natively, and books holding wallets without a
freshness source (node, file, silent-payment) no longer refuse a run
(docs/reference/rust-native-wallet-gaps.md, S4 and S5). Each case runs the
same daemon requests through Python and through the hybrid's Rust daemon on
the same starting bytes, over a loopback Greenfield fake: response and event
lines and every manifest table compare with generated ids and clocks masked.
"""
import json
from pathlib import Path
import re
import sqlite3

import pytest

from tests.test_rust_connections import (
    PASSPHRASE, RUST_BIN, SECRET, btcpay_server, masked, masked_dump, provenance_book, run, started_stamp,
    wire,
)
from tests.rust_freshness_session import Session
from tests.test_rust_wallet_sync_btcpay import routes

pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the kassiber test-harness build")

ROUTE = [{"backend": "merchant", "store_id": "shop", "payment_method_id": "btc-chain"}]


def _book(tmp_path, url, wallets, *, sql="", encrypted=False):
    root, env = provenance_book(tmp_path, url, encrypted)
    if encrypted:
        from tests.test_rust_connections import connect
        conn = connect(root, True)
    else:
        conn = sqlite3.connect(root / "kassiber.sqlite3")
    for wallet_id, label, kind, config in wallets:
        conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                     "VALUES(?,'w','p',?,?,?,'2026-01-01T00:00:00Z')", (wallet_id, label, kind, json.dumps(config)))
    if sql:
        conn.executescript(sql)
    conn.commit()
    conn.close()
    return root, env


FRACTIONAL = re.compile(rb"20[2-9][0-9]-[01][0-9]-[0-3][0-9]T[0-2][0-9]:[0-5][0-9]:[0-5][0-9]\.[0-9]+Z")


def _masked(data, started):
    """`masked`, plus clocks with fractional seconds (the rate cache's `fetched_at`)."""
    data = masked(data, started)
    return FRACTIONAL.sub(lambda m: b"<now>" if m.group(0)[:19] >= started[:19] else m.group(0), data)


def _settled(lines):
    """Response and event lines as key-sorted JSON (key order is not semantic)."""
    return [json.dumps(json.loads(line), sort_keys=True) for line in lines]


def _rust_lines(root, env, requests):
    """The Rust daemon answers freshness runs off its request loop: send each
    request only after the previous one's answer, as the desktop does."""
    with Session(root, env) as session:
        for index, (kind, args) in enumerate(requests):
            session.send(kind, str(index), args)
            session.until(str(index), None if kind == "daemon.unlock" else kind)
        session.finish()
    return [json.dumps(row).encode() for row in session.rows[1:]]


def _compare(root, env, hits, requests, url, *, encrypted=False):
    database = root / "kassiber.sqlite3"
    initial = database.read_bytes()
    started = started_stamp()
    if encrypted:
        requests = [("daemon.unlock", {"auth_response": {"passphrase_secret": PASSPHRASE}}), *requests]
    outcomes = []
    for reference in [True, False]:
        database.write_bytes(initial)
        hits.clear()
        if reference:
            result = run(root, ["daemon"], env, reference=True, wire=wire(requests))
            assert (result.returncode, result.stderr) == (0, b""), result.stderr
            lines = result.stdout.splitlines()[1:]
        else:
            lines = _rust_lines(root, env, requests)
        lines = [_masked(line.replace(url.encode(), b"URL").replace(str(root).encode(), b"ROOT"), started)
                 for line in lines]
        assert not any(SECRET.encode() in line for line in lines)
        tables = {name: sorted(_masked(row, started) for row in rows)
                  for name, rows in masked_dump(root, started, encrypted).items()}
        outcomes.append((_settled(lines), sorted(path for path, _ in hits), tables))
    (root.parent / "freshness-parity.json").write_text(json.dumps(
        {"expected": [json.loads(line) for line in outcomes[0][0]],
         "actual": [json.loads(line) for line in outcomes[1][0]]}, indent=1, sort_keys=True))
    if outcomes[1][0] != outcomes[0][0]:
        from tests.rust_port.json_diff import json_diff
        for want, got in zip(outcomes[0][0], outcomes[1][0]):
            if want != got:
                raise AssertionError(json_diff(json.loads(want), json.loads(got)))
        raise AssertionError((len(outcomes[0][0]), len(outcomes[1][0])))
    assert outcomes[1][1] == outcomes[0][1]
    for table in outcomes[0][2]:
        assert outcomes[1][2][table] == outcomes[0][2][table], table
    assert outcomes[1][2] == outcomes[0][2]
    return [json.loads(line) for line in outcomes[1][0]]


def _last(rows):
    """The last request's answer (the daemon's shutdown answer follows it)."""
    return [row for row in rows if row.get("request_id") not in {"stop", None}
            and not row["kind"].endswith(".progress")][-1]


def _btcpay_wallets(tmp_path):
    exports = tmp_path / "exports"
    exports.mkdir(exist_ok=True)
    csv = exports / "mapped.csv"
    csv.write_text("txid,occurred_at,direction,asset,amount,fee,description\n"
                   f"{'ab' * 32},2023-11-14T22:15:00Z,inbound,BTC,0.001,0,Mapped\n")
    return [
        ("shop", "Shop", "custom", {"sync_source": "btcpay", "backend": "merchant", "store_id": "shop"}),
        ("ledger", "Ledger", "custom", {"sync_source": "btcpay", "backend": "merchant", "store_id": "shop",
                                        "source_mode": "payments", "payment_method_ids": ["BTC-LN", "btc-chain"]}),
        ("mapped", "Mapped", "custom", {"source_file": str(csv), "source_format": "csv", "btcpay_provenance": ROUTE}),
        ("ro", "Read only", "custom", {"source_file": str(csv), "source_format": "csv",
                                       "btcpay_provenance": [{**ROUTE[0], "store_id": "ro"}]}),
        ("broken", "Broken", "custom", {"sync_source": "btcpay", "backend": "merchant", "store_id": "broken"}),
    ]


ACCOUNT_ROUTE = """
    INSERT INTO btcpay_account_routes(id,workspace_id,profile_id,backend_name,store_id,payment_method_id,action,label,
                                      created_at,updated_at)
    VALUES('route-1','w','p','merchant','shop','BTC-LN','provenance_only','Shop LN','2026-01-01T00:00:00Z',
           '2026-01-01T00:00:00Z');
"""


# Jobs of one priority dispatch in (created_at, id) order: one random uuid
# decides between two same-second jobs in either implementation, so each case
# holds at most one BTCPay wallet source and one provenance job.
COMBOS = {
    "source-and-wallet-route": (["shop", "mapped"], ""),
    "ledger-and-account-route": (["ledger"], "ACCOUNT"),
    "failing-source-and-read-only-route": (["broken", "ro"], ""),
}


@pytest.mark.parametrize("encrypted", [False, True], ids=["sqlite", "sqlcipher"])
@pytest.mark.parametrize("combo", COMBOS)
def test_btcpay_freshness_jobs_match_python(tmp_path, combo, encrypted):
    """Wallet sources, payment ledgers, provenance wallet and account routes,
    three runs: the later ones read the checkpoints the first stored, the last
    is forced."""
    ids, account = COMBOS[combo]
    wallets = [wallet for wallet in _btcpay_wallets(tmp_path) if wallet[0] in ids]
    with btcpay_server(routes()) as (url, hits):
        root, env = _book(tmp_path, url, wallets, sql=ACCOUNT_ROUTE if account else "", encrypted=encrypted)
        run_args = {"rates": False, "journals": False}
        rows = _compare(root, env, hits, [("ui.freshness.run", run_args), ("ui.freshness.run", run_args),
                                          ("ui.freshness.run", {**run_args, "force_full": True})],
                        url, encrypted=encrypted)
    responses = [row for row in rows if row.get("kind") == "ui.freshness.run"]
    jobs = {job["source_label"]: job["status"] for job in responses[0]["data"]["completed"]}
    assert len(jobs) == 2, jobs
    if combo.startswith("failing"):
        assert jobs["Broken BTCPay wallet source"] == "error", jobs
    else:
        assert set(jobs.values()) == {"done"}, jobs


def test_btcpay_wallet_sync_runs_its_freshness_jobs_like_python(tmp_path):
    """`ui.wallets.sync` plans the BTCPay source and provenance jobs and runs them."""
    with btcpay_server(routes()) as (url, hits):
        wallets = [wallet for wallet in _btcpay_wallets(tmp_path) if wallet[0] in {"shop", "mapped"}]
        root, env = _book(tmp_path, url, wallets)
        rows = _compare(root, env, hits, [("ui.wallets.sync", {"process_journals": False}),
                                          ("ui.wallets.sync", {"wallet": "Shop", "process_journals": False})], url)
    results = [row for row in rows if row.get("kind") == "ui.wallets.sync"]
    assert len(results[0]["data"]["results"]) == 2, results[0]


@pytest.mark.parametrize("switch", ["offline", "no-egress"])
def test_btcpay_freshness_jobs_refuse_egress_without_traffic(tmp_path, switch):
    with btcpay_server(routes()) as (url, hits):
        root, env = _book(tmp_path, url, _btcpay_wallets(tmp_path)[:1])
        if switch == "offline":
            Path(env["KASSIBER_OFFLINE_PREFERENCE_FILE"]).write_text('{"enabled":true}')
        else:
            env["KASSIBER_NO_EGRESS"] = "1"
        rows = _compare(root, env, hits, [("ui.freshness.run", {"rates": False, "journals": False})], url)
        assert hits == []
    job = _last(rows)["data"]["completed"][0]
    assert job["status"] == "error", job


POLICY = """
    INSERT OR REPLACE INTO settings(key,value) VALUES('freshness.policy.profile.p',
      '{"background_enabled": false, "report_read_sync": false, "source_classes": {"onchain_wallet": true,
        "btcpay_wallet": true, "btcpay_provenance": true, "market_rates": true, "journals": false}}');
"""


@pytest.mark.parametrize("provider", ["coinbase-exchange", "coingecko"])
def test_market_rate_coverage_seeds_then_refuses_live_lookups_without_egress(tmp_path, provider):
    """The job fills the bundled hourly seed offline, then reaches the live
    provider only under the enabled `market_rates` class; without egress that
    lookup fails the job, after the seed, with no traffic."""
    with btcpay_server({}) as (url, hits):
        sql = POLICY + f"INSERT OR REPLACE INTO settings(key,value) VALUES('market_rate_provider','{provider}');"
        root, env = _book(tmp_path, url, [], sql=sql)
        env["KASSIBER_NO_EGRESS"] = "1"
        rows = _compare(root, env, hits, [("ui.freshness.run", {"rates": True, "journals": False}),
                                          ("ui.freshness.run", {"rates": True, "journals": False})], url)
    jobs = _last(rows)["data"]["completed"]
    assert [job["job_type"] for job in jobs] == ["market_rate_coverage"], jobs
    assert jobs[0]["status"] == "error", jobs


NO_SOURCE_WALLETS = {
    "coreln": ("coreln", {"backend": "cln"}),
    "lnd": ("lnd", {"backend": "lnd"}),
    "nwc": ("nwc", {}),
    "phoenix": ("phoenix", {}),
    "silent-payment": ("silent-payment", {}),
    # Still refused natively (a Core RPC backend), but only if it would run.
    "core-silent-payment": ("silent-payment", {
        "sp_descriptor": "sp(spscan1q" + "q" * 40 + ")", "backend": "core", "chain": "bitcoin", "network": "main",
        "sp_scan_mode": "local_index", "sp_scan_start_height": 850_000, "sp_full_history": False}),
}
CORE_BACKEND = ("INSERT INTO backends(name,kind,url,token,timeout,config_json,created_at,updated_at) "
                "VALUES('core','bitcoinrpc','http://127.0.0.1:9','',2,"
                "'{\"silent_payment_scan_file\":\"/nonexistent\",\"silent_payments\":true}','fixed','fixed');")


@pytest.mark.parametrize("shape", NO_SOURCE_WALLETS)
def test_wallets_without_freshness_sources_do_not_refuse_a_run(tmp_path, shape):
    """Python plans no job for node, Phoenix, NWC or silent-payment wallets;
    a run beside them runs the book's other sources."""
    kind, config = NO_SOURCE_WALLETS[shape]
    with btcpay_server(routes()) as (url, hits):
        root, env = _book(tmp_path, url, [*_btcpay_wallets(tmp_path)[:1], ("node", "Node", kind, config)],
                          sql=CORE_BACKEND if shape == "core-silent-payment" else "")
        rows = _compare(root, env, hits, [("ui.freshness.run", {"rates": False, "journals": False})], url)
    assert [job["job_type"] for job in _last(rows)["data"]["completed"]] == ["btcpay_wallet_source"], rows[-1]


def test_file_sourced_wallet_selection_syncs_directly_like_python(tmp_path):
    """A file-sourced wallet has no freshness job, so `ui.wallets.sync` of it
    runs the direct sync, natively."""
    with btcpay_server({}) as (url, hits):
        wallets = [w for w in _btcpay_wallets(tmp_path) if w[0] == "mapped"]
        wallets = [(w[0], w[1], w[2], {k: v for k, v in w[3].items() if k != "btcpay_provenance"}) for w in wallets]
        root, env = _book(tmp_path, url, wallets)
        rows = _compare(root, env, hits, [("ui.wallets.sync", {"wallet": "Mapped", "process_journals": False})], url)
    result = _last(rows)["data"]["results"][0]
    assert (result["wallet"], result["status"]) == ("Mapped", "synced"), result
