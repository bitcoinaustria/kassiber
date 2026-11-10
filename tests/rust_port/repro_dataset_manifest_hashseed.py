"""Record unchanged Python's hash-seed-dependent dataset manifest order.

``normalize_manifest`` iterates the ``_MANIFEST_REQUIRED`` set, so the key
order of a previewed manifest, and which of several invalid required fields
is reported, follow ``PYTHONHASHSEED``. Values are identical across seeds.
The native port uses sorted order; Python is not changed here.
"""
from __future__ import annotations

import io
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = {'dataset_key': 'pack-1', 'name': 'Pack', 'version': '1', 'chain': 'bitcoin', 'network': 'main',
            'source': 'Local file', 'license': 'CC0', 'attribution_method': 'published', 'visibility': 'public'}


def reproduce():
    sys.path.insert(0, str(ROOT))
    from kassiber.core.chain_analysis_datasets import preview_dataset
    from kassiber.errors import AppError
    result = preview_dataset(MANIFEST, io.BytesIO(b'subject,label\n' + b'a' * 64 + b',A\n'))
    try:
        preview_dataset({**MANIFEST, 'name': '', 'version': '', 'license': ''}, io.BytesIO(b''))
        error = None
    except AppError as exc:
        error = str(exc)
    return {'manifest_keys': list(result['manifest']), 'canonical': json.dumps(result, sort_keys=True), 'first_error': error}


def variants():
    values = {}
    for seed in range(8):
        env = {**os.environ, 'PYTHONHASHSEED': str(seed)}
        p = subprocess.run([sys.executable, str(Path(__file__).resolve())], env=env, capture_output=True, check=True, timeout=30)
        values[str(seed)] = json.loads(p.stdout)
    return values


if __name__ == '__main__':
    print(json.dumps(variants() if '--seeds' in sys.argv else reproduce(), sort_keys=True, indent=2))
