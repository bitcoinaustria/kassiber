#!/usr/bin/env bash
# Build the Windows x86_64 release artifacts with the Rust backend, the way the
# Windows legs of .github/workflows/prerelease-binaries.yml do, on a Windows
# machine (Git Bash, with the MSVC build tools, rustup, Node 22, pnpm 10.33.0,
# Python 3.11 and 7-Zip on PATH):
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
for command in cargo rustup pnpm 7z; do
  command -v "$command" >/dev/null 2>&1 || { echo "Missing required tool: $command" >&2; exit 2; }
done
python="${PYTHON:-$(command -v python || command -v python3 || true)}"
[ -n "$python" ] || { echo "Missing required tool: python" >&2; exit 2; }
export PYTHON="$python"
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
binary="$work/dist/kassiber.exe"

run "$python" scripts/write_build_info.py --output "$work/BUILD_INFO.json" \
  --version "$version" --commit "$commit" --ref "$(git rev-parse --abbrev-ref HEAD)" \
  --run-id local --channel "$channel"

run pnpm --dir ui-tauri install --frozen-lockfile
run pnpm --dir ui-tauri run broker:build
broker="$ROOT/kassiber/ai/provider_broker/index.mjs"
test -s "$broker"

run "$ROOT/scripts/smoke-packaged-cli.sh" --cli "$binary" --scratch "$work/smoke" \
  --expect-commit "$short_commit" --operator-unlock

run "$ROOT/scripts/package-cli-archive.sh" --binary "$binary" --broker "$broker" \
  --build-info "$work/BUILD_INFO.json" --name "kassiber-cli-$TARGET" --format zip \
  --output-dir "$output"

# Flat sidecar with the broker and build record beside it, plus the copy the
# installers put on PATH (bin/kassiber.exe), which finds the broker beside
# itself as well.
binaries="$ROOT/ui-tauri/src-tauri/binaries"
find "$binaries" -mindepth 1 -maxdepth 1 ! -name .gitignore ! -name .gitkeep -exec rm -rf -- {} +
mkdir -p "$binaries/provider_broker"
cp "$binary" "$binaries/kassiber-cli-$TRIPLE.exe"
cp "$broker" "$binaries/provider_broker/index.mjs"
cp "$work/BUILD_INFO.json" "$binaries/BUILD_INFO.json"
cp "$binary" "$ROOT/ui-tauri/src-tauri/bin/kassiber.exe"
mkdir -p "$ROOT/ui-tauri/src-tauri/bin/provider_broker"
cp "$broker" "$ROOT/ui-tauri/src-tauri/bin/provider_broker/index.mjs"

tauri_config="$work/tauri-build.json"
printf '{ "version": "%s" }\n' "$version" > "$tauri_config"
run env KASSIBER_BUILD_VERSION="$channel" KASSIBER_BUILD_COMMIT="$short_commit" \
  pnpm --dir ui-tauri tauri build --bundles msi,nsis --ci --config "$tauri_config"

"$ROOT/ui-tauri/src-tauri/bin/kassiber.exe" --version | grep "$short_commit"
bundle_dir="$ROOT/ui-tauri/src-tauri/target/release/bundle"
msi="$(find "$bundle_dir/msi" -maxdepth 1 -type f -name '*.msi' -print -quit)"
nsis="$(find "$bundle_dir/nsis" -maxdepth 1 -type f -name '*.exe' -print -quit)"
test -n "$msi" && test -n "$nsis"
cp "$msi" "$output/kassiber-$TARGET.msi"
cp "$nsis" "$output/kassiber-$TARGET.exe"

echo
echo "Windows x86_64 artifacts ($channel, commit $short_commit):"
(cd "$output" && for artifact in *; do printf '  %12s  %s\n' "$(wc -c < "$artifact" | tr -d ' ')" "$artifact"; done)
echo
echo "Install the MSI or the NSIS setup, then check in a new terminal:"
echo "  kassiber --version     (must name commit $short_commit)"
echo "and that Settings -> AI lists the CLI-backed providers (the broker is found)."
