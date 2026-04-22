#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <phase-dir> <screen-dir>" >&2
  echo "example: $0 phase-2 overview" >&2
  exit 1
fi

phase_dir="$1"
screen_dir="$2"

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
target_dir="$repo_root/docs/design/$phase_dir/$screen_dir"
templates_dir="$repo_root/docs/design/templates"

mkdir -p "$target_dir/refs" "$target_dir/source"

if [[ ! -f "$target_dir/screen-spec.md" ]]; then
  cp "$templates_dir/screen-spec.md" "$target_dir/screen-spec.md"
fi

if [[ ! -f "$target_dir/screenshot-review.md" ]]; then
  cp "$templates_dir/screenshot-review.md" "$target_dir/screenshot-review.md"
fi

cat <<EOF
Created design workspace:
  $target_dir

Next steps:
  1. Add frozen reference screenshots under refs/
  2. Record exported mockup details under source/
  3. Fill in screen-spec.md before touching QML
  4. Use screenshot-review.md after the static QML pass
EOF
