import argparse
import copy
from dataclasses import asdict
import json
from pathlib import Path
import sys
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core.lightning import lnd

OUTPUT = ROOT / "crates/chain/fixtures/python_lightning.json"
NOW = 2_000_000_000
SECRET = "DISCARDED_LIGHTNING_SECRET"


def base():
    return {
        "info": {"best_header_timestamp": NOW, "identity_pubkey": SECRET},
        "onchain": {}, "channel_balance": {}, "fees": {},
        "open": {"channels": [
            {"chan_id": "1", "capacity": "9007199254740993", "private": True,
             "remote_pubkey": SECRET, "pending_htlcs": [{"payment_preimage": SECRET}]},
            {"chan_id": "2", "capacity": 23, "peer_alias": "Comma, newline\n雪"},
        ]},
        "closed": {"channels": [{"chan_id": "3", "capacity": 99}]},
        "forwards": {"forwarding_events": [
            {"timestamp": NOW, "chan_id_in": "2", "chan_id_out": "1", "fee_msat": "1501", "erring_node": SECRET},
            {"timestamp": NOW, "chan_id_in": "2", "chan_id_out": "1", "fee_msat": "1501", "route": {"hops": [SECRET]}},
        ]},
        "payments": {"payments": [{"creation_date": NOW, "status": "SUCCEEDED", "fee_msat": "1999", "payment_preimage": SECRET, "payment_request": SECRET,
            "htlcs": [{"route": {"hops": [SECRET]}, "failure_source_pubkey": SECRET}]}]},
        "invoices": {"invoices": [{"r_preimage": SECRET, "payment_request": SECRET, "payment_addr": SECRET, "route_hints": [SECRET]}]},
        "aliases": {},
    }


def cases():
    yield "base-discard", base(), 30
    empty = {k: {} for k in base()}
    yield "empty", empty, 30
    for window in [1, 30, 365]:
        for now in [None, 0, "0", NOW, str(NOW)]:
            data = base()
            data["info"]["best_header_timestamp"] = now
            yield f"window-{window}-now-{now}", data, window
    for amount in [None, "", "bad", 0, "0", -1, -1001, "1501", 1501.9, True, "١٥٠١", "9_007_199_254_740_993", 2**100]:
        data = base()
        data["forwards"]["forwarding_events"] = [{"timestamp": NOW, "chan_id_out": "1", "fee_msat": amount, "fee": "7"}]
        yield f"forward-amount-{amount}", data, 30
        data = base()
        data["payments"]["payments"] = [{"creation_date": NOW, "status": "SUCCEEDED", "fee_msat": amount, "fee_sat": "7"}]
        yield f"payment-amount-{amount}", data, 30
    for status in ["SUCCEEDED", "SUCCESS", "COMPLETE", "succeeded", "FAILED", "FAILURE", "IN_FLIGHT", None]:
        data = base()
        data["payments"]["payments"][0]["status"] = status
        yield f"payment-status-{status}", data, 30
    start = NOW - 30 * 86400
    for timestamp in [0, start - 1, start, start + 1, NOW]:
        data = base()
        data["forwards"]["forwarding_events"][0]["timestamp"] = timestamp
        data["payments"]["payments"][0]["creation_date"] = timestamp
        yield f"window-boundary-{timestamp}", data, 30
    for seconds, nanos in [(start - 1, start), (start, start - 1), (0, start), (0, start - 1)]:
        data = base()
        data["payments"]["payments"][0].update(creation_date=seconds, creation_time_ns=nanos * 1_000_000_000)
        yield f"payment-or-timestamp-{seconds}-{nanos}", data, 30
    for private in [True, False, "true", "TRUE", "yes", "1", "false", "0", 1, 0, "\x1cTRUE\x1f"]:
        data = base()
        row = data["open"]["channels"][0]
        row.update(private=private, remote_pubkey="peer-public-or-private")
        data["aliases"] = {"peer-public-or-private": "  Local alias  "}
        yield f"private-{private!r}", data, 30
    for alias in [None, "", " Alias ", "\ud800", "雪\U0001f680"]:
        data = base()
        data["open"]["channels"][0]["peer_alias"] = alias
        yield f"alias-{alias!r}", data, 30
    for index, rows in enumerate([
        [{"chan_id": "123", "private": True}],
        [{"chan_id": "123", "remote_pubkey": "public-peer"}],
        [{"channel_point": "ab" * 32 + ":0", "capacity": "23"}],
        [{"chan_id": "1x2x3", "capacity": -10}],
        [{"chan_id": "1", "capacity": 2**100}],
        [{"chan_id": "bad-secret"}],
        [{"chan_id": "1", "channel_point": "bad-secret"}],
        [None, "ignored", {"chan_id": "2"}],
    ]):
        data = base()
        data["open"]["channels"] = rows
        yield f"channels-{index}", data, 30
    data = base()
    data["open"]["channels"] = [
        {"chan_id": "1", "private": True, "remote_pubkey": "b"},
        {"chan_id": "2", "private": True, "remote_pubkey": "a"},
        {"chan_id": "3", "private": True, "remote_pubkey": "b"},
    ]
    data["closed"]["channels"] = [{"chan_id": "4", "private": True, "remote_pubkey": "c"}]
    data["aliases"] = {"a": " A ", "b": " B ", "c": " C "}
    yield "sorted-deduplicated-aliases-including-closed", data, 30


def evaluate(name, data, window):
    calls = []
    keys = {
        "/v1/getinfo": "info", "/v1/balance/blockchain": "onchain",
        "/v1/balance/channels": "channel_balance", "/v1/fees": "fees",
        "/v1/channels": "open", "/v1/channels/closed": "closed",
        "/v1/switch": "forwards", "/v1/payments": "payments", "/v1/invoices": "invoices",
    }

    class Client:
        def __init__(self, backend):
            pass

        def get(self, path, *, params=None):
            if path.startswith("/v1/graph/node/"):
                payload = {"node": {"alias": data["aliases"].get(path.rsplit("/", 1)[1], "")}}
            else:
                payload = data[keys[path]]
            calls.append({"method": "GET", "path": path, "params": params, "payload": None, "response": copy.deepcopy(payload)})
            return copy.deepcopy(payload)

        def post(self, path, payload=None):
            response = data[keys[path]]
            calls.append({"method": "POST", "path": path, "params": None, "payload": payload, "response": copy.deepcopy(response)})
            return copy.deepcopy(response)

    result = {"name": name, "window": window}
    with patch.object(lnd, "LndRestClient", Client):
        try:
            snapshot = lnd.LndAdapter().fetch_node_snapshot({}, {"kind": "lnd"}, window_days=window)
            result["expected"] = {
                "channels": [{"id": c.id, "peer_alias": c.peer_alias, "capacity_sat": c.capacity_sat, "earned_routing_sat": c.earned_routing_sat} for c in snapshot.channels],
                "routing": asdict(snapshot.routing) if snapshot.routing else None,
            }
        except (ValueError, TypeError, OverflowError) as error:
            result["error"] = type(error).__name__
    result["calls"] = calls
    return result


def render():
    return json.dumps([evaluate(*case) for case in cases()], ensure_ascii=True, indent=2) + "\n"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    rendered = render()
    if args.check:
        if OUTPUT.read_text() != rendered:
            raise SystemExit("Lightning acquisition vectors differ from Python")
    else:
        OUTPUT.parent.mkdir(parents=True, exist_ok=True)
        OUTPUT.write_text(rendered)
    print(f"Lightning acquisition: {len(json.loads(rendered))} reference cases")


if __name__ == "__main__":
    main()
