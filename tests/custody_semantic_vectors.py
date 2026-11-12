"""CPython 3.11 custody parser vectors; regenerate explicitly with --write."""
import json
from dataclasses import asdict
from pathlib import Path
import sys

from kassiber.core.custody_evidence import canonical_event_key
from kassiber.core.onchain import parse_ownership_tx

PATH = Path(__file__).parent / "fixtures/custody_semantics.json"


def vectors():
    rows = []
    base = {"id": "fallback", "wallet_id": "wallet", "asset": "BTC", "wallet_kind": "descriptor", "config_json": '{}'}
    for name, raw, changes in [
        ("split_hex_pair", {"txid": "a b" + "c" * 59 + " d"}, {}),
        ("new_case_mapping_key", {"t\ua7dcxid": "ab" * 32}, {}),
        ("new_case_mapping_native", {"txid": "\u1c89"}, {}),
        ("unicode_version_key", {"t\u1c89xid": "ab" * 32}, {}),
        ("combining_key", {"tͅxid": "ab" * 32}, {}),
        ("python_strip", {"txid": "ab" * 32 + "\x1f"}, {}),
        ("false_chain", {"chain": False, "txid": "ab" * 32}, {"wallet_kind": "lnd", "config_json": '{"chain":"liquid"}'}),
        ("zero_chain", {"chain": 0, "txid": "ab" * 32}, {"wallet_kind": "lnd"}),
        ("lowercase_network_error", {"network": "Testnet4"}, {}),
        ("contextual_lowercase_network_error", {"network": "AΣ"}, {}),
        ("contextual_lowercase_ignorable", {"network": "AΣ\u0345"}, {}),
        ("contextual_lowercase_following", {"network": "AΣ'A"}, {}),
        ("contextual_lowercase_preceding", {"network": "A\u0345Σ"}, {}),
        ("contextual_lowercase_uncased", {"network": "Σ"}, {}),
    ]:
        row = {**base, **changes, "raw_json": json.dumps(raw)}
        try:
            output = {"key": asdict(canonical_event_key(row))}
        except ValueError as exc:
            output = {"error": str(exc)}
        rows.append({"case": name, "row": row, **output})
    for side, field, values in [("vout", "n", [None, "", False, 0]), ("vin", "vout", [None, "", [], False, 0])]:
        for value in values:
            raw = {"chain": "bitcoin", "network": "main", "txid": "ab" * 32, "vin": [{"txid": "cd" * 32, "vout": 1}], "vout": [{"n": 0}, {"n": 1}]}
            raw[side][-1][field] = value
            rows.append({"case": f"{side}_{value!r}", "raw": raw, "graph": parse_ownership_tx(raw)})
    from kassiber.transfers import onchain_transfer_scope
    for name, key in [("scope_combining_key", "tͅxid"), ("scope_unicode_version_key", "t\u1c89xid")]:
        row = {**base, "external_id": "ab" * 32, "raw_json": json.dumps({key: "ab" * 32})}
        scope = onchain_transfer_scope(row)
        rows.append({"case": name, "scope_row": row, "onchain_scope": list(scope) if scope is not None else None})
    from kassiber.core.book_network import _evidence
    for name, row in [
        ("lightning_alias", {"raw_json": {"chain": "core_lightning", "network": "regtest"}}),
        ("ln_is_not_book_evidence", {"raw_json": {"chain": "ln", "network": "main"}}),
        ("raw_config_conflict", {"raw_json": {"network": "main"}, "config_json": {"network": "regtest"}}),
        ("config_precedence", {"config_json": {"network": "main"}, "wallet_config_json": {"network": "regtest"}}),
        ("importer_rail", {"raw_json": {"source": "bullbitcoin_wallet_csv", "network": "lightning"}}),
        ("importer_rail_whitespace", {"raw_json": {"source": "bullbitcoin_wallet_csv", "network": " lightning "}}),
        ("coinbase_network_object", {"wallet_kind": "coinbase", "raw_json": {"network": {"hash": "abcd"}}}),
        ("instance_conflict", {"chain_instance_id": "a", "raw_json": {"chain_instance_id": "b", "network": "regtest"}}),
        ("row_scope_conflict", {"chain": "bitcoin", "network": "regtest", "raw_json": {"network": "main"}}),
        ("liquid_asset_conflict", {"asset": "BTC", "raw_json": {"chain": "liquid"}}),
        ("binance_btc_rail", {"wallet_kind": "binance", "asset": "BTC", "raw_json": {"network": "BTC"}}),
        ("binance_btc_rail_text", {"wallet_kind": "binance", "asset": "BTC", "raw_json": '{"network": "BTC"}'}),
        ("binance_lightning_rail", {"wallet_kind": "binance", "asset": "BTC", "raw_json": {"network": "LIGHTNING"}}),
        ("binance_token_rail", {"wallet_kind": "binance", "asset": "BTC", "raw_json": {"network": "BSC"}}),
        ("binance_lowercase_rail", {"wallet_kind": "binance", "asset": "BTC", "raw_json": {"network": "btc"}}),
        ("binance_rail_on_liquid", {"wallet_kind": "binance", "asset": "LBTC", "raw_json": {"network": "BTC"}}),
        ("binance_rail_without_asset", {"wallet_kind": "binance", "raw_json": {"network": "BTC"}}),
        ("binance_rail_regtest_config", {"wallet_kind": "binance", "asset": "BTC", "raw_json": {"network": "BTC"}, "wallet_config_json": {"network": "regtest"}}),
        ("btc_rail_outside_binance", {"wallet_kind": "custom", "asset": "BTC", "raw_json": {"network": "BTC"}}),
    ]:
        rows.append({"case": name, "network_row": row, "network_evidence": list(_evidence(row))})
    return rows


if __name__ == "__main__":
    text = json.dumps(vectors(), indent=2, ensure_ascii=True) + "\n"
    if "--write" in sys.argv:
        PATH.write_text(text)
    else:
        assert PATH.read_text() == text
