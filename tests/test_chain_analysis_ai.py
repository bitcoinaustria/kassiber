import json
from unittest.mock import patch
import uuid

import pytest

from kassiber.ai.tools import redact_ai_tool_result
from kassiber.core.chain_analysis import run_analysis, run_entropy
from kassiber.core.chain_analysis.index import tx_node_id
from kassiber.core.chain_analysis_ai import decode_ai_args, project_ai_result
from kassiber.errors import AppError

from .test_chain_analysis import add_tx, connection, txid


@pytest.fixture
def book():
    conn = connection()
    add_tx(conn, 1, outputs=(600, 400))
    add_tx(conn, 2, [(1, 0)], outputs=(500,))
    yield conn
    conn.close()


def test_remote_projection_preserves_topology_amounts_without_chain_identifiers(book):
    full = run_analysis(book, "p", {"mode": "trace", "subject": txid(1), "depth": 10})
    projected = redact_ai_tool_result(project_ai_result(book, "p", full))
    encoded = json.dumps(projected)
    assert txid(1) not in encoded and txid(2) not in encoded
    assert "bitcoin:main:out:" not in encoded and "bitcoin:main:tx:" not in encoded
    assert all(key not in encoded for key in ('"address"', '"outpoint"', '"txid"', '"scriptpubkey"'))
    assert projected["summary"] == full["summary"]
    assert projected["snapshot_id"] == full["snapshot_id"]
    assert projected["ai_reference_scope"] == "current_process_and_book"
    nodes = {row["id"]: row for row in projected["nodes"]}
    assert nodes and all(ident.startswith("ca-ref:") for ident in nodes)
    assert all(edge["source"] in nodes and edge["target"] in nodes for edge in projected["edges"])
    assert sorted(row["amount_msat"] for row in projected["nodes"] if row.get("amount_msat")) == sorted(row["amount_msat"] for row in full["nodes"] if row.get("amount_msat"))
    assert full["query"]["subject"] == txid(1)  # no mutation of local result


def test_opaque_subject_roundtrip_queries_and_nested_case_recipe(book):
    full = run_analysis(book, "p", {"mode": "trace", "subject": txid(1)})
    projected = project_ai_result(book, "p", full)
    args = {"title": "My investigation", "query": projected["query"], "expected_snapshot_id": projected["snapshot_id"]}
    decoded = decode_ai_args(book, "p", args)
    assert decoded["query"] == full["query"]
    assert decoded["expected_snapshot_id"] == full["snapshot_id"]
    again = run_analysis(book, "p", decoded["query"])
    assert again["snapshot_id"] == full["snapshot_id"]
    assert project_ai_result(book, "p", again)["nodes"] == projected["nodes"]


def test_graph_node_reference_is_valid_physical_label_subject(book):
    full = run_analysis(book, "p", {"mode": "trace", "subject": txid(1)})
    projected = project_ai_result(book, "p", full)
    canonical = tx_node_id("bitcoin", "main", txid(1))
    position = next(i for i, node in enumerate(full["nodes"]) if node["id"] == canonical)
    args = {"subject": projected["nodes"][position]["id"], "chain": "bitcoin", "network": "main", "label": "Exchange", "source": "My statement", "category": "exchange", "confidence": "user_confirmed"}
    assert decode_ai_args(book, "p", args)["subject"] == canonical


def test_process_restart_wrong_book_wrong_profile_and_stale_refs_fail_closed(book):
    projected = project_ai_result(book, "p", run_analysis(book, "p", {"mode": "trace", "subject": txid(1)}))
    args = {"subject": projected["query"]["subject"]}
    with patch("kassiber.core.chain_analysis_ai._PROCESS_KEY", b"new-process-key"), pytest.raises(AppError) as caught:
        decode_ai_args(book, "p", args)
    assert caught.value.code == "chain_analysis_reference_stale"
    with pytest.raises(AppError):
        decode_ai_args(book, "foreign-profile", args)
    other = connection()
    try:
        add_tx(other, 1)
        with pytest.raises(AppError):
            decode_ai_args(other, "p", args)
    finally:
        other.close()
    book.execute("DELETE FROM transactions")
    with pytest.raises(AppError):
        decode_ai_args(book, "p", args)


def test_only_subject_and_target_are_decoded_and_raw_arguments_need_no_scan(book):
    with patch("kassiber.core.chain_analysis_ai.build_index", side_effect=AssertionError("unexpected scan")):
        args = {"subject": txid(1), "title": "ca-ref:unknown", "query": {"target": txid(2)}}
        assert decode_ai_args(book, "p", args) == args
    projected = project_ai_result(book, "p", {"subject": txid(1), "target": txid(2)})
    args = {**projected, "label": projected["subject"], "source": projected["target"]}
    decoded = decode_ai_args(book, "p", args)
    assert decoded["subject"] == txid(1) and decoded["target"] == txid(2)
    assert decoded["label"] == args["label"] and decoded["source"] == args["source"]


