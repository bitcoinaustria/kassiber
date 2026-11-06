#!/usr/bin/env python3
"""Freeze unchanged Python profitability, CSV, lookup and window behavior.

This is a report-kernel oracle, not a live adapter or process acceptance test.
Every book and generated file stays under this checkout's target directory.
"""
from __future__ import annotations

import copy
from contextlib import closing
import csv
import json
from pathlib import Path
import random
import sqlite3
import sys
import tempfile

from kassiber.core.lightning import (
    NodeChannel, NodeRoutingSnapshot, NodeSnapshot, build_profitability_report,
    profitability_csv_rows, resolve_lightning_connection,
)
from kassiber.daemon import _coerce_int
from kassiber.db import open_db
from kassiber.envelope import build_envelope
from kassiber.errors import AppError

ROOT = Path(__file__).resolve().parents[3]
OUT = ROOT / "crates/reports/tests/data/lightning.json"
SCRATCH = ROOT / "target/r5-lightning/oracle"
# Every table of the current schema, from the store manifest both runtimes admit.
TABLES = sorted(o[1] for o in json.loads((ROOT / "crates/store/fixtures/current-schema.json").read_text())["objects"]
                if o[0] == "table")
CONNECTION = dict(id="node", label="Café node", kind="coreln", profile_id="p", backend_name=None)
SEED = """
DELETE FROM schema_migration_audits;
INSERT INTO schema_migration_audits(id,migration_name,schema_version,impact_json,created_at)
VALUES('lightning-fixture','lightning-fixture',1,'{"canary":"preserve"}','fixed');
INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES
('node','ws','p','Café node','coreln','{}','fixed'),
('lnd','ws','p','Home node','lnd','{}','fixed'),
('nwc','ws','p','Remote','nwc','{}','fixed'),
('other-node','ws','other','Home node','lnd','{}','fixed');
"""


def wire(value):
    return json.dumps(value, ensure_ascii=True) + "\n"


def error(exc):
    return dict(code=exc.code if isinstance(exc, AppError) else "internal_error",
                message=str(exc), hint=getattr(exc, "hint", None),
                details=getattr(exc, "details", None), retryable=getattr(exc, "retryable", False))


def snapshot(raw):
    channels = tuple(NodeChannel(
        id=c["id"], peer_alias=c["peer_alias"], peer_pubkey=None,
        capacity_sat=int(c["capacity_sat"]), local_balance_sat=0, remote_balance_sat=0,
        state="active", is_private=True,
        earned_routing_sat=None if c["earned_routing_sat"] is None else int(c["earned_routing_sat"]),
    ) for c in raw["channels"])
    routing = None if raw["routing"] is None else NodeRoutingSnapshot(**{
        k: v if k == "window_label" else int(v) for k, v in raw["routing"].items()
    })
    return NodeSnapshot(alias="private operator alias", pubkey="private operator pubkey",
                        network="regtest", peer_count=0, onchain_balance_sat=0,
                        total_local_balance_sat=0, total_remote_balance_sat=0,
                        total_capacity_sat=0, channels=channels, routing=routing)


