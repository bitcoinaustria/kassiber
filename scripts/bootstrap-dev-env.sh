#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PYTHON="${PYTHON:-python3}"
VENV="${KASSIBER_VENV:-"$ROOT/.venv"}"

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

"$PYTHON" -m venv "$VENV"
"$VENV/bin/python" -m pip install -U pip setuptools wheel
"$VENV/bin/python" -m pip install -e "$ROOT"

"$VENV/bin/python" - <<'PY'
import embit
import sqlcipher3

print(f"Verified embit from {embit.__file__}")
print(f"Verified sqlcipher3 from {sqlcipher3.__file__}")
PY

cat <<EOF

Kassiber dev environment is ready.

Use:
  export KASSIBER_PYTHON="$VENV/bin/python"
  $VENV/bin/python -m unittest ...
EOF
