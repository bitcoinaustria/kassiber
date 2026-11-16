#!/usr/bin/env bash
# Builds Kassiber.app around the Rune desktop: the shape the Tauri bundle had, so the release
# tooling (`kassiber-release macos sign/notarize/verify`, the Homebrew cask, the CLI archive)
# takes it unchanged.
#
#   Kassiber.app/Contents/
#     Info.plist                      packaging/macos/Info.plist.in (bundle id, kassiber:// scheme)
#     PkgInfo
#     MacOS/kassiber-ui               the Rune app, built WITHOUT Rune's `tooling` feature
#     Resources/icon.icns             packaging/icons/icon.icns (the Tauri app's icon)
#     Resources/bin/kassiber          terminal launcher (packaging/macos/kassiber-launcher)
#     Resources/binaries/kassiber-cli/kassiber-cli-aarch64-apple-darwin   the `kassiber` daemon/CLI
#     Resources/binaries/kassiber-cli/BUILD_INFO.json                     build provenance
#
# Signing here is ad hoc only (`codesign -s -`, hardened runtime, no entitlements: the
# Keychain group needs the Developer ID provisioning profile, and an ad-hoc binary that claims it
# is killed at launch). Release signing stays in `kassiber-release macos sign`, which takes the
# `--zip` output, embeds the profile, applies packaging/macos/Kassiber.entitlements to the outer
# app only and signs inside out with the team identity (docs/reference/macos-release.md).
#
# Usage: packaging/macos/build-app.sh --kassiber-bin PATH [options]
#   --kassiber-bin PATH       the `kassiber` binary to bundle (default: $KASSIBER_BIN). Release
#                             builds pass the one scripts/build-rust-cli.sh built and smoked.
#   --rune-bin PATH           a prebuilt Rune app binary (default: cargo build --release
#                             --no-default-features in this crate)
#   --identity release|dev    release: "Kassiber", at.bitcoinaustria.kassiber (default);
#                             dev: "Kassiber Dev", at.bitcoinaustria.kassiber.dev (installs beside)
#   --version V               bundle version (default: the repo's crates/kassiber version, if any)
#   --commit SHA              full commit for BUILD_INFO.json (default: git HEAD, if any)
#   --channel dev|prerelease|release   BUILD_INFO.json channel (default dev; release tooling
#                             refuses dev artifacts)
#   --build-info PATH         use this BUILD_INFO.json (CI's, from `kassiber-release build-info`)
#   --provisioning-profile P  place P at Contents/embedded.provisionprofile (the slot
#                             `kassiber-release macos sign` fills; normally left to it)
#   --unsigned                leave the bundle unsealed (input for `kassiber-release macos
#                             sign`); its two Mach-Os still carry ad-hoc signatures
#   --output DIR              default: <crate>/dist/macos-arm64
#   --zip                     also write kassiber-macos-arm64.app.zip (ditto, --keepParent)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
crate="$(cd "$here/../.." && pwd)"
# The Kassiber checkout (version, commit): the crate sits at crates/kassiber-desktop in it.
repo="${KASSIBER_REPO:-$(cd "$crate/../.." && pwd)}"

kassiber_bin="${KASSIBER_BIN:-}"
rune_bin=""
identity=release
version=""
commit=""
channel=dev
build_info=""
profile=""
sign=1
output="$crate/dist/macos-arm64"
make_zip=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --kassiber-bin) kassiber_bin="$2"; shift ;;
    --rune-bin) rune_bin="$2"; shift ;;
    --identity) identity="$2"; shift ;;
    --version) version="$2"; shift ;;
    --commit) commit="$2"; shift ;;
    --channel) channel="$2"; shift ;;
    --build-info) build_info="$2"; shift ;;
    --provisioning-profile) profile="$2"; shift ;;
    --unsigned) sign=0 ;;
    --output) output="$2"; shift ;;
    --zip) make_zip=1 ;;
    -h|--help) sed -n '2,40p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

