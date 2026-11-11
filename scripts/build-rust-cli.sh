#!/usr/bin/env bash
# Build the production `kassiber` binary for one release target and refuse it
# unless it reports the expected build identity and links only what the
# target's floor guarantees.
#
# Release CI and the local Linux/Windows packaging runs share this script, so a
# locally verified binary and the published one come from the same command.
# macOS keeps scripts/build-macos-arm64-rust-app.sh for the local app, which
# runs the same cargo invocation.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

usage() {
  cat >&2 <<'EOF'
Usage: scripts/build-rust-cli.sh --target TRIPLE --output DIR
                                 [--channel dev|prerelease|release] [--commit SHA]
                                 [--glibc-floor VERSION]

Builds `cargo build --release --locked -p kassiber --bin kassiber --target TRIPLE`
without the test-harness feature, strips symbols, and copies the executable to
DIR/kassiber (DIR/kassiber.exe on Windows). Linux targets must stay at or below
the glibc floor (default 2.35, the Ubuntu 22.04 AppImage baseline).
EOF
}

target=""
output=""
channel="dev"
commit=""
glibc_floor="2.35"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --target) target="${2:-}"; shift 2 ;;
    --output) output="${2:-}"; shift 2 ;;
    --channel) channel="${2:-}"; shift 2 ;;
    --commit) commit="${2:-}"; shift 2 ;;
    --glibc-floor) glibc_floor="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage; exit 2 ;;
  esac
done
[ -n "$target" ] && [ -n "$output" ] || { usage; exit 2; }
case "$channel" in
  dev|prerelease|release) ;;
  *) echo "Unknown build channel: $channel" >&2; exit 2 ;;
esac
case "$target" in
  x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu) platform=linux; exe_ext="" ;;
  aarch64-apple-darwin) platform=macos; exe_ext="" ;;
  x86_64-pc-windows-msvc) platform=windows; exe_ext=".exe" ;;
  *) echo "Unsupported release target: $target" >&2; exit 2 ;;
esac
if [ -z "$commit" ]; then
  commit="$(git rev-parse HEAD)"
fi
case "$commit" in
  *[!0-9a-f]*|"") echo "--commit must be a lowercase hex commit id" >&2; exit 2 ;;
esac
short_commit="$(printf '%s' "$commit" | cut -c1-12)"

