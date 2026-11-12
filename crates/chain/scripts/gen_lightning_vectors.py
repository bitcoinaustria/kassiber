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
from kassiber.core.lightning.types import snapshot_to_dict, snapshot_to_dict_for_ai

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
    values = [None, "", "bad", [], {}, True, 1, -1, "１２３", "1_001", 2**100,
              float("inf"), float("nan")]
    for resource, key in [("info", "best_header_timestamp"), ("info", "num_peers"),
                          ("info", "block_height"), ("onchain", "confirmed_balance"),
                          ("channel_balance", "balance"), ("channel_balance", "remote_balance")]:
        for index, amount in enumerate(values):
            data = base()
            data[resource][key] = amount
            yield f"ancillary-{resource}-{key}-{index}", data, 30
    for side in ["local_balance", "remote_balance"]:
        for index, amount in enumerate(values):
            data = base()
            data["channel_balance"][side] = {"sat": amount}
            yield f"ancillary-nested-{side}-{index}", data, 30
    for resource in ["open", "closed"]:
        for key in ["capacity", "local_balance", "remote_balance", "pending_htlcs",
                    "close_time", "close_timestamp"]:
            for index, amount in enumerate(values + [253402300799, 253402300800]):
                data = base()
                data[resource]["channels"][0][key] = amount
                yield f"ancillary-{resource}-{key}-{index}", data, 30
    for key in ["base_fee_msat", "fee_per_mil"]:
        for index, amount in enumerate(values):
            data = base()
            data["fees"]["channel_fees"] = [{"chan_id": "1", key: amount}]
            yield f"ancillary-fees-{key}-{index}", data, 30
    for key in ["amt_in_msat", "amt_out_msat", "amt_in", "amt_out", "timestamp"]:
        for index, amount in enumerate(values + [253402300799, 253402300800]):
            data = base()
            data["forwards"]["forwarding_events"][0][key] = amount
            yield f"ancillary-forwards-{key}-{index}", data, 30
    for resource, key in [("info", "chains"), ("fees", "channel_fees"),
                          ("open", "channels"), ("closed", "channels"),
                          ("forwards", "forwarding_events"), ("payments", "payments"),
                          ("invoices", "invoices")]:
        for index, collection in enumerate([None, False, 0, True, 1, 1.5, "text", {"x": []}, [None, "x", {}]]):
            data = base()
            data[resource][key] = collection
            yield f"collection-{resource}-{index}", data, 30
    for resource in ["open", "closed"]:
        data = base()
        data[resource]["channels"][0].update(chan_id="1\n", channel_point="ab" * 32 + ":0\n")
        yield f"identifier-terminal-newline-{resource}", data, 30
    # Node-snapshot fields beyond the profitability projection.
    data = base()
    data["info"].update(identity_pubkey="operator-node", version="0.18.0-beta", alias=7, num_peers="4", block_height="900000")
    data["onchain"]["confirmed_balance"] = "21"
    data["channel_balance"] = {"local_balance": {"sat": "8"}, "remote_balance": {"sat": 9}}
    yield "snapshot-info-and-balances", data, 30
    for index, close_type in enumerate([None, "", "COOPERATIVE_CLOSE", "LOCAL_FORCE_CLOSE", "breach_close", "ABANDONED", 7, "Force"]):
        data = base()
        data["closed"]["channels"][0].update(close_type=close_type, close_time=NOW, initiator="true", private=False, remote_pubkey={"raw": 1})
        yield f"snapshot-close-type-{index}", data, 30
    for index, (active, initiator) in enumerate([(True, True), ("TRUE", "1"), ("no", "no"), (1, 0), (None, None)]):
        data = base()
        data["open"]["channels"][1].update(active=active, initiator=initiator, remote_pubkey=12345)
        yield f"snapshot-active-initiator-{index}", data, 30
    data = base()
    data["fees"]["channel_fees"] = [{"chan_id": 2, "base_fee_msat": "1000", "fee_per_mil": 0}, {"channel_id": "1", "base_fee_msat": 0, "fee_per_mil": "250"},
                                    {"chan_id": "2", "channel_id": "x", "base_fee_msat": 7, "fee_per_mil": 8}, {"base_fee_msat": 9}]
    yield "snapshot-fee-lookup", data, 30
    for index, pending in enumerate([[], [1, 2], {"a": 1}, "abc", None]):
        data = base()
        data["open"]["channels"][1]["pending_htlcs"] = pending
        yield f"snapshot-pending-htlcs-{index}", data, 30
    data = base()
    data["forwards"]["forwarding_events"] = [
        {"timestamp": NOW, "timestamp_ns": str(NOW) + "000000001", "chan_id_in": 2, "chan_id_out": "1", "peer_alias_in": "In", "amt_in": "3", "amt_out_msat": "2500", "fee": "1"},
        {"timestamp": 0, "chan_id_out": "", "amt_in_msat": "-5", "fee_msat": "0", "fee": 0},
        {"timestamp": 1.5, "peer_alias_out": "雪", "chan_id_in": None},
    ]
    yield "snapshot-forward-mapping", data, 30
    data = base()
    data["invoices"]["invoices"] = [{"settled": "TRUE"}, {"state": "settled"}, {"state": "EXPIRED"}, {"state": "OPEN"}, {"settled": False, "state": "Expired"}, None]
    data["payments"]["payments"] = [{"creation_date": NOW, "status": s} for s in ["SUCCEEDED", "success", "COMPLETE", "FAILED", "failure", "IN_FLIGHT", None]]
    yield "snapshot-invoice-and-payment-tallies", data, 30
    # Paging (8f): later pages, short pages behind an advancing cursor,
    # duplicates across pages, and stuck cursors that refuse.
    for resource, key, row in [
        ("forwards", "forwarding_events", {"timestamp": NOW, "chan_id_out": "1", "fee_msat": "3000"}),
        ("payments", "payments", {"creation_date": NOW, "status": "SUCCEEDED", "fee_msat": "5000", "payment_index": "9"}),
        ("invoices", "invoices", {"add_index": "9", "r_hash": SECRET}),
    ]:
        data = base()
        data["pages"] = {resource: [{key: [row]}, {key: [row, dict(row, fee_msat="7000")]}]}
        yield f"paging-{resource}-later-pages", data, 30
        for cursor in [None, 0, "0", "", "bad"]:
            data = base()
            data["pages"] = {resource: [{key: [row], "cursor": cursor}]}
            yield f"paging-{resource}-stuck-{cursor!r}", data, 30
        data = base()
        data["pages"] = {resource: [{key: [row]}, {key: [row], "cursor": 1}]}
        yield f"paging-{resource}-cursor-goes-back", data, 30
        data = base()
        data["pages"] = {resource: [{key: [None, "x"]}]}
        yield f"paging-{resource}-junk-only-page-ends", data, 30


