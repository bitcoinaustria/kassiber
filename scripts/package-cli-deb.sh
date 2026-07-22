#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

usage() {
  echo "Usage: $0 --binary PATH --version VERSION --output PATH [--architecture ARCH]" >&2
}

binary=""
version=""
output=""
architecture=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --binary) binary="${2:-}"; shift 2 ;;
    --version) version="${2:-}"; shift 2 ;;
    --output) output="${2:-}"; shift 2 ;;
    --architecture) architecture="${2:-}"; shift 2 ;;
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
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
install_context="$script_dir/../packaging/linux/install-context/deb-cli.json"
if [ ! -f "$install_context" ]; then
  echo "CLI install-context marker is missing: $install_context" >&2
  exit 1
fi
mkdir -p \
  "$package_root/DEBIAN" \
  "$package_root/usr/bin" \
  "$package_root/usr/lib/kassiber" \
  "$(dirname "$output")"
chmod 0755 "$package_root" "$package_root/DEBIAN"
install -m 0755 "$binary" "$package_root/usr/bin/kassiber"
install -m 0644 \
  "$install_context" \
  "$package_root/usr/lib/kassiber/install-context.json"

printf '%s\n' \
  'Package: kassiber-cli' \
  "Version: $version" \
  "Architecture: $architecture" \
  'Maintainer: Bitcoin Austria' \
  'Section: utils' \
  'Priority: optional' \
  'Depends: libc6 (>= 2.35), zlib1g' \
  'Conflicts: kassiber' \
  'Replaces: kassiber' \
  'Provides: kassiber-command' \
  'X-Kassiber-Install-Context: /usr/lib/kassiber/install-context.json' \
  'Description: Kassiber local-first Bitcoin accounting CLI' \
  ' Standalone command-line package without desktop GUI dependencies.' \
  > "$package_root/DEBIAN/control"

dpkg-deb --root-owner-group --build "$package_root" "$output"
