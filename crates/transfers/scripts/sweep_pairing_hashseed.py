#!/usr/bin/env python3
"""Re-export both oracles under distinct CPython hash seeds, without masking results."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]

def main():
    target = ROOT / 'target/r4-pairing/hashseeds'
    target.mkdir(parents=True, exist_ok=True)
    results = []
    for seed in ('0', '1', '2', '42', '123', '4294967295', 'random'):
        hashes = {}
        for name in ('pairing', 'pairing_services'):
            output = target / f'{name}-{seed}.json'
            result = subprocess.run(
                [sys.executable, str(Path(__file__).with_name(f'export_{name}.py')), str(output)],
                cwd=ROOT, env=dict(os.environ, PYTHONHASHSEED=seed),
                capture_output=True, text=True,
            )
            if result.returncode:
                raise RuntimeError(result.stdout + result.stderr)
            hashes[name] = hashlib.sha256(output.read_bytes()).hexdigest()
        results.append(dict(seed=seed, sha256=hashes))
        print(json.dumps(results[-1]), flush=True)
    (target / 'results.json').write_text(json.dumps(results, indent=2) + '\n')
    assert all(r['sha256'] == results[0]['sha256'] for r in results), 'Python depends on PYTHONHASHSEED; keep the differing artifacts as a repro'
    for name in ('pairing', 'pairing_services'):
        expected = hashlib.sha256((ROOT / f'crates/transfers/tests/{name if name != "pairing" else "pairing_matcher"}.json').read_bytes()).hexdigest()
        assert results[0]['sha256'][name] == expected, (name, 'checked-in oracle drift')

if __name__ == '__main__':
    main()
