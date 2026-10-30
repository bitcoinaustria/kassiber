"""Python and the webview use exactly the same deterministic test vectors."""
import json
from pathlib import Path

import pytest

from kassiber import redaction

CORPUS = json.loads((Path(__file__).parent / 'fixtures/operational_redaction_cases.json').read_text())


@pytest.mark.parametrize('case', CORPUS['cases'])
def test_operational_redaction_shared_corpus(monkeypatch, case):
    monkeypatch.setattr(redaction, '_AMOUNT_PSEUDONYM_SALT', CORPUS['salt'])
    assert redaction.redact_operational_text(case['input']) == case['expected']
