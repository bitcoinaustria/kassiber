# Reset book data

"Reset book data" (`ui.profiles.reset_data`, Settings → Data & storage)
rebuilds what Kassiber observes or derives for the active book and keeps what
the user decided. Owner decision D1 (2026-11-09) set this contract: before
it, a reset deleted every authored decision, and on the regtest demo a reset
plus full sync booked 12,190.32 EUR more realized gain with no hold, because
the loan marks were gone.

Code: `kassiber/core/book_reset.py` (keep and re-bind),
`kassiber/core/maintenance.py` (the reset), and the claim in
`core/imports.insert_wallet_records`. Tests:
`tests/test_book_reset_keeps_decisions.py`.

## Two actions

| Action | Daemon args | What survives |
| --- | --- | --- |
| Reset book data (default) | `{"confirm":"RESET","confirm_profile":"…"}` | Connections, configuration, every decision and document. Synced transactions, journals and observer state are rebuilt. |
| Wipe everything | the same plus `"wipe_everything": true` | Connections and configuration only: the pre-D1 behaviour. Also discards decisions kept by an earlier reset that never bound, and removes managed attachment files. |

Both need the book-name confirmation and the usual sensitive-local
authorization (passphrase, or `DELETE LOCAL DATA` for plaintext books). The
desktop shows the wipe as a separate checkbox with its own typed challenge,
`WIPE EVERYTHING`. There is no CLI reset command in either runtime, and no
AI tool reaches either action: the kind stays out of the assistant's tool
surface, so the assistant gains no destructive path.

## Inventory

What the default reset does with each table of the active book.

| Class | Tables | Reset |
| --- | --- | --- |
| Observed | `transactions`, `chain_observation_provenance`, `chain_analysis_observations`, `chain_observer_instances` / `_coverage` / `_values` | Deleted; the resync re-observes them. Wallet sync checkpoints and last-sync times are cleared so the next sync re-imports every wallet in full ([backends](backends.md)). |
| Derived | `journal_entries`, `journal_quarantines`, `journal_tax_summary`, `journal_account_holdings`, `journal_wallet_holdings`, `journal_quantity_postings` / `_issues` / `_balances`, `journal_custody_decisions` / `_economic_relations` / `_gap_inputs` / `_reconciliations`, `import_batch_transactions` | Deleted; the next journal run rebuilds them. Journal markers are reset so it does. |
| Decision, parked | `loan_legs`, `transaction_pairs`, `transaction_pair_dismissals`, `direct_swap_payouts`, `custody_component_transaction_memberships`, `transaction_tags`, `commercial_links`, transaction `attachments`, `external_document_attachments`, `source_funds_links`, `source_funds_link_attachments`, `source_funds_source_attachments`, `source_funds_cases`, `source_funds_snapshots`, `transaction_edit_events`, `transaction_edit_fields` | Copied verbatim into `book_reset_parked_rows` (parents first), then cascaded away with the transactions. Restored when their transactions are back. Attachment files stay on disk; attachment GC treats parked files as live. |
| Decision, on the transaction row | `note`, `review_status`, `taxability_override`, `at_regime_override`, `at_category_override`, `kind_override`, `excluded`, `pricing_review_accepted_price`, `commercial_applied_link_id`; the price columns when the user set the price (`manual_override`, legacy `manual`, a reviewed commercial link, or a pricing edit in the history) | Kept in `book_reset_transactions.metadata_json` and written back onto the returned row. Provider and rate-cache prices are re-derived instead. |
| Decision, kept in place | `custody_components` and their legs, allocations, terms and commitments; `custody_gap_reviews` (+ sets, transactions); `custody_authored_evidence_snapshots`; `custody_ai_assistance_audits`; `filed_report_snapshots` and their impacts, resolutions and chain captures; `review_workflow_receipts`; `external_documents`; book-level `attachments`; `source_funds_sources`, `source_funds_recipients`; `tags`; `saved_views`; `swap_matching_rules`; `bip329_labels`; `chain_analysis_cases`, `_labels`, `_label_history`, `_datasets`, `_dataset_claims`; `ai_chat_sessions` / `_messages` | Untouched. Custody legs lose their `transaction_id` through `ON DELETE SET NULL` but keep `anchor_transaction_id`; the returned row re-points them. |
| Configuration | `workspaces`, `profiles` (tax settings), `accounts`, `wallets`, `backends`, `book_network_bindings`, `freshness_source_states` flags, `wallet_utxos`, `btcpay_provenance_records`, `lightning_node_records`, `import_batches`, `rates_cache` (unless `clear_shared_rates`) | Kept. |

