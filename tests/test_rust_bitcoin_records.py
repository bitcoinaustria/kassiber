"""Pure record/composition oracles; no persisted product-sync parity claim."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

import pytest

from tests.rust_sync_env import child_env as _env
from tests.rust_fixture_binary import fixture_binary

ROOT = Path(__file__).resolve().parents[1]
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
pytestmark = pytest.mark.skipif(not RUST_BIN, reason="requires records_fixture harness build")
SPEC = importlib.util.spec_from_file_location("bitcoin_record_vectors", ROOT / "crates/wallets/scripts/generate_record_vectors.py")
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)


def project(tmp_path, cases, *, offline=False):
    binary = fixture_binary("records_fixture")
    env = _env(tmp_path)
    if offline:
        env["KASSIBER_NO_EGRESS"] = "1"
    result = subprocess.run([str(binary)], input=json.dumps(cases).encode(), env=env,
        capture_output=True, check=True, timeout=30)
    assert result.stderr == b""
    return json.loads(result.stdout)


@pytest.mark.parametrize("offline", [False, True])
def test_normalized_records_and_composition_match_live_python_without_consent(tmp_path, offline):
    cases = REFERENCE.vectors()
    assert json.loads(REFERENCE.DEST.read_text()) == cases
    # The pure seam receives no grant, endpoint, book handle or transport.
    assert project(tmp_path, cases, offline=offline) == [REFERENCE.evaluate(c) for c in cases]


def test_bitcoin_record_vectors_still_match_python(tmp_path):
    for script in ("generate_record_vectors.py", "generate_persistence_vectors.py", "generate_publication_vectors.py",
                   "generate_physical_vectors.py", "generate_insert_vectors.py", "generate_provenance_vectors.py",
                   "generate_inventory_vectors.py", "generate_policy_vectors.py", "generate_report_vectors.py"):
        result = subprocess.run([sys.executable, str(ROOT / "crates/wallets/scripts" / script), "--check"],
            capture_output=True, timeout=30, env=_env(tmp_path))
        assert result.returncode == 0, result.stderr.decode()
