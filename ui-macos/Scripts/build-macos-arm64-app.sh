#!/usr/bin/env bash
set -euo pipefail

NATIVE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "$NATIVE_ROOT/.." && pwd)"
cd "$REPO_ROOT"

PRODUCT_NAME="kassiber_native"
TARGET_TRIPLE="arm64-apple-macosx"
PYTHON_VERSION="${PYTHON_VERSION:-3.11}"
BUILD_ROOT="${BUILD_ROOT:-$NATIVE_ROOT/build}"
APP="$BUILD_ROOT/${PRODUCT_NAME}.app"
ZIP="$BUILD_ROOT/${PRODUCT_NAME}-macos-arm64.zip"
SWIFT_BUILD_DIR="$NATIVE_ROOT/.build/$TARGET_TRIPLE/release"
SIGN_IDENTITY="${SIGN_IDENTITY:-${DEVELOPER_ID_APPLICATION:--}}"
NOTARIZE="${NOTARIZE:-0}"
CREATE_ZIP="${CREATE_ZIP:-1}"
swift_sandbox_args=()
if [[ "${SWIFTPM_DISABLE_SANDBOX:-0}" == "1" ]]; then
  # Useful only when this script already runs inside a stricter outer sandbox
  # that rejects SwiftPM's nested sandbox-exec call.
  swift_sandbox_args+=(--disable-sandbox)
fi

run() {
  printf '\n> '
  printf '%q ' "$@"
  printf '\n'
  "$@"
}

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 2
  fi
}

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This script builds the native macOS arm64 app and must run on macOS." >&2
  exit 2
fi

if [[ "$(uname -m)" != "arm64" ]]; then
  echo "This script must run natively on Apple Silicon (uname -m must be arm64)." >&2
  echo "It intentionally does not use Rosetta or build Intel artifacts." >&2
  exit 2
fi

for command in cmp codesign ditto file install_name_tool lipo otool plutil python3 shasum swift uv; do
  require_command "$command"
done

# Never let a failed new build leave an older bundle looking current.
rm -rf "$APP" "$ZIP"

if [[ "$NOTARIZE" == "1" ]]; then
  require_command spctl
  require_command xcrun
  if [[ "$SIGN_IDENTITY" == "-" ]]; then
    echo "NOTARIZE=1 requires a Developer ID signing identity." >&2
    exit 2
  fi
  : "${APPLE_TEAM_ID:?Set APPLE_TEAM_ID when NOTARIZE=1}"
  : "${NOTARY_PROFILE:?Set NOTARY_PROFILE to a notarytool keychain profile when NOTARIZE=1}"
fi

run python3 "$NATIVE_ROOT/Scripts/generate_daemon_kinds.py" --check
run python3 "$NATIVE_ROOT/Scripts/sync_string_catalog.py" --check
if [[ "${SKIP_TESTS:-0}" != "1" ]]; then
  run swift test "${swift_sandbox_args[@]}" --package-path "$NATIVE_ROOT"
fi

run uv sync --frozen --python "$PYTHON_VERSION"
APP_VERSION="$(
  uv run --python "$PYTHON_VERSION" python -c \
    'import tomllib; print(tomllib.load(open("pyproject.toml", "rb"))["project"]["version"])'
)"
APP_BUILD="$(git rev-list --count HEAD 2>/dev/null || printf 1)"
APP_COMMIT_BASE="$(git rev-parse --short=12 HEAD 2>/dev/null || printf unknown)"
APP_DIRTY=false
GIT_STATUS="$(git status --porcelain --untracked-files=normal 2>/dev/null || true)"
if [[ -n "$GIT_STATUS" ]]; then
  APP_DIRTY=true
  APP_COMMIT="${APP_COMMIT_BASE}-dirty"
else
  APP_COMMIT="$APP_COMMIT_BASE"
fi

if [[ -n "${KASSIBER_SIDECAR_SOURCE:-}" ]]; then
  SIDECAR_SOURCE="$KASSIBER_SIDECAR_SOURCE"
