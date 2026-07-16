#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
PYTHON="${PYTHON:-python3}"
VENV="${KASSIBER_VENV:-"$ROOT/.venv"}"

if ! command -v uv >/dev/null 2>&1; then
  cat >&2 <<'EOF'
Kassiber development requires uv.

Install it from https://docs.astral.sh/uv/getting-started/installation/ and rerun:
  ./scripts/bootstrap-dev-env.sh
EOF
  exit 1
fi

if ! command -v "$PYTHON" >/dev/null 2>&1; then
  echo "Python interpreter not found: $PYTHON" >&2
  exit 1
fi

if command -v apt-get >/dev/null 2>&1; then
  if ! command -v pkg-config >/dev/null 2>&1 || ! pkg-config --exists sqlcipher; then
    cat >&2 <<'EOF'
SQLCipher development files are required before installing Kassiber's Python deps.

On Debian/Ubuntu:
  sudo apt-get update
  sudo apt-get install -y build-essential pkg-config libsqlcipher-dev sqlcipher

Then rerun:
  ./scripts/bootstrap-dev-env.sh
EOF
    exit 1
  fi
fi

UV_PROJECT_ENVIRONMENT="$VENV" uv sync --locked --python "$PYTHON"

"$VENV/bin/python" - <<'PY'
import platform
import sys
from importlib.metadata import version

import embit
import sqlcipher3

print(f"Verified embit from {embit.__file__}")
bdk_supported = sys.version_info < (3, 14) and (
    sys.platform == "darwin"
    or (sys.platform == "linux" and platform.machine() == "x86_64")
    or (sys.platform == "win32" and platform.machine() == "AMD64")
)
if bdk_supported:
    import bdkpython

    print(f"Verified bdkpython {version('bdkpython')} from {bdkpython.__file__}")
else:
    print("Skipped optional bdkpython verification on an unsupported wheel platform")
lwk_supported = (
    (sys.platform == "darwin" and platform.machine() == "arm64")
    or (sys.platform == "linux" and platform.machine() == "x86_64")
    or (sys.platform == "win32" and platform.machine() == "AMD64")
)
if lwk_supported:
    import lwk

    print(f"Verified lwk {version('lwk')} from {lwk.__file__}")
else:
    print("Skipped optional lwk verification on an unsupported wheel platform")
print(f"Verified sqlcipher3 from {sqlcipher3.__file__}")
PY

cat <<EOF

Kassiber dev environment is ready.

Use:
  export KASSIBER_PYTHON="$VENV/bin/python"
  UV_PROJECT_ENVIRONMENT="$VENV" uv run --locked python -m pytest ...
EOF
