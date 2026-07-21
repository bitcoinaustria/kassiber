#!/usr/bin/env bash
set -euo pipefail

MARKER="# Kassiber desktop CLI launcher. Managed by Kassiber Settings."
PATH_MARKER_START="# >>> kassiber terminal command >>>"
PATH_MARKER_END="# <<< kassiber terminal command <<<"

usage() {
  echo "Usage: $0 /absolute/path/to/Kassiber.app" >&2
}

shell_quote() {
  printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\"'\\\"'/g")"
}

path_contains() {
  case ":${PATH:-}:" in
    *":$1:"*) return 0 ;;
    *) return 1 ;;
  esac
}

managed_profile_for_shell() {
  case "$(basename "${SHELL:-/bin/zsh}")" in
    zsh) printf '%s\n' "$HOME/.zprofile" ;;
    bash)
      if [ -e "$HOME/.bash_profile" ]; then
        printf '%s\n' "$HOME/.bash_profile"
      else
        printf '%s\n' "$HOME/.profile"
      fi
      ;;
    sh|dash|ksh) printf '%s\n' "$HOME/.profile" ;;
    fish) printf '%s\n' "$HOME/.config/fish/conf.d/kassiber.fish" ;;
    *) return 1 ;;
  esac
}

ensure_profile_path() {
  local bin_dir="$1"
  local profile shell_name display_dir cleaned
  if ! profile="$(managed_profile_for_shell)"; then
    echo "The launcher was installed, but this shell is not managed automatically." >&2
    echo "Add $bin_dir to PATH and open a new terminal." >&2
    return 0
  fi
  if [ -f "$profile" ] && {
    grep -F "$PATH_MARKER_START" "$profile" >/dev/null 2>&1 ||
      grep -F "$PATH_MARKER_END" "$profile" >/dev/null 2>&1
  }; then
    if ! grep -F "$PATH_MARKER_START" "$profile" >/dev/null 2>&1 ||
      ! grep -F "$PATH_MARKER_END" "$profile" >/dev/null 2>&1; then
      echo "Incomplete Kassiber PATH block in $profile; repair it manually." >&2
      return 1
    fi
    cleaned="$(mktemp "$(dirname "$profile")/.kassiber-profile.XXXXXX")"
    awk -v start="$PATH_MARKER_START" -v end="$PATH_MARKER_END" '
      $0 == start { managed = 1; next }
      $0 == end && managed { managed = 0; next }
      !managed { print }
    ' "$profile" > "$cleaned"
    awk 'BEGIN { blank = 0 } { lines[NR] = $0; if ($0 != "") last = NR } END { for (i = 1; i <= last; i++) print lines[i] }' \
      "$cleaned" > "$profile"
    rm -f "$cleaned"
  fi
  mkdir -p "$(dirname "$profile")"
  shell_name="$(basename "${SHELL:-/bin/zsh}")"
  case "$bin_dir" in
    "$HOME"/*) display_dir="\$HOME/${bin_dir#"$HOME"/}" ;;
    *) display_dir="$bin_dir" ;;
  esac
  {
    [ ! -s "$profile" ] || printf '\n'
    printf '%s\n' "$PATH_MARKER_START"
    if [ "$shell_name" = "fish" ]; then
      printf 'if not contains -- %s $PATH\n' "$(shell_quote "$bin_dir")"
      printf '    set -gx PATH %s $PATH\n' "$(shell_quote "$bin_dir")"
      printf 'end\n'
    else
      printf 'case ":$PATH:" in\n'
      printf '  *":%s:"*) ;;\n' "$display_dir"
      printf '  *) export PATH="%s:$PATH" ;;\n' "$display_dir"
      printf 'esac\n'
    fi
    printf '%s\n' "$PATH_MARKER_END"
  } >> "$profile"
  echo "Updated PATH in $profile; open a new terminal before using kassiber by name."
}

if [ "$#" -ne 1 ]; then
  usage
  exit 2
fi
if [ "$(uname -s)" != "Darwin" ]; then
  echo "This installer is for local macOS desktop builds." >&2
  exit 2
fi

app_bundle="$1"
case "$app_bundle" in
  /*) ;;
  *) echo "Kassiber.app path must be absolute." >&2; exit 2 ;;
esac
case "$app_bundle" in
  /Volumes/*|*/AppTranslocation/*)
    echo "Move Kassiber.app to a stable local path before installing its terminal command." >&2
    exit 2
    ;;
esac
target="$app_bundle/Contents/Resources/bin/kassiber"
if [ ! -x "$target" ]; then
  echo "Bundled Kassiber launcher not found at $target" >&2
  exit 1
fi

bin_dir="$HOME/.local/bin"
if path_contains "$HOME/.local/bin"; then
  bin_dir="$HOME/.local/bin"
elif path_contains "$HOME/bin"; then
  bin_dir="$HOME/bin"
fi
command_path="$bin_dir/kassiber"

resolved="$(command -v kassiber 2>/dev/null || true)"
if [ -n "$resolved" ] && [ "$resolved" != "$command_path" ]; then
  echo "Refusing to shadow the existing kassiber command at $resolved" >&2
  exit 1
fi
if [ -e "$command_path" ] || [ -L "$command_path" ]; then
  if [ ! -f "$command_path" ] || ! grep -F "$MARKER" "$command_path" >/dev/null 2>&1; then
    echo "Refusing to replace $command_path because it is not managed by Kassiber." >&2
    exit 1
  fi
fi

mkdir -p "$bin_dir"
tmp_launcher="$(mktemp "$bin_dir/.kassiber.XXXXXX")"
trap 'rm -f "$tmp_launcher"' EXIT
{
  printf '#!/bin/sh\n'
  printf '%s\n' "$MARKER"
  printf 'exec %s "$@"\n' "$(shell_quote "$target")"
} > "$tmp_launcher"
chmod 755 "$tmp_launcher"
mv -f "$tmp_launcher" "$command_path"
trap - EXIT

if ! path_contains "$bin_dir"; then
  ensure_profile_path "$bin_dir"
fi
"$command_path" --version
echo "Installed $command_path -> $target"
