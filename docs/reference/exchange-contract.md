# Exchange contract

Every exchange importer works the same way; only the provider's API and
export formats differ. An exchange module in
`crates/wallets/src/file_import/exchange_api/` is:

1. a **transport** for its API (signed reads with a read-only key the user
   creates in their own account) and/or a **parser** for its file exports,
   each reading the provider's rows into the shared `Event`;
2. the **table** of what each of its row types means;
3. one `Descriptor` in the registry (`exchange_api/contract/registry.rs`).

Everything else is shared and lives in `exchange_api/contract.rs`, written
once for every exchange.

## Events and the registry

An `Event` is one provider transaction: its record identity, time, kind
(`Movement(kind)` or `Trade`), what left the account (`sent`), what reached it
(`received`), its fee, the exact execution value the provider states for a
trade, an optional description and the provider's evidence (`raw_json`).

The `Descriptor` names the backend and wallet kind, the provider, which assets
the importer reads (`AllAssets` or `BitcoinOnly`), the API's default URL and
credentials (key alone or key and secret), the API's `fetch` and desktop
`probe`, and the envelope's `skipped` keys where the exchange reports them.
The CLI sync, the desktop key check and the provider wallet's label read the
registry; no caller matches on an exchange's name.

## The asset scope is a filter

`Mapper::admit` decides, from an event's assets alone and before its type is
read, whether the event is mapped:

- an event that moves Bitcoin (`BTC`, `LBTC`) is always mapped, the same way
  in every scope;
- an event that moves only fiat is counted as fiat-only and left out;
- an event without Bitcoin from a `BitcoinOnly` importer is counted per asset
  in `assets.unread` (with a `next_step` naming the provider) and left out;
- from an `AllAssets` importer it is kept in a `crypto` wallet, and counted in
  `assets.skipped` and left out in a `bitcoin` wallet.

So a row type Kassiber does not map in another coin never stops a Bitcoin
wallet's import, and turning the scope on never changes a Bitcoin row.

`Mapper::admit_classed` takes the provider's own class for each asset
(Bitpanda's stocks and metals are `Excluded`): an excluded asset is counted in
`assets.excluded` and never kept; traded against a kept crypto asset, the
exchange refuses the event, because that crypto leg is a disposal.

`Mapper::push` turns each admitted event into its record through the shared
record builder: a `Movement` as its kind, a `Trade` as a buy or sell, a
`Swap(reference)` leg as an unpriced `swap` row carrying `swap_ref` (the import
pairs both legs), and a `Fee` in a third asset as a fee-only `fee` row. A
sync's envelope always carries `assets` and `swap_pairs`, and `skipped` where
the exchange reports it.

## Balances

An exchange that states its own BTC balance returns it from `fetch` as a
`Balance` (whole msat or why it is unknown, `as_of`, and which balance it is).
The envelope then carries `provider_balance` (the figure against the wallet's
book balance after the import: `matches`, `mismatch` or `unavailable`) and,
when the figure is known, `reported_balance` (`{asset, amount_msat, as_of,
scope}`). A mismatch is reported, never corrected.

## Stored identities

Each exchange builds its events' identities itself, exactly as books already
store them (`kraken:<ledger id>`, Coinbase's transaction id or chain hash,
Binance's BTC transfer txId, `bitpanda:<transaction id>`); the contract never
rewrites one. Kraken, Coinbase and Binance keep the Python reference's reading
of their BTC rows, pinned by `crates/wallets/tests/vectors/python_exchange_api.json`.
