from dataclasses import replace

import pytest

from kassiber.core.accounting import artifacts, sources
from kassiber.errors import AppError
from tests.test_accounting_sources import sourced  # noqa: F401
from tests.test_accounting_evidence import accounting_db  # noqa: F401


@pytest.fixture
def captured(sourced):
    artifacts.ensure_schema(sourced)
    snapshot = sources.capture_sources(sourced, 'p')
    return artifacts.CapturedCalculation(
        profile_id='p', source_snapshot_id=snapshot['id'], source_digest=snapshot['input_digest'],
        dependency_revision='a' * 40, adapter_version='synthetic-retention-test-v1',
        cutoff_exclusive_utc='2027-01-01T00:00:00Z', calculation_timezone='Europe/Vienna',
        policy=snapshot['snapshot']['calculation_policy'],
        inputs=dict(finalized_projection={}, prepared_transactions=[], source_event_map={}, basis_overrides={}),
        assets=[], blockers=[])


def test_capture_retains_exact_inputs_and_is_not_claimed_replayed(sourced, captured):
    # These synthetic values exercise retention, not a claim about tax rules.
    value = replace(captured, inputs={**captured.inputs, 'basis_overrides': {'quantity_msat': 2**53+1, 'basis_exact': '0.00000000001'}})
    result = artifacts.retain_calculation(sourced, 'p', capture=value)
    assert result['capture']['inputs']['basis_overrides']['quantity_msat'] == 2**53+1
    assert result['verification']['calculation_replay'] == 'not_performed'
    assert artifacts.retain_calculation(sourced, 'p', capture=value)['id'] == result['id']
    with pytest.raises(AppError):
        artifacts.get_calculation(sourced, 'other', result['id'])
    with pytest.raises(Exception, match='accounting_calculation_retained'):
        sourced.execute('INSERT OR REPLACE INTO gl_calculation_artifacts SELECT * FROM gl_calculation_artifacts')
    sourced.rollback()
    assert sourced.execute('SELECT count(*) FROM gl_calculation_artifacts').fetchone()[0] == 0


@pytest.mark.parametrize('bad', [float('nan'), 0.1, b'secret', object()])
def test_captures_reject_floats_and_runtime_objects(sourced, captured, bad):
    value = replace(captured, inputs={**captured.inputs, 'basis_overrides': {'value': bad}})
    with pytest.raises(AppError):
        artifacts.retain_calculation(sourced, 'p', capture=value)
    assert sourced.execute('SELECT count(*) FROM gl_calculation_artifacts').fetchone()[0] == 0


def test_source_changes_block_new_capture_and_proposal_use_but_not_history(sourced, captured):
    saved = artifacts.retain_calculation(sourced, 'p', capture=captured)
    sourced.execute("UPDATE external_documents SET fiat_value_exact='4.00' WHERE id='invoice'")
    with pytest.raises(AppError) as error:
        artifacts.retain_calculation(sourced, 'p', capture=replace(captured, adapter_version='new-run'))
    assert error.value.code == 'accounting_source_stale'
    assert artifacts.get_calculation(sourced, 'p', saved['id'])['payload_digest'] == saved['payload_digest']
    with pytest.raises(AppError) as error:
        artifacts.require_calculation_current(sourced, 'p', saved['id'])
    assert error.value.code == 'accounting_source_stale'


def test_capture_requires_scoped_complete_typed_contract(sourced, captured):
    for invalid in (captured.__dict__, replace(captured, profile_id='other'),
                    replace(captured, dependency_revision='latest'), replace(captured, inputs={}),
                    replace(captured, cutoff_exclusive_utc='2026-12-31'),
                    replace(captured, calculation_timezone='not-a-zone'),
                    replace(captured, policy={'gains_algorithm': 'Invented'})):
        with pytest.raises(AppError):
            artifacts.retain_calculation(sourced, 'p', capture=invalid)


def test_individual_gain_arithmetic_and_fragment_identity(sourced, captured):
    fragment = dict(row_id='event:lot:override-200', event_id='sale', lot_id='lot', quantity_msat=1,
                    basis_exact='0.00000000002', proceeds_exact='0.00000000005', gain_exact='0.00000000003',
                    unit_basis_override_exact='2', transaction_type='SELL')
    asset = dict(asset='BTC', acquisitions=[], gain_losses=[fragment], open_positions=[], custody_balances=[], transfers=[])
    saved = artifacts.retain_calculation(sourced, 'p', capture=replace(captured, assets=[asset]))
    assert saved['capture']['assets'][0]['gain_losses'][0]['gain_exact'] == '0.00000000003'
    for fragments in ([fragment, fragment], [{**fragment, 'gain_exact': '0.01'}], [{**fragment, 'quantity_msat': True}]):
        with pytest.raises(AppError):
            artifacts.retain_calculation(sourced, 'p', capture=replace(captured, assets=[{**asset, 'gain_losses': fragments}]))


def test_blocked_capture_is_retained_but_not_postable(sourced, captured):
    saved = artifacts.retain_calculation(sourced, 'p', capture=replace(captured, blockers=[{'code': 'missing_basis'}]))
    with pytest.raises(AppError) as error:
        artifacts.require_calculation_current(sourced, 'p', saved['id'])
    assert error.value.code == 'accounting_calculation_blocked'
