#!/usr/bin/env bash
set -euo pipefail

: "${DEVELOPER_ID_APPLICATION:?Set DEVELOPER_ID_APPLICATION to the Developer ID Application identity}"
: "${APPLE_TEAM_ID:?Set APPLE_TEAM_ID}"
: "${NOTARY_PROFILE:?Set NOTARY_PROFILE to a notarytool keychain profile}"
: "${KASSIBER_SIDECAR_SOURCE:?Set KASSIBER_SIDECAR_SOURCE to a self-contained arm64 Kassiber sidecar executable}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "$ROOT/.." && pwd)"

if [[ -n "$(git -C "$REPO_ROOT" status --porcelain --untracked-files=normal)" ]]; then
  echo "Refusing a Developer ID/notarized distribution from a dirty worktree." >&2
  echo "Commit or remove tracked and untracked release inputs first." >&2
  exit 2
fi

SIGN_IDENTITY="$DEVELOPER_ID_APPLICATION" \
NOTARIZE=1 \
CREATE_ZIP=1 \
KASSIBER_SIDECAR_SOURCE="$KASSIBER_SIDECAR_SOURCE" \
  "$ROOT/Scripts/build-macos-arm64-app.sh"
