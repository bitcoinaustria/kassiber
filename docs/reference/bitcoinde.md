# Bitcoin.de

Rust only. The Trading API v4 (`wallets sync-bitcoinde`, a read-only key the
user creates in their own account) and the BTC account statement CSV
(`wallets import-bitcoinde`) share one mapping
(`file_import/exchange_api/bitcoinde.rs`, CSV reader `file_import/bitcoinde.rs`),
so a booking both return is stored once. Transport rules are the
[framework's](rust-exchange-sync.md). Sources (2026-11-13): the
[v4 documentation as archived 2020-12-02](https://web.archive.org/web/20201202191159/https://www.bitcoin.de/de/api/tapi/v4/docu)
(the live page answers 404 since Bitcoin.de largely suspended trading in June
2026; `api.bitcoin.de/v4` still answers), the
[fee FAQ](https://www.bitcoin.de/de/faq/wie-hoch-sind-die-gebuehren-bei-bitcoin-de/17.html),
the [statement FAQ](https://www.bitcoin.de/de/faq/wo-kann-ich-meine-letzten-transaktionen-einsehen/183.html)
and rotki's v4 client. The CSV layouts are pinned by public sample exports.

## Key setup

mein Bitcoin.de → Trading-API (<https://www.bitcoin.de/de/userprofile/tapi>)
→ new key. Tick only **showPermissions**, **showAccountLedger** and
**showMyTrades**; leave every order, trade, withdrawal and deposit-address
method off; optionally restrict the key to this computer's IP address. Copy
the key and the secret (shown once). Kassiber refuses a key granted anything
but reads.

```bash
kassiber backends create bitcoinde-main --kind bitcoinde \
  --url https://api.bitcoin.de --token-stdin --auth-header-stdin
kassiber wallets sync-bitcoinde --backend bitcoinde-main
kassiber wallets import-bitcoinde --file kontoauszug-btc.csv
```

## API sync

Signed GETs only: `X-API-KEY`, `X-API-NONCE` (milliseconds, raised above the
last nonce this process sent; a lock keeps one process's requests in nonce
order, taken only after any wait for API credits) and `X-API-SIGNATURE` =
hex HMAC-SHA256(secret,
`GET#<full URL>#<key>#<nonce>#d41d8cd98f00b204e9800998ecf8427e`), re-signed
with a new nonce on every retry.

1. `/v4/permissions`: every granted method must be a documented read
   (`listAddressPool`, `showAccountInfo`, `showAccountLedger`, `showDeposit(s)`,
   `showMyOrder(Details|s)`, `showMyTrade(Details|s)`, `showOrderDetails`,
   `showOrderbook(Compact)`, `showPermissions`, `showPublicTradeHistory`,
   `showRates`, `showWithdrawal(s)`, `showWithdrawalMinNetworkFee`), else
   `auth_error` with `details.permissions`; a missing required read gives
   `details.missing_permissions`. Nothing else is read.
2. `/v4/btc/account/ledger?datetime_start=2011-01-01T00:00:00+01:00&page=N`
   and 3. `/v4/btceur/trades?state=1&page=N`, each until `page.current`
   reaches `page.last`. A page that is not the one asked for fails closed.

Credits: the last answer's `credits` plus one per idle second must cover the
next read (2 for permissions, 3 for a page); Kassiber waits up to 120 s, a
longer deficit is a retryable `rate_limited`. A 429 is retried by the
transport. Amounts are plain decimal strings or JSON numbers read by their
written digits; exponents, decimal commas and floats are refused. 400, 401,
403, 404, 422 and a 2xx with `errors` fail with `details.http_status` and
`bitcoinde_error_code`, a documented number (1–5, 9, 10, 12–14, 31–33, 46,
94) or `"unknown"`, never Bitcoin.de's text: 4 is a refused nonce (another
program or sync used the key at the same moment or with larger nonces, or
this computer's clock is behind the one that last used the key; check the
system clock and sync again), 14 a missing
method, 12/13 a retryable `upstream_error`, the key codes and plain 401/403
`auth_error`, anything else `protocol_error`. The envelope reports
`skipped.registration_rows`, `skipped.trades_not_yet_in_statement` (the
statement lists bookings up to the previous day) and `reported_balance`
(scope `account_statement_balance_after_last_booking`). The sync stores that figure as an
`api` [balance checkpoint](balance-checkpoints.md#exchange-syncs). The desktop key check
reads the permissions and one statement page and reports `read_only: true`.

## CSV export

mein Bitcoin.de → Kontoauszug → BTC, type all, whole period → "Kontoauszug als
CSV herunterladen". `;`-separated UTF-8, dot decimals, `Datum` as
`YYYY-MM-DD HH:MM:SS` German wall time (a time the summer-time change skips
or repeats is refused). German and English headings are read; required:
date, type, currency (`BTC`), reference, BTC before fee, BTC after the
Bitcoin.de fee, amount and unit after the fee, incoming / outgoing, balance.
`EUR nach Bitcoin.de- und Fidor-Gebühr` (older express trades), when filled,
is the cash that changed hands. A trade's `Einheit (Kurs)` must be
`BTC / <its cash unit>`. An unknown or repeated heading, a row with extra
fields are refused by name. Another coin's statement (`ETH-Adresse`, `ETH
vor Gebühr`) or a row in another coin is skipped and named (see below). An
active custodial ledger, imported in `full` mode.

## Mapping

| Type (API / German / English) | Record |
| --- | --- |
| `buy` / Kauf / Purchase | `buy` of the BTC after the whole fee, exact `exchange_execution` pricing at the euros paid (agreed amount minus half the fee) |
| `sell` / Verkauf / Sale | `sell` of the agreed BTC at the euros received |
| `inpayment` / Einzahlung / Deposit | `deposit`, unpriced |
| `payout` / Auszahlung / Disbursement | `withdrawal`; the network fee of its transaction next to it in the balance order becomes its fee |
| `affiliate`, `kickback`, `welcome_btc` (and German/English names) | `income`, unpriced |
| `registration` with zero BTC | skipped (the API sync counts them in `skipped.registration_rows`; the CSV import reports no count) |

Trades are identified `bitcoinde:<trade id>`; the API joins each with
`showMyTrades` and stores the trade without the partner's name, bank or
rating. Every other booking has no id of its own: it is
`bitcoinde:<UTC second>:<msat before>:<msat after>` of the running balance,
and a 64-hex reference travels as `raw_json.txid` for transfer pairing.
The balance must chain from zero: bookings of one second are ordered by
their balances, and records keep that order as their source position. Each
import is bound to what earlier Bitcoin.de runs stored in the wallet: those
rows must come back unchanged and new rows must follow the newest stored
second, so a corrected statement never adds rows beside stored ones. A
priced booking must keep its stored fiat value (the CSV and the API state a
trade's cash in different fields): a different value refuses the import,
naming the booking. Older express trades settled through Fidor price at
the cash after the bank's fee in the CSV (`EUR nach Bitcoin.de- und
Fidor-Gebühr`) but not in the API, so a book with such trades must use one
source per wallet: the CSV or the API sync, not both.

Kassiber reads only BTC from Bitcoin.de, whatever the wallet's
[`asset_scope`](../plan/24-crypto-assets.md): the mapping chains the BTC
balance and the sync reads only BTC endpoints. Both scopes book the same BTC
rows ([exchange contract](exchange-contract.md)). The CSV skips another
coin's statement or row and names it, rows per coin, in the envelope's
`assets.unread` and `assets.next_step` (not as `skipped`, which would offer a
scope that does not keep it); a `crypto`-scope sync says in
`assets.next_step` that other coins are not read. Record the other coins
yourself (generic CSV/JSON import).

## Refusals

One refused booking stops the whole import, naming its position, type and
reference; record it through the generic ledger with its fiat value.
Refused: any type the table does not list (shop purchases, transfers,
corrections, unknown types) and a registration that moves BTC; a trade
missing from the BTC/EUR trade list (named with its pair, e.g. `dashbtc`), a
trade list entry on another pair, external-wallet or unsuccessful; a trade
whose type, BTC amount or cash disagrees with its details; a booking with
the wrong sign, no BTC or no reference; a network fee without its payout; a
BTC amount or balance finer than a millisatoshi; a statement that does not
start from zero, has a gap, or a second whose balances allow more than one
order (or holds over 20,000 bookings); any difference from earlier runs,
a changed fiat value included (roll those back with `imports rollback`,
then import the full statement).