else
  SIDECAR_BUILD_ROOT="$BUILD_ROOT/sidecar"
  mkdir -p \
    "$SIDECAR_BUILD_ROOT/dist" \
    "$SIDECAR_BUILD_ROOT/spec" \
    "$SIDECAR_BUILD_ROOT/work"
  run uv run --python "$PYTHON_VERSION" --with pyinstaller==6.20.0 pyinstaller \
    --clean \
    --noconfirm \
    --onefile \
    --name kassiber-cli \
    --distpath "$SIDECAR_BUILD_ROOT/dist" \
    --workpath "$SIDECAR_BUILD_ROOT/work" \
    --specpath "$SIDECAR_BUILD_ROOT/spec" \
    --paths . \
    --collect-data kassiber \
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
  SIDECAR_SOURCE="$SIDECAR_BUILD_ROOT/dist/kassiber-cli"
fi

if [[ ! -x "$SIDECAR_SOURCE" ]]; then
  echo "Kassiber sidecar is missing or not executable: $SIDECAR_SOURCE" >&2
  exit 1
fi

sidecar_arch="$(file "$SIDECAR_SOURCE")"
echo "$sidecar_arch"
case "$sidecar_arch" in
  *"arm64"*) ;;
  *)
    echo "The Kassiber sidecar is not an arm64 executable; refusing to bundle it." >&2
    exit 1
    ;;
esac
SIDECAR_SHA256="$(shasum -a 256 "$SIDECAR_SOURCE")"
SIDECAR_SHA256="${SIDECAR_SHA256%% *}"
if [[ -n "${KASSIBER_VERIFIED_SIDECAR_SHA256:-}" ]]; then
  if [[ "$SIDECAR_SHA256" != "$KASSIBER_VERIFIED_SIDECAR_SHA256" ]]; then
    echo "The supplied sidecar does not match KASSIBER_VERIFIED_SIDECAR_SHA256." >&2
    exit 1
  fi
  echo "Using externally verified sidecar SHA-256: $SIDECAR_SHA256"
fi
run "$SIDECAR_SOURCE" --help

smoke_sidecar() {
  local binary="$1"
  local smoke_root
  local smoke_out
  smoke_root="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-native-sidecar-smoke.XXXXXX")"
  smoke_out="$smoke_root/daemon.jsonl"
  if ! printf '%s\n' \
      '{"request_id":"kraken-bundled-1","kind":"ui.rates.kraken_csv.import","args":{"use_bundled":true}}' \
      '{"request_id":"shutdown-1","kind":"daemon.shutdown"}' \
      | "$binary" --data-root "$smoke_root/data" daemon > "$smoke_out"; then
    sed -n '1,120p' "$smoke_out" >&2
    rm -rf "$smoke_root"
    return 1
  fi
  if ! python3 "$NATIVE_ROOT/Scripts/verify_sidecar_smoke.py" \
      --jsonl "$smoke_out" \
      --version "$APP_VERSION" \
      --manifest "$NATIVE_ROOT/Generated/DaemonKinds.generated.json"; then
    sed -n '1,120p' "$smoke_out" >&2
    rm -rf "$smoke_root"
    return 1
  fi
  rm -rf "$smoke_root"
}

run smoke_sidecar "$SIDECAR_SOURCE"
run swift build \
  "${swift_sandbox_args[@]}" \
  --package-path "$NATIVE_ROOT" \
  --configuration release \
  --triple "$TARGET_TRIPLE" \
  --product "$PRODUCT_NAME"

# SwiftPM generates Bundle.module accessors for command-line executables using
# Bundle.main.bundleURL. That would put *.bundle directories beside Contents,
# which codesign rejects. Recompile those generated files against the standard
# Contents/Resources location without asking SwiftPM to regenerate them.
run python3 "$NATIVE_ROOT/Scripts/rebuild_swiftpm_resource_accessors.py" \
  --build-dir "$SWIFT_BUILD_DIR" \
  --product "$PRODUCT_NAME"

