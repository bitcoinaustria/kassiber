#!/usr/bin/env bash
# Build the Windows x86_64 release artifacts with the Rust backend, the way the
# Windows legs of .github/workflows/prerelease-binaries.yml do, on a Windows
# machine (Git Bash, with the MSVC build tools, rustup, 7-Zip, jq, WiX Toolset v3
# `candle`/`light` and NSIS `makensis` on PATH):
#
#   dist/windows-x64/kassiber-cli-windows-x64.zip
#   dist/windows-x64/kassiber-windows-x64.msi
#   dist/windows-x64/kassiber-windows-x64.exe   (NSIS setup)
#
# Unsigned: Windows code signing is not part of the release flow yet.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

TRIPLE="x86_64-pc-windows-msvc"
TARGET="windows-x64"
output="$ROOT/dist/windows-x64"
channel="dev"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --output) output="${2:-}"; shift 2 ;;
    --channel) channel="${2:-}"; shift 2 ;;
    -h|--help)
      echo "Usage: scripts/build-windows-x64-rust-app.sh [--output DIR] [--channel dev|prerelease|release]"
      exit 0
      ;;
    *) echo "Unknown argument: $1" >&2; exit 2 ;;
  esac
done

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) ;;
  *) echo "This script builds the Windows artifacts and must run in Git Bash on Windows." >&2; exit 2 ;;
esac
for command in cargo rustup 7z jq candle light makensis; do
  command -v "$command" >/dev/null 2>&1 || { echo "Missing required tool: $command" >&2; exit 2; }
done
[ ! -e "$output" ] || { echo "Output directory already exists: $output" >&2; exit 2; }

run() {
  echo
  echo "> $*"
  "$@"
}

version="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' crates/kassiber/Cargo.toml | head -n 1 | tr -d '\r')"
commit="$(git rev-parse HEAD)"
short_commit="$(git rev-parse --short=12 HEAD)"
SOURCE_DATE_EPOCH="$(git show -s --format=%ct HEAD)"
export SOURCE_DATE_EPOCH

RUST_CHANNEL="$(sed -n 's/^channel *= *"\([^"]*\)".*/\1/p' rust-toolchain.toml | tr -d '\r')"
run rustup toolchain install "$RUST_CHANNEL" --profile minimal --component rustfmt --component clippy

work="$ROOT/target/windows-x64-rust-packaging"
case "$work" in
  "$ROOT"/target/*) rm -rf -- "${work:?}" ;;
esac
mkdir -p "$work" "$output"

run "$ROOT/scripts/build-rust-cli.sh" --target "$TRIPLE" --channel "$channel" \
  --commit "$commit" --output "$work/dist"

# The release tool (crates/release) writes BUILD_INFO.json and the manifest.
release_tool="${KASSIBER_RELEASE_TOOL:-}"
if [ -z "$release_tool" ]; then
  run cargo build --locked --release -p kassiber-release --no-default-features --bin kassiber-release
  release_tool="$ROOT/target/release/kassiber-release.exe"
fi
binary="$work/dist/kassiber.exe"

run "$release_tool" build-info --output "$work/BUILD_INFO.json" \
  --version "$version" --commit "$commit" --ref "$(git rev-parse --abbrev-ref HEAD)" \
  --run-id local --channel "$channel"

run "$ROOT/scripts/smoke-packaged-cli.sh" --cli "$binary" --scratch "$work/smoke" \
  --expect-commit "$short_commit" --operator-unlock

run "$ROOT/scripts/package-cli-archive.sh" --binary "$binary" \
  --build-info "$work/BUILD_INFO.json" --name "kassiber-cli-$TARGET" --format zip \
  --output-dir "$output"

# The Rune desktop (without agent tooling) around this exact binary: the
# installers stage it as the sidecar and as the PATH copy (bin/kassiber.exe).
run bash -c 'cd crates/kassiber-desktop && cargo build --release --locked --no-default-features'
run "$ROOT/crates/kassiber-desktop/packaging/windows/build-installers.sh" \
  --rune-bin "${CARGO_TARGET_DIR:-$ROOT/crates/kassiber-desktop/target}/release/kassiber-desktop.exe" \
  --kassiber-bin "$binary" --build-info "$work/BUILD_INFO.json" \
  --version "$version" --output "$output"
"$binary" --version | grep "$short_commit"
test -s "$output/kassiber-$TARGET.msi"
test -s "$output/kassiber-$TARGET.exe"

echo
echo "Windows x86_64 artifacts ($channel, commit $short_commit):"
(cd "$output" && for artifact in *; do printf '  %12s  %s\n' "$(wc -c < "$artifact" | tr -d ' ')" "$artifact"; done)
echo
echo "Install the MSI or the NSIS setup, then check in a new terminal:"
echo "  kassiber --version     (must name commit $short_commit)"
echo "and that Settings -> AI lists the CLI-backed providers (the provider broker answers)."
