#!/bin/zsh
set -euo pipefail

ROOT=${0:A:h:h}
APP=${KASSIBER_CAPTURE_APP:-$ROOT/build/kassiber_native.app/Contents/MacOS/kassiber_native}
DATA_ROOT=${KASSIBER_CAPTURE_DATA_ROOT:-/Users/dev/.kassiber/regtest-demo/data}
CAPTURE_HOME=${KASSIBER_CAPTURE_HOME:-/tmp/kassiber-native-capture-home}
TXID=3bf88da969196ae425cc3a34993a21b0e0534a77256a4255e42e1cfb0fa4afa5
CAPTURE_DELAY_SECONDS=${KASSIBER_CAPTURE_DELAY_SECONDS:-5}
CAPTURE_TIMEOUT_SECONDS=${KASSIBER_CAPTURE_TIMEOUT_SECONDS:-30}
CAPTURE_WIDTH=${KASSIBER_CAPTURE_WIDTH:-1440}
CAPTURE_HEIGHT=${KASSIBER_CAPTURE_HEIGHT:-900}
BATCH_TIMEOUT_SECONDS=${KASSIBER_CAPTURE_BATCH_TIMEOUT_SECONDS:-180}
SESSION_SETTLE_SECONDS=${KASSIBER_CAPTURE_SESSION_SETTLE_SECONDS:-2}
RECEIPT_DIR="$ROOT/verification/capture-receipts"
ACTIVE_PID=''

cleanup() {
  if [[ -n "$ACTIVE_PID" ]]; then
    kill "$ACTIVE_PID" 2>/dev/null || true
    wait "$ACTIVE_PID" 2>/dev/null || true
    ACTIVE_PID=''
  fi
}
trap cleanup EXIT INT TERM

