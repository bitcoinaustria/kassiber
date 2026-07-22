#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: $0 /absolute/path/to/app-bundle" >&2
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
  *) echo "The Kassiber .app path must be absolute." >&2; exit 2 ;;
esac
case "$app_bundle" in
  /Volumes/*|*/AppTranslocation/*)
    echo "Move the Kassiber .app to a stable local path before installing its terminal command." >&2
    exit 2
    ;;
esac
launcher="$app_bundle/Contents/Resources/bin/kassiber"
if [ ! -x "$launcher" ]; then
  echo "Bundled Kassiber launcher not found at $launcher" >&2
  exit 1
fi
app_executable="$app_bundle/Contents/MacOS/kassiber-ui"
if [ ! -x "$app_executable" ]; then
  echo "Kassiber desktop executable not found at $app_executable" >&2
  exit 1
fi

# The desktop binary owns launcher selection, conflict detection, shell-profile
# updates, and marker formats. Keeping this wrapper argument-only prevents the
# local build path from drifting away from Settings.
exec "$app_executable" --install-terminal-command