def evaluate(name, data, window):
    calls = []
    keys = {
        "/v1/getinfo": "info", "/v1/balance/blockchain": "onchain",
        "/v1/balance/channels": "channel_balance", "/v1/fees": "fees",
        "/v1/channels": "open", "/v1/channels/closed": "closed",
        "/v1/switch": "forwards", "/v1/payments": "payments", "/v1/invoices": "invoices",
    }

    def paged(path, offset):
        """Honor `index_offset`: page n answers its cursor n + 1, then empty."""
        resource = keys[path]
        cursor_key = "last_offset_index" if path == "/v1/switch" else "last_index_offset"
        pages = data.get("pages", {}).get(resource)
        if pages is None:
            pages = [copy.deepcopy(data[resource])]
            if isinstance(pages[0], dict):
                pages[0].setdefault(cursor_key, 1)
        else:
            pages = [{**{k: v for k, v in page.items() if k != "cursor"},
                      cursor_key: page.get("cursor", number + 1)}
                     for number, page in enumerate(copy.deepcopy(pages))]
        if offset < len(pages):
            return pages[offset]
        rows_key = {"forwards": "forwarding_events", "payments": "payments", "invoices": "invoices"}[resource]
        return {rows_key: []}

    class Client:
        def __init__(self, backend):
            pass

        def get(self, path, *, params=None):
            if path.startswith("/v1/graph/node/"):
                payload = {"node": {"alias": data["aliases"].get(path.rsplit("/", 1)[1], "")}}
            elif params and "index_offset" in params:
                payload = paged(path, int(params["index_offset"]))
            else:
                payload = data[keys[path]]
            calls.append({"method": "GET", "path": path, "params": params, "payload": None, "response": copy.deepcopy(payload)})
            return copy.deepcopy(payload)

        def post(self, path, payload=None):
            response = paged(path, int(payload["index_offset"]))
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
            result["snapshot"] = snapshot_to_dict(snapshot)
            result["ai_snapshot"] = snapshot_to_dict_for_ai(snapshot)
        except (ValueError, TypeError, OverflowError) as error:
            result["error"] = type(error).__name__
        except lnd.AppError as error:
            result["error"] = f"AppError:{error.code}"
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
