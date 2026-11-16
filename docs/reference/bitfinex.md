# Bitfinex

Rust only. The signed REST API v2 (`wallets sync-bitfinex`, the user's own
read-only key) and the Bitfinex Reports CSV exports (`wallets
import-bitfinex`) share one mapping and one row reader
(`file_import/exchange_api/bitfinex/mapping.rs`; API
`file_import/exchange_api/bitfinex.rs`, CSV `file_import/bitfinex.rs`) and
the same ids (`bitfinex:trade:<ID>`, `bitfinex:movement:<ID>`), so both in
one wallet keep one record per event. Transport rules are the
[framework's](rust-exchange-sync.md). Sources (2026-11-12):
[authenticated requests](https://docs.bitfinex.com/docs/rest-auth),
[key permissions](https://docs.bitfinex.com/reference/key-permissions),
[ledgers](https://docs.bitfinex.com/reference/rest-auth-ledgers),
[trades](https://docs.bitfinex.com/reference/rest-auth-trades),
[movements](https://docs.bitfinex.com/reference/rest-auth-movements),
[wallets](https://docs.bitfinex.com/reference/rest-auth-wallets); the CSV
columns from `bfx-report` (<https://github.com/bitfinexcom/bfx-report>).

## Key setup

In Bitfinex: Account → API Keys (<https://setting.bitfinex.com/api>) →
Create New Key. Turn on only read permissions, at least **Account History**
and **Wallets** (get); leave every write permission off (create and cancel
orders, claim a position, offer funding, transfer between wallets, create a
withdrawal, write settings). Confirm the e-mail link and copy the key and
secret, which Bitfinex shows once. Give Kassiber its own key (see nonces).

```bash
kassiber backends create bitfinex-main --kind bitfinex \
  --url https://api.bitfinex.com --token-stdin --auth-header-stdin
kassiber wallets sync-bitfinex --backend bitfinex-main
```

## API sync

Signed POSTs with a JSON body (`bfx-apikey`, `bfx-nonce`, `bfx-signature` =
hex HMAC-SHA384 of `/api/` + path + nonce + body), re-signed on every retry:

1. `/v2/auth/r/permissions`: a key with any write flag refuses the sync
   (`auth_error`, `details.write_permissions`) before any history is read.
2. `/v2/auth/r/ledgers/BTC/hist`, `/v2/auth/r/trades/hist` (`"sort":-1`) and
   `/v2/auth/r/movements/BTC/hist`, paged by `end` = the oldest millisecond
   of the last page (inclusive; limits 2,500 and 1,000). A repeated entry
   must be identical; a full page within one millisecond fails
   `protocol_error`.
3. `/v2/auth/r/wallets`: `reported_balance`, scope
   `exchange_and_funding_wallets` (the BTC `BALANCE` of both, orders'
   reserved and lent-out BTC included; the margin wallet is not counted).

JSON numbers are kept as written and must be whole msat, never read as
floats. Error arrays (`["error", code, message]`) keep only the status and
a documented code (10000, 10001, 10020, 10100, 10111–10114, 11000) or
`"unknown"`, never Bitfinex's text: 10100 and 10111–10113 are `auth_error`,
10114 is a nonce refusal. `ERR_RATE_LIMIT` is `rate_limited` (retryable);
Bitfinex then blocks for a minute. **Nonces**: one signed request of this
process runs at a time, each nonce above the last (microseconds at least).
Another program, or another Kassiber process, using the same key at once
can send a larger nonce; the sync then stops on error 10114 with nothing
imported and can simply run again. The desktop key check reads the
permissions, one ledger entry and the wallets and reports `read_only: true`.

## Mapping

Trades, completed movements and funding interest make the records;
non-BTC trades and movements, canceled or unfinished movements and paired
wallet transfers are skipped (the API reports `skipped.non_btc_entries`,
`canceled_movements`, `unfinished_movements`, `internal_transfers` per leg).

Kassiber reads only BTC from Bitfinex, whatever the wallet's
[`asset_scope`](../plan/24-crypto-assets.md): the cross-checks below pair
BTC ledger entries with trades and movements, and the sync reads only the
BTC ledger and BTC movements. In `bitcoin` scope both sources name the
other crypto assets they left out, entries per ticker, in the envelope's
`assets.unread` and `assets.next_step` (not as `skipped`, which would offer a scope that does
not keep them). A wallet whose scope is `crypto` is refused: the sync before
any request, since it cannot see another asset's deposits; the Reports
import at the first trade, movement or ledger entry in another crypto
asset, by name. Set the scope to `bitcoin` and record the other assets
yourself (generic CSV/JSON import).

| Event | Record |
| --- | --- |
| Trade of BTC against USD, EUR, GBP or JPY (API order type `EXCHANGE …`) | `buy`/`sell`, `pricing_method` `bitfinex_trade`, exact value amount × price without rounding: a buy's fiat fee raises the cost, a BTC fee leaves fewer coins for the same cost; a sale's fiat fee lowers the proceeds, a BTC fee leaves besides the coins sold |
| Completed BTC deposit | `deposit`, unpriced; a 64-hex `TRANSACTION_ID` lowercased into `raw_json.txid` for transfer pairing |
| Completed BTC withdrawal | `withdrawal`: `AMOUNT` sent, `FEES` apart (see below when the ledger does not reach its date) |
| `Margin Funding Payment`, a credit in the funding wallet | `lending_interest`, id `bitfinex:ledger:<ID>`, unpriced (`journals process` holds it until priced) |

Every other BTC ledger entry must be in the exchange wallet and one of these
descriptions (lowercased, ending `on wallet <its wallet>`), paired
**one-to-one**:

| Description | Pairs with |
| --- | --- |
| `Exchange <amount> BTC for <FIAT> @ <price>` (another base in the BTC ledger refuses) | the one trade with that quote, price and signed amount within 60 s |
| `Trading fees for <amount> BTC (BTC<FIAT>) @ <price> …`, a debit | the one such trade whose BTC fee equals the debit |
| `Deposit [(<NAME>)] #<id>`, `<Name> Withdrawal #<id>`, `Cancel… #<id>` | movement `<id>`: its entries add up to the amount when completed, to zero when canceled |
| `<Name> Withdrawal [express] fee`, a debit | the one completed withdrawal with that fee between a day before it started and a day after it completed |
| `Transfer of <amount> BTC from wallet <A> to <B>`, A and B the exchange and funding wallets (either side) | the one other leg: the same amount the other way within 60 s; both legs are skipped, the account's BTC does not change |

A trade booked in the ledger with a BTC fee needs its fee entry, and a
completed withdrawal with a fee needs its own entries (adding up to
`AMOUNT`) and its fee entry: that is what proves `AMOUNT` excludes `FEES`.
Bitfinex books a withdrawal's entries when it is requested, so a withdrawal
the ledger shows always needs its fee entry, whatever the dates, and so does
one requested at or after the ledger read's first entry (the API's read
reaches the present; a Reports ledgers export ends at its last row, and a
withdrawal requested after it is refused, asking for a ledgers export that
covers it). Only a fee-bearing withdrawal without any ledger entry,
requested (`MTS_STARTED`) before the first entry read, or any withdrawal
when the import has no ledgers export, keeps the fee its movement states:
the record is the same, described "Bitfinex withdrawal (fee from movements, not
cross-checked against the ledger)" and tagged `bitfinex-fee-unchecked` for
review. A later import that the ledger covers does not remove the tag.

## Refusals

One refused row stops the whole import, from either source, naming it
(`Bitfinex transaction '<trade|movement|ledger entry> <id>' (type '<pair,
kind or description>') …`); record it through the generic ledger.

- BTC traded against anything but USD, EUR, GBP or JPY (altcoins,
  stablecoins such as UST), named with the coin, from a trade or from a
  ledger entry in any currency: a ledgers-only export never turns the BTC
  leg of BTC → ETH into a deposit or withdrawal.
- Margin (a non-`EXCHANGE` order type; any entry in the `margin` wallet,
  transfers to or from it), anything in the `funding` wallet but interest
  payments and transfers (funding charges, offers), a transfer without its
  other leg, and derivatives (`F0` pairs).
- Any BTC ledger description not listed above, an entry that fits no or
  several trades or withdrawals, or one that books what another entry
  already books (equal partial fills within 60 s, equal withdrawal fees
  close together); a movement whose entries do not add up.
- A withdrawal with a fee the ledger does not prove, unless it has no
  ledger entry and was requested before the ledger's first entry;
  a withdrawal requested after a ledgers export's last row;
  a CSV trade without its exchange-wallet entry, or ledger entries whose
  export is missing.
- A pair Kassiber cannot read (the whole import, naming the pair), a fee in
  a third currency or a rebate, a deposit fee, a rail other than `BITCOIN`,
  amounts finer than a millisatoshi, an unreadable date.
