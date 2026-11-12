"""Reviewed pair residuals keep observed miner fees out of suspense."""

import pytest

from kassiber.cli.handlers import create_transaction_pair
from kassiber.core.custody_components import get_component
from kassiber.core.custody_component_claims import compile_component_quantity_claims
from kassiber.core.custody_evidence import build_canonical_quantity_input, enriched_quantity_rows
from kassiber.core.custody_interpreters import _observations_by_transaction
from kassiber.db import open_db
from tests.test_custody_authored_migration import NOW, _scope, _tx


@pytest.mark.parametrize("principal", [1000, 9007199254740993])
@pytest.mark.parametrize("shortfall", [0, 2, 20])
@pytest.mark.parametrize("inclusive", [False, True])
def test_pair_allocates_separate_observed_fee_once(tmp_path, principal, shortfall, inclusive):
    conn = open_db(tmp_path / "data")
    try:
        _scope(conn)
        _tx(conn, "out", "btc", "outbound", "BTC", principal + (10 if inclusive else 0), NOW, fee=10)
        _tx(conn, "in", "btc2", "inbound", "BTC", principal - shortfall, NOW)
        conn.execute("UPDATE transactions SET amount_includes_fee=? WHERE id='out'", (inclusive,))
        conn.commit()
        pair = create_transaction_pair(conn, "ws", "profile", "out", "in")
        component = get_component(conn, pair["component_id"])
        totals = {role: sum(leg["amount_msat"] for leg in component["legs"] if leg["role"] == role)
                  for role in ("source", "destination", "fee", "suspense")}
        assert totals == {"source": principal + 10, "destination": principal - shortfall, "fee": 10, "suspense": shortfall}
        assert sum(edge["source_amount_msat"] for edge in component["allocations"]) == principal + 10
        assert component["state"] == "active"
        canonical = build_canonical_quantity_input(enriched_quantity_rows(
            [dict(row) for row in conn.execute("SELECT * FROM transactions")]
        ))
        compiled = compile_component_quantity_claims(component, _observations_by_transaction(canonical))
        assert compiled.suspense_msat == shortfall
        if shortfall:
            _tx(conn, "remaining", "btc2", "inbound", "BTC", shortfall, NOW)
            conn.commit()
            completed = create_transaction_pair(conn, "ws", "profile", "out", "remaining")
            component = get_component(conn, completed["component_id"])
            totals = {role: sum(leg["amount_msat"] for leg in component["legs"] if leg["role"] == role)
                      for role in ("source", "destination", "fee", "suspense")}
            assert totals == {"source": principal + 10, "destination": principal, "fee": 10, "suspense": 0}
    finally:
        conn.close()


@pytest.mark.parametrize("observed_fee", [0, 10])
def test_swap_refund_separates_observed_fee_from_reviewed_principal_fee(tmp_path, observed_fee):
    conn = open_db(tmp_path / "data")
    try:
        _scope(conn)
        _tx(conn, "out", "btc", "outbound", "BTC", 1000, NOW, fee=observed_fee)
        _tx(conn, "in", "btc", "inbound", "BTC", 998, NOW)
        conn.commit()
        pair = create_transaction_pair(conn, "ws", "profile", "out", "in", kind="swap-refund", policy="carrying-value")
        component = get_component(conn, pair["component_id"])
        fees = {leg["notes"]: leg["amount_msat"] for leg in component["legs"] if leg["role"] == "fee"}
        assert fees == {"reviewed_residual:network_fee": 2, **({"observed_network_fee": observed_fee} if observed_fee else {})}
        canonical = build_canonical_quantity_input(enriched_quantity_rows(
            [dict(row) for row in conn.execute("SELECT * FROM transactions")]
        ))
        compiled = compile_component_quantity_claims(component, _observations_by_transaction(canonical))
        assert compiled.suspense_msat == 0
        assert sum(claim.source.amount_msat for claim in compiled.claims) == 1000
        assert sum(claim.source.amount_msat for claim in compiled.claims if claim.reason == "reviewed_network_fee") == 2
    finally:
        conn.close()
