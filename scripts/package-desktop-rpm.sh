#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

usage() {
  echo "Usage: $0 --deb PATH --version VERSION --output PATH [--source-output PATH] [--architecture ARCH] [--release RELEASE]" >&2
}

die() {
  echo "$*" >&2
  exit 2
}

deb=""
version=""
output=""
source_output=""
architecture=""
release="1"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --deb) deb="${2:-}"; shift 2 ;;
    --version) version="${2:-}"; shift 2 ;;
    --output) output="${2:-}"; shift 2 ;;
    --source-output) source_output="${2:-}"; shift 2 ;;
    --architecture) architecture="${2:-}"; shift 2 ;;
    --release) release="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "Unknown argument: $1" ;;
  esac
done

[ -n "$deb" ] || die "--deb is required"
[ -f "$deb" ] || die "Desktop Debian package does not exist: $deb"
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
for command in cmp dpkg-deb rpmbuild tar; do
  command -v "$command" >/dev/null 2>&1 || die "$command is required"
done

package_name="$(dpkg-deb -f "$deb" Package)"
deb_version="$(dpkg-deb -f "$deb" Version)"
deb_architecture="$(dpkg-deb -f "$deb" Architecture)"
[ "$package_name" = "kassiber" ] || die "Expected package kassiber, got: $package_name"
[ "$deb_version" = "$version" ] || die "Debian package version $deb_version does not match $version"
case "$deb_architecture" in
  amd64) expected_architecture="x86_64" ;;
  arm64) expected_architecture="aarch64" ;;
  *) die "Unsupported Debian package architecture: $deb_architecture" ;;
esac
if [ -z "$architecture" ]; then
  architecture="$expected_architecture"
fi
case "$architecture" in
  amd64) architecture="x86_64" ;;
  arm64) architecture="aarch64" ;;
esac
case "$architecture" in
  x86_64|aarch64) ;;
  *) die "Unsupported RPM architecture: $architecture" ;;
esac
[ "$architecture" = "$expected_architecture" ] \
  || die "Debian package architecture $deb_architecture does not match $architecture"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$script_dir/.." && pwd)"
spec="$root/packaging/linux/rpm/kassiber.spec"
expected_marker="$root/packaging/linux/install-context/rpm-desktop.json"
deb_marker="$root/packaging/linux/install-context/deb-desktop.json"
license="$root/LICENSE"
for required in "$spec" "$expected_marker" "$deb_marker" "$license"; do
  [ -f "$required" ] || die "Required packaging input is missing: $required"
done

topdir="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-desktop-rpm.XXXXXX")"
trap 'rm -rf "$topdir"' EXIT
mkdir -p "$topdir/BUILD" "$topdir/BUILDROOT" "$topdir/RPMS" \
  "$topdir/SOURCES" "$topdir/SPECS" "$topdir/SRPMS" "$topdir/payload"
dpkg-deb -x "$deb" "$topdir/payload"
for required_path in \
  usr/bin/kassiber \
  usr/bin/kassiber-ui \
  usr/lib/Kassiber \
  usr/lib/kassiber/install-context.json \
  usr/share/applications/Kassiber.desktop; do
  [ -e "$topdir/payload/$required_path" ] \
    || die "Desktop payload is missing /$required_path"
done
cmp -s \
  "$topdir/payload/usr/lib/kassiber/install-context.json" \
  "$deb_marker" \
  || die "Desktop Debian install-context marker does not match its package surface"
install -m 0644 \
  "$expected_marker" \
  "$topdir/payload/usr/lib/kassiber/install-context.json"

tar_args=(--sort=name --owner=0 --group=0 --numeric-owner)
if [ -n "${SOURCE_DATE_EPOCH:-}" ]; then
  case "$SOURCE_DATE_EPOCH" in
    *[!0-9]*|"") die "SOURCE_DATE_EPOCH must be a non-negative integer" ;;
  esac
  tar_args+=(--mtime="@$SOURCE_DATE_EPOCH")
fi
tar "${tar_args[@]}" -C "$topdir/payload" -czf \
  "$topdir/SOURCES/kassiber-desktop-rootfs.tar.gz" .
install -m 0644 "$license" "$topdir/SOURCES/LICENSE"
rendered_spec="$topdir/SPECS/kassiber.spec"
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

built_rpm="$topdir/RPMS/$architecture/kassiber-$version-$release.$architecture.rpm"
[ -f "$built_rpm" ] || die "Expected RPM was not built: $built_rpm"
mkdir -p "$(dirname "$output")"
install -m 0644 "$built_rpm" "$output"
if [ -n "$source_output" ]; then
  built_srpm="$topdir/SRPMS/kassiber-$version-$release.src.rpm"
  [ -f "$built_srpm" ] || die "Expected source RPM was not built: $built_srpm"
  mkdir -p "$(dirname "$source_output")"
  install -m 0644 "$built_srpm" "$source_output"
fi