`BookResetInventoryTest` walks the schema's foreign keys from
`transactions` and fails when a table the delete reaches is neither parked
nor declared derived, so a new table must be classified before it ships.

## Stable binding

Decisions point at transactions by row id, which a resync would regenerate.
Instead of rewriting every reference, the resync gives the returned row its
old id back:

- `book_reset_transactions` holds one row per deleted transaction: its id,
  wallet, fingerprint, external id, asset, direction, the facts below, and the
  kept metadata. The wallet id survives the reset; a deleted wallet cascades
  its records away.
- When `insert_wallet_records` would mint a new id, it first claims a pending
  record of the same wallet with the same fingerprint. Failing that it claims
  the only record with the same wallet, external id, asset and direction. Two
  or more candidates, or one already claimed, claim nothing: the row gets a
  fresh id and the decisions on the old one stay held.
- Facts are `direction`, `asset`, `amount`, `fee` and `amount_includes_fee`.
  A row that returns with equal facts is bound: its metadata is written back,
  custody legs anchored to it get their `transaction_id` again, and the
  record is deleted. A row with different facts keeps the old id but is
  marked `changed` with the changed field names; nothing is applied to it.
  Timestamps are not facts: a confirmation time can move, and journal rules
  such as pair chronology still check them.

Because ids are reused, everything else keyed by transaction id binds without
change: component anchors and their evidence commitments, gap reviews,
filed-report dependencies, edit history and the deterministic journal entry
ids. A reset followed by a full sync therefore rebuilds identical journals
when every source returns the same rows.

After each import that ran with kept records, every parked row is restored,
in `seq` order until nothing more binds, once each transaction it names is
bound and each of its other foreign-key parents exists. A component's
memberships bind only all together, so an incomplete component never looks
effective. A row that hits a constraint (for example a newer pair already
occupying the place) stays parked and held rather than override either.

## Holds

A kept decision that names a returned transaction but cannot bind holds that
transaction from the journals with `book_reset_decision_held` (category
`needs_decision`, action `sync_wallet`). It is fed through the custody
snapshot (`reset_hold_quarantines`) like a migration hold, so later rows
behind it get the usual `custody_basis_barrier`. Its detail names:

- `held_decisions`: kept rows by table that hold it;
- `missing_transactions`: the stable identity (old id, wallet, external id,
  asset, direction) of each transaction that has not come back;
- `changed_transactions`: returned rows whose facts changed, with the fields;
- `metadata_held`: whether the row's own kept fields were not applied.

A decision whose transactions are all still absent waits and holds nothing:
none of its rows are in the book. Kept records are never applied to other
rows and never deleted except by "Wipe everything" or by deleting the wallet
or book.

## Envelopes

The default reset adds `mode: "keep_decisions"` and:

```json
"decisions": {
  "kept": 31,
  "bind_after_resync": {"loan_legs": 1, "transaction_tags": 1, "transaction_metadata": 3},
  "kept_in_place": {"custody_components": 1, "tags": 1, "attachments": 0}
}
```

(abridged; every parked and kept-in-place table is listed, zero or not).
`removed` lists only the observed and derived counts plus `sync_checkpoints`,
`wallet_sync_times` and `attachment_files` (0). The wipe returns
`mode: "wipe_everything"`, the full pre-D1 `removed` map, and
`book_reset_transactions` / `book_reset_parked_rows` counts of discarded
kept records.

While a book has kept records, each import outcome (`wallets sync`, imports)
adds:

```json
"reset_decisions": {"rebound": 13, "held": 0, "waiting": 1}
```

`rebound` counts rows restored and transactions whose kept fields were
written back by this import; `held` and `waiting` count what is still kept
for the whole book (held: a named transaction is back; waiting: none is yet).
Books without kept records get no new key.

## Limits

- A held decision is resolved by syncing the wallet that brings its
  transaction back, or by "Wipe everything". There is no targeted discard
  yet ([TODO](../../TODO.md)).
- Splitting a book by network (`book_network_migration`) copies authored
  tables only; kept reset records stay in the original book.
- Journal semantics are unchanged: books without kept records build exactly
  as before, so no semantics-version bump is needed.
