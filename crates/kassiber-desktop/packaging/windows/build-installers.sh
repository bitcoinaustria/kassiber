#!/usr/bin/env bash
# Windows MSI + NSIS setup for the Rune desktop. NEEDS A WINDOWS RUNNER (Git Bash with WiX v3
# `candle`/`light` and NSIS `makensis` on PATH: GitHub's windows-2022 image has both;
# windows-2025 has no NSIS).
# Usage: packaging/windows/build-installers.sh --rune-bin kassiber-desktop.exe \
#          --kassiber-bin kassiber.exe --build-info BUILD_INFO.json --version V [--output DIR]
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
crate="$(cd "$here/../.." && pwd)"
rune_bin="" kassiber_bin="" build_info="" version="" output="$crate/dist/windows-x64"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --rune-bin) rune_bin="$2"; shift ;;
    --kassiber-bin) kassiber_bin="$2"; shift ;;
    --build-info) build_info="$2"; shift ;;
    --version) version="$2"; shift ;;
    --output) output="$2"; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done
for f in "$rune_bin" "$kassiber_bin" "$build_info"; do [ -f "$f" ] || { echo "missing input: '$f'" >&2; exit 1; }; done
[ -n "$version" ] || { echo "--version is required" >&2; exit 1; }
stage="$(mktemp -d)"; trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/binaries" "$stage/bin" "$stage/windows" "$output"
# Absolute: makensis resolves OutFile from the script's directory, not the working one.
output="$(cd "$output" && pwd)"
cp "$rune_bin" "$stage/kassiber-ui.exe"
cp "$kassiber_bin" "$stage/binaries/kassiber-cli-x86_64-pc-windows-msvc.exe"
cp "$build_info" "$stage/binaries/BUILD_INFO.json"
# The PATH entry runs a native copy, so cmd.exe never reparses arguments.
cp "$kassiber_bin" "$stage/bin/kassiber.exe"
cp "$here/update-path.ps1" "$stage/windows/update-path.ps1"
"$stage/bin/kassiber.exe" --version
# The shipped exe is GUI-subsystem (src/main.rs, no `tooling`): it must refuse headless runs.
if "$stage/kassiber-ui.exe" --tree NUL >/dev/null 2>&1; then echo "kassiber-ui.exe accepted --tree (tooling build)" >&2; exit 1; fi
icons="$(cygpath -w "$crate/packaging/icons")"; wstage="$(cygpath -w "$stage")"
candle -nologo -arch x64 -dVersion="$version" -dStage="$wstage" -dIconsDir="$icons" \
  -out "$stage/" "$(cygpath -w "$here/kassiber.wxs")" "$(cygpath -w "$here/cli-path.wxs")"
light -nologo -out "$(cygpath -w "$output/kassiber-windows-x64.msi")" "$stage/kassiber.wixobj" "$stage/cli-path.wixobj"
makensis -V2 -DVERSION="$version" -DSTAGE="$wstage" -DICONS="$icons" \
  -DOUTFILE="$(cygpath -w "$output/kassiber-windows-x64.exe")" "$(cygpath -w "$here/kassiber.nsi")"
echo "built $output/kassiber-windows-x64.msi and .exe"
