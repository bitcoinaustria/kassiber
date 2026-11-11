#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TARGET_TRIPLE="aarch64-apple-darwin"
SIDECAR_NAME="kassiber-cli-${TARGET_TRIPLE}"
# A separate name and bundle identifier, so this local build installs beside
# a released Kassiber.
APP_PRODUCT_NAME="Kassiber Rust Dev"
APP_IDENTIFIER="at.bitcoinaustria.kassiber.rust-dev"
BINARIES_DIR="$ROOT/ui-tauri/src-tauri/binaries"
BUNDLE_DIR="$ROOT/ui-tauri/src-tauri/target/$TARGET_TRIPLE/release/bundle"
OUTPUT_DIR="$ROOT/dist/macos-arm64"
BUNDLES="${BUNDLES:-app,dmg}"
BACKUP=1

usage() {
  cat <<'EOF'
Usage: ./scripts/build-macos-arm64-rust-app.sh [--no-backup]

Build the local ad-hoc-signed macOS arm64 app with the Rust backend as its
sidecar: "Kassiber Rust Dev" (at.bitcoinaustria.kassiber.rust-dev). It installs
beside a released Kassiber and opens the same books.

  --no-backup        Skip the snapshot of your Kassiber data. By default the
                     script snapshots every local book (only when it changed
                     since the last snapshot) before the Rust app can open it;
                     see scripts/macos-kassiber-data.sh.

Finished apps and DMGs are copied to dist/macos-arm64/.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --no-backup) BACKUP=0 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

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
require_command otool
require_command codesign
require_command strip
require_command ditto
require_command osascript
require_command pnpm
require_command rustup
require_command cargo

DATA_HELPER="$ROOT/scripts/macos-kassiber-data.sh"
# Fail before the long build rather than after it: the snapshot needs every
# Kassiber process stopped. The snapshot itself runs last, closest to the Rust
# app's first launch.
if [ "$BACKUP" -eq 1 ]; then
  running="$(pgrep -fl 'Contents/MacOS/kassiber-ui|kassiber-cli-[a-z0-9_]+-apple-darwin|--operator-broker-server|-m kassiber( |$)' || true)"
  if [ -n "$running" ]; then
    echo "Kassiber is running. Quit every Kassiber app and CLI so your books can be" >&2
    echo "backed up before the Rust app opens them (or pass --no-backup):" >&2
    printf '%s\n' "$running" >&2
    exit 3
  fi
fi

# The product version lives in the backend crate.
APP_VERSION="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' crates/kassiber/Cargo.toml | head -n 1)"
if [ -z "$APP_VERSION" ]; then
  echo "crates/kassiber/Cargo.toml names no version." >&2
  exit 1
fi
APP_COMMIT_FULL="$(git rev-parse HEAD 2>/dev/null || printf unknown)"
# `--version` prints the first 12 characters of the embedded commit.
APP_COMMIT="$(printf '%s' "$APP_COMMIT_FULL" | cut -c1-12)"
APP_DISPLAY_VERSION="dev-rust"
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
  SOURCE_DATE_EPOCH="$(git show -s --format=%ct HEAD 2>/dev/null || date +%s)"
  export SOURCE_DATE_EPOCH
fi
if [ -n "$(git status --porcelain --untracked-files=no 2>/dev/null)" ]; then
  echo "Note: the checkout has uncommitted changes; the build is labelled $APP_COMMIT anyway."
fi

mkdir -p "$OUTPUT_DIR"


