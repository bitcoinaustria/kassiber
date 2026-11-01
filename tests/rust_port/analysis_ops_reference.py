"""Test-only clocks and UUIDs around unchanged Python CLI/daemon dispatch."""
import contextlib
import importlib.util
import os
from pathlib import Path
import sys
from types import SimpleNamespace
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT))
SPEC = importlib.util.spec_from_file_location('analysis_ops_vectors',ROOT/'crates/wallets/scripts/generate_analysis_ops.py')
REFERENCE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)


def main():
    from kassiber.cli.main import main as cli
    from kassiber.operator import project
    source = REFERENCE.Sources()
    source.counter = int(os.environ.get('KASSIBER_RUST_TEST_ANALYSIS_ID_OFFSET', '0'))
    with contextlib.ExitStack() as stack:
        for module in [REFERENCE.cases, REFERENCE.datasets, REFERENCE.projection_store]:
            stack.enter_context(patch.object(module,'now_iso',return_value=REFERENCE.NOW))
        stack.enter_context(patch.object(REFERENCE.cases,'uuid',SimpleNamespace(uuid4=source.label_uuid)))
        stack.enter_context(patch.object(REFERENCE.projection_store,'uuid',SimpleNamespace(uuid4=source.uuid)))
        stack.enter_context(patch.object(project,'_owner_lock_root',return_value=Path(os.environ['KASSIBER_RUST_TEST_OWNER_ROOT'])))
        return cli(sys.argv[1:])


if __name__ == '__main__':
    raise SystemExit(main())