# The verifier launches the packaged Mach-O as its own exact child PID. Normal
# Finder/open launches still reuse the user's running app, while this deliberate
# child remains independently capturable and cleanup cannot target another app.
launch_capture_app() {
  local app_bundle=${APP:h:h:h}
  if [[ "$app_bundle" != *.app || ! -d "$app_bundle" ]]; then
    echo "Capture requires a packaged .app executable: $APP" >&2
    return 1
  fi
  local assignment
  local -a environment
  while (( $# > 0 )); do
    if [[ "$1" != "--env" || $# -lt 2 ]]; then
      echo "Unsupported capture launch option: $1" >&2
      return 1
    fi
    assignment=$2
    if [[ -z "${assignment%%=*}" || "$assignment" != *=* ]]; then
      echo "Invalid capture environment assignment: $assignment" >&2
      return 1
    fi
    environment+=("$assignment")
    shift 2
  done
  env "${environment[@]}" "$APP" &
  ACTIVE_PID=$!
}

# A failed or partial run must never leave an older success manifest behind.
rm -f "$ROOT/verification/manifest.json"
if (( $# == 0 )); then
  rm -rf "$RECEIPT_DIR"
fi
mkdir -p "$RECEIPT_DIR"
mkdir -p "$CAPTURE_HOME"

wait_for_session() {
  local pid=$1
  local done_marker=$2
  local failure_marker=$3
  local timeout_seconds=$4
  local attempts=0
  local max_attempts=$(( timeout_seconds * 4 ))
  while [[ ! -s "$done_marker" && ! -s "$failure_marker" ]] && (( attempts < max_attempts )); do
    if ! kill -0 "$pid" 2>/dev/null; then
      break
    fi
    sleep 0.25
    (( attempts += 1 ))
  done
  if [[ -s "$done_marker" ]]; then
    return 0
  fi
  if [[ -s "$failure_marker" ]]; then
    echo "Native capture failed: $(<"$failure_marker")" >&2
  else
    echo "Timed out waiting for native capture session" >&2
  fi
  return 1
}

capture() {
  local screen=$1
  local language=$2
  local filename=$3
  local onboarding=${4:-0}
  local done_marker="$ROOT/verification/.capture-single-$filename.done"
  local failure_marker="$ROOT/verification/.capture-single-$filename.failed"
  rm -f \
    "$ROOT/verification/$filename" \
    "$RECEIPT_DIR/$filename.capture.json" \
    "$done_marker" "$failure_marker"
  launch_capture_app \
    --env "HOME=$CAPTURE_HOME" \
    --env "CFFIXED_USER_HOME=$CAPTURE_HOME" \
    --env "KASSIBER_REPO_ROOT=$ROOT/.." \
    --env "KASSIBER_DATA_ROOT=$DATA_ROOT" \
    --env "KASSIBER_PREVIEW_SCREEN=$screen" \
    --env "KASSIBER_LANGUAGE=$language" \
    --env KASSIBER_PREVIEW_AI=1 \
    --env "KASSIBER_PREVIEW_RECONCILE=$TXID" \
    --env "KASSIBER_PREVIEW_IMPORT_FILE=$ROOT/verification/sample-ledger.csv" \
    --env "KASSIBER_PREVIEW_ONBOARDING=$onboarding" \
    --env KASSIBER_PREVIEW_CAPTURE_BACKEND=appkit \
    --env "KASSIBER_PREVIEW_DELAY_SECONDS=$CAPTURE_DELAY_SECONDS" \
    --env "KASSIBER_PREVIEW_WIDTH=$CAPTURE_WIDTH" \
    --env "KASSIBER_PREVIEW_HEIGHT=$CAPTURE_HEIGHT" \
    --env "KASSIBER_PREVIEW_OUTPUT=$ROOT/verification/$filename" \
    --env "KASSIBER_PREVIEW_RECEIPT_DIR=$RECEIPT_DIR" \
    --env "KASSIBER_PREVIEW_DONE=$done_marker" \
    --env "KASSIBER_PREVIEW_FAILED=$failure_marker"
  local session_status=0
  wait_for_session \
    "$ACTIVE_PID" "$done_marker" "$failure_marker" \
    "$CAPTURE_TIMEOUT_SECONDS" || session_status=$?
  cleanup
  ACTIVE_PID=''
  sleep "$SESSION_SETTLE_SECONDS"
  if (( session_status != 0 )); then
    return "$session_status"
  fi
  python3 "$ROOT/Scripts/write_verification_manifest.py" \
    --verify "$ROOT/verification/$filename"
  rm -f "$done_marker" "$failure_marker"
}

capture_batch() {
  local language=$1
  local plan=$2
  shift 2
  local done_marker="$ROOT/verification/.capture-$language.done"
  local failure_marker="$ROOT/verification/.capture-$language.failed"
  rm -f "$done_marker" "$failure_marker"
  local filename
  for filename in "$@"; do
    rm -f \
      "$ROOT/verification/$filename" \
      "$RECEIPT_DIR/$filename.capture.json"
  done
  launch_capture_app \
    --env "HOME=$CAPTURE_HOME" \
    --env "CFFIXED_USER_HOME=$CAPTURE_HOME" \
    --env "KASSIBER_REPO_ROOT=$ROOT/.." \
    --env "KASSIBER_DATA_ROOT=$DATA_ROOT" \
    --env KASSIBER_PREVIEW_SCREEN=dashboard \
    --env "KASSIBER_LANGUAGE=$language" \
    --env KASSIBER_PREVIEW_AI=1 \
    --env "KASSIBER_PREVIEW_RECONCILE=$TXID" \
    --env "KASSIBER_PREVIEW_IMPORT_FILE=$ROOT/verification/sample-ledger.csv" \
    --env KASSIBER_PREVIEW_ONBOARDING=0 \
    --env KASSIBER_PREVIEW_CAPTURE_BACKEND=appkit \
    --env "KASSIBER_PREVIEW_DELAY_SECONDS=$CAPTURE_DELAY_SECONDS" \
    --env "KASSIBER_PREVIEW_WIDTH=$CAPTURE_WIDTH" \
    --env "KASSIBER_PREVIEW_HEIGHT=$CAPTURE_HEIGHT" \
    --env "KASSIBER_PREVIEW_PLAN=$plan" \
    --env "KASSIBER_PREVIEW_OUTPUT_DIR=$ROOT/verification" \
    --env "KASSIBER_PREVIEW_RECEIPT_DIR=$RECEIPT_DIR" \
    --env "KASSIBER_PREVIEW_DONE=$done_marker" \
    --env "KASSIBER_PREVIEW_FAILED=$failure_marker"
  local session_status=0
  wait_for_session \
    "$ACTIVE_PID" "$done_marker" "$failure_marker" \
    "$BATCH_TIMEOUT_SECONDS" || session_status=$?
  cleanup
  ACTIVE_PID=''
  sleep "$SESSION_SETTLE_SECONDS"
  if (( session_status != 0 )); then
    return "$session_status"
  fi
  for filename in "$@"; do
    python3 "$ROOT/Scripts/write_verification_manifest.py" \
      --verify "$ROOT/verification/$filename"
  done
  rm -f "$done_marker" "$failure_marker"
}

if (( $# == 3 || $# == 4 )); then
  capture "$1" "$2" "$3" "${4:-0}"
  exit 0
fi
if (( $# != 0 )); then
  echo "usage: $0 [screen language filename [onboarding]]" >&2
  exit 2
fi

capture books en foundation-en.png 1
english_plan='dashboard|dashboard-en.png,transactions|transactions-en.png,wallets|wallets-en.png,reports|reports-en.png,journals|journals-en.png,quarantine|quarantine-en.png,swaps|swaps-en.png,reconcile|reconcile-en.png,books|books-en.png,connections|connections-en.png,imports|imports-en.png,exitTax|exit-tax-en.png,sourceFunds|source-funds-en.png,activity|activity-en.png,privacyMirror|privacy-mirror-en.png,birdsEye|birds-eye-en.png,egress|egress-en.png,logs|logs-en.png,settings|settings-en.png,assistant|assistant-en.png'
capture_batch en "$english_plan" \
  dashboard-en.png transactions-en.png wallets-en.png reports-en.png \
  journals-en.png quarantine-en.png swaps-en.png reconcile-en.png \
  books-en.png connections-en.png imports-en.png exit-tax-en.png \
  source-funds-en.png activity-en.png privacy-mirror-en.png birds-eye-en.png \
  egress-en.png logs-en.png settings-en.png assistant-en.png
german_plan='dashboard|dashboard-de.png,transactions|transactions-de.png,assistant|assistant-de.png'
capture_batch de "$german_plan" dashboard-de.png transactions-de.png assistant-de.png
python3 "$ROOT/Scripts/write_verification_manifest.py" \
  --app-executable "$APP" \
  --verification-dir "$ROOT/verification" \
  --output "$ROOT/verification/manifest.json"