# The desktop shell and the backend both use the toolchain rust-toolchain.toml
# pins; install it before the first build.
RUST_CHANNEL="$(sed -n 's/^channel *= *"\([^"]*\)".*/\1/p' rust-toolchain.toml)"
run rustup toolchain install "$RUST_CHANNEL" --profile minimal --component rustfmt --component clippy
run rustup target add "$TARGET_TRIPLE"

echo
echo "Building $APP_PRODUCT_NAME desktop for macOS arm64 only."
echo "Package version: $APP_VERSION"
echo "Displayed build: $APP_DISPLAY_VERSION ($APP_COMMIT)"
echo "Bundle identifier: $APP_IDENTIFIER"
echo "Bundled sidecar: $SIDECAR_NAME (Rust)"
echo "Bundles: $BUNDLES"

# The production binary through the release build script: no test-harness
# feature, symbols stripped with Xcode's strip (rustup's minimal rust-objcopy
# cannot load libLLVM.dylib), any OPENSSL_DIR dropped so SQLCipher links
# CommonCrypto, an otool check that only macOS system libraries are linked, and
# an ad-hoc signature under the sidecar's signing identifier.
RUST_OUT="$ROOT/target/macos-arm64-rust"
run "$ROOT/scripts/build-rust-cli.sh" --target "$TARGET_TRIPLE" --channel dev \
  --commit "$APP_COMMIT_FULL" --output "$RUST_OUT"
RUST_BIN="$RUST_OUT/kassiber"

# build-rust-cli.sh already refused any other --version text.
version_text="$("$RUST_BIN" --version)"

# The packaged-binary smoke CI runs, on disposable books under $TMPDIR, with
# the machine-wide preference files redirected there too.
smoke_root="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-rust-bundled-smoke.XXXXXX")"
trap 'rm -rf "$smoke_root"' EXIT
# $TMPDIR sits under /var -> /private/var. The store resolves such links, but
# a physical path keeps the smoke independent of that.
smoke_root="$(cd "$smoke_root" && pwd -P)"
run "$ROOT/scripts/smoke-packaged-cli.sh" --cli "$RUST_BIN" --scratch "$smoke_root/smoke" \
  --expect-commit "$APP_COMMIT"

# The AI provider broker ships beside the sidecar, where the binary looks for
# it (<executable dir>/provider_broker/index.mjs).
run pnpm --dir ui-tauri install --frozen-lockfile
run pnpm --dir ui-tauri run broker:build
test -s "$ROOT/ui-tauri/provider-broker/dist/index.mjs"

# Same one-dir location and file name the Python sidecar uses: the supervisor,
# the CLI launcher, and the Touch ID helper's identity check all look there.
# Sweep both layouts so a previous Python build never shadows this binary.
mkdir -p "$BINARIES_DIR"
find "$BINARIES_DIR" -maxdepth 1 -type f -name 'kassiber-cli-*' -delete
rm -rf "${BINARIES_DIR:?}/kassiber-cli" "${BINARIES_DIR:?}/provider_broker"
mkdir -p "$BINARIES_DIR/kassiber-cli/provider_broker"
run cp "$RUST_BIN" "$BINARIES_DIR/kassiber-cli/$SIDECAR_NAME"
run chmod 755 "$BINARIES_DIR/kassiber-cli/$SIDECAR_NAME"
run cp "$ROOT/ui-tauri/provider-broker/dist/index.mjs" "$BINARIES_DIR/kassiber-cli/provider_broker/index.mjs"

TAURI_CONFIG_DIR="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-tauri-config.XXXXXX")"
trap 'rm -rf "$smoke_root" "$TAURI_CONFIG_DIR"' EXIT
TAURI_BUILD_CONFIG="$TAURI_CONFIG_DIR/build.json"
printf '{ "version": "%s", "productName": "%s", "identifier": "%s" }\n' \
  "$APP_VERSION" "$APP_PRODUCT_NAME" "$APP_IDENTIFIER" > "$TAURI_BUILD_CONFIG"

# CI=true makes Tauri skip create-dmg's flaky Finder styling step (it scripts
# Finder through AppleScript and fails without a logged-in GUI session).
run env CI=true KASSIBER_BUILD_VERSION="$APP_DISPLAY_VERSION" KASSIBER_BUILD_COMMIT="$APP_COMMIT" \
  pnpm --dir ui-tauri tauri build --target "$TARGET_TRIPLE" --bundles "$BUNDLES" --ci --config "$TAURI_BUILD_CONFIG"

