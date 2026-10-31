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


@pytest.mark.parametrize('separator', ['-', '.'])
def test_long_identifier_redaction_is_linear(separator):
    # CPU measures only the call; a generous wall limit still contains a runaway
    # child without charging interpreter startup or host scheduling to redaction.
    import subprocess
    import sys

    text = ('word' + separator) * 20000
    script = """
from kassiber.redaction import redact_secret_text
import sys, time
text = sys.stdin.read()
start = time.process_time()
result = redact_secret_text(text)
elapsed = time.process_time() - start
assert result == text
assert elapsed < 0.8, elapsed
"""
    subprocess.run([sys.executable, '-c', script], input=text, text=True, check=True, timeout=120)


def test_adversarial_secret_shapes_keep_a_bounded_deadline():
    import subprocess
    import sys

    recipes = Path(__file__).parent / 'fixtures/secret_redaction_stress_cases.json'
    script = """
import json, sys, time
from kassiber.redaction import redact_secret_text
cpu_seconds = 0
for case in json.load(sys.stdin):
    text = case['prefix'] + (case['repeat'] * (40000 // len(case['repeat']) + 1)) + case['suffix']
    start = time.process_time()
    redact_secret_text(text)
    cpu_seconds += time.process_time() - start
assert cpu_seconds < 2, cpu_seconds
"""
    subprocess.run([sys.executable, '-c', script], input=recipes.read_text(),
                   text=True, check=True, timeout=120)
