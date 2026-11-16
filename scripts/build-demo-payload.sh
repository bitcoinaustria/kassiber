#!/usr/bin/env bash
# Builds the bundled demo book (crates/kassiber/assets/demo-book.sql.gz) from a
# demo book the regtest harness produced (scripts/integration-harness.sh
# demo-full). Developer tool: it reads only the book directory it is given and
# writes only the output file; it contacts nothing.
#
#   scripts/build-demo-payload.sh <book dir> [out.sql.gz]
#
# <book dir> holds config/settings.json, data/kassiber.sqlite3 and attachments/.
# Run the harness's wallet syncs first so transaction_graph_cache holds the
# graphs' outside prevouts (docs/reference/demo.md#regenerating-the-payload);
# this script keeps that table.
#
# The payload is a gzip'd `sqlite3 .dump` of a stripped copy:
# - no backend rows, no default backend, manual backend setup with the four
#   built-in defaults tombstoned, so the demo can contact no server;
# - no wallet `backend` or `source_file` keys (local paths of the generator);
# - no AI providers, provider checks or chats;
# - an empty local analysis index (rebuilt by `kassiber demo create`);
# - the regtest stack's live quote, which the harness stores as a `mempool`
#   price, relabelled as the synthetic continuation it is
#   (`regtest-demo-synthetic`), so no row claims to be market data it is not;
# - attachment files embedded in a staging table `demo_payload_files`, which
#   `kassiber demo create` extracts and drops.
set -euo pipefail
book="${1:?usage: scripts/build-demo-payload.sh <book dir> [out.sql.gz]}"
repo="$(cd "$(dirname "$0")/.." && pwd)"
out="${2:-$repo/crates/kassiber/assets/demo-book.sql.gz}"
book="$(cd "$book" && pwd)"
[ -f "$book/data/kassiber.sqlite3" ] || { echo "no data/kassiber.sqlite3 under $book" >&2; exit 2; }
command -v sqlite3 >/dev/null || { echo "sqlite3 is required" >&2; exit 2; }
if LC_ALL=C head -c 16 "$book/data/kassiber.sqlite3" | grep -qv "SQLite format 3"; then
  echo "the book database is encrypted; build from a plaintext demo book" >&2; exit 2
fi
work="$(mktemp -d "${TMPDIR:-/tmp}/kassiber-demo-payload.XXXXXX")"
trap 'rm -rf "$work"' EXIT
db="$work/demo.sqlite3"
cp "$book/data/kassiber.sqlite3" "$db"
chmod 600 "$db"

# Wallet configuration is stored in Python's json.dumps format (sort_keys,
# ", " separators); SQLite's json_remove would rewrite it minified. The
# rewrite therefore goes through the stdlib json module, isolated (-I).
python3 -I - "$db" <<'PY'
import json, sqlite3, sys
conn = sqlite3.connect(sys.argv[1])
rows = conn.execute("SELECT id, config_json FROM wallets").fetchall()
for wallet_id, raw in rows:
    config = json.loads(raw or "{}")
    changed = False
    for key in ("backend", "source_file"):
        if key in config:
            del config[key]
            changed = True
    if changed:
        conn.execute("UPDATE wallets SET config_json=? WHERE id=?",
                     (json.dumps(config, sort_keys=True), wallet_id))
conn.commit()
PY

sqlite3 "$db" <<'SQL'
PRAGMA foreign_keys=OFF;
BEGIN;
DELETE FROM backends;
DELETE FROM settings WHERE key IN ('default_backend', 'bootstrap_default_backend');
INSERT OR REPLACE INTO settings(key, value) VALUES ('backend_bootstrap_mode', 'manual');
INSERT OR REPLACE INTO settings(key, value)
  VALUES ('bootstrap_backend_tombstones', '["fulcrum", "liquid", "liquid-blockstream", "mempool"]');
DELETE FROM ai_chat_messages;
DELETE FROM ai_chat_sessions;
DELETE FROM ai_provider_checks;
DELETE FROM ai_provider_secret_refs;
DELETE FROM ai_providers;
DELETE FROM chain_index_finding_nodes;
DELETE FROM chain_index_findings;
DELETE FROM chain_index_label_nodes;
DELETE FROM chain_index_labels;
DELETE FROM chain_index_aliases;
DELETE FROM chain_index_edges;
DELETE FROM chain_index_nodes;
DELETE FROM chain_index_source_nodes;
DELETE FROM chain_index_sources;
DELETE FROM chain_index_state;
-- The local regtest stack serves an invented BTC-EUR quote (the synthetic
-- continuation's last price) as if it were mempool.space's: label it as
-- the other synthetic rows. A clash with an existing row fails the build.
UPDATE rates_cache
   SET source = 'regtest-demo-synthetic',
       method = COALESCE(
         (SELECT replace(method, '-synthetic-continuation', '-synthetic-latest')
            FROM rates_cache
           WHERE source = 'regtest-demo-synthetic'
             AND method LIKE '%-synthetic-continuation'
           LIMIT 1),
         'regtest-demo-synthetic-latest')
 WHERE source = 'mempool';
CREATE TABLE demo_payload_files(relpath TEXT PRIMARY KEY, content BLOB NOT NULL);
COMMIT;
SQL

# Attachment files travel inside the dump, under their stored relative path.
if [ -d "$book/attachments" ]; then
  (cd "$book/attachments" && find . -type f ! -name '.*' | sed 's#^\./##' | LC_ALL=C sort) |
  while IFS= read -r rel; do
    case "$rel" in *"'"*|*..*) echo "refusing attachment path: $rel" >&2; exit 1;; esac
    sqlite3 "$db" "INSERT INTO demo_payload_files VALUES('$rel', readfile('$book/attachments/$rel'))"
  done
fi

# No leftover backend credentials, generator paths or provider-labelled rates.
if sqlite3 "$db" .dump | grep -Eq '"password": |/var/folders/|"backend": "'; then
  echo "stripped book still holds backend configuration or a generator path" >&2; exit 1
fi
if [ "$(sqlite3 "$db" "SELECT count(*) FROM rates_cache WHERE source = 'mempool' OR method = 'mempool_prices'")" != 0 ]; then
  echo "stripped book still holds a rate labelled as mempool market data" >&2; exit 1
fi
sqlite3 "$db" "VACUUM"
mkdir -p "$(dirname "$out")"
sqlite3 "$db" .dump | gzip -9n > "$out.tmp"
mv "$out.tmp" "$out"
raw=$(stat -f %z "$db" 2>/dev/null || stat -c %s "$db")
packed=$(stat -f %z "$out" 2>/dev/null || stat -c %s "$out")
graphs=$(sqlite3 "$db" "SELECT count(*) FROM transaction_graph_cache")
txs=$(sqlite3 "$db" "SELECT count(*) FROM transactions")
echo "demo payload: $out ($packed bytes; database $raw bytes, $txs transactions, $graphs cached graphs)"
