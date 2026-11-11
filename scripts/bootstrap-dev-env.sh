#!/usr/bin/env bash
# Check and prepare a Kassiber development checkout: the pinned Rust toolchain
# that builds the `kassiber` binary, the frontend's pnpm dependencies, and the
# standard-library python3 the regtest harness runs under (`python3 -I`).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

RUST_CHANNEL="$(sed -n 's/^channel *= *"\([^"]*\)".*/\1/p' "$ROOT/rust-toolchain.toml")"
if ! command -v cargo >/dev/null 2>&1; then
  cat >&2 <<EOF
Kassiber is a Rust workspace: Rust $RUST_CHANNEL, which rust-toolchain.toml
pins, builds the \`kassiber\` binary and the desktop shell.

Install rustup from https://rustup.rs and rerun:
  ./scripts/bootstrap-dev-env.sh
EOF
  exit 1
fi
if command -v rustup >/dev/null 2>&1; then
  rustup toolchain install "$RUST_CHANNEL" --profile minimal --component rustfmt --component clippy
fi
case "$(rustc --version)" in
  "rustc $RUST_CHANNEL "*) ;;
  *)
    echo "warning: rustc is $(rustc --version), not the pinned Rust $RUST_CHANNEL (rust-toolchain.toml);" >&2
    echo "the quality gate requires the pinned toolchain." >&2
    ;;
esac

# SQLCipher is compiled from source with the books' store; outside macOS it
# also compiles OpenSSL, which needs a C compiler, make and perl.
missing=()
for tool in cc make perl; do
  command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
done
if [ "${#missing[@]}" -gt 0 ]; then
  cat >&2 <<EOF
The Rust build compiles SQLCipher (and OpenSSL outside macOS) and needs: ${missing[*]}

On Debian/Ubuntu:
  sudo apt-get update
  sudo apt-get install -y build-essential perl

Then rerun:
  ./scripts/bootstrap-dev-env.sh
EOF
  exit 1
fi

if command -v pnpm >/dev/null 2>&1; then
  pnpm --dir ui-tauri install --frozen-lockfile
else
  cat >&2 <<'EOF'
warning: pnpm not found; the desktop frontend needs the pnpm version
ui-tauri/package.json pins. Install it, then run:
  pnpm --dir ui-tauri install --frozen-lockfile
EOF
fi

# The regtest and demo harnesses are standard-library Python run with
# `python3 -I` (no virtual environment, no packages).
if command -v python3 >/dev/null 2>&1 \
  && python3 -I -c 'import sys; sys.exit(sys.version_info < (3, 11))' >/dev/null 2>&1; then
  harness_note="python3 $(python3 -I -c 'import platform; print(platform.python_version())') for the regtest harness"
else
  harness_note="no python3 3.11 or newer: only the regtest harness needs it (standard library only)"
fi

cat <<EOF

Kassiber dev environment is ready ($harness_note).

Use:
  scripts/fetch-test-data.sh
  cargo build -p kassiber --locked
  cargo test --workspace --locked
EOF
