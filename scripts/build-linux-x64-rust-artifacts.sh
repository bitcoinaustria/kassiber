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
appimage_runtime="${APPIMAGE_RUNTIME:-}"

usage() {
  cat >&2 <<'EOF'
Usage: scripts/build-linux-x64-rust-artifacts.sh [--output DIR] [--channel dev|prerelease|release]
                                                 [--glibc-floor VERSION] [--allow-partial]
                                                 [--appimage-runtime PATH]

  --output DIR        Where the artifacts go (default dist/linux-x64; must not exist).
  --glibc-floor V     Accept a binary up to glibc V. Anything above 2.35 is a
                      local functional build, not a release artifact.
  --allow-partial     Skip artifacts whose packaging tool is missing instead of
                      stopping (dpkg-deb, rpmbuild, appimagetool).
  --appimage-runtime PATH
                      The AppImage type-2 runtime for appimagetool (default
                      $APPIMAGE_RUNTIME). Use the release the prerelease workflow
                      pins (APPIMAGE_RUNTIME_URL/_SHA256); this script never
                      downloads one, and neither may appimagetool.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --output) output="${2:-}"; shift 2 ;;
    --channel) channel="${2:-}"; shift 2 ;;
    --glibc-floor) glibc_floor="${2:-}"; shift 2 ;;
    --allow-partial) allow_partial=1; shift ;;
    --appimage-runtime) appimage_runtime="${2:-}"; shift 2 ;;
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
for command in cargo jq cc readelf objdump tar gzip sha256sum pkg-config; do
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
  missing_optional+=("dpkg-deb (CLI and desktop .deb and the desktop AppImage; the desktop .rpm is built from the desktop .deb)")
fi
for command in rpmbuild rpm2cpio cpio; do
  if ! command -v "$command" >/dev/null 2>&1; then
    want_rpm=0
    missing_optional+=("$command (CLI and desktop .rpm)")
  fi
done
# The AppImage needs appimagetool on PATH and a runtime file: both are
# downloads this script never makes itself (see --appimage-runtime).
if ! command -v appimagetool >/dev/null 2>&1; then
  want_appimage=0
  missing_optional+=("appimagetool (AppImage)")
fi
if [ ! -f "$appimage_runtime" ]; then
  want_appimage=0
  missing_optional+=("an AppImage runtime file, --appimage-runtime PATH (AppImage)")
fi
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

# The release tool (crates/release) writes BUILD_INFO.json and the manifest.
release_tool="${KASSIBER_RELEASE_TOOL:-}"
if [ -z "$release_tool" ]; then
  run cargo build --locked --release -p kassiber-release --no-default-features --bin kassiber-release
  release_tool="$ROOT/target/release/kassiber-release"
fi
binary="$work/dist/kassiber"

run "$release_tool" build-info --output "$work/BUILD_INFO.json" \
  --version "$version" --commit "$commit" --ref "$(git rev-parse --abbrev-ref HEAD)" \
  --run-id local --channel "$channel"

smoke_root="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-linux-smoke.XXXXXX")"
trap 'rm -rf "$smoke_root"' EXIT
run "$ROOT/scripts/smoke-packaged-cli.sh" --cli "$binary" --scratch "$smoke_root/cli" \
  --expect-commit "$short_commit"
run "$ROOT/scripts/check-packaged-cli-no-egress.sh" --cli "$binary" \
  --scratch "$smoke_root/no-egress"

name="kassiber-cli-$TARGET"
run "$ROOT/scripts/package-cli-archive.sh" --binary "$binary" \
  --build-info "$work/BUILD_INFO.json" --name "$name" --format tar --output-dir "$release"
if [ "$want_deb" -eq 1 ]; then
  run "$ROOT/scripts/package-cli-deb.sh" --binary "$binary" \
    --version "$version" --architecture amd64 --output "$release/$name.deb"
fi
if [ "$want_rpm" -eq 1 ]; then
  run "$ROOT/scripts/package-cli-rpm.sh" --binary "$binary" \
    --version "$version" --architecture x86_64 --output "$release/$name.rpm"
fi

# Desktop: the Rune app (crates/kassiber-desktop, without agent tooling)
# packaged around this exact binary as its sidecar, as CI does.
if [ "$want_deb" -eq 1 ]; then
  run bash -c 'cd crates/kassiber-desktop && cargo build --release --locked --no-default-features'
  rune_bin="${CARGO_TARGET_DIR:-$ROOT/crates/kassiber-desktop/target}/release/kassiber-desktop"
  appimage_args=(--no-appimage)
  if [ "$want_appimage" -eq 1 ]; then
    appimage_args=(--appimage-runtime "$appimage_runtime")
  fi
  desktop_out="$work/desktop"
  run "$ROOT/crates/kassiber-desktop/packaging/linux/build-packages.sh" \
    --rune-bin "$rune_bin" --kassiber-bin "$binary" --build-info "$work/BUILD_INFO.json" \
    --version "$version" --arch amd64 --glibc-floor "$glibc_floor" \
    --output "$desktop_out" "${appimage_args[@]}"
  cp "$desktop_out/kassiber-$TARGET.deb" "$release/kassiber-$TARGET.deb"

  # The sidecar in the deb is byte-for-byte the CLI built above.
  payload="$work/desktop-deb"
  mkdir -p "$payload"
  dpkg-deb -x "$release/kassiber-$TARGET.deb" "$payload"
  cmp "$payload/usr/lib/Kassiber/binaries/kassiber-cli-$TRIPLE" "$binary"
  test -x "$payload/usr/bin/kassiber"

  if [ "$want_appimage" -eq 1 ]; then
    cp "$desktop_out/kassiber-$TARGET.AppImage" "$release/kassiber-$TARGET.AppImage"
    chmod 755 "$release/kassiber-$TARGET.AppImage"
    APPIMAGE_EXTRACT_AND_RUN=1 "$release/kassiber-$TARGET.AppImage" --cli --version | grep "$short_commit"
  fi
fi

if [ "$want_rpm" -eq 1 ] && [ "$want_deb" -eq 1 ]; then
  run "$ROOT/scripts/package-desktop-rpm.sh" --deb "$release/kassiber-$TARGET.deb" \
    --version "$version" --architecture x86_64 --output "$release/kassiber-$TARGET.rpm"
fi

run "$release_tool" manifest generate --release-dir "$release" --version "$version"

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
