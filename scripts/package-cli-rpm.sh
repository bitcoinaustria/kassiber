#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

usage() {
  echo "Usage: $0 --binary PATH --version VERSION --output PATH [--source-output PATH] [--architecture ARCH] [--release RELEASE]" >&2
}

die() {
  echo "$*" >&2
  exit 2
}

binary=""
version=""
output=""
source_output=""
architecture=""
release="1"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --binary) binary="${2:-}"; shift 2 ;;
    --version) version="${2:-}"; shift 2 ;;
    --output) output="${2:-}"; shift 2 ;;
    --source-output) source_output="${2:-}"; shift 2 ;;
    --architecture) architecture="${2:-}"; shift 2 ;;
    --release) release="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "Unknown argument: $1" ;;
  esac
done

[ -n "$binary" ] || die "--binary is required"
[ -x "$binary" ] || die "CLI binary is not executable: $binary"
[ -n "$version" ] || die "--version is required"
[ -n "$output" ] || die "--output is required"
[ ! -e "$output" ] || die "Output path already exists: $output"
if [ -n "$source_output" ]; then
  [ ! -e "$source_output" ] || die "Source output path already exists: $source_output"
fi
case "$version" in
  *[!0-9A-Za-z.+_~^]*|""|*-*) die "Invalid RPM version: $version" ;;
esac
case "$release" in
  *[!0-9A-Za-z.+_~^]*|""|*-*) die "Invalid RPM release: $release" ;;
esac
if [ -z "$architecture" ]; then
  architecture="$(uname -m)"
fi
case "$architecture" in
  amd64) architecture="x86_64" ;;
  arm64) architecture="aarch64" ;;
esac
case "$architecture" in
  x86_64|aarch64) ;;
  *) die "Unsupported RPM architecture: $architecture" ;;
esac
command -v rpmbuild >/dev/null 2>&1 || die "rpmbuild is required"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$script_dir/.." && pwd)"
spec="$root/packaging/linux/rpm/kassiber-cli.spec"
marker="$root/packaging/linux/install-context/rpm-cli.json"
license="$root/LICENSE"
for required in "$spec" "$marker" "$license"; do
  [ -f "$required" ] || die "Required packaging input is missing: $required"
done

topdir="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-cli-rpm.XXXXXX")"
trap 'rm -rf "$topdir"' EXIT
mkdir -p "$topdir/BUILD" "$topdir/BUILDROOT" "$topdir/RPMS" \
  "$topdir/SOURCES" "$topdir/SPECS" "$topdir/SRPMS"
install -m 0755 "$binary" "$topdir/SOURCES/kassiber"
install -m 0644 "$marker" "$topdir/SOURCES/install-context.json"
install -m 0644 "$license" "$topdir/SOURCES/LICENSE"
rendered_spec="$topdir/SPECS/kassiber-cli.spec"
{
  printf '%%global kassiber_version %s\n' "$version"
  printf '%%global kassiber_release %s\n' "$release"
  printf '%%global kassiber_arch %s\n' "$architecture"
  cat "$spec"
} > "$rendered_spec"

mode="-bb"
if [ -n "$source_output" ]; then
  mode="-ba"
fi
rpmbuild "$mode" \
  --define "_topdir $topdir" \
  "$rendered_spec"

built_rpm="$topdir/RPMS/$architecture/kassiber-cli-$version-$release.$architecture.rpm"
[ -f "$built_rpm" ] || die "Expected RPM was not built: $built_rpm"
mkdir -p "$(dirname "$output")"
install -m 0644 "$built_rpm" "$output"
if [ -n "$source_output" ]; then
  built_srpm="$topdir/SRPMS/kassiber-cli-$version-$release.src.rpm"
  [ -f "$built_srpm" ] || die "Expected source RPM was not built: $built_srpm"
  mkdir -p "$(dirname "$source_output")"
  install -m 0644 "$built_srpm" "$source_output"
fi