def report_specs():
    base = dict(routing=dict(window_label="Last 30 days", routing_revenue_sat="9999",
        payment_cost_sat="250", rebalance_cost_sat="49", onchain_cost_sat="100",
        net_profit_sat="9599", forward_count="17", payment_count="3", rebalance_count="2"), channels=[
        dict(id="800000x2x0", peer_alias="Café, \"North\"\r\nLine", capacity_sat="1000000", earned_routing_sat="2499"),
        dict(id="42", peer_alias="private peer", capacity_sat="2000000", earned_routing_sat="2500"),
        dict(id="43", peer_alias="=SUM(1,2)", capacity_sat="-1", earned_routing_sat=None)])
    yield "baseline", copy.deepcopy(base), "2500"
    yield "no-routing", dict(routing=None, channels=copy.deepcopy(base["channels"])), "2500"
    yield "empty", dict(routing=None, channels=[]), "2500"
    changed = copy.deepcopy(base)
    changed["routing"]["net_profit_sat"] = "-98765"
    yield "authoritative-net", changed, "2500"
    for name, label in [("controls", "NUL\x00TAB\tCR\rLF\n"), ("astral", "雪 🚀"),
                        ("surrogate", "a\ud800b"), ("surrogate-run", "a\ud800\udfffz")]:
        for slot in ("window", "alias", "id"):
            changed = copy.deepcopy(base)
            if slot == "window":
                changed["routing"]["window_label"] = label
            else:
                changed["channels"][0]["peer_alias" if slot == "alias" else "id"] = label
            yield f"{name}-{slot}", changed, "2500"
    for cost in ("0", "-1", "2501", "9007199254740993"):
        yield "cost-" + cost, copy.deepcopy(base), cost
    for n in (0, 1, -1, 999, -999, 2**53-1, 2**53, 2**53+1,
              2**63-1, -(2**63), 900719925474099301, 10**60+12345, 10**316, 10**317):
        for sign in (1, -1):
            changed = copy.deepcopy(base)
            for key in changed["routing"]:
                if key != "window_label":
                    changed["routing"][key] = str(sign*n)
            changed["channels"][1]["earned_routing_sat"] = str(sign*n)
            yield f"amount-{n}-{sign}", changed, "2500"
    rng = random.Random(210017)
    for i in range(96):
        changed = copy.deepcopy(base)
        for key in changed["routing"]:
            if key != "window_label":
                changed["routing"][key] = str(rng.randrange(-(10**40), 10**40))
        for channel in changed["channels"]:
            channel["earned_routing_sat"] = str(rng.randrange(-(2**80), 2**80))
        rng.shuffle(changed["channels"])
        yield f"seeded-{i}", changed, str(rng.randrange(-10000, 10000))


def make_report(raw, cost):
    return build_profitability_report(connection_id=CONNECTION["id"], connection_label=CONNECTION["label"],
        connection_kind=CONNECTION["kind"], snapshot=snapshot(raw), default_open_cost_sat=int(cost))


def reports():
    result = []
    for name, raw, cost in report_specs():
        report = make_report(raw, cost)
        payload = report.to_envelope_payload()
        expected = dict(payload_wire=wire(payload), envelope_wire=wire(build_envelope("reports.lightning-profitability", payload)))
        with tempfile.TemporaryDirectory(dir=SCRATCH, prefix="csv-") as root:
            path = Path(root) / "export.csv"
            path.write_bytes(b"previous contents must be truncated\n")
            try:
                rows = profitability_csv_rows(report)
                expected["rows_wire"] = wire(rows)
                with path.open("w", newline="", encoding="utf-8") as handle:
                    csv.writer(handle).writerows(rows)
                metadata = dict(connection={k: CONNECTION[k] for k in ("id", "label", "kind")},
                                file="<root>/export.csv", rows=len(rows)-1)
                expected["export_wire"] = wire(metadata)
            except Exception as exc:
                expected["error_wire"] = wire(error(exc))
            expected["file_hex"] = path.read_bytes().hex()
        # JSON decoders combine a high/low surrogate escape pair. Preserve the
        # original Python code points explicitly for these adversarial inputs.
        encoded = copy.deepcopy(raw)
        for item, key in ([(encoded["routing"], "window_label")] if encoded["routing"] else []) + [
                (channel, key) for channel in encoded["channels"] for key in ("id", "peer_alias")]:
            if any(0xD800 <= ord(c) <= 0xDFFF for c in item[key]):
                item[key] = dict(codepoints=[ord(c) for c in item[key]])
        result.append(dict(name=name, snapshot=encoded, open_cost_sat=cost, expected=expected))
    return result


