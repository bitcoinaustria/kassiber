"""Independent comparison-only fusion oracle; never supplies production state."""
import json

from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.core.engines import ledger_request
from tests.custody_fusion_builder import outcome


def python_fusion(snapshot, projection):
    refs = {w['id']: {k: v for k, v in w.items()
                     if k not in {'config_json', 'config_text_mentions_samourai'}}
            for w in snapshot['wallets']}
    inputs = TaxEngineLedgerInputs(finalized_tax_projection=projection, wallet_refs_by_id=refs)
    result = outcome(snapshot['profile'], inputs)
    if result['outcome'] == 'refusal':
        # Re-execute only this pure builder to identify its actual refusing
        # boundary from the traceback, independently of Rust's diagnostic.
        try:
            ledger_request.build_request(snapshot['profile'], inputs)
        except Exception as exc:
            names = []
            tb = exc.__traceback__
            while tb is not None:
                names.append(tb.tb_frame.f_code.co_name)
                tb = tb.tb_next
            stage = 'construction'
            for function, boundary in [('check_configuration', 'configuration'),
                                       ('profile_cost_basis_pool_scope', 'pool_scope'),
                                       ('check_invariants', 'invariants'), ('_holds', 'holds')]:
                if function in names:
                    stage = boundary
                    break
            if isinstance(exc, AssertionError) and str(exc) == 'finalized same-asset move pair crossed assets':
                stage = 'crossed_assets'
            result['error']['stage'] = stage
    elif result['outcome'] == 'request':
        import kassiber_tax
        text = ledger_request.dumps(result['request'])
        result.update(request_text=text, response=json.loads(kassiber_tax.ledger(text)))
    return result


def add_fusion(snapshot, stages):
    if 'finalized_projection' in stages and 'fusion' not in stages:
        stages['fusion'] = python_fusion(snapshot, stages['finalized_projection'])
    return stages
