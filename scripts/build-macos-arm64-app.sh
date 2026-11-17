#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TARGET_TRIPLE="aarch64-apple-darwin"
SIDECAR_NAME="kassiber-cli-${TARGET_TRIPLE}"
# A separate name and bundle identifier, so this local build installs beside
# a released Kassiber.
APP_PRODUCT_NAME="Kassiber Dev"
APP_IDENTIFIER="at.bitcoinaustria.kassiber.dev"
PACKAGER="$ROOT/crates/kassiber-desktop/packaging/macos/build-app.sh"
OUTPUT_DIR="$ROOT/dist/macos-arm64"
BUNDLES="${BUNDLES:-app,dmg}"
BACKUP=1
INSTALL=0

usage() {
  cat <<'EOF'
Usage: ./scripts/build-macos-arm64-app.sh [--install] [--no-backup]

Build the local ad-hoc-signed macOS arm64 desktop app, the Rune app with the
`kassiber` CLI bundled: "Kassiber Dev" (at.bitcoinaustria.kassiber.dev). It
installs beside a released Kassiber and opens the same books.

  --install          Replace an installed Kassiber Dev (in /Applications or
                     ~/Applications), or copy the app to ~/Applications, and
                     install its terminal command.
                     The command never takes over a \`kassiber\` that
                     Homebrew or another installer manages.
  --no-backup        Skip the snapshot of your Kassiber data. By default the
                     script snapshots every local book (only when it changed
                     since the last snapshot) before this build can open it;
                     see scripts/macos-kassiber-data.sh.

Finished apps and DMGs are copied to dist/macos-arm64/. BUNDLES=app skips the
DMG.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --install) INSTALL=1 ;;
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
require_command hdiutil
require_command rustup
require_command cargo

DATA_HELPER="$ROOT/scripts/macos-kassiber-data.sh"
# Fail before the long build rather than after it: the snapshot needs every
# Kassiber process stopped. The snapshot itself runs last, closest to the
# app's first launch.
if [ "$BACKUP" -eq 1 ]; then
  running="$(pgrep -fl 'Contents/MacOS/kassiber-ui|kassiber-cli-[a-z0-9_]+-apple-darwin|--operator-broker-server|-m kassiber( |$)' || true)"
  if [ -n "$running" ]; then
    echo "Kassiber is running. Quit every Kassiber app and CLI so your books can be" >&2
    echo "backed up before this build opens them (or pass --no-backup):" >&2
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
APP_DISPLAY_VERSION="dev"
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
echo "Bundled sidecar: $SIDECAR_NAME"
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

# The Rune app, built without agent tooling, around the CLI built and smoked
# above: crates/kassiber-desktop/packaging/macos/build-app.sh assembles the
# bundle (sidecar at Resources/binaries/kassiber-cli/$SIDECAR_NAME, terminal
# launcher at Resources/bin/kassiber), signs it ad hoc inside out under the
# release identifiers, and checks both entry points reach the sidecar.
case ",$BUNDLES," in *,app,*|*,dmg,*) ;; *) echo "BUNDLES names neither app nor dmg: $BUNDLES" >&2; exit 2 ;; esac
run "$PACKAGER" --kassiber-bin "$RUST_BIN" --identity dev --version "$APP_VERSION" \
  --commit "$APP_COMMIT_FULL" --channel dev --output "$OUTPUT_DIR"

APP_BUNDLE="$OUTPUT_DIR/$APP_PRODUCT_NAME.app"
bundled_sidecar="$APP_BUNDLE/Contents/Resources/binaries/kassiber-cli/$SIDECAR_NAME"
if [ "$("$bundled_sidecar" --version)" != "$version_text" ]; then
  echo "The bundled sidecar is not the kassiber binary built above." >&2
  exit 1
fi
bundled_version="$("$APP_BUNDLE/Contents/Resources/bin/kassiber" --version)"
if [ "$bundled_version" != "$version_text" ]; then
  echo "The bundled CLI launcher reports '$bundled_version', expected '$version_text'." >&2
  exit 1
fi
run "$APP_BUNDLE/Contents/Resources/bin/kassiber" --help >/dev/null

case ",$BUNDLES," in
  *,dmg,*)
    # A plain compressed image of the sealed app; no Finder styling (it
    # scripts Finder and fails without a logged-in GUI session).
    dmg="$OUTPUT_DIR/${APP_PRODUCT_NAME}_${APP_VERSION}_aarch64.dmg"
    dmg_stage="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-dmg.XXXXXX")"
    trap 'rm -rf "$smoke_root" "$dmg_stage"' EXIT
    run ditto "$APP_BUNDLE" "$dmg_stage/$APP_PRODUCT_NAME.app"
    ln -s /Applications "$dmg_stage/Applications"
    rm -f "$dmg"
    run hdiutil create -quiet -volname "$APP_PRODUCT_NAME" -srcfolder "$dmg_stage" -ov -format UDZO "$dmg"
    ;;
esac
case ",$BUNDLES," in
  *,app,*) ;;
  *) rm -rf "${APP_BUNDLE:?}" ;;
esac

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
  if run "$DATA_HELPER" snapshot --if-changed --label "before-dev-$APP_COMMIT"; then
    snapshot_note="Backup: your books were snapshotted (or were unchanged since the last
snapshot) before this build can open them. List or restore with:
  $DATA_HELPER list
  $DATA_HELPER restore <snapshot>"
  else
    snapshot_note="Backup: FAILED. Quit every Kassiber app, then run
  $DATA_HELPER snapshot
before opening $APP_PRODUCT_NAME."
  fi
fi

install_note="Install it with --install, or run it from $OUTPUT_DIR."
if [ "$INSTALL" -eq 1 ]; then
  if [ ! -d "$OUTPUT_DIR/$APP_PRODUCT_NAME.app" ]; then
    echo "--install needs the app bundle; include app in BUNDLES." >&2
    exit 1
  fi
  # Update the copy macOS already knows: two bundles with one identifier make
  # Finder, Spotlight and `open -a` start whichever LaunchServices prefers.
  # Without one, install to ~/Applications, which needs no administrator.
  installed=""
  for dir in /Applications "$HOME/Applications"; do
    if [ -d "$dir/$APP_PRODUCT_NAME.app" ]; then
      if [ -n "$installed" ]; then
        echo "WARNING: $dir/$APP_PRODUCT_NAME.app is a second copy; macOS may open it instead." >&2
      else
        installed="$dir/$APP_PRODUCT_NAME.app"
      fi
    fi
  done
  installed="${installed:-$HOME/Applications/$APP_PRODUCT_NAME.app}"
  if pgrep -f "$installed/Contents/MacOS/" >/dev/null 2>&1; then
    echo "Quit $APP_PRODUCT_NAME ($installed) before installing over it, then run this script again." >&2
    exit 1
  fi
  if [ ! -e "$installed" ]; then
    mkdir -p "$(dirname "$installed")"
    run ditto "$OUTPUT_DIR/$APP_PRODUCT_NAME.app" "$installed"
  elif [ -w "$(dirname "$installed")" ]; then
    rm -rf "${installed:?}"
    run ditto "$OUTPUT_DIR/$APP_PRODUCT_NAME.app" "$installed"
  elif [ -w "$installed" ]; then
    # /Applications itself needs an administrator, but a bundle this user
    # installed there is theirs: replace its contents in place.
    rm -rf "${installed:?}/Contents"
    run ditto "$OUTPUT_DIR/$APP_PRODUCT_NAME.app/Contents" "$installed/Contents"
  else
    echo "Cannot replace $installed: it belongs to another user. Remove it, then run this script again." >&2
    exit 1
  fi
  run codesign --verify --deep --strict "$installed"
  run "$ROOT/scripts/install-macos-desktop-cli.sh" "$installed"
  install_note="Installed: $installed
Open it with: open \"$installed\""
fi

cat <<EOF

macOS arm64 desktop build complete.

Look under:
  $OUTPUT_DIR

$APP_PRODUCT_NAME ($version_text) uses the same books as an installed
Kassiber:
  ~/Library/Application Support/at.bitcoinaustria.kassiber
Both apps can be open at once, but one book opens in one desktop app at a
time: the other app reports that another desktop app owns the project. Quit
one app to open that book in the other.

$install_note

$snapshot_note
EOF
