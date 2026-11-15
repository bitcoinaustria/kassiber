#!/usr/bin/env bash
# Assemble the portable CLI-only archive from a built Rust binary:
#
#   <name>/kassiber[.exe]
#   <name>/BUILD_INFO.json
#
# GNU tar archives are byte-reproducible for a fixed SOURCE_DATE_EPOCH.
set -euo pipefail
export LC_ALL=C

usage() {
  cat >&2 <<'EOF'
Usage: scripts/package-cli-archive.sh --binary PATH --build-info PATH
                                      --name NAME --format tar|zip --output-dir DIR
EOF
}

binary=""
build_info=""
name=""
format=""
output_dir=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --binary) binary="${2:-}"; shift 2 ;;
    --build-info) build_info="${2:-}"; shift 2 ;;
    --name) name="${2:-}"; shift 2 ;;
    --format) format="${2:-}"; shift 2 ;;
    --output-dir) output_dir="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage; exit 2 ;;
  esac
done
for value in "$binary" "$build_info" "$name" "$format" "$output_dir"; do
  [ -n "$value" ] || { usage; exit 2; }
done
[ -x "$binary" ] || { echo "CLI binary is not executable: $binary" >&2; exit 1; }
[ -s "$build_info" ] || { echo "BUILD_INFO.json is missing: $build_info" >&2; exit 1; }
case "$name" in
  kassiber-cli-[a-z0-9-]*) ;;
  *) echo "Unexpected archive name: $name" >&2; exit 2 ;;
esac
case "$format" in
  tar) extension=tar.gz ;;
  zip) extension=zip ;;
  *) echo "--format must be tar or zip" >&2; exit 2 ;;
esac

exe_name="$(basename "$binary")"
case "$exe_name" in
  kassiber|kassiber.exe) ;;
  *) echo "The executable must be named kassiber or kassiber.exe: $binary" >&2; exit 2 ;;
esac

mkdir -p "$output_dir"
# Absolute before the archive path is derived: the zip branch runs 7z from
# inside the staging directory, where a relative path would land the archive
# in staging (removed on exit) instead of the output directory.
output_dir="$(cd "$output_dir" && pwd)"
archive="$output_dir/$name.$extension"
[ ! -e "$archive" ] || { echo "Output already exists: $archive" >&2; exit 1; }
staging="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-cli-archive.XXXXXX")"
trap 'rm -rf "$staging"' EXIT
mkdir -p "$staging/$name"
cp "$binary" "$staging/$name/$exe_name"
cp "$build_info" "$staging/$name/BUILD_INFO.json"
chmod 755 "$staging/$name" "$staging/$name/$exe_name"
chmod 644 "$staging/$name/BUILD_INFO.json"

if [ "$format" = zip ]; then
  (cd "$staging" && 7z a -tzip "$archive" "$name" >/dev/null)
elif tar --version 2>/dev/null | grep -q 'GNU tar'; then
  tar_args=(--sort=name --owner=0 --group=0 --numeric-owner --format=gnu)
  if [ -n "${SOURCE_DATE_EPOCH:-}" ]; then
    tar_args+=(--mtime="@$SOURCE_DATE_EPOCH" --clamp-mtime)
  fi
  tar "${tar_args[@]}" -C "$staging" -cf - "$name" | gzip -n -9 > "$archive"
else
  # bsdtar on macOS: same layout, without the reproducibility flags.
  tar -C "$staging" -czf "$archive" "$name"
fi
echo "$archive"
