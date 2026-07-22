#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

usage() {
  echo "Usage: $0 --source-rpm PATH --output DIR" >&2
}

die() {
  echo "$*" >&2
  exit 2
}

source_rpm=""
output=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --source-rpm) source_rpm="${2:-}"; shift 2 ;;
    --output) output="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "Unknown argument: $1" ;;
  esac
done

[ -n "$source_rpm" ] || die "--source-rpm is required"
[ -f "$source_rpm" ] || die "Source RPM does not exist: $source_rpm"
source_rpm="$(cd "$(dirname "$source_rpm")" && pwd)/$(basename "$source_rpm")"
[ -n "$output" ] || die "--output is required"
[ ! -e "$output" ] || die "Output path already exists: $output"
for command in cpio rpm rpm2cpio; do
  command -v "$command" >/dev/null 2>&1 || die "$command is required"
done

package_name="$(rpm -qp --queryformat '%{NAME}' "$source_rpm")"
package_architecture="$(rpm -qp --queryformat '%{ARCH}' "$source_rpm")"
case "$package_name" in
  kassiber|kassiber-cli) ;;
  *) die "Unexpected Kassiber source package: $package_name" ;;
esac
[ "$package_architecture" = "src" ] \
  || die "Expected a source RPM, got architecture: $package_architecture"

mkdir -p "$(dirname "$output")"
stage="$(mktemp -d "${output}.tmp.XXXXXX")"
cleanup() {
  if [ -n "${stage:-}" ] && [ -d "$stage" ]; then
    rm -rf "$stage"
  fi
}
trap cleanup EXIT
(
  cd "$stage"
  rpm2cpio "$source_rpm" | cpio --quiet -idmu
)
[ -f "$stage/$package_name.spec" ] \
  || die "Source RPM did not contain $package_name.spec"
chmod -R u=rwX,go=rX "$stage"
mv "$stage" "$output"
stage=""
echo "Prepared OBS package sources: $output"
