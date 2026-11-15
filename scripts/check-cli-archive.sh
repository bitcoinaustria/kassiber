#!/usr/bin/env bash
# Package both CLI archive formats into a relative output directory, as
# release CI does (`--output-dir release`), and require each archive there.
# The zip branch runs 7z from inside its staging directory, so the archive
# path must already be absolute; a stand-in 7z refuses a relative one and
# records where it was told to write. Ported from the stack's
# test_cli_archive_lands_in_a_relative_output_dir_for_both_formats (Python).
#
#   scripts/check-cli-archive.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
work="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-cli-archive-check.XXXXXX")"
trap 'rm -rf "$work"' EXIT

printf '#!/bin/sh\nexit 0\n' > "$work/kassiber"
chmod 755 "$work/kassiber"
printf '{}\n' > "$work/BUILD_INFO.json"
mkdir "$work/tools"
cat > "$work/tools/7z" <<'SH'
#!/bin/sh
[ "$1 $2" = "a -tzip" ] || exit 9
case "$3" in /*) ;; *) exit 8 ;; esac
[ -d "$4" ] || exit 7
printf zip > "$3"
SH
chmod 755 "$work/tools/7z"

for spec in zip:zip tar:tar.gz; do
  format="${spec%%:*}"
  suffix="${spec#*:}"
  dir="$work/run-$format"
  mkdir "$dir"
  printed="$(cd "$dir" && PATH="$work/tools:$PATH" "$ROOT/scripts/package-cli-archive.sh" \
    --binary "$work/kassiber" --build-info "$work/BUILD_INFO.json" \
    --name kassiber-cli-test-x64 --format "$format" --output-dir release)" ||
    { echo "check-cli-archive: packaging $format failed" >&2; exit 1; }
  archive="$dir/release/kassiber-cli-test-x64.$suffix"
  [ -s "$archive" ] || { echo "check-cli-archive: no $format archive at $archive" >&2; exit 1; }
  [ "$(cd "$(dirname "$printed")" && pwd -P)/$(basename "$printed")" = "$(cd "$dir/release" && pwd -P)/kassiber-cli-test-x64.$suffix" ] ||
    { echo "check-cli-archive: $format printed $printed, not $archive" >&2; exit 1; }
done
echo "check-cli-archive: both formats land in a relative output directory"
