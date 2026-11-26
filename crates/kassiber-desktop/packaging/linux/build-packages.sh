#!/usr/bin/env bash
# Linux desktop packages for the Rune app: the `kassiber` .deb (then the .rpm, repacked from it
# by scripts/package-desktop-rpm.sh exactly as for the Tauri deb) and an AppImage. Same file
# ownership as the Tauri packages (docs/reference/linux-packaging.md, "Release gates"):
#
#   /usr/bin/kassiber-ui                                      the Rune app (no `tooling` feature)
#   /usr/bin/kassiber                                         forwarder -> kassiber-ui --cli
#   /usr/lib/Kassiber/binaries/kassiber-cli-<triple>          the `kassiber` daemon/CLI
#   /usr/lib/Kassiber/binaries/BUILD_INFO.json
#   /usr/share/applications/Kassiber.desktop                  x-scheme-handler/kassiber
#   /usr/share/icons/hicolor/{32x32,128x128,256x256@2}/apps/kassiber-ui.png
#
# The deb Provides kassiber-command and Conflicts/Replaces kassiber-cli (both own
# /usr/bin/kassiber). Its Depends drop the Tauri package's GTK/WebKitGTK and add what Rune
# needs: xkbcommon, the Wayland client, the X11 libraries winit loads at run time, and a
# Vulkan loader plus ICD (wgpu has no OpenGL fallback here).
#
# NEEDS A LINUX RUNNER (ubuntu-22.04 for the glibc 2.35 floor): dpkg-deb, appimagetool,
# readelf. `--stage-only` assembles and checks the trees without any of them (works on macOS
# with dummy binaries).
#
# The AppImage needs `--appimage-runtime PATH`, a pinned AppImage type-2 runtime: without one,
# appimagetool would download the latest runtime itself. The release workflow and
# scripts/build-linux-x64-rust-artifacts.sh fetch and verify both tools.
#
# Usage: packaging/linux/build-packages.sh --rune-bin PATH --kassiber-bin PATH --build-info PATH
#          --version V [--arch amd64|arm64] [--output DIR] [--stage-only]
#          [--no-appimage | --appimage-runtime PATH] [--glibc-floor V]
#   --glibc-floor V   accept binaries up to glibc V (default 2.35, the release floor; anything
#                     higher is a local functional build that must never be published)
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
crate="$(cd "$here/../.." && pwd)"
repo="${KASSIBER_REPO:-$(cd "$crate/../.." && pwd)}"

rune_bin="" kassiber_bin="" build_info="" version="" arch=amd64 output="$crate/dist/linux" stage_only=0 appimage=1 appimage_runtime="" glibc_floor=2.35
while [ "$#" -gt 0 ]; do
  case "$1" in
    --rune-bin) rune_bin="$2"; shift ;;
    --kassiber-bin) kassiber_bin="$2"; shift ;;
    --build-info) build_info="$2"; shift ;;
    --version) version="$2"; shift ;;
    --arch) arch="$2"; shift ;;
    --output) output="$2"; shift ;;
    --stage-only) stage_only=1 ;;
    --no-appimage) appimage=0 ;;
    --appimage-runtime) appimage_runtime="$2"; shift ;;
    --glibc-floor) glibc_floor="$2"; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done
die() { echo "build-packages: $*" >&2; exit 1; }
for f in "$rune_bin" "$kassiber_bin" "$build_info"; do [ -f "$f" ] || die "missing input: '$f'"; done
[ -n "$version" ] || die "--version is required"
case "$arch" in amd64) triple=x86_64-unknown-linux-gnu; appimage_arch=x86_64; label=linux-x64 ;; arm64) triple=aarch64-unknown-linux-gnu; appimage_arch=aarch64; label=linux-arm64 ;; *) die "--arch is amd64 or arm64" ;; esac

mkdir -p "$output"
work="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-linux.XXXXXX")"; work="$(cd "$work" && pwd)"
trap 'rm -rf "$work"' EXIT

# --- the shared tree ------------------------------------------------------------------------
stage_tree() { # $1: root (a deb root or an AppDir's usr parent)
  local root="$1"
  install -d "$root/usr/bin" "$root/usr/lib/Kassiber/binaries" "$root/usr/share/applications"
  install -m 0755 "$rune_bin" "$root/usr/bin/kassiber-ui"
  install -m 0755 "$here/kassiber" "$root/usr/bin/kassiber"
  install -m 0755 "$kassiber_bin" "$root/usr/lib/Kassiber/binaries/kassiber-cli-$triple"
  install -m 0644 "$build_info" "$root/usr/lib/Kassiber/binaries/BUILD_INFO.json"
  install -m 0644 "$here/Kassiber.desktop" "$root/usr/share/applications/Kassiber.desktop"
  local icons="$crate/packaging/icons" size
  # (portable: BSD install has no GNU -D)
  for size in 32x32 128x128 256x256@2; do install -d "$root/usr/share/icons/hicolor/$size/apps"; done
  install -m 0644 "$icons/32x32.png" "$root/usr/share/icons/hicolor/32x32/apps/kassiber-ui.png"
  install -m 0644 "$icons/128x128.png" "$root/usr/share/icons/hicolor/128x128/apps/kassiber-ui.png"
  install -m 0644 "$icons/128x128@2x.png" "$root/usr/share/icons/hicolor/256x256@2/apps/kassiber-ui.png"
}

