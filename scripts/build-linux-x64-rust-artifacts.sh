#!/usr/bin/env bash
# Build every Linux x86_64 release artifact with the Rust backend, the way
# .github/workflows/prerelease-binaries.yml does, on a local machine:
#
#   kassiber-cli-linux-x64.tar.gz   kassiber-linux-x64.deb
#   kassiber-cli-linux-x64.deb      kassiber-linux-x64.rpm
#   kassiber-cli-linux-x64.rpm      kassiber-linux-x64.AppImage
#   kassiber-<version>-manifest.txt (unsigned; see release-signing.md)
#
# Release artifacts must be built on the glibc 2.35 floor (Ubuntu 22.04, as
# CI does); on a newer host the floor check fails before anything is
# packaged. `--glibc-floor <host version>` builds a local, functional set that
# must never be published.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TRIPLE="x86_64-unknown-linux-gnu"
TARGET="linux-x64"
output="$ROOT/dist/linux-x64"
channel="dev"
glibc_floor="2.35"
allow_partial=0

usage() {
  cat >&2 <<'EOF'
Usage: scripts/build-linux-x64-rust-artifacts.sh [--output DIR] [--channel dev|prerelease|release]
                                                 [--glibc-floor VERSION] [--allow-partial]

  --output DIR        Where the artifacts go (default dist/linux-x64; must not exist).
  --glibc-floor V     Accept a binary up to glibc V. Anything above 2.35 is a
                      local functional build, not a release artifact.
  --allow-partial     Skip artifacts whose packaging tool is missing instead of
                      stopping (dpkg-deb, rpmbuild, Tauri's cached linuxdeploy).
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --output) output="${2:-}"; shift 2 ;;
    --channel) channel="${2:-}"; shift 2 ;;
    --glibc-floor) glibc_floor="${2:-}"; shift 2 ;;
    --allow-partial) allow_partial=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage; exit 2 ;;
  esac
done

[ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ] || {
  echo "This script builds the Linux x86_64 artifacts and must run on Linux x86_64." >&2
  exit 2
}
[ ! -e "$output" ] || { echo "Output directory already exists: $output" >&2; exit 2; }

run() {
  echo
  echo "> $*"
  "$@"
}

# Preflight: name every missing tool before the long build.
missing_required=()
for command in cargo pnpm python3 readelf objdump tar gzip sha256sum; do
  command -v "$command" >/dev/null 2>&1 || missing_required+=("$command")
done
if [ "${#missing_required[@]}" -gt 0 ]; then
  echo "Missing required tools: ${missing_required[*]}" >&2
  exit 2
fi
want_deb=1
want_rpm=1
want_appimage=1
missing_optional=()
if ! command -v dpkg-deb >/dev/null 2>&1; then
  want_deb=0
  missing_optional+=("dpkg-deb (CLI .deb; the desktop .rpm is built from the desktop .deb)")
fi
for command in rpmbuild rpm2cpio cpio; do
  if ! command -v "$command" >/dev/null 2>&1; then
    want_rpm=0
    missing_optional+=("$command (CLI and desktop .rpm)")
  fi
done
# Tauri downloads linuxdeploy and its plugins into ~/.cache/tauri on first
# use. That download is a network fetch this script never triggers itself.
tauri_cache="${XDG_CACHE_HOME:-$HOME/.cache}/tauri"
for tool in linuxdeploy-x86_64.AppImage linuxdeploy-plugin-appimage.AppImage linuxdeploy-plugin-gtk.sh; do
  if [ ! -x "$tauri_cache/$tool" ]; then
    want_appimage=0
    missing_optional+=("$tauri_cache/$tool (AppImage; Tauri would download it)")
  fi
done
if [ "${#missing_optional[@]}" -gt 0 ]; then
  printf 'Missing packaging tools:\n' >&2
  printf '  %s\n' "${missing_optional[@]}" >&2
  if [ "$allow_partial" -ne 1 ]; then
    echo "Install them, or pass --allow-partial to build the rest." >&2
    exit 2
  fi
fi

version="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' crates/kassiber/Cargo.toml | head -n 1)"
commit="$(git rev-parse HEAD)"
short_commit="$(git rev-parse --short=12 HEAD)"
SOURCE_DATE_EPOCH="$(git show -s --format=%ct HEAD)"
export SOURCE_DATE_EPOCH
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "Note: the checkout has uncommitted changes; the build is labelled $short_commit anyway."
fi

