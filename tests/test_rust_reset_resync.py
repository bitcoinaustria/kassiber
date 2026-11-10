"""Reset book data, then Sync: the native runtime matches Python byte for byte.

File-source wallets re-import from local files, so the whole reset and resync
runs without egress. The reset clears wallet sync checkpoints and last-sync
times; sync-all then includes retired wallets until their history is back.
"""
import json

import pytest

from tests.test_rust_connections import RUST_BIN, btcpay_book, masked, masked_dump, run, served, started_stamp, wire
from tests.test_rust_wallet_sync_sources import compare_cli

pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires the kassiber test-harness build")

SYNCED_AT = "2026-01-02T00:00:00Z"
RESET = {"confirm": "RESET", "confirm_profile": "Book", "auth_response": {"plaintext_delete_ack": "DELETE LOCAL DATA"}}
CSV = ("txid,occurred_at,direction,asset,amount,fee,description\n"
       f"{'1' * 64},2026-01-02T10:00:00Z,inbound,BTC,0.0015,0.000002,Salary\n")
JSON = [{"txid": "json-1", "occurred_at": "2026-02-01T00:00:00Z", "direction": "inbound", "asset": "BTC",
         "amount": 0.25, "fee": 0, "fiat_rate": 40000, "description": "Funding"}]


def reset_book(tmp_path, *, unreadable=False):
    exports = tmp_path / "exports"
    exports.mkdir()
    (exports / "live.csv").write_text(CSV)
    (exports / "old.json").write_text(json.dumps(JSON))
    wallets = [
        ("live", "Live", {"source_file": str(exports / "live.csv"), "source_format": "csv"}),
        ("old", "Old", {"source_file": str(exports / "old.json"), "source_format": "json", "deprecated": True}),
        ("gone", "Gone", {"deprecated": "yes", "last_synced_at": SYNCED_AT}),
        *([("odd", "Odd", None)] if unreadable else []),
    ]
    sql = "DELETE FROM wallets WHERE id='a';" + "".join(
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES"
        f"('{wallet_id}','w','p','{label}','custom','{'not json' if config is None else json.dumps(config)}',"
        "'2026-01-01T00:00:00Z');"
        for wallet_id, label, config in wallets)
    states = [
        ("onchain_wallet:live", "onchain_wallet", "fresh", 1, 0, '{"electrum_statuses": {"s": "h"}}'),
        ("btcpay_wallet:live", "btcpay_wallet", "blocking_reports", 0, 1, '{"cursor": 7}'),
        ("market_rates:BTC-EUR", "market_rates", "fresh", 0, 0, '{"since": "2026-01-01"}'),
    ]
    sql += "".join(
        "INSERT INTO freshness_source_states(profile_id,source_key,source_type,source_label,status,state,paused,"
        f"blocking_reports,retry_count,last_success_at,last_error_code,checkpoint_json,progress_json,updated_at) VALUES"
        f"('p','{key}','{kind}','{key}','{status}','{status}',{paused},{blocking},2,'{SYNCED_AT}','old_error',"
        f"'{checkpoint}','{{\"phase\": \"done\"}}','{SYNCED_AT}');"
        for key, kind, status, paused, blocking, checkpoint in states)
    return btcpay_book(tmp_path, sql=sql)


def compare_daemon(root, env, requests):
    database = root / "kassiber.sqlite3"
    initial = database.read_bytes()
    started = started_stamp()
    outcomes = []
    for reference in (True, False):
        database.write_bytes(initial)
        result = run(root, ["daemon"], env, reference=reference, wire=wire(requests))
        assert (result.returncode, result.stderr) == (0, b""), result.stderr
        lines = masked(result.stdout.replace(str(root).encode(), b"ROOT"), started).splitlines()
        outcomes.append((lines[1:], masked_dump(root, started)))
    assert outcomes[1][0] == outcomes[0][0]
    assert outcomes[1][1] == outcomes[0][1]
    return [json.loads(line) for line in outcomes[1][0]]


def synced(result):
    return {row["wallet"]: row["status"] for row in result["data"]}


def test_reset_then_sync_reimports_every_wallet_like_python(tmp_path):
    root, env = reset_book(tmp_path)
    # A retired wallet that has never synced joins sync-all once.
    first, _ = compare_cli(root, env, ["wallets", "sync", "--all"])
    assert synced(first) == {"Live": "synced", "Old": "synced"}, first
    ongoing, _ = compare_cli(root, env, ["wallets", "sync", "--all"])
    assert synced(ongoing) == {"Live": "synced"}, ongoing

    responses = compare_daemon(root, env, [("ui.profiles.reset_data", RESET)])
    removed = responses[0]["data"]["removed"]
    assert (removed["transactions"], removed["sync_checkpoints"], removed["wallet_sync_times"]) == (2, 2, 3), removed

    again, tables = compare_cli(root, env, ["wallets", "sync", "--all"])
    assert synced(again) == {"Gone": "skipped", "Live": "synced", "Old": "synced"}, again
    assert sum(row.get("imported", 0) for row in again["data"]) == 2, again
    assert len(tables["transactions"]) == 2

    # A retired wallet with no source never records a sync, so it stays in
    # sync-all as a no-I/O skip; one whose history is back drops out again.
    ongoing, _ = compare_cli(root, env, ["wallets", "sync", "--all"])
    assert synced(ongoing) == {"Gone": "skipped", "Live": "synced"}, ongoing
    forced, _ = compare_cli(root, env, ["wallets", "sync", "--all", "--force-full"])
    assert synced(forced) == {"Gone": "skipped", "Live": "synced", "Old": "synced"}, forced
    assert {entry["served_by"] for entry in served(env)} == {"rust"}


def test_reset_skips_an_unreadable_wallet_config_like_python(tmp_path):
    root, env = reset_book(tmp_path, unreadable=True)
    responses = compare_daemon(root, env, [("ui.profiles.reset_data", RESET)])
    assert responses[0]["data"]["removed"]["wallet_sync_times"] == 1, responses
