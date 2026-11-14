# Journal reproducibility

The reproducibility check answers "are these numbers right?" without trusting
the stored journals on faith (`kassiber journals verify`, daemon
`ui.journals.verify`, assistant read tool `ui_journals_verify`): it
recomputes the active book's journals from its current inputs in memory and
compares them with the stored journals. Local, read-only, writes nothing.

## Reproducibility check

The check never writes to the book. It copies the open book into a private
in-memory SQLite database with SQLite's online backup (the same copy review
simulation uses; an encrypted book's copy gets a fresh random key), runs the
production `journals process` pipeline on that copy, and compares the copy's
journal tables with the book's. Running the real pipeline, instead of a
second implementation, is the point: the check exercises exactly the code that
wrote the stored rows.

Status values:

| `status` | Meaning |
| --- | --- |
| `identical` | Every compared row matches byte for byte (after dropping volatile columns). |
| `different` | At least one row differs; `differences` lists the first `limit`. |
| `stale` | The book's inputs changed since processing (input version, transaction count, calculation semantics), or processing now would first repair inputs or price transactions from the rates cache. Process first; this is not a diff. |
| `not_processed` | Journals were never processed. |
| `no_transactions` | Nothing to compare. |
| `refused` | Recomputation refused (the same refusal `journals process` would give, e.g. a custody blocker); `error` carries its code and message. |

When rows differ the check recomputes a second time on a fresh copy and
reports `deterministic: true|false`: whether two recomputations agree with
each other. `false` points at nondeterminism in the code rather than stale or
damaged stored rows.

### Compared outputs

| Table | Row key | Context reported |
| --- | --- | --- |
| `journal_entries` | entry `id` (derived from transaction, wallet, type, asset, ordinal) | transaction, wallet, year |
| `journal_tax_summary` | year, asset, transaction type, gains type | year |
| `journal_wallet_holdings` | wallet, asset | wallet |
| `journal_account_holdings` | account, asset | — |
| `journal_quantity_balances` | location kind, location id, asset | wallet when the location is one |
| `journal_quantity_postings` | `posting_id` | transaction, year |
| `journal_quantity_issues` | `issue_id` | year |
| `journal_custody_decisions` | `decision_id` | transaction, year |
| `journal_custody_economic_relations` | `relation_id` | transaction, year |
| `journal_quarantines` | transaction | transaction |

Volatile columns are dropped before comparison: `created_at`, random row `id`s
of the holdings and tax-summary tables, `workspace_id`/`profile_id` (constant
per book). Values compare by SQLite type and exact value; REAL by bit pattern,
so `0.1` and `0.1000000000000001` differ.

### Diff format

```json
{"status":"different","deterministic":true,"limit":20,"truncated":false,
 "tables":[{"table":"journal_entries","stored":6,"recomputed":6,"differing":1}],
 "by_wallet":{"wallet":1},"by_year":{"2024":1},
 "differences":[{"table":"journal_entries","change":"changed","key":"…",
   "transaction_id":"…","wallet_id":"wallet","year":"2024",
   "columns":[{"column":"cost_basis_exact","stored":"100.00","recomputed":"90.00"}]}]}
```

`change` is `changed` (same key, different columns), `missing` (stored row
the recomputation does not produce) or `unexpected` (recomputed row not
stored). `limit` defaults to 20 and is capped at 200; counts always cover every
row.

## Implementation map

- [`crates/journals/src/reproduce.rs`](../../crates/journals/src/reproduce.rs):
  the check, the compared tables, the diff.
