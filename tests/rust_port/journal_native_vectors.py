"""Derive native ledger witnesses directly from the unchanged Python reader."""
import json
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from kassiber.core.engines.base import TaxEngineLedgerInputs
from kassiber.core.engines import ledger_response
from tests.custody_snapshot import to_jsonable
from tests.rust_port.journal_render_oracle import canonical, refusal

DATA = Path(__file__).resolve().parents[2] / 'crates/journals/tests/data'
PATH = DATA / 'render-native-ledger.json'
SOURCES = ('render.json', 'render-legacy-floats.json', 'render-response-refusals.json',
           'render-scalar-coercions.json')


def capture(record):
    projection = SimpleNamespace(**record['custody']['finalized_tax_projection'])
    wallets = dict(zip(record['custody']['wallet_refs_by_id'], record['wallets'], strict=True))
    inputs = TaxEngineLedgerInputs(finalized_tax_projection=projection,
                                  wallet_refs_by_id=wallets,
                                  direct_payout_records=record['custody']['direct_payout_records'])
    ids = iter(record['entry_ids'])
    consumed = 0

    def identity(*args):
        nonlocal consumed
        consumed += 1
        return next(ids)

    try:
        with patch.object(ledger_response, 'journal_entry_id', identity):
            result = (ledger_response.empty_result(inputs) if record['response'] is None else
                      ledger_response.read_ledger(record['profile'], inputs, record['response']))
    except Exception as exc:
        expected = {'error': refusal(exc)}
    else:
        expected = {'ledger': canonical(to_jsonable(dict(
            entries=result.entries, quarantines=result.quarantines,
            intra_audit=result.intra_audit, cross_asset_pairs=result.cross_asset_pairs,
            direct_swap_payouts=result.direct_swap_payouts, tax_summary=result.tax_summary,
            account_holdings=result.account_holdings, wallet_holdings=result.wallet_holdings,
        ), redact_config=False))}
    return dict(expected=expected, consumed=consumed)


def vectors():
    return [dict(source=source, index=i, name=record['name'], **capture(record))
            for source in SOURCES
            for i, record in enumerate(json.loads((DATA / source).read_text()))]


if __name__ == '__main__':
    PATH.write_text(json.dumps(vectors(), ensure_ascii=True, separators=(',', ':')) + '\n')
