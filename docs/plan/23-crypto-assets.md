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
  deduplicate by external id.
- **Units.** Every crypto asset uses the same integer unit as BTC: 10^-11 of
  a whole unit (msat for BTC). Exchanges and exports report at most about 10
  decimals, so this is exact for them. For any asset other than BTC/LBTC, an
  amount finer than 11 decimals or larger than the column holds (about
  92 million units per row) is refused, never rounded. A smaller scale for
  large-supply tokens can follow if users need it.
- **Pricing.** Trades carry the exchange's own execution value
  (`exchange_execution`, `exact`) for any asset; no rate provider is needed.
  Rows without an execution value (deposits, rewards, airdrops) stay unpriced
  and land in the existing `missing_price` review until priced; the rates
  crate stays BTC-only for now.
- **Swaps.** A crypto-to-crypto trade is two rows: the outbound leg (kind
  `swap`) and the inbound leg (kind `swap`), each with its fiat value when the
  provider gives one, and the same `swap_ref`. The import authors the pair
  between them in the same write (`transfers.pair`, kind `manual`, pair source
  `provider`), with
  the policy the profile's country recommends: carrying value where the
  country carries basis across assets (Austria: crypto-to-crypto is
  tax-neutral), taxable elsewhere. The tax engine already handles both,
  including Austrian Alt/Neu lots. A cross-asset pair is sized by its own
  conversion relation, so the raw units of the two assets are never compared.
  Its stored `swap_fee_msat` is still the difference of those raw units and is
  meaningless; views must not show it as a fee when the assets differ.
- **Fees in a third asset** (BNB, KCS, BEST) become their own outbound row of
  that asset (kind `fee`), linked by the trade's reference. In `bitcoin` scope
  a BTC trade with a third-asset fee stays refused, as today.

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
   withdrawals and rewards as today. A mapper returns its tally in
   `Fetched.tally` (`Some`), which also turns on the `assets` and
   `swap_pairs` envelope fields for that exchange.
4. Refuses (fails closed) anything it cannot map exactly, naming the row.

The coordinator stores the rows (other assets exactly, or refused), authors
the swap pairs, and adds `swap_pairs` and `assets: {scope, skipped: {TICKER:
rows}, excluded: {TICKER: rows}, next_step?}` to the result envelope.

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
   Bitcoin reports stay BTC-only unless the user asks for other assets.
5. Further exchanges and export formats, prioritised from the Rotki survey
   (Rotki is AGPL; Kassiber reimplements from public API docs and samples,
   never from its code).

## Verification

Each importer: loopback or fixture tests for every mapped row type, the
`bitcoin`-scope skip tally, the `crypto`-scope rows, swap pairing and its
idempotent re-run, a third-asset fee, the exact-unit refusal, and mutation
entries for each rule. End to end: an Austrian book with a BTC -> ETH swap
reports no gain and carries the BTC basis into ETH.
