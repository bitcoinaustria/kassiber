"""The schema-2 engine boundary on the Python side (plan 20 phase 4, design §5.4).

``kassiber/core/engines/ledger_request.py`` builds the engine request from a
finalized projection, ``ledger_response.py`` reads the response back into the
adapter's shapes, and ``errors.py`` raises what the adapter raised. Here:

- the builder over every stored golden input reproduces the stored request
  (``tax-engine/core/tests/data/ledger_fixtures.json``), or fails with the
  stored outcome itself; the engine answers every stored request with the
  stored response, byte for byte, where production routes the book to it,
  and with ``Unsupported`` elsewhere;
- the reader renders every stored response into the stored ``expected``
  outcome exactly (§3.8);
- the engine's manifest equals the names and tables Python spells (§3.11);
- the stage table reproduces every stored failure of an engine stage,
  including the method gate the multi-asset hook wraps (§3.9);
- the ``detail_kind`` enrichment reproduces every quarantine detail the
  oracle wrote, from the engine's part of it and the named row (§3.7);
- the builder refuses each shape a finalized projection cannot have (LD-5)
  and fails the configuration checks with the oracle's own errors;
- the production class keeps the adapter's raw-row ``TypeError`` and its
  empty-projection fast path.

The configuration checks run the oracle, which needs RP2 (a development
dependency): without it they skip, except under ``KASSIBER_CI``.
"""

from __future__ import annotations

import json
import os
import uuid
from decimal import Decimal
from functools import lru_cache
from importlib.util import find_spec
from pathlib import Path
from typing import Any, Optional
from unittest.mock import patch

import kassiber_tax
import pytest

from kassiber.core import austrian, exit_tax, journal_markers, quarantine_catalog, transaction_kinds
from kassiber.core.custody_tax_projection import FinalizedTaxProjection
from kassiber.core.engines import KassiberTaxEngine, errors, ledger, ledger_request, ledger_response, native_request
from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.errors import AppError
from kassiber.tax_policy import AUSTRIAN_LONG_TERM_DAYS
from tests import tax_engine_compare as compare
from tests import tax_ledger_codec as codec
from tests.tax_ledger_generator import (
    WALLET_REFS,
    direct,
    finalized,
    ledger_history,
    move,
    profile,
    projected,
)
from tests.tax_oracle import projection_range

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "tax-engine" / "core" / "tests" / "data" / "ledger_fixtures.json"


@lru_cache(maxsize=1)
def _document() -> dict[str, Any]:
    return json.loads(FIXTURES.read_text(encoding="utf-8"))


def _case_names() -> list[str]:
    return [case["name"] for case in _document()["cases"]]


def _case(name: str) -> dict[str, Any]:
    return next(case for case in _document()["cases"] if case["name"] == name)