def test_unknown_label_subject_remains_resolvable_without_inventing_chain_observation(book):
    unknown = txid(999)
    label_id = str(uuid.uuid4())
    book.execute("INSERT INTO chain_analysis_labels VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)", (label_id, "p", "bitcoin", "main", unknown, "Private account", "exchange", "Private statement", "user_confirmed", 0, 1, 0, "2026-09-01T12:00:00Z", "2026-09-01T12:00:00Z"))
    projected = project_ai_result(book, "p", {"id": label_id, "subject": unknown, "label": "Private account", "source": "Private statement", "category": "exchange", "confidence": "user_confirmed", "revision": 1})
    assert projected["id"] == label_id
    assert "label" not in projected and "source" not in projected
    assert decode_ai_args(book, "p", {"subject": projected["subject"]})["subject"] == unknown
    assert projected["category"] == "exchange"


def test_case_handles_stay_usable_but_stored_titles_sources_and_identifiers_are_removed(book):
    case_id, label_id = str(uuid.uuid4()), str(uuid.uuid4())
    secret = "Alice private wallet and account 123"
    tx = tx_node_id("bitcoin", "main", txid(1))
    value = {"id": case_id, "snapshot_id": "b" * 64, "title": secret, "query": {"subject": tx},
             "result": {"nodes": [{"id": tx, "kind": "transaction", "label": secret, "txid": txid(1), "wallet_ids": ["private-wallet"], "address": "bc1qprivate", "raw_json": {"witness": secret}, "amount_msat": "123000"}],
                        "exposure": [{"claim": {"id": label_id, "label": secret, "source": secret, "category": "exchange", "confidence": "user_confirmed", "revision": 2}, "node_ids": [tx]}],
                        "findings": [{"id": "finding1", "code": "common_input_control", "detail": secret, "evidence": [{"source": "stored_transaction", "reference": "private-record-id"}]}]}}
    projected = project_ai_result(book, "p", value)
    encoded = json.dumps(projected)
    for private in (secret, txid(1), "bc1qprivate", "private-wallet", "private-record-id", "raw_json", "witness"):
        assert private not in encoded
    assert projected["id"] == case_id and projected["snapshot_id"] == "b" * 64
    assert projected["result"]["exposure"][0]["claim"]["id"] == label_id
    assert projected["result"]["findings"][0]["code"] == "common_input_control"
    assert projected["result"]["findings"][0]["evidence"][0]["source"] == "stored_transaction"


def test_entropy_links_remain_consistent_and_no_amounts_or_model_counts_are_lost(book):
    # Use fresh funding; the shared fixture already spends transaction 1:0.
    add_tx(book, 4, outputs=(600, 400))
    add_tx(book, 3, [(4, 0), (4, 1)], outputs=(300, 600))
    full = run_entropy(book, "p", {"subject": txid(3)})
    projected = project_ai_result(book, "p", full)
    assert projected["status"] == full["status"] == "exact"
    assert projected["interpretation_count"] == full["interpretation_count"]
    assert projected["participant_group_counts"] == full["participant_group_counts"]
    assert projected["entropy_bits"] == full["entropy_bits"]
    assert projected["assumptions"] == full["assumptions"]
    assert projected["limitations"] == full["limitations"]
    assert projected["model"] == full["model"]
    assert all(row["input_id"].startswith("ca-ref:") and row["output_id"].startswith("ca-ref:") for row in projected["link_counts"])
    assert txid(4) not in json.dumps(projected) and txid(3) not in json.dumps(projected)
    assert decode_ai_args(book, "p", {"subject": projected["subject"]})["subject"] == full["subject"]


def test_entropy_projection_preserves_unresolved_competing_spend_limits(book):
    add_tx(book, 3, [(1, 0), (1, 1)], outputs=(300, 600))
    full = run_entropy(book, "p", {"subject": txid(3)})
    projected = project_ai_result(book, "p", full)
    assert projected["status"] == full["status"] == "unsupported"
    assert projected["reason"] == "incomplete_transaction"
    assert projected["interpretation_count"] is None
    assert projected["deterministic_links"] == []
    assert txid(1) not in json.dumps(projected) and txid(3) not in json.dumps(projected)


def test_projection_denies_unknown_fields_and_chain_identifiers_hidden_as_codes(book):
    value = {"reason": "a" * 64, "code": "bc1qlongprivateaddress", "unknown_raw_dump": {"address": "bc1qprivate"}, "detail": "secret", "coverage": {"depth": 3, "complete": False}, "source": "https://private.example/a", "snapshot_id": "c" * 64}
    projected = project_ai_result(book, "p", value)
    assert set(projected) == {"coverage", "snapshot_id", "ai_reference_scope", "ai_projection"}
    assert projected["coverage"] == {"depth": 3, "complete": False}


def test_projection_and_reference_decoding_do_not_write(book):
    before = book.total_changes
    result = project_ai_result(book, "p", {"subject": txid(1)})
    decode_ai_args(book, "p", {"subject": result["subject"]})
    assert book.total_changes == before
