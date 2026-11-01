#!/usr/bin/env python3
"""Freeze synthetic SQL/byte vectors from the live loader oracle tests.

Use TMPDIR under the worktree's target directory. No owner books are read and
no production data recording hook is installed.
"""
import argparse
import json
from pathlib import Path
import sys
import tempfile
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core.custody_snapshot import load_custody_snapshot
from kassiber.db import open_db
from tests import test_rust_custody_loader as cases
from tests.test_rp2_ownership_transfers import OwnershipDeriverHandlerTest
from tests.rust_port.custody_loader_oracle import exception_boundaries

DEST = ROOT / "crates/journals/tests/data/loader.json"


def vectors():
    result = []
    current_name = None

    def record(conn):
        profile = dict(cases._profile(conn))
        snapshot = load_custody_snapshot(conn, profile, as_of=cases.AS_OF)
        assert snapshot["transactions"], f"{current_name}: fixture has no admitted SQL observations"
        if current_name.startswith(("owned-", "retired-", "receipts-")):
            assert snapshot["owned_index"] is not None, f"{current_name}: ownership branch inactive"
        if current_name.startswith("receipts-"):
            assert snapshot["receipt_sources"], f"{current_name}: receipt branch inactive"
        result.append({"name": current_name, "sql": "\n".join(conn.iterdump()),
                       "profile": profile, "as_of": cases.AS_OF,
                       "expected": json.dumps(snapshot, ensure_ascii=True, allow_nan=True)})

    original = cases.check
    cases.check = record
    try:
        with tempfile.TemporaryDirectory(prefix="custody-loader-vectors-") as directory, patch(
            "kassiber.core.chain_observer.provenance.now_iso", return_value=cases.AS_OF
        ):
            source = open_db(Path(directory))
            helper = OwnershipDeriverHandlerTest()
            helper._seed(source)
            selected = [
                *[("numeric-revision-" + str(i), cases.test_loader_preserves_numeric_revision_bindings, (value,))
                  for i, value in enumerate((7, 1.5, 9007199254740993, float("inf"), float("-inf")))],
                *[("stale-confirmation-" + stale, cases.test_loader_skips_confirmation_quantity_when_authority_is_stale, (stale,))
                  for stale in ("version", "graph")],
                ("structural-columns", cases.test_loader_covers_every_structural_table_column_and_filter, ()),
                ("legacy-rates", cases.test_loader_supports_missing_network_table_and_legacy_rate_columns, ()),
                *[("closed-confirmation-" + str(i), cases.test_loader_drops_closed_confirmation_from_safe_rows, (storage,))
                  for i, storage in enumerate(("NULL", "7", "1.5", "'physical legacy value'", "x'ff00'"))],
                ("retired-policies", cases.test_loader_preserves_retired_policies_warnings_and_core_outpoints, ()),
                *[("owned-" + str(descriptor), cases.test_loader_owns_inventory_addresses_and_descriptors, (descriptor,)) for descriptor in (False, True)],
                *[("owned-cache-" + str(reverse), cases.test_loader_descriptor_cache_separates_policies_and_wallet_facts, (reverse,)) for reverse in (False, True)],
                *[("owned-adversarial-" + str(i), cases.test_loader_adversarial_descriptor_fields, (overrides,))
                  for i, overrides in enumerate(cases.DESCRIPTOR_CASES)],
                *[("receipts-" + str(descriptor), cases.test_loader_reads_receipt_sources, (descriptor,)) for descriptor in (False, True)],
                *[("receipts-identity-" + str(escaped), cases.test_loader_receipt_identity_from_graph_without_external_txid, (escaped,)) for escaped in (False, True)],
                *[("receipts-counts-" + str(i), cases.test_loader_receipt_index_preserves_sources_counts_and_profile_scope, (deprecated,))
                  for i, deprecated in enumerate((False, True, "yes"))],
                *[("receipts-label-" + str(i), cases.test_loader_inventory_labels_preserve_source_specific_fallbacks, (label,))
                  for i, label in enumerate((None, ""))],
                *[("inventory-fallback-" + str(i), cases.test_loader_receipt_inventory_script_fallback, args) for i, args in enumerate((
                    (None, {}, None, "invalid", 0),
                    (None, {"script_pubkey": cases.SCRIPT_A}, None, False, 1),
                    ("", {}, cases.ADDR_A, False, 1),
                    (" \t", {"script_pubkey": cases.SCRIPT_A}, None, False, 1),
                ))],
                *[("receipt-horizon-" + str(i), cases.test_loader_receipt_high_water_precedence, args) for i, args in enumerate((
                    ({"0": -99}, 1, None), ({"0": 5, "00": 0}, 3, None),
                    ({"0": "3", "bad": 999}, 4, None), ({"0": 2.9}, 4, None),
                    ({"0": None}, 1, None), ({"0": -99}, 3, 3),
                ))],
                *[("owned-horizon-" + str(i), cases.test_loader_owned_high_water_and_retired_floor, args) for i, args in enumerate((
                    (-2, None, 502, 502), ("505", None, 0, 505),
                    (False, 501, 0, 503), ("invalid", 501, "invalid", 503),
                    (None, None, 0, 501),
                ))],
                *[("inventory-scope-" + str(i), cases.test_loader_inventory_scope_overrides_wallet_config, (override,))
                  for i, override in enumerate((False, True))],
                ("owned-liquid-alias", cases.test_loader_infers_outpoint_from_liquid_unconfidential_alias, ()),
                *[("owned-legacy-" + case, cases.test_loader_legacy_outpoint_inference_requires_matching_exclusive_evidence, (case,))
                  for case in ("valid", "mismatched", "bool", "ambiguous")],
                *[("receipts-missing-" + missing, cases.test_loader_receipt_address_lists_require_explicit_scope, (missing,))
                  for missing in ("chain", "network")],
                *[("config-" + str(i), cases.test_loader_normalizes_config_and_preserves_storage, (config,))
                  for i, config in enumerate(cases.CONFIGS)],
            ]
            for current_name, function, arguments in selected:
                book = cases.loose_book.__wrapped__((source, helper))
                try:
                    function(next(book), *arguments)
                finally:
                    book.close()
            source.close()
    finally:
        cases.check = original
    return result


