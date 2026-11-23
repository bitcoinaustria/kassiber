# Other crypto assets from exchanges and tax-platform exports

Status: accepted 2026-11-12 by the owner. Bitcoin keeps first-class support;
other crypto assets become an opt-in per wallet. This narrows the overview's
"broad altcoin product scope" non-goal: exchange API syncs and provider
exports may carry other crypto assets into the book, under the rules below.
Chain observation, ownership inference and every on-chain feature stay
Bitcoin and Liquid only.

## Decisions

- **Scope: crypto only.** Coins, tokens and stablecoins. Stocks, ETFs, ETCs,
  metals, indices and other securities stay out of the book; they are named
  in the import result as excluded, never imported, never taxed.
- **Opt-in per wallet.** A wallet's `asset_scope` config is `bitcoin` (the
  default, today's behaviour) or `crypto`. In `bitcoin` scope an import or
  sync still skips other assets, but reports which tickers it skipped and how
  many rows, with the command that turns the scope on (`kassiber wallets
  update --wallet W --config '{"asset_scope": "crypto"}'`); the desktop shows
  the same as a toggle. Turning it on and re-running is idempotent: rows
  deduplicate by external id. Turning it off only stops later imports and
  syncs from keeping other assets; rows already imported stay in the book.
- **Desktop toggle.** The exchange card and the import result offer "Also
  track these assets", which stores the scope through `ui.wallets.update`
  (`asset_scope`, behind the usual passphrase or typed acknowledgement) and
  then syncs, or imports the same file again (or asks for it again when the
  path was not kept). No separate daemon kind: a new kind would change the
  `supported_kinds` every recorded daemon session pins. Neither
  `ui.wallets.update` nor the scope is an AI tool; giving the assistant the
  toggle is a separate decision. See [exchanges in the
  desktop](../reference/exchange-desktop.md#other-crypto-assets).
- **Units.** Every crypto asset uses the same integer unit as BTC: 10^-11 of
  a whole unit (msat for BTC). Exchanges and exports report at most about 10
  decimals, so this is exact for them. For any asset other than BTC/LBTC, an
  amount finer than 11 decimals or larger than the column holds (about
  92 million units per row) is refused, never rounded. A per-asset scale for
  large-supply tokens is designed but not built; see
  [per-asset scale](#per-asset-scale-design-not-implemented).
- **Pricing.** Trades carry the exchange's own execution value
  (`exchange_execution`, `exact`) for any asset; no rate provider is needed.
  Rows without an execution value (deposits, rewards, airdrops) stay unpriced
  and land in the existing `missing_price` review until priced; the rates
  crate stays BTC-only for now.
- **Counter-leg swap pricing** (2026-11-14). A swap is one trade, so when
  journal processing prices its Bitcoin (or Liquid Bitcoin) leg from the
  rates cache, the same pass values the other leg at that value and copies
  its provenance (`pricing_method` `swap_counter_leg`), so repricing the rate
  pair clears both. The value is derived on every pass (2026-11-15, after
  review): each pass clears every counter-leg price and re-derives it from the
  pairs that exist, so an unpaired, re-paired, excluded or hand-priced Bitcoin
  leg never leaves a stale basis on the other leg. No new rate source and no egress: it only uses a cached
  rate the book already has. A crypto-to-crypto swap without a Bitcoin leg
  stays unpriced in review, and no asset is ever priced from a rate of its
  own the rates layer does not have. It lives in the one place rows are
  priced from the cache (`journals/src/prebuild/pricing.rs`).
- **Withdrawals to an own untracked wallet** (2026-11-14). No chain observer
  exists for other assets, so an exchange withdrawal of ETH to the owner's
  wallet would otherwise be a disposal. The existing custody component
  already covers it for any asset: the withdrawal as source, its fee, and a
  `retained` leg in an `untracked_wallet` marked as reviewed retained custody
  ([custody components](../reference/custody-components.md#missing-historical-wallets)).
  No new mechanism; the assistant reaches it through the same component
  tools.
- **A partial basis is never the full one** (2026-11-14). The overview's
  other assets carry `costBasis`, `basisQuantity` and `basisComplete`, and an
  unrealized result only with a complete basis. The assistant reads them
  through its projection: when `basisComplete` is false the basis arrives as
  `partialCostBasis` and `costBasis` is `null`, in the book overview and in
  each book of the book-set overview. The desktop does not show other assets
  yet (step 4); when it does, it shows such a basis as incomplete.
- **Swaps.** A crypto-to-crypto trade is two rows: the outbound leg (kind
  `swap`) and the inbound leg (kind `swap`), each with its fiat value when the
  provider gives one, and the same `swap_ref`. The import authors the pair
  between them in the same write (`transfers.pair`, kind `manual`, pair source
  `provider`; a re-run keeps a pair of exactly these legs, and a leg
  already in another pair or custody review refuses the whole import), with
  the policy the profile's country recommends: carrying value where the
  country carries basis across assets (Austria: crypto-to-crypto is
  tax-neutral), taxable elsewhere. The tax engine already handles both,
  including Austrian Alt/Neu lots, but both need each leg's price: until
  then journals hold the pair in review (`at_swap_link` in Austria,
  `missing_spot_price` elsewhere), and the user prices the legs (`journals
  quarantine resolve price-override`). Coinbase converts carry a fiat value;
  Bitpanda, Kraken, CoinTracking and Blockpit swaps do not, but a swap with
  a Bitcoin leg is priced from the cached BTC rate (counter-leg pricing,
  below). A cross-asset pair is sized by its own
  conversion relation, so the raw units of the two assets are never compared.
  Its stored `swap_fee_msat` is still the difference of those raw units and is
  meaningless; views must not show it as a fee when the assets differ.
- **Fees in a third asset** (BNB, KCS, BEST) become their own outbound row of
  that asset (kind `fee`), linked by the trade's reference. The row is a
  fee-only spend (amount 0, the whole quantity as its fee,
  `assets::fee_only`), so the tax engine books it under the country's fee
  rule; a principal would make it a sale at the fee's value. In `bitcoin`
  scope a BTC trade with a third-asset fee stays refused, as today.

## Per-asset scale (design, not implemented)

Status: stopped at design on 2026-11-14 for the owner's decision. The
correct change touches far more than the ~300 source lines the round allowed
and changes what an older build may read, so nothing is implemented.

**Problem.** A row of any asset stores at most about 92 million whole units
(i64 at 10^-11). Large-supply tokens overflow at small values (about EUR
1,000 of PEPE, EUR 100 of BTTC): `exact_units` refuses the row by name, and
one such row stops the whole sync or import. The `bitcoin` scope is
unaffected.

**Blast radius.** No conversion is shared; each crate divides by its own
10^11 (`Decimal::from(100_000_000_000)`, `format!("{n}e-11")`,
`from_parts(.., -11)`, `exponent + 11`). A read-only survey found about
90-110 such sites in about 45 Rust files and 10 desktop files; about 40-50 of
them are reached for other assets:

- import: `assets::exact_units` (the one encoder), `file_import.rs`
  (`btc_to_msat` before price validation for every asset),
  `document.rs`, `pydecimal/src/price.rs`;
- custody: `interpretation/fanout.rs`, `interpretation/pairs.rs`,
  `finalized_projection/valuation.rs`; `evidence.rs` hashes raw
  `amount_msat`, so a changed scale changes quantity hashes;
- journals: `pipeline.rs` (per-asset rate), `prebuild/pricing.rs`,
  `quarantine.rs`, `views.rs`, `evidence_summary.rs`, `journal_events.rs`,
  `transaction_graph.rs`, and the write-back of the engine's quantity in
  `render.rs`/`persistence.rs`;
- tax engine: the request carries `amount_msat: u64`; `ledger/normalize.rs`
  turns it into whole units (`MSAT_PER_BTC`), used again in `carry.rs` and
  `swaplinks.rs`;
- reports: `lib.rs` (capital-gains quantity), `austrian.rs` (E 1kv),
  `tax_summary.rs`, `summary_pdf.rs`, `full_csv.rs`, `report_design`,
  `history.rs`, `cross_check*.rs`, `verification.rs`, `year_handoff.rs`
  (`"_decimals": 11`), the source-of-funds readers;
- views: `workspace/src/overview.rs` (other assets), `transactions`
  (`listing.rs`, `metadata*.rs`, `commercial.rs`, `desktop/render.rs`),
  transfers (`review_context.rs`, `records.rs`, `component_review.rs`,
  `component_rows.rs`, `payout_create.rs`, `pairing/service.rs`), AI
  `undo.rs`;
- desktop: `ReviewTransfers.tsx`, `custodyGapsModel.ts`,
  `swapPairHistoryModel.ts`, `source-funds/model.ts`,
  `guidedComponentModel.ts` (and three files that already read every asset
  as sats).

Every integer column that holds a quantity is in the same unit for all
assets (`transactions.amount/fee`, `journal_entries.quantity`, the holdings
tables, `journal_quantity_*.amount_msat`, the custody relation, decision,
leg, allocation and term columns, `swap_fee_msat`). Sums can already pass
i64 for in-range rows (`SUM(amount_msat)` in the overview raises an
SQLite overflow; `persistence/quantity.rs` writes an i128 sum back through
`i64::try_from`).

**Options.**

1. *Per-asset exponent, stored per book* (recommended). A shared helper
   `kassiber_store::assets::unit_exponent(conn, asset)` returns 11 for every
   asset unless the book's new `asset_units(asset, exponent, created_at)`
   table names it. The first import that stores a listed large-supply token
   (a static table in `assets.rs`: SHIB, PEPE, BONK, BTTC, FLOKI, ...) writes
   its exponent there, and it never changes for that book. BTC, LBTC and
   every asset already in a book keep 10^-11, so no stored row changes and no
   semantics bump is needed for existing books. Every site above reads the
   helper (or a per-row exponent carried with the row), and the tax-engine
   request carries the exponent per asset. An older Rust build refuses the
   book because it does not know the table; the released Python build
   ignores unknown tables and would read such rows 10^9 too large, which
   needs an owner decision (the Bisq holds table took the same exception).
   Estimate: 600-900 source lines plus tests, mostly mechanical.
2. *Static per-asset exponent in code, nothing stored.* Smaller (no table),
   but an older build cannot tell it is misreading a book, and the table can
   never change an asset once shipped. Not recommended.
3. *One coarser unit for every other asset* (for example 10^-8). Changes
   every stored non-BTC row (a migration and semantics bump), refuses finer
   exchange amounts, and still caps large-supply tokens. Not recommended.
4. *Keep refusing* (today). Correct and fail-closed, but a single PEPE or
   BTTC row stops a whole exchange sync. A cheap middle step, independent of
   1-3: skip and count such a row in the import tally with its name (as the
   `bitcoin` scope counts other assets) instead of stopping the sync.

**Recommendation.** Option 1, as its own change with owner sign-off for the
released-Python exception, after the views read assets generically (plan step
4). Until then, consider option 4's skip-and-count so one oversized row no
longer stops a sync.

## Contract for importers

Shared code: `crates/wallets/src/file_import/assets.rs` (`class`, `Class`,
`Scope`, `Tally`, `FIAT`, `exact_units`), `swaps.rs` (pairing, run by the
coordinator for every file import and exchange sync), and in
`exchange_api.rs` the `Entry`/`record` builder, `swap_leg`, the mapper's
`Fetched { records, skipped, tally, reported }` and `Client::scope`. An
importer:

1. Maps each provider ticker through its own alias table to a canonical
   code (`XXBT` -> `BTC`, `XETH` -> `ETH`, `ZEUR` -> `EUR`), then classifies it
   with `assets::class`: `Bitcoin`, `Fiat`, `Crypto`, or `Excluded` (from the
   provider's own asset type where it has one: Bitpanda `type`, Coinbase
   account type; never guessed from the ticker alone).
2. Asks the wallet's `AssetScope` whether to emit a `Crypto` row, and records
   every row it leaves out in the shared `Skipped` tally (`skipped` /
   `excluded` by ticker).
3. Emits rows through the shared builders: buys and sells against fiat with
   exact pricing; crypto-to-crypto swaps as two `swap` legs with a shared
   `swap_ref` (`swap_leg`); third-asset fees as `fee` rows; deposits,
   withdrawals and rewards as today. An inbound row of another asset whose
   fee is in that same asset stores the net quantity and no fee
   (`assets::net_inbound`, for the shared `record` builder and the exports),
   because custody counts only an inbound row's amount; its value covers the
   net quantity. Bitcoin rows keep their fee. A mapper returns its tally in
   `Fetched.tally` (`Some`), which also turns on the `assets` and
   `swap_pairs` envelope fields for that exchange.
4. Refuses (fails closed) anything it cannot map exactly, naming the row.

The coordinator stores the rows (other assets exactly, or refused), authors
the swap pairs, and adds `swap_pairs` and `assets: {scope, skipped: {TICKER:
rows}, excluded: {TICKER: rows}, next_step?}` to the result envelope.
File exports without a provider asset type take a row's ticker
through `assets::file_row`. A format with no tally of its own (generic
CSV/JSON, BTCPay) is screened by the coordinator instead: Bitcoin and Liquid
asset ids always, another crypto ticker only in the `crypto` scope, never a
fiat code, each refusal naming the row.

## Sequence

1. Foundation: `assets.rs` (classes, scope, tally, exact units), the wallet
   `asset_scope` setting, swap pairing in the coordinator. Done.
2. Bitpanda end to end (crypto assets, swaps, rewards), with the desktop
   toggle on the exchange card and in the import result.
3. In parallel on the foundation: Kraken and Binance APIs; Coinbase API;
   CoinTracking and Blockpit exports (one importer, two formats; also asks
   which fiat currency the export's value columns use).
4. Views: per-asset overview, transactions and portfolio (the overview's
   `asset IN ('BTC','LBTC')` filters), asset-aware formatting in the desktop.
   Bitcoin-specific views (chain, UTXO, privacy, the Bitcoin balance) stay
   BTC-only; tax reports (capital gains, E 1kv) cover every asset the book
   holds, as tax law does. An asset the book has no price for has an unknown
   (`null`) market value and unrealized result in every portfolio view (the
   overview, holdings, the summary and the summary PDF, as of today or a past
   date), never zero, and every total it would add to is unknown too. Bitcoin
   without a price keeps its zero valuation.
5. Further exchanges and export formats, prioritised from the Rotki survey
   (Rotki is AGPL; Kassiber reimplements from public API docs and samples,
   never from its code).

## Verification

Each importer: loopback or fixture tests for every mapped row type, the
`bitcoin`-scope skip tally, the `crypto`-scope rows, swap pairing and its
idempotent re-run, a third-asset fee, the exact-unit refusal, and mutation
entries for each rule. End to end: an Austrian book with a BTC -> ETH swap
reports no gain and carries the BTC basis into ETH
(`crates/kassiber/tests/tax_platform_swaps.rs`). The ETH canary
(`crates/kassiber/tests/exchange_swaps.rs`) runs a Kraken API sync against a
loopback fake through the binary: ETH bought for EUR, swapped into BTC, the
BTC sold. It pins the stored rows, the provider pair, the Austrian carry of
the ETH basis into BTC and the realized gain, and that the `bitcoin` scope
refuses the swap's BTC leg instead of booking it as a sale or a buy.