work="$ROOT/target/linux-x64-rust-packaging"
case "$work" in
  "$ROOT"/target/*) rm -rf -- "${work:?}" ;;
esac
mkdir -p "$work" "$output"
release="$output"

run "$ROOT/scripts/build-rust-cli.sh" --target "$TRIPLE" --channel "$channel" \
  --commit "$commit" --output "$work/dist" --glibc-floor "$glibc_floor"
binary="$work/dist/kassiber"

run python3 scripts/write_build_info.py --output "$work/BUILD_INFO.json" \
  --version "$version" --commit "$commit" --ref "$(git rev-parse --abbrev-ref HEAD)" \
  --run-id local --channel "$channel"

run pnpm --dir ui-tauri install --frozen-lockfile
run pnpm --dir ui-tauri run broker:build
broker="$ROOT/kassiber/ai/provider_broker/index.mjs"
test -s "$broker"

smoke_root="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-linux-smoke.XXXXXX")"
trap 'rm -rf "$smoke_root"' EXIT
run "$ROOT/scripts/smoke-packaged-cli.sh" --cli "$binary" --scratch "$smoke_root/cli" \
  --expect-commit "$short_commit"
run env KASSIBER_PACKAGED_CLI="$binary" TMPDIR="$smoke_root" \
  python3 -m unittest -v tests.test_packaged_cli_no_egress

name="kassiber-cli-$TARGET"
run "$ROOT/scripts/package-cli-archive.sh" --binary "$binary" --broker "$broker" \
  --build-info "$work/BUILD_INFO.json" --name "$name" --format tar --output-dir "$release"
if [ "$want_deb" -eq 1 ]; then
  run "$ROOT/scripts/package-cli-deb.sh" --binary "$binary" --broker "$broker" \
    --version "$version" --architecture amd64 --output "$release/$name.deb"
fi
if [ "$want_rpm" -eq 1 ]; then
  run "$ROOT/scripts/package-cli-rpm.sh" --binary "$binary" --broker "$broker" \
    --version "$version" --architecture x86_64 --output "$release/$name.rpm"
fi

# Desktop: the same binary as the flat sidecar, with the broker and the build
# record beside it, then the Tauri bundles.
binaries="$ROOT/ui-tauri/src-tauri/binaries"
find "$binaries" -mindepth 1 -maxdepth 1 ! -name .gitignore ! -name .gitkeep -exec rm -rf -- {} +
mkdir -p "$binaries/provider_broker"
cp "$binary" "$binaries/kassiber-cli-$TRIPLE"
chmod 755 "$binaries/kassiber-cli-$TRIPLE"
cp "$broker" "$binaries/provider_broker/index.mjs"
cp "$work/BUILD_INFO.json" "$binaries/BUILD_INFO.json"

bundles="deb"
if [ "$want_appimage" -eq 1 ]; then
  bundles="deb,appimage"
fi
tauri_config="$work/tauri-build.json"
printf '{ "version": "%s" }\n' "$version" > "$tauri_config"
run env KASSIBER_BUILD_VERSION="$channel" KASSIBER_BUILD_COMMIT="$short_commit" \
  pnpm --dir ui-tauri tauri build --bundles "$bundles" --ci --config "$tauri_config"

bundle_dir="$ROOT/ui-tauri/src-tauri/target/release/bundle"
desktop_deb="$(find "$bundle_dir/deb" -maxdepth 1 -type f -name '*.deb' -print -quit)"
test -n "$desktop_deb"
cp "$desktop_deb" "$release/kassiber-$TARGET.deb"

# The deb payload: shell and sidecar on the floor, broker beside the sidecar,
# and the sidecar is byte-for-byte the CLI built above.
payload="$work/desktop-deb"
mkdir -p "$payload"
(cd "$payload" && ar x "$desktop_deb" && tar -xf data.tar.* && rm -f data.tar.* control.tar.* debian-binary)
"$ROOT/scripts/check-linux-glibc-floor.sh" --any-libraries "$payload/usr/bin/kassiber-ui" "$glibc_floor"
sidecar="$payload/usr/lib/Kassiber/binaries/kassiber-cli-$TRIPLE"
"$ROOT/scripts/check-linux-glibc-floor.sh" "$sidecar" "$glibc_floor"
cmp "$sidecar" "$binary"
test -s "$payload/usr/lib/Kassiber/binaries/provider_broker/index.mjs"
test -x "$payload/usr/bin/kassiber"

if [ "$want_appimage" -eq 1 ]; then
  appdir="$(find "$bundle_dir/appimage" -maxdepth 1 -type d -name '*.AppDir' -print -quit)"
  appimage="$(find "$bundle_dir/appimage" -maxdepth 1 -type f -name '*.AppImage' -print -quit)"
  # Same reseal as CI: drop the bundled Wayland client so the host's is used.
  rm "$appdir/usr/lib/libwayland-client.so.0"
  APPIMAGE_EXTRACT_AND_RUN=1 LDAI_OUTPUT="$appimage" ARCH=x86_64 \
    "$tauri_cache/linuxdeploy-plugin-appimage.AppImage" --appimage-extract-and-run --appdir="$appdir"
  cp "$appimage" "$release/kassiber-$TARGET.AppImage"
  chmod 755 "$release/kassiber-$TARGET.AppImage"
  APPIMAGE_EXTRACT_AND_RUN=1 "$release/kassiber-$TARGET.AppImage" --cli --version | grep "$short_commit"
fi

if [ "$want_rpm" -eq 1 ]; then
  run "$ROOT/scripts/package-desktop-rpm.sh" --deb "$release/kassiber-$TARGET.deb" \
    --version "$version" --architecture x86_64 --output "$release/kassiber-$TARGET.rpm"
fi

run python3 scripts/release_manifest.py generate --release-dir "$release" --version "$version"

echo
echo "Linux x86_64 artifacts ($channel, commit $short_commit, glibc floor $glibc_floor):"
(cd "$release" && for artifact in *; do printf '  %12s  %s\n' "$(wc -c < "$artifact" | tr -d ' ')" "$artifact"; done)
if [ "$glibc_floor" != "2.35" ]; then
  echo
  echo "WARNING: built against glibc $glibc_floor, above the 2.35 release floor."
  echo "These artifacts are for local testing only; never publish them."
fi
if [ "${#missing_optional[@]}" -gt 0 ]; then
  echo
  echo "Skipped for missing tools:"
  printf '  %s\n' "${missing_optional[@]}"
fi
