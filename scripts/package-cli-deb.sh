#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

usage() {
  echo "Usage: $0 --binary PATH --version VERSION --output PATH [--architecture ARCH] [--broker PATH]" >&2
}

# With --broker, the executable lives in /usr/lib/kassiber-cli beside
# provider_broker/index.mjs (where the Rust binary looks for the AI provider
# broker) and /usr/bin/kassiber is a symlink to it.

binary=""
version=""
output=""
architecture=""
broker=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --binary) binary="${2:-}"; shift 2 ;;
    --version) version="${2:-}"; shift 2 ;;
    --output) output="${2:-}"; shift 2 ;;
    --architecture) architecture="${2:-}"; shift 2 ;;
    --broker) broker="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage; exit 2 ;;
  esac
done

if [ -z "$binary" ] || [ -z "$version" ] || [ -z "$output" ]; then
  usage
  exit 2
fi
if [ ! -x "$binary" ]; then
  echo "CLI binary is not executable: $binary" >&2
  exit 1
fi
if [ -n "$broker" ] && [ ! -s "$broker" ]; then
  echo "Provider broker bundle is missing: $broker" >&2
  exit 1
fi
if ! command -v dpkg-deb >/dev/null 2>&1; then
  echo "dpkg-deb is required to build the CLI-only Debian package." >&2
  exit 2
fi
case "$version" in
  *[!0-9A-Za-z.+:~-]*|"") echo "Invalid Debian version: $version" >&2; exit 2 ;;
esac
if [ -z "$architecture" ]; then
  architecture="$(dpkg --print-architecture)"
fi
case "$architecture" in
  ""|*[!a-z0-9-]*|-*|*-)
    echo "Invalid Debian architecture." >&2
    exit 2
    ;;
esac

package_root="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-cli-deb.XXXXXX")"
trap 'rm -rf "$package_root"' EXIT
mkdir -p \
  "$package_root/DEBIAN" \
  "$package_root/usr/bin" \
  "$(dirname "$output")"
chmod 0755 "$package_root" "$package_root/DEBIAN"
if [ -n "$broker" ]; then
  install -d -m 0755 "$package_root/usr/lib/kassiber-cli/provider_broker"
  install -m 0755 "$binary" "$package_root/usr/lib/kassiber-cli/kassiber"
  install -m 0644 "$broker" "$package_root/usr/lib/kassiber-cli/provider_broker/index.mjs"
  ln -s ../lib/kassiber-cli/kassiber "$package_root/usr/bin/kassiber"
else
  install -m 0755 "$binary" "$package_root/usr/bin/kassiber"
fi

printf '%s\n' \
  'Package: kassiber-cli' \
  "Version: $version" \
  "Architecture: $architecture" \
  'Maintainer: Bitcoin Austria' \
  'Section: utils' \
  'Priority: optional' \
  'Depends: libc6 (>= 2.35), libgcc-s1' \
  'Conflicts: kassiber' \
  'Replaces: kassiber' \
  'Provides: kassiber-command' \
  'Description: Kassiber local-first Bitcoin accounting CLI' \
  ' Standalone command-line package without desktop GUI dependencies.' \
  > "$package_root/DEBIAN/control"

dpkg-deb --root-owner-group --build "$package_root" "$output"
