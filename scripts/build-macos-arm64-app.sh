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

run uv sync --frozen --python "$PYTHON_VERSION"

APP_VERSION="$(
  uv run --python "$PYTHON_VERSION" python -c \
    'import tomllib; print(tomllib.load(open("pyproject.toml", "rb"))["project"]["version"])'
)"
APP_COMMIT="$(git rev-parse --short=12 HEAD 2>/dev/null || printf unknown)"
APP_DISPLAY_VERSION="dev"
# macOS (BSD) mktemp only substitutes a trailing run of X's, so a
# ".XXXXXX.json" template is NOT randomized — it yields a fixed filename that
# fails with "File exists" if a previous run was killed before its cleanup trap
# could fire. Create a uniquely-named temp dir (trailing X's) and keep the
# config file inside it instead.
TAURI_VERSION_DIR="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-tauri-version.XXXXXX")"
trap 'rm -rf "$TAURI_VERSION_DIR"' EXIT
TAURI_VERSION_CONFIG="$TAURI_VERSION_DIR/version.json"
printf '{ "version": "%s" }\n' "$APP_VERSION" > "$TAURI_VERSION_CONFIG"

echo "Building Kassiber desktop for macOS arm64 only."
echo "Package version: $APP_VERSION"
echo "Displayed build: $APP_DISPLAY_VERSION ($APP_COMMIT)"
echo "Bundled sidecar: $SIDECAR_NAME"
echo "Bundles: $BUNDLES"
echo "Python: $PYTHON_VERSION"

run uv run --python "$PYTHON_VERSION" --with pyinstaller==6.20.0 pyinstaller \
  --clean \
  --noconfirm \
  --onefile \
  --name kassiber-cli \
  --specpath build \
  --paths . \
  --collect-data kassiber \
  --collect-submodules bdkpython \
  --collect-data bdkpython \
  --collect-submodules embit \
  --collect-data embit \
  --collect-submodules rp2 \
  --collect-data rp2 \
  --collect-submodules prezzemolo \
  --collect-submodules keyring.backends \
  --copy-metadata keyring \
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

kraken_smoke_root="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-kraken-bundled-smoke.XXXXXX")"
kraken_smoke_out="$kraken_smoke_root/daemon.jsonl"
printf '{"request_id":"kraken-bundled-1","kind":"ui.rates.kraken_csv.import","args":{"use_bundled":true}}\n{"request_id":"shutdown-1","kind":"daemon.shutdown"}\n' \
  | dist/kassiber-cli --data-root "$kraken_smoke_root/data" daemon \
  > "$kraken_smoke_out"
grep '"kind":"ui.rates.kraken_csv.import"' "$kraken_smoke_out" >/dev/null
grep '"bundled":true' "$kraken_smoke_out" >/dev/null
grep '"pairs":2' "$kraken_smoke_out" >/dev/null
grep '"rows":255181' "$kraken_smoke_out" >/dev/null

mkdir -p "$BINARIES_DIR"
find "$BINARIES_DIR" -maxdepth 1 -type f -name 'kassiber-cli-*' -delete
run cp dist/kassiber-cli "$BINARIES_DIR/$SIDECAR_NAME"
run chmod 755 "$BINARIES_DIR/$SIDECAR_NAME"

run rustup target add "$TARGET_TRIPLE"
run pnpm --dir ui-tauri install --frozen-lockfile
# Force CI=true for the bundle step. Tauri only passes create-dmg's
# `--skip-jenkins` (skip the Finder/AppleScript window-styling step) when the
# CI env var is set; the `--ci` flag alone does NOT trigger it. Locally that
# AppleScript step is flaky, and when it fails it leaves an orphaned
# `/Volumes/dmg.*` scratch mount plus an `rw.*.dmg` behind, which then makes
# every subsequent build fail too. CI=true makes the DMG bundle
# deterministically, matching the GitHub Actions prerelease build (which always
# runs with CI=true, so its shipped DMG is unstyled as well — the layout is
# cosmetic for an unsigned bundle).
run env CI=true KASSIBER_BUILD_VERSION="$APP_DISPLAY_VERSION" KASSIBER_BUILD_COMMIT="$APP_COMMIT" \
  pnpm --dir ui-tauri tauri build --target "$TARGET_TRIPLE" --bundles "$BUNDLES" --ci --config "$TAURI_VERSION_CONFIG"

cat <<EOF

macOS arm64 desktop build complete.

Look under:
  $ROOT/ui-tauri/src-tauri/target/$TARGET_TRIPLE/release/bundle

The app bundle includes:
  binaries/$SIDECAR_NAME
EOF