die() { echo "build-app: $*" >&2; exit 1; }
[ "$(uname -s)" = Darwin ] || die "builds the macOS app and must run on macOS"
[ "$(uname -m)" = arm64 ] || die "macOS is Apple Silicon only (docs/reference/prerelease-binaries.md)"
for tool in codesign ditto plutil file otool strip; do command -v "$tool" >/dev/null || die "missing $tool"; done

case "$identity" in
  release) product="Kassiber"; bundle_id="at.bitcoinaustria.kassiber" ;;
  dev) product="Kassiber Dev"; bundle_id="at.bitcoinaustria.kassiber.dev" ;;
  *) die "--identity is release or dev" ;;
esac
case "$channel" in dev|prerelease|release) ;; *) die "--channel is dev, prerelease or release" ;; esac
[ -n "$kassiber_bin" ] && [ -x "$kassiber_bin" ] || die "--kassiber-bin (or KASSIBER_BIN) must name the kassiber binary"
if [ -z "$version" ] && [ -f "$repo/crates/kassiber/Cargo.toml" ]; then
  version="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' "$repo/crates/kassiber/Cargo.toml" | head -n1)"
fi
[ -n "$version" ] || die "--version is required outside the Kassiber checkout"
if [ -z "$commit" ]; then commit="$(git -C "$repo" rev-parse HEAD 2>/dev/null || echo unknown)"; fi

sidecar_name="kassiber-cli-aarch64-apple-darwin"
min_os=11.0

# --- the Rune binary, without agent tooling -------------------------------------------------
if [ -z "$rune_bin" ]; then
  (cd "$crate" && cargo build --release --locked --no-default-features)
  rune_bin="${CARGO_TARGET_DIR:-$crate/target}/release/kassiber-desktop"
fi
[ -x "$rune_bin" ] || die "no Rune binary at $rune_bin"
# A shipped app must refuse every headless/scripted run (Rune prints WITHOUT_TOOLING, exits 1).
if probe="$(cd "${TMPDIR:-/tmp}" && "$rune_bin" --tree /dev/null 2>&1)"; then
  die "the Rune binary accepted --tree: it was built with the tooling feature"
fi
case "$probe" in *"no agent tooling"*) ;; *) die "unexpected answer to --tree: $probe" ;; esac

# --- both binaries: arm64, system libraries only, deployment target -------------------------
for bin in "$rune_bin" "$kassiber_bin"; do
  case "$(file -b "$bin")" in *"Mach-O 64-bit executable arm64"*) ;; *) die "$bin is not an arm64 Mach-O executable" ;; esac
  if otool -L "$bin" | tail -n +2 | awk '{print $1}' | grep -vE '^(/usr/lib/|/System/Library/)' | grep -q .; then
    die "$bin links a non-system library"
  fi
  minos="$(vtool -show-build "$bin" 2>/dev/null | awk '/minos/ {print $2; exit}')"
  if [ -n "$minos" ] && [ "$(printf '%s\n%s\n' "$minos" "$min_os" | sort -V | tail -n1)" != "$min_os" ]; then
    die "$bin needs macOS $minos, above the declared $min_os"
  fi
done
cli_version="$("$kassiber_bin" --version)"
case "$cli_version" in "Kassiber $version"*) ;; *) echo "build-app: note: $kassiber_bin reports '$cli_version', bundle version is $version" >&2 ;; esac

# --- assemble ------------------------------------------------------------------------------
mkdir -p "$output"
stage="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-app.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
app="$stage/$product.app"
contents="$app/Contents"
mkdir -p "$contents/MacOS" "$contents/Resources/bin" "$contents/Resources/binaries/kassiber-cli"

sed -e "s|@@PRODUCT_NAME@@|$product|g" -e "s|@@BUNDLE_ID@@|$bundle_id|g" \
    -e "s|@@VERSION@@|$version|g" -e "s|@@MIN_OS@@|$min_os|g" \
    "$here/Info.plist.in" > "$contents/Info.plist"
plutil -lint -s "$contents/Info.plist"
printf 'APPL????' > "$contents/PkgInfo"

