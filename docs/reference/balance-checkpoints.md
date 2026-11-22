# Balance checkpoints

A balance checkpoint is a balance someone other than the book reports for one
wallet at one time, compared with the balance the book gives that wallet at
that time. It is the most direct signal that an import or sync is complete:
when an exchange says the account holds 0.5 BTC and the book holds 0.4 BTC
for the same moment, history is missing.

Checkpoints are diagnostics, like
`kassiber_custody::quantity::balances::compare_wallet_balances`. They never
change custody, journals, cost basis, tax results or report blockers. A
differing checkpoint is a review item and a readiness warning ("the source
may be incomplete"), never an automatic correction.

The model lives in `crates/wallets/src/checkpoints.rs`; the derived chain
checkpoints and the request adapter in `crates/workspace/src/balance_checks.rs`.

## Origins

| Origin | Where it comes from | Stored | Created by |
| --- | --- | --- | --- |
| `api` | the exchange's own BTC balance, read inside the same consented sync as the history ([reported balances](rust-exchange-sync.md#reported-balances)) | yes | `wallets sync-<exchange>`, `ui.connections.exchange.sync` |
| `file` | the running balance after the last row of an imported file | reserved | not produced yet (see [limits](#limits)) |
| `chain` | the unspent outputs the wallet's last sync stored (`wallet_utxos` under the same source filter as the overview's chain balance), as of that refresh's `last_seen_at` | no, derived on every read | nothing: reading it opens no connection |
| `node` | the Core Lightning bookkeeper balances (`balance_snapshot` records, on-chain wallet plus channels) of the wallet's last successful node sync, as of its completion | no, derived on every read | nothing: reading it opens no connection |
| `manual` | the user's own entry, e.g. from a statement, or a CSV import | yes | `wallets checkpoints add|import`, `ui.wallets.checkpoints.add|import` |

Stored checkpoints come only from an explicit action: a user-triggered sync
or import, or a manual entry. Display reads (the desktop card, the wallets
list, report readiness, the notification list) compare stored data and open no
socket, HTTP request, RPC or probe.

## Storage

`balance_checkpoints` (additive; see [database compatibility](database-compatibility.md)):

| Column | Meaning |
| --- | --- |
| `id`, `workspace_id`, `profile_id`, `wallet_id` | the wallet's book; deleting the wallet or book deletes its checkpoints |
| `asset` | `BTC` or `LBTC`; anything else is refused (altcoins and stablecoins by name) |
| `as_of` | UTC ISO-8601 instant the balance refers to |
| `amount_msat` | exact integer msat, non-negative (`CHECK(typeof(amount_msat) = 'integer' AND amount_msat >= 0)`) |
| `origin` | `api`, `file`, `chain`, `node` or `manual` (only `api`, `file` and `manual` are written) |
| `scope` | which balance it is, e.g. `kraken_spot` |
| `origin_ref` | the source, e.g. `kraken:api:<backend>` or `csv:<file name>` |
| `attachment_id` | an existing attachment holding the statement (optional) |
| `note`, `created_at` | |

Recording an identical checkpoint again (same wallet, asset, origin, time,
amount and scope) adds nothing, so a replayed sync or re-imported file is
idempotent.

## Comparison

For each checkpoint the book balance is the sum of the wallet's
`journal_quantity_postings` (`location_kind = 'wallet'`, same asset) dated at
or before `as_of`, in exact msat. The float values of the balance history
report are never used. The result is:

- `matches`: the reported amount equals the book balance exactly. Tolerance
  is zero.
- `differs`: with `difference_msat` = reported minus book (positive when the
  source holds more). The message names quarantined and excluded transactions
  of the wallet (they are not in the book balance) and, for scopes that can
  legitimately differ, what the reported figure covers (staking, Earn,
  pending transfers, Binance spot trades the API history does not import).
  Known benign differences are explained, not hidden.
- `unknown`: not comparable, with a reason:
  - `journals_stale`: journals are not current (`not_processed` or `stale`).
  - `no_history`: the wallet has no active transaction in the asset.
  - `before_coverage`: `as_of` is before the wallet's first active
    transaction, so the book's zero says nothing about the source.
  - `undated_postings`: a posting of the wallet has no date.
  - `after_last_sync`: a `chain` checkpoint differs, the book holds
    transactions dated after that sync's coin snapshot (a block timestamp
    ahead of the local clock), and their sum could explain the gap (same sign,
    no smaller). The difference stays visible, and the mismatch summaries
    (readiness, the wallets list) keep it as `provisional`. When the later
    postings cannot explain the gap the checkpoint `differs`, with the note
    `after_last_sync`: the comparison is incomplete until the next sync.

Times typed by the user are a date (`2024-12-31`: the whole UTC day, every
instant before the next midnight; stored as `23:59:59.999999Z`, which at the
stored microsecond precision is that exclusive bound) or a date-time with an
explicit offset, compared inclusively. A date-time without
an offset is ambiguous and refused. BTC amounts are plain decimals with at
most 11 fractional digits; a sign, exponent or finer amount is refused.

## Reading

`report(profile, wallet?, history)` lists every wallet with checkpoints: for
each wallet and each origin and asset the latest checkpoint (`latest: true`),
the wallet's worst state (`differs` > `unknown` > `matches`), and counts over
the latest checkpoints. With `history` every checkpoint is listed.

## Surfaces

| Surface | Read | Write |
| --- | --- | --- |
| CLI | `wallets checkpoints list [--wallet W] [--history]` | `wallets checkpoints add --wallet W --btc N --date D [--asset BTC|LBTC] [--note T] [--attachment ID]`, `remove --id ID`, `import --file F`, `export [--wallet W] [--file F]` |
| Daemon | `ui.wallets.checkpoints.list {wallet?, history?}` (read) | `.add {wallet, btc, date, asset?, note?, attachment?, expected_input_version?}`, `.remove {id, expected_input_version?}`, `.import {csv, source?}`, `.export {wallet?}` (operator) |
| Desktop | the **Balance checks** card on a connection's page, a mismatch mark in the wallets list, one Needs attention row naming the wallets whose reported balance differs (or waits for the next sync) | **Add balance** on the card; **View history** lists every checkpoint with **Remove** for stored ones |
| Assistant and MCP | `ui_wallets_checkpoints_list` ([AI](ai.md#balance-checkpoints)), without import references or attachment ids | `ui_wallets_checkpoints_add`, `ui_wallets_checkpoints_remove`: consented, bound to `expected_input_version` from the read |
| Report readiness | `ui.report.blockers` adds a `balance_checkpoint_mismatch` warning listing the differing checkpoints; `ready` and the blockers are unchanged. The desktop Reports page shows a "Reported balance differs" notice naming the wallets, after the journal, quarantine and completeness notices | |

All five kinds and leaves are Rust only. The daemon never receives a path:
the desktop sends CSV text it read itself, and `export` returns the CSV
text. The CLI `export --file` writes a new file and refuses an existing one.
The Needs attention row names the wallets, never an amount, and opens the
connection (the wallets list when several differ); it clears when a later read shows the wallet matching again.
Like every notification it lives in memory only.

## CSV

Columns `date,wallet_id,wallet,btc_amount,asset,origin,note,checkpoint_id`;
`wallet_id`, `asset`, `origin`, `note` and `checkpoint_id` are optional on import, and one of `wallet_id`
and `wallet` is required. A row names its wallet by `wallet_id` when that id
is in this book, else by `wallet` (id or label; a label export prefixed
with `'` is also found without it); an ambiguous label is refused. Every row
is checked before any is stored; one bad row refuses the file (the message
names the row), and so does a row with more or fewer fields than the header
(an unquoted decimal comma such as `0,5` splits into two). Imported rows
are `manual` checkpoints with `origin_ref` `csv:<file>`: a balance read
from a file is the user's statement, not an API or chain fact, so an
`origin` column other than `manual` is kept only as a note prefix
(`imported api checkpoint: ...`). Export writes stored
checkpoints oldest first; a wallet label or note starting with `=`, `+`, `-`
or `@` is prefixed with `'` so a spreadsheet does not run it. A row this book exported
names its checkpoint (`checkpoint_id`); when that stored row still has the
same wallet, asset, time, amount, origin and note (a `'`-escaped note counts
as its stored text), the import leaves it alone, so exporting and
re-importing adds nothing. A row that keeps the id but changes the note is
new evidence and becomes a manual checkpoint. Every other row is the user's own
statement and becomes a manual checkpoint, even when an API or chain
checkpoint has the same time and amount; only an identical manual row (same
note included) is not repeated. A file never gains another origin's
authority.

## Limits

- `file` checkpoints are not produced yet. Importers whose files carry a
  running balance (the Kraken ledger CSV `balance` column, Strike and Bull
  wallet statements, a generic ledger with a balance column) are listed in
  [TODO](../../TODO.md) for a follow-up; until then such a figure can be
  entered by hand or imported as CSV.
- Lightning node checkpoints exist for Core Lightning only; LND's balances
  are read live by `ui.connections.node.snapshot` and never stored, so there
  is nothing to compare without a request.
- The exchange-sync explanation for a missing balance
  (`reported_balance_unavailable`) is kept for the open session only, as
  the rest of the desktop's last-sync record is.
- Attaching a statement takes an existing attachment id; uploading a new
  document from the card is not offered yet.
