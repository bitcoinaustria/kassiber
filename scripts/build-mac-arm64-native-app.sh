#!/usr/bin/env bash
set -euo pipefail

# Build the separate Swift/AppKit native frontend.  The repository's existing
# scripts/build-macos-arm64-app.sh remains the Tauri/React web frontend build;
# this wrapper intentionally has a different name and delegates only to the
# native implementation under ui-macos.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec "$ROOT/ui-macos/Scripts/build-macos-arm64-app.sh" "$@"