cp "$rune_bin" "$contents/MacOS/kassiber-ui"
strip -x "$contents/MacOS/kassiber-ui"
chmod 755 "$contents/MacOS/kassiber-ui"
cp "$crate/packaging/icons/icon.icns" "$contents/Resources/icon.icns"
cp "$here/kassiber-launcher" "$contents/Resources/bin/kassiber"
chmod 755 "$contents/Resources/bin/kassiber"

sidecar="$contents/Resources/binaries/kassiber-cli/$sidecar_name"
cp "$kassiber_bin" "$sidecar"
# Release CLI builds come out of scripts/build-rust-cli.sh already stripped; a local cargo
# build doesn't. `strip -x` is idempotent.
strip -x "$sidecar"
chmod 755 "$sidecar"
if [ -n "$build_info" ]; then
  cp "$build_info" "$contents/Resources/binaries/kassiber-cli/BUILD_INFO.json"
else
  built_at="$(date -u -r "${SOURCE_DATE_EPOCH:-$(date +%s)}" +%Y-%m-%dT%H:%M:%SZ)"
  ref="$(git -C "$repo" rev-parse --abbrev-ref HEAD 2>/dev/null || echo local)"
  # The schema `kassiber-release build-info` writes (crates/release notes.rs).
  printf '{\n  "schema_version": 1,\n  "version": "%s",\n  "channel": "%s",\n  "commit": "%s",\n  "ref": "%s",\n  "run_id": "local",\n  "built_at": "%s"\n}\n' \
    "$version" "$channel" "$commit" "$ref" "$built_at" > "$contents/Resources/binaries/kassiber-cli/BUILD_INFO.json"
fi
if [ -n "$profile" ]; then
  cp "$profile" "$contents/embedded.provisionprofile"
fi

# The release signer refuses symlinks and anything but files and directories in the app zip.
if find "$app" -type l | grep -q .; then die "the bundle contains a symlink"; fi

# --- ad-hoc signature, inside out ----------------------------------------------------------
# `strip` invalidated both binaries' linker signatures, and arm64 code must be signed to run at
# all, so each Mach-O always gets an ad-hoc signature under its release identifier (as
# scripts/build-rust-cli.sh signs the CLI). `--unsigned` only leaves the bundle unsealed.
# The sidecar's identifier is its file name: the Touch ID helper's code requirement and the
# Keychain items the earlier sidecar wrote are bound to it (rust-packaging.md, "macOS arm64").
codesign --force --sign - --options runtime --identifier "$sidecar_name" "$sidecar"
codesign --force --sign - --options runtime --identifier "$bundle_id" "$contents/MacOS/kassiber-ui"
if [ "$sign" -eq 1 ]; then
  codesign --force --sign - --options runtime --identifier "$bundle_id" "$app"
  codesign --verify --deep --strict "$app"
  ident="$(codesign -d --verbose=2 "$sidecar" 2>&1 | sed -n 's/^Identifier=//p')"
  [ "$ident" = "$sidecar_name" ] || die "sidecar signed as '$ident'"
  ident="$(codesign -d --verbose=2 "$app" 2>&1 | sed -n 's/^Identifier=//p')"
  [ "$ident" = "$bundle_id" ] || die "app signed as '$ident'"
fi

# --- smoke: terminal forwarding through both entry points ----------------------------------
"$contents/Resources/bin/kassiber" --version | grep -q "Kassiber" || die "the bin/kassiber launcher did not reach the sidecar"
"$contents/MacOS/kassiber-ui" --cli --version | grep -q "Kassiber" || die "kassiber-ui --cli did not reach the sidecar"

rm -rf "${output:?}/$product.app"
ditto "$app" "$output/$product.app"
if [ "$make_zip" -eq 1 ]; then
  [ "$product" = Kassiber ] || die "--zip needs --identity release (kassiber-release expects Kassiber.app)"
  rm -f "$output/kassiber-macos-arm64.app.zip"
  # No AppleDouble members for local extended attributes (provenance, quarantine).
  ditto -c -k --norsrc --noextattr --keepParent "$output/$product.app" "$output/kassiber-macos-arm64.app.zip"
fi
echo "built $output/$product.app ($bundle_id $version, sidecar $sidecar_name, channel $channel)"
