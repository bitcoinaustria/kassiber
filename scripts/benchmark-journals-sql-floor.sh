#!/usr/bin/env bash
# SQLite's own cost for the reads and writes of one `journals process`.
#
#   scripts/benchmark-journals-sql-floor.sh <book-dir> [scratch-dir]
#
# Runs on a copy of <book-dir>/data/kassiber.sqlite3 (plaintext, already
# processed once) with the sqlite3 shell, so no Python or Rust row handling is
# included. Reads are the journal snapshot's largest queries with output
# discarded; writes fully replace every stored journal table with the rows the
# last build stored. This measures the original full-rewrite floor; the current
# writer retains unchanged entries and quantity postings. It does not measure
# that no-op path. Times are observations for R4, not pass/fail thresholds.
set -euo pipefail
book=$1
scratch=${2:-$(dirname "$0")/../target/r4-bench/sql-floor}
mkdir -p "$scratch"
db="$scratch/floor.sqlite3"
rm -f "$db" "$db-wal" "$db-shm"
sqlite3 "$book/data/kassiber.sqlite3" ".backup '$db'"
profile=$(sqlite3 "$db" "SELECT id FROM profiles LIMIT 1")
tables="journal_entries journal_quarantines journal_tax_summary journal_account_holdings
journal_wallet_holdings journal_custody_gap_inputs journal_custody_reconciliations
journal_quantity_postings journal_quantity_issues journal_quantity_balances
journal_custody_decisions journal_custody_economic_relations"

{
  # The connection settings kassiber/db.py applies to a plaintext WAL book.
  echo "PRAGMA journal_mode = wal; PRAGMA synchronous = NORMAL; PRAGMA temp_store = MEMORY;"
  echo "PRAGMA cache_size = -16000; PRAGMA mmap_size = 268435456; PRAGMA foreign_keys = ON;"
  echo ".timer on"
  echo "-- read: custody transactions query"
  echo "SELECT t.*, w.label, w.kind, w.account_id, w.config_json, o.authority_version, o.observer_kinds_json,
  o.graph_hash, o.quantity_hash, o.fee_attribution, o.application_revision, o.observed_at,
  COALESCE(a.code, 'treasury'), COALESCE(a.label, 'Treasury')
  FROM transactions t JOIN wallets w ON w.id = t.wallet_id LEFT JOIN accounts a ON a.id = w.account_id
  LEFT JOIN chain_observation_provenance o ON o.transaction_id = t.id
  WHERE t.profile_id = '$profile' AND t.excluded = 0 ORDER BY t.occurred_at, t.created_at, t.id;"
  echo "-- read: rate rows"
  echo "SELECT asset, fiat_rate, fiat_value, fiat_rate_exact, fiat_value_exact, amount FROM transactions WHERE profile_id = '$profile' AND excluded = 0 ORDER BY occurred_at DESC, created_at DESC, id DESC;"
  for table in $tables; do echo "CREATE TEMP TABLE saved_$table AS SELECT * FROM $table WHERE profile_id = '$profile';"; done
  echo "-- write: replace every stored journal table, then commit"
  echo "BEGIN IMMEDIATE;"
  for table in $tables; do
    echo "DELETE FROM $table WHERE profile_id = '$profile';"
    echo "INSERT INTO $table SELECT * FROM saved_$table;"
  done
  echo "UPDATE profiles SET last_processed_at = last_processed_at WHERE id = '$profile';"
  echo "COMMIT;"
} > "$scratch/floor.sql"
echo "book: $book ($(sqlite3 "$db" "SELECT COUNT(*) FROM transactions") transactions)"
sqlite3 "$db" < "$scratch/floor.sql" 2>&1 | grep -a '^Run Time' | awk '{n++; print n": "$0}'
echo "statement order: 1 transactions read, 2 rate read, then per table: saved copy (untimed setup),"
echo "BEGIN, DELETE/INSERT pairs in table order, profile UPDATE, COMMIT"
rm -f "$db" "$db-wal" "$db-shm"
