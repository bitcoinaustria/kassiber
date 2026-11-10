#!/usr/bin/env bash
# Refuse a Linux ELF executable that needs a newer glibc than the release
# floor, or that links a shared library outside glibc and the GCC runtime.
#
# The floor is the AppImage/Ubuntu 22.04 baseline the deb and rpm metadata
# declare (`libc6 (>= 2.35)`, `glibc >= 2.35`). A binary built on a newer host
# binds new symbol versions (for example `__isoc23_strtol@GLIBC_2.38` from C
# code compiled against glibc 2.38+ headers) and then fails to load on the
# floor distributions with "version `GLIBC_2.38' not found".
#
# Version needs flagged WEAK do not stop the loader (Rust's standard library
# references `pidfd_spawnp` that way and checks for it at run time), so only
# non-weak needs count. The `objdump -T` maximum is printed for the record.
set -euo pipefail
export LC_ALL=C

usage() {
  echo "Usage: $0 [--any-libraries] BINARY [FLOOR]   (FLOOR defaults to 2.35)" >&2
}

# --any-libraries: only the glibc floor; for the desktop shell, which links
# GTK and WebKitGTK as declared package dependencies.
any_libraries=0
if [ "${1:-}" = --any-libraries ]; then
  any_libraries=1
  shift
fi
if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
  usage
  exit 2
fi
binary="$1"
floor="${2:-2.35}"
case "$floor" in
  2.[0-9]|2.[0-9][0-9]) ;;
  *) echo "Invalid glibc floor: $floor" >&2; exit 2 ;;
esac
for command in readelf objdump sort; do
  command -v "$command" >/dev/null 2>&1 || { echo "$command is required" >&2; exit 2; }
done
[ -f "$binary" ] || { echo "No such file: $binary" >&2; exit 2; }

header="$(readelf -h "$binary" 2>/dev/null)" || {
  echo "$binary is not an ELF file." >&2
  exit 1
}
case "$header" in
  *"Advanced Micro Devices X86-64"*|*"AArch64"*) ;;
  *) echo "$binary is neither an x86-64 nor an AArch64 ELF file." >&2; exit 1 ;;
esac

# glibc's own libraries, the dynamic loader, and the GCC unwinder runtime are
# present on every glibc distribution; anything else would have to be
# packaged as a dependency (the PyInstaller CLI needed zlib).
allowed_libraries='^(libc\.so\.6|libm\.so\.6|libgcc_s\.so\.1|libdl\.so\.2|libpthread\.so\.0|librt\.so\.1|ld-linux-x86-64\.so\.2|ld-linux-aarch64\.so\.1)$'
needed="$(readelf -d "$binary" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p')"
foreign="$(printf '%s\n' "$needed" | grep -vE "$allowed_libraries" | grep . || true)"
if [ -n "$foreign" ] && [ "$any_libraries" -eq 0 ]; then
  echo "$binary links shared libraries outside glibc and libgcc_s:" >&2
  printf '  %s\n' $foreign >&2
  exit 1
fi

version_greater() {
  # True when glibc version $1 sorts after $2.
  [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n 1)" = "$1" ]
}

strong_needs="$(readelf -V --wide "$binary" \
  | sed -n 's/.*Name: GLIBC_\([0-9.]*\)[[:space:]]*Flags: none.*/\1/p' | sort -uV)"
weak_needs="$(readelf -V --wide "$binary" \
  | sed -n 's/.*Name: GLIBC_\([0-9.]*\)[[:space:]]*Flags: WEAK.*/\1/p' | sort -uV)"
objdump_max="$(objdump -T "$binary" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -uV | tail -n 1)"
strong_max="$(printf '%s\n' "$strong_needs" | grep . | tail -n 1 || true)"

echo "Linked libraries: $(printf '%s ' $needed)"
echo "Highest GLIBC symbol version (objdump -T): ${objdump_max:-none}"
echo "Highest required GLIBC version (non-weak): ${strong_max:-none}"
if [ -n "$weak_needs" ]; then
  echo "Weak GLIBC versions (optional at run time): $(printf '%s ' $weak_needs)"
fi

too_new=""
for version in $strong_needs; do
  if version_greater "$version" "$floor"; then
    too_new="$too_new $version"
  fi
done
if [ -n "$too_new" ]; then
  echo "$binary requires glibc newer than the $floor floor:$too_new" >&2
  objdump -T "$binary" | awk -v floor="$floor" '
    match($0, /GLIBC_[0-9.]+/) {
      version = substr($0, RSTART + 6, RLENGTH - 6)
      split(version, have, "."); split(floor, want, ".")
      if (have[1] > want[1] || (have[1] == want[1] && have[2] + 0 > want[2] + 0)) print "  " $0
    }' >&2
  exit 1
fi
echo "glibc floor $floor: ok"
