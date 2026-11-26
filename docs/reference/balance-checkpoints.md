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
| `api` | the exchange's own BTC balance, which the exchange contract already reads in the user's sync ([balances](exchange-contract.md#balances)) | yes | `wallets sync-<exchange>`, `ui.connections.exchange.sync` |
| `file` | the running balance after the last row of an imported file | reserved | not produced yet (see [limits](#limits)) |
| `chain` | the unspent outputs the wallet's last sync stored (`wallet_utxos` under the same source filter as the overview's chain balance), as of that refresh's `last_seen_at` | no, derived on every read | nothing: reading it opens no connection |
| `node` | the Core Lightning bookkeeper balances (`balance_snapshot` records, on-chain wallet plus channels) of the wallet's last successful node sync, as of its completion | no, derived on every read | nothing: reading it opens no connection |
| `manual` | the user's own entry, e.g. from a statement, or a CSV import | yes | `wallets checkpoints add\|import`, `ui.wallets.checkpoints.add\|import` |

Stored checkpoints come only from an explicit action: a user-triggered sync
or import, or a manual entry. Reading and comparing checkpoints uses stored
data only and opens no socket, HTTP request, RPC or probe.

### Exchange syncs

An exchange sync stores the provider's balance it already read as an `api`
checkpoint; there is no second balance read, extra request or background
read. `execute` (`crates/wallets/src/file_import/exchange_api.rs`) records it
with the imported rows, in a nested `WriteLock` inside the sync's write, before
the route is rechecked for publication:

- `asset` `BTC`, `amount_msat` the contract's whole-msat figure, `as_of` the
  balance's own time, `scope` the exchange's balance scope (the same text as
  `reported_balance.scope`), `origin_ref` `<kind>:api:<backend>`.
- The envelope adds `balance_checkpoint_id`; `provider_balance` and
  `reported_balance` are unchanged.
- An unknown figure (`provider_balance.status` `unavailable`) stores nothing.
- A checkpoint that cannot be stored rolls back alone; the sync still imports
  its history and reports `provider_balance`, without `balance_checkpoint_id`.
- A sync that fails, or whose route is revoked or goes offline before
  publication, stores neither rows nor checkpoint.
- A sync whose balance read itself fails keeps the
  [contract's](exchange-contract.md#balances) behaviour: an exchange that
  states its balance fails the sync closed on an answer it cannot read
  exactly.

Which exchanges state a balance, and which balance it is, is listed per
exchange: [Binance](rust-exchange-sync.md#request-sequences) (`spot_free_plus_locked`),
[Bitpanda](bitpanda.md) (`wallets_balance_after_latest_transaction`),
[Bitvavo](bitvavo.md) (`spot_available_plus_in_order`),
[Bitstamp](bitstamp.md) (`account_available_plus_reserved`),
[Bitfinex](bitfinex.md) (`exchange_and_funding_wallets`) and Bitcoin.de
(`account_statement_balance_after_last_booking`). Kraken and Coinbase syncs
read no balance and store no checkpoint. A differing checkpoint names what its
scope covers (`checkpoints::scope_note`), e.g. that Binance's API history does
not import spot trades.

## Storage

`balance_checkpoints` (additive; see [database compatibility](database-compatibility.md)):

| Column | Meaning |
| --- | --- |
| `id`, `workspace_id`, `profile_id`, `wallet_id` | the wallet's book; deleting the wallet or book deletes its checkpoints |
| `asset` | `BTC` or `LBTC`; anything else is refused (altcoins and stablecoins by name) |
| `as_of` | UTC ISO-8601 instant the balance refers to |
| `amount_msat` | exact integer msat, non-negative |
| `origin` | `api`, `file`, `chain`, `node` or `manual` (only `api`, `file` and `manual` are written) |
| `scope` | which balance it is, e.g. `spot_free_plus_locked` |
| `origin_ref` | the source, e.g. `binance:api:<backend>` or `csv:<file name>` |
| `attachment_id` | an existing attachment holding the statement (optional) |
| `note`, `created_at` | |

Recording an identical checkpoint again (same wallet, asset, origin, time,
amount, scope and note) adds nothing, so a replayed sync or re-imported file
is idempotent.

## Comparison

For each checkpoint the book balance is the sum of the wallet's
`journal_quantity_postings` (`location_kind = 'wallet'`, same asset) dated at
or before `as_of`, in exact msat. The result is:

- `matches`: the reported amount equals the book balance exactly. Tolerance
  is zero.
- `differs`: with `difference_msat` = reported minus book (positive when the
  source holds more). The message names quarantined and excluded transactions
  of the wallet (they are not in the book balance) and, for scopes that can
  legitimately differ, what the reported figure covers.
- `unknown`: not comparable, with a reason: `journals_stale`, `no_history`,
  `before_coverage`, `undated_postings` or `after_last_sync` (a `chain`
  checkpoint the book's later postings could explain).

Times typed by the user are a date (`2024-12-31`: the whole UTC day) or a
date-time with an explicit offset; a date-time without an offset is refused.
BTC amounts are plain decimals with at most 11 fractional digits.

## Reading

`report(profile, wallet?, history)` lists every wallet with checkpoints: for
each wallet and each origin and asset the latest checkpoint (`latest: true`),
the wallet's worst state (`differs` > `unknown` > `matches`), and counts over
the latest checkpoints. With `history` every checkpoint is listed.

## Surfaces

| Surface | Read | Write |
| --- | --- | --- |
| CLI | `wallets checkpoints list [--wallet W] [--history]` | `wallets checkpoints add --wallet W --btc N --date D [--asset BTC\|LBTC] [--note T] [--attachment ID]`, `remove --id ID`, `import --file F`, `export [--wallet W] [--file F]` |
| Daemon | `ui.wallets.checkpoints.list {wallet?, history?}` (read) | `.add {wallet, btc, date, asset?, note?, attachment?, expected_input_version?}`, `.remove {id, expected_input_version?}`, `.import {csv, source?}`, `.export {wallet?}` (operator) |
| Report readiness | `ui.report.blockers` adds a `balance_checkpoint_mismatch` warning listing the differing checkpoints; `ready` and the blockers are unchanged | |

All five kinds and leaves are Rust only (`spec::RUST_ONLY_NEW_COMMANDS`,
`RUST_ONLY_KINDS`). The operator broker classifies each leaf by its own path
(`checkpoints_command` extends the command path and the envelope kind). The
daemon never receives a path: a client sends CSV text it read itself, and
`export` returns the CSV text. The CLI `export --file` writes a new file and
refuses an existing one.

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
  wallet statements) are a follow-up.
- Lightning node checkpoints exist for Core Lightning only; LND's balances
  are read live and never stored, so there is nothing to compare without a
  request.