deb_root="$work/deb"
stage_tree "$deb_root"
install -d "$deb_root/DEBIAN"
installed_kb="$(du -sk "$deb_root/usr" | cut -f1)"
sed -e "s/@@VERSION@@/$version/" -e "s/@@ARCH@@/$arch/" -e "s/@@INSTALLED_KB@@/$installed_kb/" \
  "$here/debian/control.in" > "$deb_root/DEBIAN/control"
# The paths scripts/package-desktop-rpm.sh requires before it repacks the deb.
for p in usr/bin/kassiber usr/bin/kassiber-ui usr/lib/Kassiber usr/share/applications/Kassiber.desktop; do
  [ -e "$deb_root/$p" ] || die "deb tree lacks /$p"
done
# The Rune app finds its daemon at <prefix>/lib/Kassiber/binaries (`resource_dirs` in src/daemon.rs).
[ -x "$deb_root/usr/lib/Kassiber/binaries/kassiber-cli-$triple" ] || die "sidecar not executable"

if [ "$stage_only" -eq 1 ]; then
  rm -rf "$output/stage"; mkdir -p "$output/stage"; cp -R "$deb_root" "$output/stage/deb"
  echo "staged the deb tree at $output/stage/deb (no packages built: --stage-only)"
  find "$output/stage/deb" -type f | sed "s|$output/stage/deb||" | sort
  exit 0
fi

for tool in dpkg-deb readelf; do command -v "$tool" >/dev/null || die "missing $tool (run on the Linux runner)"; done
# A shipped app must refuse every headless/scripted run (Rune prints WITHOUT_TOOLING and exits
# before any window or GPU state exists), as packaging/macos/build-app.sh checks. It probes the
# staged copy: an absolute path (a relative --rune-bin would not survive the cd) and the shipped file.
if probe="$(cd "${TMPDIR:-/tmp}" && "$deb_root/usr/bin/kassiber-ui" --tree /dev/null 2>&1)"; then
  die "the Rune binary accepted --tree: it was built with the tooling feature"
fi
case "$probe" in *"no agent tooling"*) ;; *) die "unexpected answer to --tree: $probe" ;; esac
# The glibc floor: the shell's and the sidecar's version needs (as the Tauri job checks).
"$repo/scripts/check-linux-glibc-floor.sh" --any-libraries "$deb_root/usr/bin/kassiber-ui" "$glibc_floor"
"$repo/scripts/check-linux-glibc-floor.sh" "$deb_root/usr/lib/Kassiber/binaries/kassiber-cli-$triple" "$glibc_floor"
# What the shell links: every NEEDED library must be covered by the Depends line above.
readelf -d "$deb_root/usr/bin/kassiber-ui" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' | sort > "$work/needed.txt"
echo "kassiber-ui NEEDED:"; sed 's/^/  /' "$work/needed.txt"
dpkg-deb --root-owner-group --build "$deb_root" "$output/kassiber-$label.deb"

if [ "$appimage" -eq 1 ]; then
  command -v appimagetool >/dev/null || die "missing appimagetool (pinned download in CI)"
  [ -f "$appimage_runtime" ] || die "--appimage-runtime PATH is required for the AppImage (or pass --no-appimage)"
  appdir="$work/Kassiber.AppDir"
  stage_tree "$appdir"
  rm "$appdir/usr/bin/kassiber"   # the AppImage forwards `--cli` itself; no /usr/bin entry
  install -m 0755 "$here/AppRun" "$appdir/AppRun"
  cp "$here/Kassiber.desktop" "$appdir/Kassiber.desktop"
  cp "$crate/packaging/icons/128x128@2x.png" "$appdir/kassiber-ui.png"
  # Nothing GPU- or display-related is bundled: libvulkan, the ICD, libxkbcommon and the
  # Wayland client must come from the host (a bundled loader cannot find the host's drivers).
  ARCH="$appimage_arch" appimagetool --no-appstream --runtime-file "$appimage_runtime" \
    "$appdir" "$output/kassiber-$label.AppImage"
fi
echo "built in $output"