def connection_specs():
    yield "by-id", {}, ""
    yield "by-label", dict(connection="home NODE"), ""
    yield "wallet-alias", dict(connection="", wallet="lnd"), ""
    yield "label-alias", dict(connection=None, wallet="", label="Remote"), ""
    yield "other-profile", dict(connection="Home node", profile="Other"), ""
    yield "explicit-scope", dict(connection="Home node", workspace="Main", profile="Book"), ""
    yield "whitespace-ref", dict(connection=" Home node "), ""
    yield "untrimmed-id", dict(connection=" node "), ""
    yield "unknown", dict(connection="absent"), ""
    yield "wrong-kind", dict(connection="Cold"), ""
    for i, ref in enumerate((None, "", 0, 1, True, [], {}, ["node"], "\ud800")):
        yield f"invalid-{i}", dict(connection=ref), ""
    for name, sql in [
        ("missing-workspace", "DELETE FROM settings WHERE key='context_workspace';"),
        ("missing-profile", "DELETE FROM settings WHERE key='context_profile';"),
        ("stale-journals", "UPDATE profiles SET last_processed_at=NULL,journal_input_version=99;"),
        ("unsupported-tax", "UPDATE profiles SET tax_country='zz';"),
        ("no-journals", "DELETE FROM journal_entries;"),
        ("collision", "UPDATE wallets SET label='node' WHERE id='lnd';"),
    ]:
        yield name, {}, sql
    for name, config in (("bad-config", "{"), ("null-config", "null"), ("list-config", "[]"),
                         ("number-config", "123"), ("backend-number", '{"backend":3}'),
                         ("backend-empty", '{"backend":""}'),
                         ("backend-space", '{"backend":"  "}'),
                         ("backend-secret", '{"backend":"local","token":"never project","rpc_file":"never project"}')):
        yield name, {}, "UPDATE wallets SET config_json='" + config + "' WHERE id='node';"
    yield "unknown-profile", dict(profile="absent"), ""
    yield "unknown-workspace", dict(workspace="absent"), ""
    yield "surrogate-missing-scope", dict(connection="\ud800"), "DELETE FROM settings WHERE key='context_workspace';"
    for kind, config in (("array", "["*1100 + "0" + "]"*1100),
                         ("object", '{"x":'*1100 + "0" + "}"*1100)):
        yield "deep-config-" + kind, {}, "UPDATE wallets SET config_json='" + config + "' WHERE id='node';"


def table_dump(conn):
    tables = [r[0] for r in conn.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
    assert tables == TABLES
    return {t: sorted([list(row) for row in conn.execute(f'SELECT * FROM "{t}"')], key=wire) for t in tables}


def connections(baseline):
    result = []
    for name, args, sql in connection_specs():
        args = {"connection": "node", **args}
        # Copy the initialized schema instead of repeating every migration for
        # each read-only lookup. Each case still owns an isolated database.
        with closing(sqlite3.connect(":memory:")) as conn:
            baseline.backup(conn)
            conn.row_factory = sqlite3.Row
            conn.execute("PRAGMA foreign_keys=ON")
            conn.executescript(sql)
            before = table_dump(conn)
            try:
                reference = args.get("connection") or args.get("wallet") or args.get("label")
                row = resolve_lightning_connection(conn, reference, workspace_ref=args.get("workspace"), profile_ref=args.get("profile"))
                del row["config"]
                expected = dict(ok=row)
            except Exception as exc:
                expected = dict(error=error(exc))
            assert table_dump(conn) == before
            result.append(dict(name=name, args_wire=wire(args), sql=sql, expected_wire=wire(expected), tables_wire=wire(before)))
    return result


def windows():
    result = []
    for raw in ("null", "true", "false", "0", "-1", "1", "365", "366", "99999999999999999999999999999",
                "2.9", "-2.9", "2024.9", '" ３６６ "', '"3_0"', '"3.5"', '"bad"', '""', "[]", "{}", "NaN", "Infinity", "-Infinity"):
        try:
            expected = dict(ok=_coerce_int(json.loads(raw), default=30, minimum=1, maximum=365))
        except Exception as exc:
            expected = dict(error=error(exc))
        result.append(dict(input=raw, expected_wire=wire(expected)))
    return result


def build():
    SCRATCH.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=SCRATCH, prefix="baseline-") as root:
        conn = open_db(Path(root) / "data")
        conn.executescript((ROOT / "crates/reports/tests/data/book.sql").read_text() + SEED)
        baseline = wire(table_dump(conn))
        try:
            cases = connections(conn)
        finally:
            conn.close()
    return json.dumps(dict(seed=SEED, tables_wire=baseline, reports=reports(), connections=cases, windows=windows()), ensure_ascii=True, indent=2) + "\n"


if __name__ == "__main__":
    content = build()
    if "--check" in sys.argv:
        assert OUT.read_text() == content, "Lightning vectors drifted"
    else:
        OUT.write_text(content)
