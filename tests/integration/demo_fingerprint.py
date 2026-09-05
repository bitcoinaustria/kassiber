"""Invalidate generated demo books when their recipe or generator changes."""

from __future__ import annotations

import hashlib
import sys
from pathlib import Path


def demo_fingerprint(root: Path, scenario: Path) -> str:
    digest = hashlib.sha256(b"kassiber-demo-generator-v1\0")
    # The selected recipe may live outside the repository. Its location is not
    # material: identical recipes and generators must produce the same key.
    digest.update(scenario.read_bytes())
    inputs = {root / "scripts/integration-harness.sh"}
    inputs.update((root / "dev/regtest").glob("*.*"))
    inputs.update(
        path for path in (root / "tests/integration").glob("*.py")
        if not path.name.startswith("test_")
    )
    for path in sorted(inputs):
        if not path.is_file() or path.suffix == ".md":
            continue
        data = path.read_bytes()
        digest.update(b"\0" + path.relative_to(root).as_posix().encode() + b"\0")
        digest.update(str(len(data)).encode() + b"\0" + data)
    return digest.hexdigest()


if __name__ == "__main__":
    print(demo_fingerprint(Path(__file__).resolve().parents[2], Path(sys.argv[1])))
