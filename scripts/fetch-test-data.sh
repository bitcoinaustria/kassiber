#!/usr/bin/env bash
# Put the generated test data that tests/test-data.lock pins in place.
#
# Recorded and generated fixtures (Python-produced vectors, oracle outputs,
# recordings) live on the data branch named in the lock, not in this tree.
# This script fetches the pinned commit from the same repository with git
# alone, writes each locked file at its path (all of them are ignored), and
# checks every file against the hash the lock records. Tests read the data
# only from there. See docs/reference/testing.md#generated-test-data.
#
#   scripts/fetch-test-data.sh            fetch and verify (the default)
#   scripts/fetch-test-data.sh --check    verify only; fail if anything is off
#   scripts/fetch-test-data.sh --publish [new-path...]
#       commit the locked files as they are now (plus any new paths) to the
#       data branch locally, rewrite the lock and the .gitignore block; push
#       the branch, then commit both with the change that regenerated the data.
#   scripts/fetch-test-data.sh --ignore   rewrite the .gitignore block from the lock
#
# KASSIBER_TEST_DATA_REMOTE names the remote to fetch from (default origin).
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
lock=tests/test-data.lock
stamp_dir=.test-data
remote=${KASSIBER_TEST_DATA_REMOTE:-origin}
mode=fetch
case "${1:-}" in
  --check) mode=check ;;
  --publish) mode=publish; shift ;;
  --ignore) mode=ignore ;;
  -h|--help) sed -n '2,22p' "$0"; exit 0 ;;
  "") ;;
  *) echo "unknown option: $1 (see --help)" >&2; exit 2 ;;
esac

[ -f "$lock" ] || { echo "$lock is missing" >&2; exit 1; }
branch=$(sed -n 's/^branch //p' "$lock")
commit=$(sed -n 's/^commit //p' "$lock")
[ -n "$branch" ] && [ -n "$commit" ] || { echo "$lock names no branch or commit" >&2; exit 1; }
entries=$(grep -E '^[0-9a-f]{40} ' "$lock" || true)

stale() {
  # Print every locked path that is missing or differs from its recorded hash.
  [ -n "$entries" ] || return 0
  local paths hashes
  paths=$(printf '%s\n' "$entries" | cut -d' ' -f2-)
  hashes=$(printf '%s\n' "$paths" | while IFS= read -r path; do
    if [ -f "$path" ]; then printf '%s\n' "$path"; else printf '%s\n' /dev/null; fi
  done | git hash-object --stdin-paths)
  paste -d' ' <(printf '%s\n' "$entries") <(printf '%s\n' "$hashes") |
    awk '$1 != $3 { print $2 }'
}

write_ignore() {
  # Every locked path is ignored, so fetched or regenerated data is never
  # committed to this tree by accident.
  local begin='# >>> generated test data: tests/test-data.lock (scripts/fetch-test-data.sh --ignore)'
  local end='# <<< generated test data'
  {
    [ -f .gitignore ] && awk -v b="$begin" -v e="$end" '$0==b{skip=1} !skip{print} $0==e{skip=0}' .gitignore
    printf '%s\n/.test-data/\n' "$begin"
    printf '%s\n' "$entries" | sed '/^$/d' | cut -d' ' -f2- | sed 's|^|/|'
    printf '%s\n' "$end"
  } > .gitignore.new
  mv .gitignore.new .gitignore
}

write_stamp() {
  mkdir -p "$stamp_dir"
  printf 'commit %s\nlock %s\n' "$commit" "$(git hash-object "$lock")" > "$stamp_dir/stamp"
}

if [ "$mode" = ignore ]; then
  write_ignore
  exit 0
fi

if [ "$mode" = check ]; then
  bad=$(stale)
  if [ -n "$bad" ]; then
    printf 'Generated test data is missing or differs from %s:\n' "$lock" >&2
    printf '%s\n' "$bad" | head -20 | sed 's/^/  /' >&2
    echo "Run scripts/fetch-test-data.sh" >&2
    exit 1
  fi
  write_stamp
  echo "Test data matches $lock ($commit)"
  exit 0
fi

if [ "$mode" = publish ]; then
  parent=$(git rev-parse -q --verify "refs/heads/$branch^{commit}" || git rev-parse -q --verify "$commit^{commit}" || true)
  index=$(mktemp)
  trap 'rm -f "$index"' EXIT
  if [ -n "$parent" ]; then GIT_INDEX_FILE=$index git read-tree "$parent"; else GIT_INDEX_FILE=$index git read-tree --empty; fi
  { printf '%s\n' "$entries" | cut -d' ' -f2-; printf '%s\n' ${@+"$@"}; } | sed '/^$/d' | sort -u > "$index.paths"
  while IFS= read -r path; do
    [ -f "$path" ] || { echo "cannot publish $path: no such file" >&2; exit 1; }
    blob=$(git hash-object -w "$path")
    GIT_INDEX_FILE=$index git update-index --add --cacheinfo 100644 "$blob" "$path"
  done < "$index.paths"
  tree=$(GIT_INDEX_FILE=$index git write-tree)
  new=$(git commit-tree "$tree" ${parent:+-p "$parent"} -m "Update generated test data")
  git update-ref "refs/heads/$branch" "$new"
  {
    sed -n '1,/^commit /p' "$lock" | sed "s/^commit .*/commit $new/"
    while IFS= read -r path; do
      printf '%s %s\n' "$(git rev-parse "$new:$path")" "$path"
    done < "$index.paths"
  } > "$lock.new"
  mv "$lock.new" "$lock"
  rm -f "$index.paths"
  write_ignore
  write_stamp
  echo "Committed $new to $branch and rewrote $lock and .gitignore."
  echo "Push the data with: git push $remote $branch; then commit $lock and .gitignore."
  exit 0
fi

if ! git cat-file -e "$commit^{commit}" 2>/dev/null; then
  git fetch --no-tags --quiet "$remote" "$commit" 2>/dev/null ||
    git fetch --no-tags --quiet "$remote" "refs/heads/$branch:refs/remotes/$remote/$branch" ||
    true
fi
git cat-file -e "$commit^{commit}" 2>/dev/null ||
  { echo "cannot fetch test data commit $commit (branch $branch) from $remote" >&2; exit 1; }

missing=$(stale)
if [ -n "$missing" ]; then
  # No associative arrays here: macOS runners still ship bash 3.2.
  while IFS= read -r path; do
    want=$(printf '%s\n' "$entries" | awk -v p="$path" '{blob=$1; $1=""; sub(/^ /, ""); if ($0 == p) print blob}')
    held=$(git rev-parse -q --verify "$commit:$path" 2>/dev/null || true)
    [ "$held" = "$want" ] ||
      { echo "$lock pins $path at $want, but $commit holds ${held:-nothing}" >&2; exit 1; }
    mkdir -p "$(dirname "$path")"
    git cat-file blob "$want" > "$path.fetch-tmp"
    mv "$path.fetch-tmp" "$path"
  done <<< "$missing"
fi
bad=$(stale)
[ -z "$bad" ] || { printf 'Test data still differs after fetching:\n%s\n' "$bad" >&2; exit 1; }
write_stamp
echo "Test data at $commit ($(printf '%s\n' "$entries" | grep -c . || true) files)"