APP_EXECUTABLE_SOURCE="$SWIFT_BUILD_DIR/$PRODUCT_NAME"
if [[ ! -x "$APP_EXECUTABLE_SOURCE" ]]; then
  echo "Swift build did not produce $APP_EXECUTABLE_SOURCE" >&2
  exit 1
fi

app_arch="$(file "$APP_EXECUTABLE_SOURCE")"
echo "$app_arch"
case "$app_arch" in
  *"arm64"*) ;;
  *)
    echo "The native app executable is not arm64; refusing to package it." >&2
    exit 1
    ;;
esac

TAURI_ICON_PNG="$REPO_ROOT/ui-tauri/src-tauri/icons/icon.png"
TAURI_ICON_ICNS="$REPO_ROOT/ui-tauri/src-tauri/icons/icon.icns"
NATIVE_ICON_PNG="$NATIVE_ROOT/Sources/KassiberApp/Resources/AppIcon-1024.png"
if ! cmp -s "$TAURI_ICON_PNG" "$NATIVE_ICON_PNG"; then
  echo "Native runtime icon drifted from ui-tauri/src-tauri/icons/icon.png." >&2
  echo "Copy the Tauri icon byte-for-byte before packaging." >&2
  exit 1
fi

mkdir -p \
  "$APP/Contents/MacOS" \
  "$APP/Contents/Resources" \
  "$APP/Contents/Frameworks"

cp "$NATIVE_ROOT/Resources/Info.plist" "$APP/Contents/Info.plist"
cp "$NATIVE_ROOT/Resources/PkgInfo" "$APP/Contents/PkgInfo"
/usr/libexec/PlistBuddy -c "Set :CFBundleDisplayName $PRODUCT_NAME" "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleExecutable $PRODUCT_NAME" "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier ${BUNDLE_IDENTIFIER:-at.bitcoinaustria.kassiber.native}" "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleURLTypes:0:CFBundleURLName ${BUNDLE_IDENTIFIER:-at.bitcoinaustria.kassiber.native}" "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleName $PRODUCT_NAME" "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $APP_VERSION" "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $APP_BUILD" "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Add :KassiberBuildCommit string $APP_COMMIT" "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Add :KassiberBuildDirty bool $APP_DIRTY" "$APP/Contents/Info.plist"
if [[ "$SIGN_IDENTITY" == "-" ]]; then
  /usr/libexec/PlistBuddy -c "Add :KassiberSigningIdentityStrength string adhoc" "$APP/Contents/Info.plist"
else
  /usr/libexec/PlistBuddy -c "Add :KassiberSigningIdentityStrength string production" "$APP/Contents/Info.plist"
fi
run plutil -lint "$APP/Contents/Info.plist"

cp "$APP_EXECUTABLE_SOURCE" "$APP/Contents/MacOS/$PRODUCT_NAME"
cp "$SIDECAR_SOURCE" "$APP/Contents/Resources/kassiber-sidecar"
cp "$TAURI_ICON_ICNS" "$APP/Contents/Resources/AppIcon.icns"
cp "$REPO_ROOT/LICENSE" "$APP/Contents/Resources/Kassiber-LICENSE"
chmod 755 "$APP/Contents/MacOS/$PRODUCT_NAME" "$APP/Contents/Resources/kassiber-sidecar"

# Preserve direct native dependency notices in the redistributable bundle.
LICENSES_DIR="$APP/Contents/Resources/ThirdPartyLicenses"
mkdir -p "$LICENSES_DIR"
cp "$REPO_ROOT/THIRD_PARTY_LICENSES.md" "$LICENSES_DIR/Kassiber-Third-Party-Licenses.md"
cp "$NATIVE_ROOT/.build/checkouts/textual/LICENSE" "$LICENSES_DIR/Textual-LICENSE"
cp "$NATIVE_ROOT/.build/checkouts/textual/LICENSE-3rdparty.csv" "$LICENSES_DIR/Textual-LICENSE-3rdparty.csv"
cp "$NATIVE_ROOT/.build/checkouts/swiftui-math/LICENSE" "$LICENSES_DIR/SwiftUI-Math-LICENSE"
cp "$NATIVE_ROOT/.build/checkouts/swift-concurrency-extras/LICENSE" "$LICENSES_DIR/Swift-Concurrency-Extras-LICENSE"
cp "$NATIVE_ROOT/Resources/Prism-LICENSE" "$LICENSES_DIR/Prism-LICENSE"
cp "$NATIVE_ROOT/.build/checkouts/Sparkle/LICENSE" "$LICENSES_DIR/Sparkle-LICENSE"

