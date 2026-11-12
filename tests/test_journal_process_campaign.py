"""Complete-process campaign resumption must never reuse different evidence."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

import pytest
from tests.rust_fixture_binary import checked

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / 'scripts/check-journal-process-campaign.py'
BINARY = checked(os.environ.get('KASSIBER_TEST_JOURNAL_PROCESS_BIN', ROOT / 'target/release/examples/journal-process'),
                 'cargo build --release -p kassiber-journals --example journal-process --features test-harness --offline --locked')


def test_process_campaign_covers_every_generated_book_and_permutation_without_materializing():
    spec = importlib.util.spec_from_file_location('process_campaign', SCRIPT)
    campaign = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(campaign)
    rows = [metadata for metadata, _ in campaign.operands()]
    assert len(rows) == 25030
    assert sum(not row['permuted'] for row in rows) == 24000
    assert sum(row['permuted'] for row in rows) == 1030
    assert len({row['suite'] for row in rows}) == 12
    assert len({tuple(row.values()) for row in rows}) == len(rows)


@pytest.mark.skipif(not BINARY.is_file(), reason='requires journal-process test-harness example')
def test_process_campaign_resumes_exact_evidence_and_refuses_changed_manifest():
    import tempfile
    parent = ROOT / 'target/r4-13/tests'
    parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=parent, prefix='campaign-') as scratch:
        output = Path(scratch)
        command = [sys.executable, str(SCRIPT), '--binary', str(BINARY), '--output', str(output),
                   '--shard-size', '1', '--max-shards', '1']
        first = subprocess.run(command, capture_output=True, text=True, timeout=120)
        assert first.returncode == 0, first.stdout + first.stderr
        records = output / '0000.jsonl'
        evidence = records.read_bytes()
        row = json.loads(evidence)
        assert row['matched'] and all(row['checks'].values())
        assert not json.loads((output / 'summary.json').read_text())['complete']
        second = subprocess.run(command, capture_output=True, text=True, timeout=120)
        assert second.returncode == 0, second.stdout + second.stderr
        assert records.read_bytes() == evidence
        manifest_path = output / 'manifest.json'
        manifest = json.loads(manifest_path.read_text())
        manifest['binary'] = 'different native build'
        manifest_path.write_text(json.dumps(manifest))
        refused = subprocess.run(command, capture_output=True, text=True, timeout=120)
        assert refused.returncode != 0
        assert 'campaign fingerprint changed' in refused.stderr
        assert records.read_bytes() == evidence
