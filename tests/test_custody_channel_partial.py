"""Retained lifecycle helper and loan authoring controls."""
import json

from tests.custody_channel_generator import channel_snapshot
from tests.custody_interpretation_stage import python_stage
from tests.custody_engine_compare import assert_same,rust_results


def test_channel_frozen_helpers():
    from tests.custody_channel_vectors import PATH,vectors
    assert json.loads(PATH.read_text()) == vectors()


def test_loan_authoring_directions_remain_separate_from_interpretation():
    from kassiber.core import loans
    from kassiber.errors import AppError
    import pytest
    for role, expected in (("collateral_lock","outbound"),("collateral_release","inbound"),("loan_principal_received","inbound"),("loan_principal_repaid","outbound")):
        loans._require_role(role)
        loans._require_role_direction(role,expected)
        other="inbound" if expected == "outbound" else "outbound"
        with pytest.raises(AppError) as error: loans._require_role_direction(role,other)
        assert error.value.code == "validation"
        assert error.value.details == dict(field="role",role=role,expected_direction=expected,actual_direction=other)
    # Interpretation consumes persisted marks without revalidating role/direction.
    # The two raw controls pin that legacy behavior, never grant loan ownership.
    for case in ("loan_wrong_direction","loan_unknown_role"):
        book=channel_snapshot(0,case=case)
        expected=python_stage(book);actual=rust_results(book)[0]["interpretation"]
        assert_same(expected,actual,"persisted loan mark")
        assert set(expected.non_event_transaction_ids) == {leg["transaction_id"] for leg in book["loan_legs"]}