# The patched accessors resolve these standard macOS resource locations.
shopt -s nullglob
resource_bundles=("$SWIFT_BUILD_DIR"/*.bundle)
if (( ${#resource_bundles[@]} == 0 )); then
  echo "Swift build produced no resource bundles." >&2
  exit 1
fi
for bundle in "${resource_bundles[@]}"; do
  run ditto "$bundle" "$APP/Contents/Resources/$(basename "$bundle")"
done

packaged_icon_pngs=()
while IFS= read -r -d '' packaged_icon_png; do
  packaged_icon_pngs+=("$packaged_icon_png")
done < <(find "$APP/Contents/Resources" -type f -name 'AppIcon-1024.png' -print0)
if (( ${#packaged_icon_pngs[@]} != 1 )) || ! cmp -s "${packaged_icon_pngs[0]:-}" "$TAURI_ICON_PNG"; then
  echo "Packaged runtime icon is not the exact Tauri icon.png." >&2
  exit 1
fi

if [[ ! -d "$SWIFT_BUILD_DIR/Sparkle.framework" ]]; then
  echo "Swift build produced no Sparkle.framework even though the app links it." >&2
  exit 1
fi
run ditto "$SWIFT_BUILD_DIR/Sparkle.framework" "$APP/Contents/Frameworks/Sparkle.framework"

# Sparkle's binary XCFramework slice is universal. The product is explicitly
# Apple-Silicon-only, so thin every embedded Mach-O before signing and reject
# any non-arm64 executable that remains in the bundle.
thin_macho_tree_to_arm64() {
  local root="$1"
  local candidate
  local description
  local archs
  while IFS= read -r -d '' candidate; do
    description="$(file "$candidate")"
    case "$description" in
      *"Mach-O"*)
        archs="$(lipo -archs "$candidate")"
        case " $archs " in
          *" arm64 "*) ;;
          *)
            echo "Embedded Mach-O has no arm64 slice: $candidate ($archs)" >&2
            return 1
            ;;
        esac
        if [[ "$archs" != "arm64" ]]; then
          run lipo "$candidate" -thin arm64 -output "$candidate.arm64"
          run mv "$candidate.arm64" "$candidate"
        fi
        ;;
    esac
  done < <(find "$root" -type f -print0)
}

verify_macho_tree_arm64_only() {
  local root="$1"
  local candidate
  local description
  local archs
  while IFS= read -r -d '' candidate; do
    description="$(file "$candidate")"
    case "$description" in
      *"Mach-O"*)
        archs="$(lipo -archs "$candidate")"
        if [[ "$archs" != "arm64" ]]; then
          echo "Embedded Mach-O is not arm64-only: $candidate ($archs)" >&2
          return 1
        fi
        ;;
    esac
  done < <(find "$root" -type f -print0)
}

run thin_macho_tree_to_arm64 "$APP/Contents/Frameworks/Sparkle.framework"
run verify_macho_tree_arm64_only "$APP/Contents"

# LaunchServices can evaluate an app executable without preserving the
# SwiftPM/Xcode run-path context that makes the original `@rpath` Sparkle
# dependency work during `swift run`. Bind the host executable directly to
# the framework inside this bundle so Finder/open and direct child launches
# both resolve the same relocatable path.
SPARKLE_DEPENDENCY='@rpath/Sparkle.framework/Versions/B/Sparkle'
SPARKLE_BUNDLE_DEPENDENCY='@loader_path/../Frameworks/Sparkle.framework/Versions/B/Sparkle'
if otool -L "$APP/Contents/MacOS/$PRODUCT_NAME" | grep -Fq "$SPARKLE_DEPENDENCY"; then
  run install_name_tool -change "$SPARKLE_DEPENDENCY" \
    "$SPARKLE_BUNDLE_DEPENDENCY" "$APP/Contents/MacOS/$PRODUCT_NAME"
fi

if ! otool -l "$APP/Contents/MacOS/$PRODUCT_NAME" | grep -Fq '@executable_path/../Frameworks'; then
  run install_name_tool -add_rpath '@executable_path/../Frameworks' "$APP/Contents/MacOS/$PRODUCT_NAME"
fi

sign_path() {
  local path="$1"
  shift
  if [[ "$SIGN_IDENTITY" == "-" ]]; then
    # Hardened runtime library validation cannot establish a shared Team ID
    # between independently ad-hoc-signed app/framework code. Keep local
    # signatures launchable; Developer ID builds below enable hardened runtime.
    run codesign --force --sign - "$@" "$path"
  else
    run codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$@" "$path"
  fi
}

# Sign nested code first, then seal the outer bundle. Sparkle's helper app,
# XPC services, and Autoupdate executable enforce IPC boundaries, so they keep
# their identifiers, requirements, and entitlements while receiving the same
# signing identity as the host app.
SPARKLE_FRAMEWORK="$APP/Contents/Frameworks/Sparkle.framework"
SPARKLE_VERSION="$SPARKLE_FRAMEWORK/Versions/Current"
sparkle_components=(
  "$SPARKLE_VERSION/Updater.app"
  "$SPARKLE_VERSION/XPCServices/Downloader.xpc"
  "$SPARKLE_VERSION/XPCServices/Installer.xpc"
  "$SPARKLE_VERSION/Autoupdate"
)
for component in "${sparkle_components[@]}"; do
  if [[ ! -e "$component" ]]; then
    echo "Sparkle signing component is missing: $component" >&2
    exit 1
  fi
  sign_path "$component" \
    --preserve-metadata=identifier,entitlements,requirements
done
sign_path "$SPARKLE_FRAMEWORK" \
  --preserve-metadata=identifier,entitlements,requirements
sign_path "$APP/Contents/Resources/kassiber-sidecar"
sign_path "$APP/Contents/MacOS/$PRODUCT_NAME"
sign_path "$APP"
run codesign --verify --deep --strict --verbose=2 "$APP"
run verify_macho_tree_arm64_only "$APP/Contents"
run smoke_sidecar "$APP/Contents/Resources/kassiber-sidecar"

if [[ "$CREATE_ZIP" == "1" || "$NOTARIZE" == "1" ]]; then
  # Avoid AppleDouble `._*` entries from extended attributes; the verifier
  # binds the archive to the exact signed app bundle tree.
  run env COPYFILE_DISABLE=1 ditto --norsrc -c -k --keepParent "$APP" "$ZIP"
fi

if [[ "$NOTARIZE" == "1" ]]; then
  run xcrun notarytool submit "$ZIP" \
    --keychain-profile "$NOTARY_PROFILE" \
    --team-id "$APPLE_TEAM_ID" \
    --wait
  run xcrun stapler staple "$APP"
  run xcrun stapler validate "$APP"
  run spctl --assess --type execute --verbose=2 "$APP"
  # Recreate the archive so it contains the stapled app.
  rm -f "$ZIP"
  run env COPYFILE_DISABLE=1 ditto --norsrc -c -k --keepParent "$APP" "$ZIP"
fi

cat <<EOF

macOS arm64 native build complete.

Product:      $PRODUCT_NAME
Bundle:       $APP
Bundle ID:    ${BUNDLE_IDENTIFIER:-at.bitcoinaustria.kassiber.native}
Version:      $APP_VERSION ($APP_BUILD, $APP_COMMIT)
Architecture: arm64
Signing:      $SIGN_IDENTITY
Sidecar:      Contents/Resources/kassiber-sidecar
Icon:         exact Tauri icon.icns + icon.png source
EOF

if [[ -f "$ZIP" ]]; then
  echo "Archive:      $ZIP"
fi
