"""Python and the webview use exactly the same deterministic test vectors."""
import json
from pathlib import Path

import pytest

from kassiber import redaction

CORPUS = json.loads((Path(__file__).parent / 'fixtures/operational_redaction_cases.json').read_text())
SECRETS = json.loads((Path(__file__).parent / 'fixtures/secret_redaction_cases.json').read_text())


@pytest.mark.parametrize('case', CORPUS['cases'])
def test_operational_redaction_shared_corpus(monkeypatch, case):
    monkeypatch.setattr(redaction, '_AMOUNT_PSEUDONYM_SALT', CORPUS['salt'])
    assert redaction.redact_operational_text(case['input']) == case['expected']


@pytest.mark.parametrize('case', SECRETS)
def test_secret_redaction_shared_corpus(case):
    assert redaction.redact_secret_text(case['input']) == case['expected']
    assert redaction.redact_secret_value({'message': case['input']}) == {'message': case['expected']}


@pytest.mark.parametrize('key', ['rpcauth', 'rpc_auth', 'rpc-auth', 'macaroon', 'rpcpassword', 'cookie',
    'scan_key', 'BITCOIN_RPC_PASSWORD', 'LND_MACAROON', 'macaroon_hex', 'db_passphrase',
    'Authorization', 'rune', 'bitcoin.rpcpassword'])
def test_rpc_and_lightning_secret_fields(key):
    assert redaction.redact_secret_value({key: 'private'}) == {key: '[redacted]'}