def error_vectors():
    from types import SimpleNamespace
    result = []
    current_name = None

    def oracle_run(_command, *, input, **_kwargs):
        request = json.loads(input)
        conn = __import__("sqlite3").connect(":memory:")
        conn.row_factory = __import__("sqlite3").Row
        conn.executescript(request["sql"])
        try:
            load_custody_snapshot(conn, request["profile"], as_of=request["as_of"])
        except Exception as error:
            refusal = {"family": type(error).__name__, "message": str(error),
                       "code": getattr(error, "code", None)}
            boundaries = exception_boundaries(error)
        else:
            raise AssertionError("expected receipt-source refusal")
        finally:
            conn.close()
        result.append({"name": current_name, **request, "expected": refusal})
        return SimpleNamespace(stderr=b"", stdout=json.dumps({"refusal": refusal, "boundaries": boundaries}).encode())

    with tempfile.TemporaryDirectory(prefix="custody-loader-refusals-") as directory, patch(
        "kassiber.core.chain_observer.provenance.now_iso", return_value=cases.AS_OF
    ), patch.object(cases.subprocess, "run", side_effect=oracle_run):
        source = open_db(Path(directory))
        helper = OwnershipDeriverHandlerTest()
        helper._seed(source)
        selected = [
            *[("embedded-scan-overflow-" + str(i), cases.test_loader_propagates_embedded_scan_overflow_before_output, args)
              for i, args in enumerate((place, value) for place in ("current", "historic", "checkpoint")
                                       for value in (float("inf"), float("-inf")))],
            *[("integer-overflow-" + str(i), cases.test_loader_propagates_integer_overflow_before_output, args)
              for i, args in enumerate(
                  (table, column, value)
                  for table, column in (("chain_observation_provenance", "authority_version"),
                                        ("wallet_utxos", "branch_index"), ("wallet_utxos", "address_index"))
                  for value in (float("inf"), float("-inf")))],
            *[("confirmation-quantity-" + str(i), cases.test_loader_refuses_confirmation_quantity_before_output, args)
              for i, args in enumerate((("legacy scalar", 0), (100000000, "legacy scalar"),
                                       ("bad amount", "bad fee"), ("", "bad fee")))],
            *[("utf8-" + str(i), cases.test_loader_utf8_refusal_matches_product_boundaries, args)
              for i, args in enumerate((
                  ("note", b"\xff\x00"), ("note", b"a" * 250 + b"\xff"),
                  ("note", "钱".encode() * 100 + b"\xff"),
                  ("_closed_confirmation_observation", b"\xff"), ("钱包", b"\xff"),
              ))],
            *[(name, cases.test_loader_receipt_error_precedence, (name,))
              for name in ("receipt", "network", "tax", "component")],
            *[("ownership-config-" + str(i), cases.test_loader_ownership_config_scalar_refusals, (config,))
              for i, config in enumerate(("null", "[]", "[1]", "true", "false", "42", "1.5", '"scalar"'))],
            *[("ownership-address-" + str(i), cases.test_loader_ownership_address_scalar_refusals, (addresses,))
              for i, addresses in enumerate((True, False, 7, 1.5))],
            ("ownership-config-order", cases.test_loader_ownership_config_refusals_follow_summary_query_order, ()),
            *[("inventory-outpoint-" + str(i), cases.test_loader_inventory_outpoint_scalar_refusals, (vout,))
              for i, vout in enumerate(("", "legacy scalar", "'quoted'", "\n钱", "9" * 4301 + "x"))],
        ]
        for current_name, function, arguments in selected:
            book = cases.loose_book.__wrapped__((source, helper))
            try:
                function(next(book), *arguments)
            finally:
                book.close()
        source.close()
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    text = json.dumps(vectors(), ensure_ascii=True, indent=2) + "\n"
    if args.check:
        if DEST.read_text() != text:
            raise SystemExit("native custody loader vectors differ from Python")
    else:
        DEST.write_text(text)
    errors = json.dumps(error_vectors(), ensure_ascii=True, indent=2) + "\n"
    error_dest = DEST.with_name("loader_errors.json")
    if args.check:
        if error_dest.read_text() != errors:
            raise SystemExit("native custody loader refusal vectors differ from Python")
    else:
        error_dest.write_text(errors)
    print(f"{len(json.loads(text))} native loader vectors and {len(json.loads(errors))} refusals match Python")


if __name__ == "__main__":
    main()
