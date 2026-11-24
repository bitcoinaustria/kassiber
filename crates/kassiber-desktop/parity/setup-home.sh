#!/usr/bin/env bash
# Creates (or resets) a parity home: two copies of a pristine demo book, one per app, and the env
# files run.sh sources. Each app writes only to its own copy; the owner's real books are never read.
#
#   parity/setup-home.sh <home> <pristine book dir> <kassiber binary> [shared dist dir]
#
# The pristine book is a directory with config/settings.json and data/kassiber.sqlite3 whose
# settings.json holds absolute paths; they are rewritten to point at each copy. The web bundle
# (dist/) is built once and shared: pass its path to symlink it, or build it from a checkout of
# commit 33fbc2b5a, the last one with ui-tauri/ (the same PARITY_TAURI_CHECKOUT run.sh needs):
#   (cd "$PARITY_TAURI_CHECKOUT/ui-tauri" && pnpm install --frozen-lockfile &&
#    VITE_DAEMON=bridge pnpm exec vite build --outDir <home>/dist)
set -euo pipefail
home="${1:?home}"; pristine="${2:?pristine book}"; bin="${3:?kassiber binary}"; dist="${4:-}"
mkdir -p "$home"; home="$(cd "$home" && pwd)"; pristine="$(cd "$pristine" && pwd)"
# The book's own root, as settings.json spells it (the directory above its config/).
old="$(grep -o '"/[^"]*/config"' "$pristine/config/settings.json" | head -1 | tr -d '"' | sed 's#/config$##')"
for side in tauri rune; do
  book="$home/book-$side"
  rm -rf "$book"; cp -R "$pristine" "$book"
  perl -pi -e "s#\Q$old\E#$book#g" "$book/config/settings.json"
  prefs="$home/prefs-$side"; rm -rf "$prefs"; mkdir -p "$prefs"
  cat > "$home/env-$side.sh" <<EOF
KASSIBER_BIN=$bin
KASSIBER_DEMO_DATA_ROOT=$book/data
KASSIBER_OFFLINE_PREFERENCE_FILE=$prefs/offline-mode.json
KASSIBER_AGENT_ACCESS_FILE=$prefs/agent-access.json
KASSIBER_UPDATE_PREFERENCE_FILE=$prefs/update.json
KASSIBER_UPDATE_CACHE_FILE=$prefs/update-cache.json
KASSIBER_WALLET_BACKEND_LOOKUP_FILE=$prefs/wallet-backend-lookup.json
KASSIBER_DESKTOP_PREFS_FILE=$prefs/desktop-ui.json
EOF
  # Parity and worker homes start offline: the demo book's backends are a remote host, so a
  # reset must never leave the app free to contact them. Tests that need online turn it off.
  printf '{"schema_version": 1, "enabled": true}\n' > "$prefs/offline-mode.json"
done
if [[ -n "$dist" ]]; then ln -sfn "$(cd "$dist" && pwd)" "$home/dist"; fi
echo "parity home ready: $home (book paths rewritten from $old)"