APP_BUNDLE="$BUNDLE_DIR/macos/$APP_PRODUCT_NAME.app"
if [ -d "$APP_BUNDLE" ]; then
  bundled_sidecar="$APP_BUNDLE/Contents/Resources/binaries/kassiber-cli/$SIDECAR_NAME"
  case "$(file "$bundled_sidecar")" in
    *"Mach-O 64-bit executable arm64"*) ;;
    *)
      echo "The app bundle does not contain the Rust backend at $bundled_sidecar." >&2
      exit 1
      ;;
  esac
  if [ "$("$bundled_sidecar" --version)" != "$version_text" ]; then
    echo "The bundled sidecar is not the Rust backend built above." >&2
    exit 1
  fi
  if [ -e "$APP_BUNDLE/Contents/Resources/binaries/kassiber-cli/_internal" ]; then
    echo "The app bundle still carries a Python sidecar payload; refusing it." >&2
    exit 1
  fi
  test -s "$APP_BUNDLE/Contents/Resources/binaries/kassiber-cli/provider_broker/index.mjs"
  # The sidecar keeps the Python sidecar's signing identifier. Tauri's ad-hoc
  # bundle signing may re-sign nested code under a derived identifier; if so,
  # restore it and re-seal the bundle. A Developer ID signature (and with it
  # the Touch ID helper and Keychain continuity) comes only from
  # scripts/macos_release.py sign, which signs every Mach-O under its file name.
  sidecar_identifier="$(codesign -d --verbose=2 "$bundled_sidecar" 2>&1 | sed -n 's/^Identifier=//p')"
  if [ "$sidecar_identifier" != "$SIDECAR_NAME" ]; then
    echo "Re-signing the bundled sidecar as $SIDECAR_NAME (Tauri signed it as '$sidecar_identifier')."
    run codesign --force --sign - --identifier "$SIDECAR_NAME" "$bundled_sidecar"
    run codesign --force --sign - --preserve-metadata=entitlements,requirements,flags,runtime "$APP_BUNDLE"
  fi
  run codesign --verify --deep --strict "$APP_BUNDLE"
  sidecar_identifier="$(codesign -d --verbose=2 "$bundled_sidecar" 2>&1 | sed -n 's/^Identifier=//p')"
  if [ "$sidecar_identifier" != "$SIDECAR_NAME" ]; then
    echo "The bundled sidecar is signed as '$sidecar_identifier', not $SIDECAR_NAME." >&2
    exit 1
  fi
  bundled_version="$("$APP_BUNDLE/Contents/Resources/bin/kassiber" --version)"
  if [ "$bundled_version" != "$version_text" ]; then
    echo "The bundled CLI launcher reports '$bundled_version', expected '$version_text'." >&2
    exit 1
  fi
  run "$APP_BUNDLE/Contents/Resources/bin/kassiber" --help >/dev/null
  rm -rf "${OUTPUT_DIR:?}/$APP_PRODUCT_NAME.app"
  run ditto "$APP_BUNDLE" "$OUTPUT_DIR/$APP_PRODUCT_NAME.app"
fi
for dmg in "$BUNDLE_DIR/dmg/$APP_PRODUCT_NAME"_*.dmg; do
  if [ -f "$dmg" ]; then
    run ditto "$dmg" "$OUTPUT_DIR/$(basename "$dmg")"
  fi
done

# Warn when an installed Kassiber comes from another commit: whichever app is
# newer may upgrade the books, and an older build then refuses them. Read its
# embedded build record rather than executing an installed app.
for installed_app in \
  "/Applications/Kassiber.app" \
  "$HOME/Applications/Kassiber.app"; do
  [ -d "$installed_app" ] || continue
  build_info="$(find "$installed_app/Contents/Resources" -name BUILD_INFO.json -print -quit 2>/dev/null || true)"
  installed_commit=""
  if [ -n "$build_info" ]; then
    installed_commit="$(sed -n 's/.*"commit": *"\([0-9a-f]*\)".*/\1/p' "$build_info" | head -n 1)"
  fi
  case "$installed_commit" in
    "$APP_COMMIT"*) ;;
    *)
      echo
      echo "WARNING: $installed_app was built from commit '${installed_commit:-unknown}',"
      echo "not $APP_COMMIT. Opening your books in this build can upgrade them to a"
      echo "format that older build refuses."
      ;;
  esac
done

snapshot_note="Backup: skipped (--no-backup)."
if [ "$BACKUP" -eq 1 ]; then
  echo
  if run "$DATA_HELPER" snapshot --if-changed --label "before-rust-$APP_COMMIT"; then
    snapshot_note="Backup: your books were snapshotted (or were unchanged since the last
snapshot) before this Rust build can open them. List or restore with:
  $DATA_HELPER list
  $DATA_HELPER restore <snapshot>"
  else
    snapshot_note="Backup: FAILED. Quit every Kassiber app, then run
  $DATA_HELPER snapshot
before opening $APP_PRODUCT_NAME."
  fi
fi

cat <<EOF

macOS arm64 Rust desktop build complete.

Look under:
  $OUTPUT_DIR

$APP_PRODUCT_NAME uses the Rust backend ($version_text) and the same books
as an installed Kassiber:
  ~/Library/Application Support/at.bitcoinaustria.kassiber
Both apps can be open at once, but one book opens in one desktop app at a
time: the other app reports that another desktop app owns the project. Quit
one app to open that book in the other.

$snapshot_note

Run the Rust CLI directly, without replacing your installed \`kassiber\`:
  alias kassiber-rust='$OUTPUT_DIR/$APP_PRODUCT_NAME.app/Contents/Resources/bin/kassiber'
EOF
