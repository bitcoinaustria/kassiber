#!/usr/bin/env bash
# Snapshot and restore every local Kassiber book on this Mac.
#
# Used before a differently built backend (such as the Rust desktop build from
# scripts/build-macos-arm64-rust-app.sh) first opens the same books, so an
# upgrade or a backend bug can be rolled back. Snapshots copy files only while
# no Kassiber process runs, which keeps SQLite/SQLCipher databases and their
# WAL files consistent; encrypted books stay encrypted in the copy.
set -euo pipefail

# Snapshots hold whole books: keep everything private to this account.
umask 077

APP_ID="at.bitcoinaustria.kassiber"
SNAPSHOT_ROOT="${KASSIBER_SNAPSHOT_DIR:-$HOME/Library/Application Support/$APP_ID-snapshots}"

usage() {
  cat <<EOF
Usage:
  $0 snapshot [--label TEXT] [--if-changed]
  $0 restore SNAPSHOT_DIR
  $0 list

snapshot     Copy every Kassiber state root and every project directory the
             project catalogs point to into a new timestamped snapshot under
               $SNAPSHOT_ROOT
             --if-changed skips the copy when nothing changed since the newest
             complete snapshot.
restore      Put a snapshot back. Current data is renamed aside to
             <path>.before-restore-<timestamp>, never deleted.
list         Show complete snapshots, newest last.

Every command refuses to run while the Kassiber app, its daemon, or a Kassiber
CLI is running. Keychain items (remembered unlock, AI provider keys) are not
part of a snapshot.
EOF
}

die() {
  echo "$*" >&2
  exit 1
}

if [ "$(uname -s)" != "Darwin" ]; then
  die "This helper manages the macOS Kassiber data locations and must run on macOS."
fi

# Kassiber's desktop shell, its bundled sidecars (Python and Rust share the
# name), a broker, and source-checkout daemons. pgrep never matches itself, and
# none of these patterns match this script's own command line.
running_kassiber() {
  pgrep -fl 'Contents/MacOS/kassiber-ui|kassiber-cli-[a-z0-9_]+-apple-darwin|--operator-broker-server|-m kassiber( |$)' || true
}

require_quiet() {
  local running
  running="$(running_kassiber)"
  if [ -n "$running" ]; then
    echo "Kassiber is running; quit every Kassiber app and CLI first:" >&2
    printf '%s\n' "$running" >&2
    exit 3
  fi
}

# The state roots both backends may use on macOS: the native one and the legacy
# home-directory one (Kassiber migrates it on first use, but it can still hold
# books). Only existing directories are listed.
state_roots() {
  local candidate
  for candidate in \
    "$HOME/Library/Application Support/$APP_ID" \
    "$HOME/.kassiber"; do
    if [ -d "$candidate" ]; then
      printf '%s\n' "$candidate"
    fi
  done
}

# Project directories that live outside a state root. The catalog stores
# `projects[].path`, optionally starting with `~`. JavaScript for Automation
# parses it on every macOS version, without Python or newer plutil modes.
CATALOG_PATHS_JXA='
function run(argv) {
  ObjC.import("Foundation");
  const text = $.NSString.stringWithContentsOfFileEncodingError(argv[0], $.NSUTF8StringEncoding, null);
  if (text.isNil()) throw new Error("unreadable");
  const projects = JSON.parse(text.js).projects;
  if (!Array.isArray(projects)) throw new Error("no projects list");
  return projects
    .map((project) => (project && typeof project.path === "string" ? project.path : ""))
    .filter((path) => path.length > 0 && !path.includes("\n"))
    .join("\n");
}'

catalog_paths() {
  osascript -l JavaScript -e "$CATALOG_PATHS_JXA" "$1"
}

external_projects() {
  local root catalog paths path
  while IFS= read -r root; do
    catalog="$root/config/projects.json"
    [ -f "$catalog" ] || continue
    if ! paths="$(catalog_paths "$catalog" 2>/dev/null)"; then
      die "Cannot read the project catalog $catalog; back it up by hand or pass --no-backup to the build."
    fi
    while IFS= read -r path; do
      case "$path" in
        "~") path="$HOME" ;;
        "~/"*) path="$HOME/${path#\~/}" ;;
      esac
      case "$path" in
        /*) ;;
        *) continue ;;
      esac
      [ -d "$path" ] || continue
      inside_state_root "$path" && continue
      printf '%s\n' "$path"
    done <<< "$paths"
  done < <(state_roots)
}

inside_state_root() {
  local root
  while IFS= read -r root; do
    case "$1/" in
      "$root/"*) return 0 ;;
    esac
  done < <(state_roots)
  return 1
}

sources() {
  { state_roots; external_projects; } | awk '!seen[$0]++'
}

# Copy directories, regular files, and symlinks. Sockets and FIFOs (a running
# broker's endpoint) are skipped, as is each state root's top-level `run/`
# directory of locks and sockets, which belongs to live processes only.
copy_tree() {
  local source="$1" destination="$2" relative
  mkdir -p "$destination"
  while IFS= read -r -d '' relative; do
    relative="${relative#./}"
    [ "$relative" = "." ] && continue
    if [ -L "$source/$relative" ]; then
      # BSD cp follows symlinks unless copying recursively; keep the link.
      ln -s "$(readlink "$source/$relative")" "$destination/$relative"
    elif [ -d "$source/$relative" ]; then
      mkdir -p "$destination/$relative"
    else
      cp -p "$source/$relative" "$destination/$relative"
    fi
  done < <(cd "$source" && find . -path ./run -prune -o \( -type d -o -type f -o -type l \) -print0)
}

checked_path() {
  case "$1" in
    *$'\n'*|*$'\t'*) die "Refusing a path that contains a tab or newline: $1" ;;
  esac
}

json_string() {
  local text="$1"
  text="${text//\\/\\\\}"
  text="${text//\"/\\\"}"
  printf '"%s"' "$text"
}

latest_snapshot() {
  local newest=""
  local candidate
  [ -d "$SNAPSHOT_ROOT" ] || return 0
  for candidate in "$SNAPSHOT_ROOT"/*/; do
    candidate="${candidate%/}"
    if [ -f "$candidate/COMPLETE" ]; then
      newest="$candidate"
    fi
  done
  printf '%s' "$newest"
}