# The product version lives in the backend crate.
version="$(sed -n 's/^version *= *"\([^"]*\)".*/\1/p' crates/kassiber/Cargo.toml | head -n 1)"
if [ -z "$version" ]; then
  echo "crates/kassiber/Cargo.toml names no version." >&2
  exit 1
fi

native_path() {
  # rustc on Windows sees drive-letter paths, not Git Bash's /d/... form.
  if command -v cygpath >/dev/null 2>&1; then cygpath -w "$1"; else printf '%s' "$1"; fi
}
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
# Checkout and registry paths otherwise end up in panic locations, which
# makes the bytes depend on where the build ran.
remap_flags="--remap-path-prefix=$(native_path "$ROOT")=/kassiber --remap-path-prefix=$(native_path "$cargo_home")=/cargo"

# Vendored OpenSSL embeds its build time ("built on: ...") unless
# SOURCE_DATE_EPOCH pins it; the commit time keeps rebuilds byte-identical.
if [ -z "${SOURCE_DATE_EPOCH:-}" ]; then
  SOURCE_DATE_EPOCH="$(git show -s --format=%ct "$commit" 2>/dev/null || git show -s --format=%ct HEAD)"
fi
build_env=(
  SOURCE_DATE_EPOCH="$SOURCE_DATE_EPOCH"
  KASSIBER_BUILD_CHANNEL="$channel"
  KASSIBER_BUILD_COMMIT="$commit"
  RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }$remap_flags"
)
unset_env=()
if [ "$platform" = macos ]; then
  # rustc strips Mach-O through rustup's rust-objcopy, which cannot load
  # libLLVM.dylib from a minimal toolchain and then only warns. Link unstripped
  # and strip with Xcode's strip below.
  build_env+=(CARGO_PROFILE_RELEASE_STRIP=false)
  # SQLCipher uses CommonCrypto on macOS unless an OpenSSL location is set,
  # in which case it would link that (typically Homebrew's) libcrypto. The
  # otool check below refuses such a binary; this keeps it from being built.
  unset_env=(-u OPENSSL_DIR -u OPENSSL_LIB_DIR -u OPENSSL_INCLUDE_DIR -u OPENSSL_STATIC)
else
  build_env+=(CARGO_PROFILE_RELEASE_STRIP=symbols)
fi
if [ "$platform" = windows ] && [ -z "${OPENSSL_SRC_PERL:-}" ]; then
  # Vendored OpenSSL's Configure needs a full Perl. Git Bash puts its own
  # first on PATH, which lacks Locale::Maketext::Simple ("Can't locate
  # Locale/Maketext/Simple.pm"); hosted Windows runners ship Strawberry Perl.
  strawberry="C:/Strawberry/perl/bin/perl.exe"
  if [ -x "$strawberry" ]; then
    build_env+=(OPENSSL_SRC_PERL="$strawberry")
  else
    echo "warning: no Strawberry Perl at $strawberry; set OPENSSL_SRC_PERL to a full Perl if OpenSSL's Configure fails" >&2
  fi
fi

echo "Building kassiber $version ($channel, commit $short_commit) for $target"
# Production binary: no test-harness feature, so no fake store, test kinds,
# probe mode or egress observer. Symbols are stripped; panic messages keep
# their file and line.
env ${unset_env[@]+"${unset_env[@]}"} "${build_env[@]}" \
  cargo build --release --locked -p kassiber --bin kassiber --target "$target"

target_dir="${CARGO_TARGET_DIR:-$ROOT/target}"
if command -v cygpath >/dev/null 2>&1; then
  target_dir="$(cygpath -u "$target_dir")"
fi
built="$target_dir/$target/release/kassiber$exe_ext"
[ -f "$built" ] || { echo "Expected build output is missing: $built" >&2; exit 1; }

mkdir -p "$output"
binary="$output/kassiber$exe_ext"
cp "$built" "$binary"
chmod 755 "$binary"

case "$platform" in
  linux)
    expected_machine="X86-64"
    [ "$target" = aarch64-unknown-linux-gnu ] && expected_machine="AArch64"
    readelf -h "$binary" | grep -q "Machine:.*$expected_machine" || {
      echo "$binary is not a $expected_machine ELF executable." >&2
      exit 1
    }
    "$ROOT/scripts/check-linux-glibc-floor.sh" "$binary" "$glibc_floor"
    ;;
  macos)
    # Stripping invalidates the linker's ad-hoc signature, which arm64 needs to
    # run at all. Re-sign ad hoc under the sidecar's signing identifier: the
    # release signer keeps that identifier (the file name), and the Touch ID
    # helper and Keychain items the Python sidecar wrote accept exactly it.
    /usr/bin/strip -x "$binary"
    /usr/bin/codesign --force --sign - --identifier "kassiber-cli-$target" "$binary"
    file "$binary" | grep -q "Mach-O 64-bit executable arm64" || {
      echo "$binary is not an arm64 Mach-O executable." >&2
      exit 1
    }
    # A Homebrew or other local dylib would load on the build Mac only and
    # fail Gatekeeper's library validation everywhere else.
    foreign="$(otool -L "$binary" | tail -n +2 | awk '{print $1}' \
      | grep -vE '^(/usr/lib/|/System/Library/)' || true)"
    if [ -n "$foreign" ]; then
      echo "$binary links libraries that are not part of macOS:" >&2
      printf '%s\n' "$foreign" >&2
      exit 1
    fi
    ;;
  windows)
    if command -v file >/dev/null 2>&1; then
      file "$binary" | grep -q "PE32+ executable.*x86-64" || {
        echo "$binary is not an x86-64 PE32+ executable." >&2
        exit 1
      }
    fi
    ;;
esac

version_text="$("$binary" --version | tr -d '\r')"
expected_version="Kassiber $version ($channel, commit $short_commit)"
if [ "$version_text" != "$expected_version" ]; then
  echo "Unexpected --version output: '$version_text' (expected '$expected_version')" >&2
  exit 1
fi
"$binary" --help >/dev/null
echo "$version_text"
echo "Built $binary ($(wc -c < "$binary" | tr -d ' ') bytes)"
