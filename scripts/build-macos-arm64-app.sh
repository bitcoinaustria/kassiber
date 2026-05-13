#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TARGET_TRIPLE="aarch64-apple-darwin"
SIDECAR_NAME="kassiber-cli-${TARGET_TRIPLE}"
BINARIES_DIR="$ROOT/ui-tauri/src-tauri/binaries"
BUNDLES="${BUNDLES:-app,dmg}"
PYTHON_VERSION="${PYTHON_VERSION:-3.11}"

run() {
  echo
  echo "> $*"
  "$@"
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 2
  fi
}

if [ "$(uname -s)" != "Darwin" ]; then
  echo "This script builds the macOS arm64 desktop app and must run on macOS." >&2
  exit 2
fi

if [ "$(uname -m)" != "arm64" ]; then
  echo "This script must run natively on Apple Silicon (uname -m must be arm64)." >&2
  echo "It intentionally does not use Rosetta or build Intel artifacts." >&2
  exit 2
fi

require_command file
require_command pnpm
require_command rustup
require_command uv

echo "Building Kassiber desktop for macOS arm64 only."
echo "Bundled sidecar: $SIDECAR_NAME"
echo "Bundles: $BUNDLES"
echo "Python: $PYTHON_VERSION"

run uv sync --frozen --python "$PYTHON_VERSION"

run uv run --python "$PYTHON_VERSION" --with pyinstaller==6.20.0 pyinstaller \
  --clean \
  --noconfirm \
  --onefile \
  --name kassiber-cli \
  --specpath build \
  --paths . \
  --collect-submodules embit \
  --collect-data embit \
  --collect-submodules rp2 \
  --collect-data rp2 \
  --collect-submodules prezzemolo \
  --hidden-import prezzemolo.avl_tree \
  --hidden-import rp2.plugin.country.at \
  scripts/kassiber_pyinstaller_entry.py

sidecar_arch="$(file dist/kassiber-cli)"
echo "$sidecar_arch"
case "$sidecar_arch" in
  *"arm64"*) ;;
  *)
    echo "dist/kassiber-cli is not an arm64 executable; refusing to bundle it." >&2
    exit 1
    ;;
esac

run dist/kassiber-cli --help

mkdir -p "$BINARIES_DIR"
find "$BINARIES_DIR" -maxdepth 1 -type f -name 'kassiber-cli-*' -delete
run cp dist/kassiber-cli "$BINARIES_DIR/$SIDECAR_NAME"
run chmod 755 "$BINARIES_DIR/$SIDECAR_NAME"

run rustup target add "$TARGET_TRIPLE"
run pnpm --dir ui-tauri install --frozen-lockfile
run pnpm --dir ui-tauri tauri build --target "$TARGET_TRIPLE" --bundles "$BUNDLES" --ci

cat <<EOF

macOS arm64 desktop build complete.

Look under:
  $ROOT/ui-tauri/src-tauri/target/$TARGET_TRIPLE/release/bundle

The app bundle includes:
  binaries/$SIDECAR_NAME
EOF