cmd_snapshot() {
  local label="" if_changed=0
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --label) [ "$#" -ge 2 ] || die "--label needs a value"; label="$2"; shift ;;
      --if-changed) if_changed=1 ;;
      *) die "unknown snapshot argument: $1" ;;
    esac
    shift
  done
  case "$label" in
    *[!A-Za-z0-9._-]*) die "--label may only use letters, digits, '.', '_' and '-'" ;;
  esac

  require_quiet

  local list
  list="$(sources)"
  if [ -z "$list" ]; then
    echo "No Kassiber data found on this Mac; nothing to back up."
    return 0
  fi

  if [ "$if_changed" -eq 1 ]; then
    local previous previous_sources changed=""
    previous="$(latest_snapshot)"
    if [ -n "$previous" ]; then
      previous_sources="$(cut -f2 "$previous/sources.tsv")"
      if [ "$previous_sources" = "$list" ]; then
        while IFS= read -r path; do
          if [ -n "$(find "$path" -path "$path/run" -prune -o -newer "$previous/COMPLETE" -print -quit)" ]; then
            changed=1
            break
          fi
        done <<< "$list"
        if [ -z "$changed" ]; then
          echo "Kassiber data is unchanged since the snapshot at:"
          echo "  $previous"
          return 0
        fi
      fi
    fi
  fi

  local stamp snapshot index=0 name sources_json=""
  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  snapshot="$SNAPSHOT_ROOT/$stamp${label:+-$label}"
  [ -e "$snapshot" ] && die "Snapshot $snapshot already exists."
  mkdir -p "$snapshot/roots"
  : > "$snapshot/sources.tsv"

  while IFS= read -r path; do
    checked_path "$path"
    name="source-$index"
    index=$((index + 1))
    echo "Copying $path"
    copy_tree "$path" "$snapshot/roots/$name"
    printf '%s\t%s\n' "$name" "$path" >> "$snapshot/sources.tsv"
    sources_json="$sources_json${sources_json:+,}{\"name\":$(json_string "$name"),\"path\":$(json_string "$path")}"
  done <<< "$list"

  printf '{"schema_version":1,"created_at":%s,"label":%s,"sources":[%s]}\n' \
    "$(json_string "$stamp")" "$(json_string "$label")" "$sources_json" > "$snapshot/manifest.json"
  # Written last: only a snapshot carrying COMPLETE is restorable.
  : > "$snapshot/COMPLETE"

  echo
  echo "Snapshot complete ($(du -sh "$snapshot" | cut -f1)):"
  echo "  $snapshot"
  echo "Restore it with:"
  echo "  $0 restore \"$snapshot\""
}

cmd_restore() {
  [ "$#" -eq 1 ] || die "restore needs exactly one snapshot directory"
  local snapshot="${1%/}"
  [ -f "$snapshot/COMPLETE" ] || die "$snapshot is not a complete Kassiber snapshot."
  [ -f "$snapshot/sources.tsv" ] || die "$snapshot has no sources.tsv."

  require_quiet

  local stamp name path aside
  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  while IFS=$'\t' read -r name path; do
    case "$name" in
      source-[0-9]*) ;;
      *) die "Unexpected snapshot entry: $name" ;;
    esac
    case "$path" in
      /*) ;;
      *) die "Unexpected snapshot path: $path" ;;
    esac
    [ -d "$snapshot/roots/$name" ] || die "Snapshot is missing roots/$name."
    if [ -e "$path" ]; then
      aside="$path.before-restore-$stamp"
      echo "Moving current $path aside to $aside"
      mv "$path" "$aside"
    fi
    echo "Restoring $path"
    copy_tree "$snapshot/roots/$name" "$path"
  done < "$snapshot/sources.tsv"

  echo
  echo "Restored $snapshot."
  echo "The data it replaced was moved aside with the suffix .before-restore-$stamp;"
  echo "delete those directories yourself once you no longer need them."
}

cmd_list() {
  local candidate found=0
  if [ -d "$SNAPSHOT_ROOT" ]; then
    for candidate in "$SNAPSHOT_ROOT"/*/; do
      candidate="${candidate%/}"
      [ -f "$candidate/COMPLETE" ] || continue
      found=1
      printf '%s  %s\n' "$(du -sh "$candidate" | cut -f1)" "$candidate"
    done
  fi
  [ "$found" -eq 1 ] || echo "No snapshots under $SNAPSHOT_ROOT."
}

[ "$#" -ge 1 ] || { usage >&2; exit 2; }
command="$1"
shift
case "$command" in
  snapshot) cmd_snapshot "$@" ;;
  restore) cmd_restore "$@" ;;
  list) cmd_list "$@" ;;
  -h|--help|help) usage ;;
  *) usage >&2; exit 2 ;;
esac