def _canonical(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def _require_oracle() -> None:
    if find_spec("rp2") is not None:
        return
    if os.environ.get("KASSIBER_CI"):
        pytest.fail("RP2 is missing, so the oracle cannot run; CI installs it with the dev dependency group")
    pytest.skip("RP2 (a development dependency) is not installed")


def _build(book_profile: Any, inputs: TaxEngineLedgerInputs) -> tuple[Optional[dict[str, Any]], Optional[BaseException]]:
    try:
        return ledger_request.build_request(book_profile, inputs), None
    except Exception as exc:  # noqa: BLE001 - a builder failure is compared too
        return None, exc


# ---------------------------------------------------------------------------
# Requests


@pytest.mark.parametrize("name", _case_names())
def test_the_builder_reproduces_the_stored_request(name: str) -> None:
    case = _case(name)
    book_profile, inputs = codec.decode_inputs(case["inputs"])
    request, refusal = _build(book_profile, inputs)
    if case["request"] is None:
        # A configuration failure or an LD-5 refusal: production's outcome.
        assert refusal is not None
        assert compare.outcome_document((None, refusal)) == codec.decode(case["expected"])
        return
    assert refusal is None, refusal
    assert _canonical(request) == _canonical(case["request"])


def test_the_engine_reproduces_every_stored_response() -> None:
    cases = [case for case in _document()["cases"] if case["request"] is not None]
    assert len(cases) >= 100
    answered = 0
    for case in cases:
        text = kassiber_tax.ledger(ledger_request.dumps(case["request"]))
        routed = ledger.route(ledger.capabilities(case["request"])) == ledger.V2
        assert (case["response"] is not None) == routed, case["name"]
        if case["response"] is None:
            # Production runs the book on the phase-3 adapter; the engine
            # read the request but does not implement it yet.
            assert json.loads(text)["error"]["stage"] == "unsupported", case["name"]
            continue
        # The engine's own bytes: key order and decimal forms included.
        assert text == ledger_request.dumps(case["response"]), case["name"]
        answered += 1
    assert answered >= 40


def _answered_case_names() -> list[str]:
    return [case["name"] for case in _document()["cases"] if case["response"] is not None]


@pytest.mark.parametrize("name", _answered_case_names())
def test_the_reader_renders_every_stored_response_as_expected(name: str) -> None:
    case = _case(name)
    book_profile, inputs = codec.decode_inputs(case["inputs"])
    try:
        outcome: compare.Outcome = (ledger_response.read_ledger(book_profile, inputs, case["response"]), None)
    except Exception as exc:  # noqa: BLE001 - a failure is the case's outcome
        outcome = (None, exc)
    actual = compare.outcome_document(outcome)
    assert compare.document_differences(codec.decode(case["expected"]), actual) == []


def test_the_builder_sends_facts_not_markers() -> None:
    # One row of each kind of fact; descriptions and marker text stay out.
    rows = [
        projected("buy", "A", "inbound", 100_000, "2024-01-01T00:00:00+01:00", kind="Mining-Reward", rate="1.50", description=None),
        projected(
            "sell", "B", "outbound", 50_000, "2024-02-01T00:00:00Z", fee=1_000, rate=None, fiat_value="NaN",
            description="at_regime=alt sale", at_regime_override="neu", at_category_override="neu_swap",
            taxability_override=0, pricing_quality="coarse_fallback",
        ),
    ]
    request = ledger_request.build_request(profile("at", "fifo", require_coarse_review="yes"), direct(rows))
    assert request["policy"] == {"country": {"kind": "at"}, "method": "fifo", "method_label": "fifo", "require_coarse_review": True}
    assert request["config"] == {"assets": ["BTC"], "exchanges": ["Cold", "Exchange", "Hot", "Liquid"], "holders": ["Default"]}
    buy, sell = request["rows"]
    assert (buy["kind"], buy["text"], buy["rate"], buy["value"], buy["coarse"]) == ("mining_reward", "Mining-Reward", "150E-2", None, False)
    assert buy["ts"]["offset_s"] == 3600 and buy["ts"]["vienna_date"] == "2024-01-01"
    assert (sell["rate"], sell["value"], sell["coarse"], sell["text"]) == (None, "NaN", True, "at_regime=alt sale")
    assert (sell["regime_override"], sell["category_override"], sell["taxability_off"]) == ("neu", "neu_swap", True)
    assert (buy["regime_override"], buy["category_override"], buy["taxability_off"]) == (None, None, False)
    assert set(buy) == {
        "id", "anchor", "asset", "wallet_id", "direction", "ts", "amount_msat", "fee_msat", "rate", "value", "coarse",
        "kind", "text", "regime_override", "category_override", "taxability_off", "privacy_hop", "hold", "chain_order",
    }


def test_the_request_helpers_match_the_v1_copies() -> None:
    # ``native_request`` keeps its own copies while the v1 fixture generator
    # loads it by path; both must read timestamps and decimals alike.
    for value in (
        "2024-01-01T00:00:00Z", "2024-03-31T01:30:00.123456789+02:00", "0999-06-01T12:00:00-05:30",
        "9999-12-31T23:59:59+14:00", "0001-01-01T00:00:00Z", "2024-01-01", "not a date", "2024-02-30T00:00:00Z", None, 20240101,
    ):
        assert ledger_request.timestamp_fields(value) == native_request.timestamp_fields(value), value
    for text in ("0", "-0", "1.50", "1E+400", "-123.456E-7"):
        value = Decimal(text)
        assert ledger_request.encode_decimal(value) == native_request.encode_decimal(value)
        assert ledger_request.decode_decimal(ledger_request.encode_decimal(value)).as_tuple() == value.as_tuple()
    assert ledger_request.decimal_text(None, "", "2.0") == "20E-1"
    assert ledger_request.decimal_text(0.1) == "1E-1"
    assert ledger_request.decimal_text("-Infinity") == "-Infinity"
    assert ledger_request.decimal_text(None, "") is None


# ---------------------------------------------------------------------------
# Manifest drift


def test_the_manifest_matches_pythons_names_and_tables() -> None:
    manifest = ledger.manifest()
    assert manifest["engine_version"] == kassiber_tax.engine_version()
    assert manifest["inbound_kind_types"] == transaction_kinds.INBOUND_KIND_TO_RP2_TYPE
    assert list(manifest["inbound_kind_types"]) == list(transaction_kinds.INBOUND_KIND_TO_RP2_TYPE)
    assert manifest["markers"] == {
        "regime": journal_markers.MARKER_REGIME,
        "regime_basis": journal_markers.MARKER_REGIME_BASIS,
        "pool": journal_markers.MARKER_POOL,
        "swap_link": journal_markers.MARKER_SWAP_LINK,
        "alt_out": journal_markers.MARKER_ALT_OUT,
        "alt_in": journal_markers.MARKER_ALT_IN,
    }
    assert manifest["regime_basis_election"] == journal_markers.REGIME_BASIS_ELECTION
    assert manifest["neu_cutoff_us"] == int(austrian.AT_NEU_CUTOFF.timestamp()) * 1_000_000
    assert manifest["default_pool"] == austrian.AT_DEFAULT_POOL
    assert manifest["at_categories"] == list(austrian.AT_CATEGORY_TO_KENNZAHL)
    assert manifest["at_long_term_days"] == AUSTRIAN_LONG_TERM_DAYS
    assert set(manifest["entry_types"]) == exit_tax.RECOGNIZED_ENTRY_TYPES
    assert len(manifest["entry_types"]) == len(exit_tax.RECOGNIZED_ENTRY_TYPES)
    assert manifest["rail_quarantine_reason"] == ledger.GENERIC_BITCOIN_RAIL_QUARANTINE_REASON
    assert manifest["swap_quarantine_reason"] == austrian.AT_SWAP_QUARANTINE_REASON
    uncatalogued = [reason for reason in manifest["quarantine_reasons"] if reason not in quarantine_catalog._REASONS]
    assert uncatalogued == []
    assert manifest["detail_kinds"] == list(ledger_response.DETAIL_ENRICHMENT)


def test_the_manifest_matches_the_oracles_kind_rules() -> None:
    # The kind tokens and the default type still live in the frozen adapter.
    from types import SimpleNamespace

    from tests.tax_oracle import adapter

    manifest = ledger.manifest()
    assert manifest["income_like_kind_tokens"] == list(adapter._INCOME_LIKE_KIND_TOKENS)
    assert manifest["non_sale_disposal_kind_tokens"] == list(adapter._NON_SALE_DISPOSAL_KIND_TOKENS)
    assert manifest["default_inbound_type"] == adapter._rp2_in_transaction_type(SimpleNamespace(raw_row={"kind": "deposit"}))
    source = Path(adapter.__file__).read_text(encoding="utf-8")
    zero_cost = "kind in {" + ", ".join(json.dumps(kind) for kind in manifest["at_zero_cost_kinds"]) + "}"
    assert zero_cost in source


# ---------------------------------------------------------------------------
# Stages


_ENGINE_CLASSES = {
    "EngineValueError": "ValueError",
    "EngineTypeError": "TypeError",
    "EngineRuntimeError": "RuntimeError",
    "InvalidOperation": "InvalidOperation",
    "DivisionByZero": "DivisionByZero",
    "Overflow": "Overflow",
    "AppError": "AppError",
}
_WRAPPERS = (
    ("compute_multi", "RP2 multi-asset tax calculation failed: "),
    ("validate", "RP2 cross-asset input validation failed: "),
    ("compute", "RP2 tax calculation failed for asset '"),
)


def _staged_error(signature: tuple[Any, ...]) -> Optional[dict[str, Any]]:
    """The engine failure that must raise ``signature``, when an engine stage raised it."""
    name, message, *fields = signature
    if name == "AppError":
        cause = fields[-1]
        for stage, prefix in _WRAPPERS:
            if message.startswith(prefix) and cause is not None:
                error: dict[str, Any] = {"stage": stage, "inner": {"class": _ENGINE_CLASSES[cause[0]], "message": cause[1]}}
                if stage == "compute":
                    error["asset"] = message[len(prefix):].split("'", 1)[0]
                return error
        return None
    if name in _ENGINE_CLASSES:
        return {"stage": "prepare", "inner": {"class": _ENGINE_CLASSES[name], "message": message}}
    return None


def test_the_stage_table_raises_every_stored_engine_failure() -> None:
    stages = set()
    for case in _document()["cases"]:
        expected = codec.decode(case["expected"])
        if "error" not in expected or case["request"] is None:
            continue  # a result, or a failure before the engine
        staged = _staged_error(expected["error"])
        assert staged is not None, (case["name"], expected["error"])
        stages.add(staged["stage"])
        assert compare.error_signature(errors.ledger_error(staged)) == expected["error"], case["name"]
    # The goldens reach every wrapped stage and an unwrapped one.
    assert stages >= {"compute_multi", "validate", "compute", "prepare"}, stages


def test_the_method_gate_is_a_wrapped_multi_asset_failure() -> None:
    error = errors.ledger_error(
        {"stage": "compute_multi", "inner": {"class": "AppError", "message": "Unsupported RP2 accounting method 'bogus'"}}
    )
    assert isinstance(error, AppError) and isinstance(error.__cause__, AppError)
    assert str(error) == "RP2 multi-asset tax calculation failed: Unsupported RP2 accounting method 'bogus'"
    assert (error.code, error.__cause__.code) == ("app_error", "app_error")
    assert error.__suppress_context__
    expected = codec.decode(_case("generic-method-bogus")["expected"])["error"]
    assert compare.error_signature(error) == expected


def test_unwrapped_and_contract_stages() -> None:
    for stage in ("prepare", "classify"):
        error = errors.ledger_error({"stage": stage, "inner": {"class": "ValueError", "message": "m"}})
        assert type(error) is errors.EngineValueError and error.__cause__ is None
    signal = errors.ledger_error({"stage": "prepare", "inner": {"class": "InvalidOperation", "message": "[<class 'decimal.InvalidOperation'>]"}})
    assert type(signal).__name__ == "InvalidOperation"
    for stage, inner_class in (("request", "RequestError"), ("unsupported", "Unsupported")):
        error = errors.ledger_error({"stage": stage, "inner": {"class": inner_class, "message": "m"}})
        assert isinstance(error, errors.EngineContractError) and str(error) == f"{inner_class}: m"
    unknown = errors.ledger_error({"stage": "elsewhere", "inner": {"class": "ValueError", "message": "m"}})
    assert isinstance(unknown, errors.EngineContractError)
    compute = errors.ledger_error({"stage": "compute", "asset": "LBTC", "inner": {"class": "RuntimeError", "message": "m"}})
    assert str(compute) == "RP2 tax calculation failed for asset 'LBTC': m"
    validate = errors.ledger_error({"stage": "validate", "inner": {"class": "ValueError", "message": "m"}})
    assert validate.code == "rp2_input_validation"
    assert set(errors.STAGES) == {"prepare", "validate", "compute_multi", "compute", "classify", "request", "unsupported"}


# ---------------------------------------------------------------------------
# Quarantine details


_ENRICHED_KEYS = {
    "pricing_review": set(ledger_response.PRICING_REVIEW_FIELDS),
    "privacy_hop": {"privacy_boundary", "privacy_hop", "source", "islikelycoinjoin", "required_for"},
    "hold": {"external_id"},
    "partner_leg": set(),
    "plain": set(),
}


def _detail_kind(reason: str, detail: dict[str, Any]) -> str:
    if detail.get("required_for") == "pricing_review":
        return "pricing_review"
    if reason == "privacy_hop_unresolved":
        return "privacy_hop"
    if reason in ("conflicting_spend", "pending_onchain_confirmation"):
        return "hold"
    if detail.get("paired_leg") is True and detail.get("direction") == "transfer" and "transfer_group_blocked" not in reason:
        return "partner_leg"
    return "plain"


def _engine_detail(value: Any) -> Any:
    # What the engine sends for a value the adapter wrote: a float tagged
    # with its canonical decimal.
    if isinstance(value, float):
        return {"float": ledger_request.encode_decimal(Decimal(repr(value)))}
    return value


def test_the_enrichment_table_reproduces_every_stored_quarantine() -> None:
    kinds: dict[str, int] = {}
    floats = 0
    for case in _document()["cases"]:
        expected = codec.decode(case["expected"])
        if "result" not in expected:
            continue
        book_profile, inputs = codec.decode_inputs(case["inputs"])
        projection = inputs.finalized_tax_projection
        rows_by_id = {str(row["id"]): row for row in projection.rows}
        for quarantine in expected["result"]["quarantines"][len(projection.quarantines):]:
            detail = json.loads(quarantine["detail_json"])
            kind = _detail_kind(quarantine["reason"], detail)
            sent = {key: _engine_detail(value) for key, value in detail.items() if key not in _ENRICHED_KEYS[kind]}
            floats += sum(1 for value in sent.values() if isinstance(value, dict))
            candidates = [
                row_id
                for row_id, row in rows_by_id.items()
                if str(row.get("journal_transaction_id") or row["id"]) == quarantine["transaction_id"]
            ]
            rendered = [
                ledger_response.render_quarantine(
                    book_profile,
                    rows_by_id,
                    {"row": row_id, "reason": quarantine["reason"], "detail_kind": kind, "detail": sent},
                )
                for row_id in candidates
            ]
            assert quarantine in rendered, (case["name"], quarantine)
            kinds[kind] = kinds.get(kind, 0) + 1
    assert set(kinds) == set(ledger_response.DETAIL_ENRICHMENT), kinds
    assert floats, "no stored detail carries a float"


def test_partner_legs_get_no_pricing_fields() -> None:
    # One reason, two templates: the partner leg of a coarse-priced move must
    # not be enriched like the priced leg (design §3.7).
    row = projected("leg", "A", "inbound", 1_000, "2024-01-01T00:00:00Z", pricing_quality="coarse_fallback")
    base = {"row": row["id"], "reason": "pricing_review_required"}
    partner = ledger_response.render_quarantine(
        profile("generic", "FIFO"), {row["id"]: row}, {**base, "detail_kind": "partner_leg", "detail": {"paired_leg": True}}
    )
    priced = ledger_response.render_quarantine(
        profile("generic", "FIFO"), {row["id"]: row}, {**base, "detail_kind": "pricing_review", "detail": {"required_for": "pricing_review"}}
    )
    assert json.loads(partner["detail_json"]) == {"paired_leg": True}
    assert json.loads(priced["detail_json"])["pricing_quality"] == "coarse_fallback"
    assert partner["transaction_id"] == "leg"
    with pytest.raises(errors.EngineContractError):
        ledger_response.render_quarantine(
            profile("generic", "FIFO"), {row["id"]: row}, {**base, "detail_kind": "pricing", "detail": {}}
        )
    assert ledger_response.detail_value({"float": "-15E-1", "nested": [{"float": "0E0"}]}) == {"float": "-15E-1", "nested": [0.0]}
    assert ledger_response.detail_value({"float": "-15E-1"}) == -1.5


def test_the_reader_renders_entries_as_the_adapter_wrote_them() -> None:
    # A synthetic response over a small projection: what the reader adds to
    # each entry the engine composed (design §3.8).
    buy = projected("buy", "A", "inbound", 100_000_000_000, "2024-01-01T00:00:00+01:00", rate="40000")
    # A move large enough that its quantities lose digits through float.
    out = projected("m", "A", "outbound", 98_765_432_109_875_543, "2024-02-01T00:00:00Z", side="move-out", fee=1_000)
    inbound = projected("m", "B", "inbound", 98_765_432_109_875_543, "2024-02-01T00:00:00Z", side="move-in")
    inputs = direct([buy, out, inbound], [move(out, inbound, group_id=None)])
    book_profile = profile("at", "fifo")
    lot = {"row": buy["id"], "spot_price": "40000E0", "fiat_fee": "0E0", "crypto_fee": "0E0", "basis": "40000E0", "crypto_in": "1E0"}
    event = {"row": out["id"], "spot_price": "50000E0", "fiat_fee": "5E-4", "crypto_fee": "1E-8"}
    response = {
        "schema_version": 2,
        "ok": True,
        "engine_version": "0.2.0",
        "entries": [
            {"entry_type": "acquisition", "row": buy["id"], "wallet": "Cold", "asset": "BTC", "quantity": "1E0",
             "fiat_value": "40000E0", "unit_cost": "40000E0", "description": "at_regime=neu buy"},
            {"entry_type": "disposal", "row": out["id"], "wallet": "Cold", "asset": "BTC", "quantity": "-1E-8",
             "proceeds": "5E-4", "cost_basis": "4E-4", "gain_loss": "1E-4", "description": "at_regime=neu fee",
             "at_category": "neu_gain", "capital_gains_type": "short",
             "fragments": [{"crypto_amount": "1E-8", "cost_basis": "4E-4", "proceeds": "5E-4", "gain_loss": "1E-4",
                            "unit_basis_override": None, "event": event, "lot": lot}]},
            {"entry_type": "transfer_out", "audit": 0},
            {"entry_type": "transfer_in", "audit": 0},
        ],
        "quarantines": [],
        "intra_audit": [{"pair": 0, "sent": "98765432109876543E-11", "received": "98765432109875543E-11", "fee": "1E-8",
                         "sent_msat": 98_765_432_109_876_543, "received_msat": 98_765_432_109_875_543, "fee_msat": 1_000,
                         "spot_price": None, "at_regime": None,
                         "regime_flows": None, "at_pool": None, "description": "Transfer Cold -> Hot"}],
        "tax_summary": [],
        "holdings": [],
        "carried_rows": [],
    }
    result = ledger_response.read_ledger(book_profile, inputs, response)
    acquisition, disposal, sent, received = result.entries
    keys = ["id", "workspace_id", "profile_id", "transaction_id", "wallet_id", "account_id", "occurred_at", "entry_type",
            "asset", "quantity", "fiat_value", "unit_cost", "cost_basis", "proceeds", "gain_loss", "description"]
    assert list(acquisition) == keys
    assert list(disposal) == [*keys, "at_category", "at_kennzahl", "capital_gains_type", "calculation"]
    assert list(sent) == keys and list(received) == keys
    assert len({entry["id"] for entry in result.entries}) == 4
    assert all(uuid.UUID(entry["id"]).version == 4 for entry in result.entries)
    assert (acquisition["transaction_id"], acquisition["wallet_id"], acquisition["occurred_at"]) == ("buy", "A", "2024-01-01T00:00:00+01:00")
    assert (acquisition["workspace_id"], acquisition["profile_id"], acquisition["account_id"]) == ("ws-1", "profile-1", "acct-1")
    assert acquisition["cost_basis"] is acquisition["proceeds"] is acquisition["gain_loss"] is None
    assert disposal["at_kennzahl"] == austrian.kennzahl_for_disposal_category("neu_gain")
    assert disposal["fiat_value"] == disposal["proceeds"] == Decimal("5E-4")
    assert str(disposal["unit_cost"]) == "0"
    calculation = disposal["calculation"]
    assert list(calculation) == ["schema_version", "engine", "method", "fragments"]
    assert (calculation["schema_version"], calculation["engine"], calculation["method"]) == (1, "kassiber_tax", "fifo")
    [fragment] = calculation["fragments"]
    assert fragment["basis_authority"] == "acquired_lot"
    assert (fragment["quantity_msat"], fragment["quantity_msat_exact"]) == (1_000, "1000")
    assert (fragment["acquisition_basis_exact"], fragment["acquisition_quantity_msat"]) == ("40000", 100_000_000_000)
    # Each fragment names its row's anchor and time, and copies its pricing.
    assert (fragment["lot"]["transaction_id"], fragment["lot"]["occurred_at"]) == ("buy", "2024-01-01T00:00:00+01:00")
    assert fragment["lot"]["pricing"]["fiat_rate_exact"] == "40000"
    assert list(fragment["lot"]["pricing"]) == list(ledger_response.FRAGMENT_PRICING_FIELDS)
    assert (fragment["event"]["engine_event_id"], fragment["event"]["occurred_at"]) == (out["id"], "2024-02-01T00:00:00+00:00")
    assert (fragment["event"]["fiat_fee_exact"], fragment["event"]["crypto_fee_msat_exact"]) == ("0.0005", "1000")
    # The legs take the audit row's float quantities, as ``dec`` reads them
    # (TODO "Keep transfer-leg quantities exact").
    exact_sent, exact_received = Decimal("987654.32109876543"), Decimal("987654.32109875543")
    assert (sent["entry_type"], sent["wallet_id"], sent["quantity"]) == ("transfer_out", "A", -Decimal(str(float(exact_sent))))
    assert (received["entry_type"], received["wallet_id"]) == ("transfer_in", "B")
    assert received["quantity"] == Decimal(str(float(exact_received))) != exact_received
    assert str(sent["quantity"]) == "-987654.3210987655"
    assert sent["description"] == received["description"] == "Transfer Cold -> Hot"
    assert sent["occurred_at"] == received["occurred_at"] == out["occurred_at"]
    assert (sent["transaction_id"], received["transaction_id"]) == ("m", "m")
    assert sent["fiat_value"] == sent["unit_cost"] == Decimal("0")
    # A realized entry without a category has neither key; an unknown wallet
    # fails as the adapter failed.
    plain = {**response["entries"][1], "at_category": None}
    assert "at_kennzahl" not in ledger_response.read_ledger(book_profile, inputs, {**response, "entries": [plain]}).entries[0]
    unknown = {**response["entries"][0], "wallet": "Nowhere"}
    with pytest.raises(AppError, match="RP2 emitted transaction for unknown wallet 'Nowhere'") as raised:
        ledger_response.read_ledger(book_profile, inputs, {**response, "entries": [unknown]})
    assert raised.value.code == "internal"


def test_the_reader_renders_summary_holdings_and_failures() -> None:
    refs = {
        "w1": {"id": "w1", "label": "Cold", "wallet_account_id": "a1", "account_code": "treasury", "account_label": "Treasury"},
        "w2": {"id": "w2", "label": "Hot", "wallet_account_id": "a1", "account_code": "treasury", "account_label": "Treasury"},
    }
    account, wallet = ledger_response.accumulate_holdings(
        [
            {"asset": "BTC", "wallet": "Cold", "quantity": "150E-2", "cost_basis": "30000E0"},
            {"asset": "BTC", "wallet": "Hot", "quantity": "5E-1", "cost_basis": "10000E0"},
            {"asset": "BTC", "wallet": "Nowhere", "quantity": "1E0", "cost_basis": "1E0"},
            {"asset": "LBTC", "wallet": "Cold", "quantity": "1E0", "cost_basis": "1E0"},
        ],
        ledger_response.wallet_refs_by_label(refs),
    )
    assert account == {
        ("a1", "treasury", "Treasury", "BTC"): {"quantity": Decimal("2.00"), "cost_basis": Decimal("40000")},
        ("a1", "treasury", "Treasury", "LBTC"): {"quantity": Decimal("1"), "cost_basis": Decimal("1")},
    }
    assert str(account[("a1", "treasury", "Treasury", "BTC")]["quantity"]) == "2.00"
    assert list(wallet) == [("w1", "Cold", "treasury", "BTC"), ("w2", "Hot", "treasury", "BTC"), ("w1", "Cold", "treasury", "LBTC")]
    summary = ledger_response.render_tax_summary(
        {"year": 2024, "asset": "BTC", "transaction_type": "sell", "long_term": False,
         "quantity": "125E-3", "proceeds": "100E0", "cost_basis": "40E0", "gain_loss": "60E0"}
    )
    assert summary == {
        "year": 2024, "asset": "BTC", "transaction_type": "sell", "capital_gains_type": "short", "quantity": 0.125,
        "quantity_msat": 12_500_000_000, "proceeds": 100.0, "cost_basis": 40.0, "gain_loss": 60.0,
    }
    carried = ledger_response.rows_after_carry([{"id": "r", "pricing_quality": "coarse_fallback"}], [{"row": "r", "value": "100", "rate": "50000"}])
    assert carried["r"]["pricing_quality"] == "exact" and carried["r"]["fiat_rate"] == 50000.0
    with pytest.raises(AppError, match="RP2 tax calculation failed for asset 'BTC': m"):
        ledger_response.raise_for_error(
            {"schema_version": 2, "ok": False, "error": {"stage": "compute", "asset": "BTC", "inner": {"class": "ValueError", "message": "m"}}}
        )
    with pytest.raises(errors.EngineContractError):
        ledger_response.loads(json.dumps({"schema_version": 1, "ok": True}))


# ---------------------------------------------------------------------------
# Builder failures


def _ledger_book(**profile_extra: Any) -> tuple[dict[str, Any], TaxEngineLedgerInputs]:
    book_profile = profile("generic", "FIFO", **profile_extra)
    rows, pairs = ledger_history()
    return book_profile, finalized(book_profile, rows, pairs)


def test_the_builder_names_every_invariant() -> None:
    # Written independently of projection_range; the names and order agree.
    assert ledger_request.INVARIANTS == projection_range.INVARIANTS
    refused = set()
    for case in _document()["cases"]:
        if case["invariant"] is None:
            continue
        book_profile, inputs = codec.decode_inputs(case["inputs"])
        with pytest.raises(errors.EngineContractError) as raised:
            ledger_request.build_request(book_profile, inputs)
        assert raised.value.invariant == case["invariant"]
        assert compare.listed_refusal(book_profile, inputs, (None, raised.value)) == case["invariant"]
        refused.add(case["invariant"])
    assert refused == set(ledger_request.INVARIANTS) - {ledger_request.NON_GLOBAL_POOL_SCOPE}


def test_an_unlisted_refusal_is_not_accepted() -> None:
    book_profile, inputs = _ledger_book()
    invented = errors.EngineContractError("RequestError", "negative_fee: invented", invariant="negative_fee")
    assert compare.listed_refusal(book_profile, inputs, (None, invented)) is None
    assert compare.listed_refusal(book_profile, inputs, (None, ValueError("negative_fee"))) is None


def _configuration_cases() -> list[tuple[str, dict[str, Any], TaxEngineLedgerInputs]]:
    rows, pairs = ledger_history()
    cases = []
    for name, profile_extra in (
        ("missing currency", {"fiat_currency": None}),
        ("unknown currency", {"fiat_currency": "XYZ"}),
        ("negative holding period", {"tax_long_term_days": -1}),
        ("empty profile label", {"label": "  "}),
        ("profile label with a newline", {"label": "Alice\nBob"}),
        ("wallet pool scope", {"cost_basis_pool_scope": "wallet"}),
    ):
        book_profile = profile("generic", "FIFO", **profile_extra)
        cases.append((name, book_profile, finalized(profile("generic", "FIFO"), rows, pairs)))
    for name, wallet, label in (
        ("percent label", "C", "50% cold"),
        ("interpolated label", "C", "a%(x)s"),
        ("delimited label", "C", "Cold, Savings"),
        ("duplicate label after strip", "C", " Cold "),
        ("unused wallet with a delimited label", "L", "Liquid; old"),
    ):
        refs = {key: dict(value) for key, value in WALLET_REFS.items()}
        refs[wallet]["label"] = label
        book_profile = profile("generic", "FIFO")
        cases.append((name, book_profile, finalized(book_profile, rows, pairs, wallet_refs=refs)))
    book_profile = profile("generic", "FIFO")
    delimited_asset = [dict(row, asset="BTC=USD") for row in rows]
    cases.append(("delimited asset", book_profile, finalized(book_profile, delimited_asset, pairs)))
    out_row = projected("cross-out", "A", "outbound", 1_000, "2024-01-01T00:00:00Z", side="move-out")
    in_row = projected("cross-in", "L", "inbound", 1_000, "2024-01-01T00:10:00Z", side="move-in", asset="LBTC")
    cases.append(("move pair across assets", book_profile, direct([out_row, in_row], [move(out_row, in_row)])))
    return cases


@pytest.mark.parametrize("name", [case[0] for case in _configuration_cases()])
def test_the_builder_fails_as_the_oracle_does(name: str) -> None:
    _require_oracle()
    _name, book_profile, inputs = next(case for case in _configuration_cases() if case[0] == name)
    oracle = compare.run_oracle(book_profile, inputs)
    production = compare.run_production(book_profile, inputs)
    _request, refusal = _build(book_profile, inputs)
    assert oracle[1] is not None and refusal is not None
    assert compare.error_signature(refusal) == compare.error_signature(oracle[1])
    assert compare.ledger_differences(oracle, production) == []
    if isinstance(refusal, AppError) and refusal.details:
        assert refusal.code == "validation"


def test_labels_with_ini_delimiters_name_their_field() -> None:
    # Formerly tests/test_rp2_packaging.py: a wallet, profile, or asset label
    # with an INI delimiter fails closed with a validation error naming the
    # field.
    rows, pairs = ledger_history()
    for field, profile_label, wallet_label, asset in (
        ("wallet_label", "Default", "Cold, Savings", "BTC"),
        ("profile_label", "Alice\nBob", "Cold", "BTC"),
        ("asset", "Default", "Cold", "BTC=USD"),
    ):
        refs = {key: dict(value) for key, value in WALLET_REFS.items()}
        refs["C"]["label"] = wallet_label
        book_profile = profile("generic", "FIFO", label=profile_label)
        book_rows = [dict(row, asset=asset) for row in rows]
        inputs = finalized(profile("generic", "FIFO"), book_rows, pairs, wallet_refs=refs)
        with pytest.raises(AppError) as raised:
            ledger_request.build_request(book_profile, inputs)
        assert raised.value.code == "validation"
        assert raised.value.details["field"] == field


# ---------------------------------------------------------------------------
# The production class


def test_raw_rows_fail_with_the_adapters_type_error() -> None:
    _require_oracle()
    book_profile, inputs = _ledger_book()
    raw = TaxEngineLedgerInputs(finalized_tax_projection=list(inputs.finalized_tax_projection.rows), wallet_refs_by_id=WALLET_REFS)  # type: ignore[arg-type]
    production = compare.run_production(book_profile, raw)
    assert isinstance(production[1], TypeError)
    assert compare.ledger_differences(compare.run_oracle(book_profile, raw), production) == []


def test_an_empty_projection_checks_nothing_and_builds_no_request() -> None:
    # A book with no rows returns its projection's quarantines and pairs
    # without running a configuration check or the builder.
    projection = FinalizedTaxProjection(
        rows=(),
        intra_pairs=(),
        cross_asset_pairs=({"pair_id": "p", "out_id": "o", "in_id": "i", "policy": "taxable"},),
        quarantines=({"transaction_id": "t", "reason": "custody_basis_barrier"},),
        selected_move_ids=(),
    )
    refs = {"w": {"id": "w", "label": "Cold, Savings"}}
    inputs = TaxEngineLedgerInputs(finalized_tax_projection=projection, wallet_refs_by_id=refs)
    with patch.object(ledger, "build_request", side_effect=AssertionError("built a request")):
        result = KassiberTaxEngine(profile("generic", "bogus", fiat_currency="XYZ")).build_ledger_state(inputs)
    assert result.quarantines == [{"transaction_id": "t", "reason": "custody_basis_barrier"}]
    assert result.cross_asset_pairs == [{"pair_id": "p", "out_id": "o", "in_id": "i", "policy": "taxable"}]
    assert (result.entries, result.intra_audit, result.tax_summary, result.direct_swap_payouts) == ([], [], [], [])
    assert (result.account_holdings, result.wallet_holdings) == ({}, {})
    assert ledger.classify(profile("at", "fifo"), inputs) == ("at", ledger.NOT_ROUTED)


def test_an_unsupported_country_fails_at_construction() -> None:
    with pytest.raises(ValueError, match="Unsupported tax country 'xx'"):
        KassiberTaxEngine(profile("xx", "FIFO"))
