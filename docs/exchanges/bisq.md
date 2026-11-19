# Exchange spec: Bisq

- **Slug:** `bisq` (`bisq_csv`, wallet kind `bisq`, `wallets import-bisq`,
  pricing provider `Bisq`). Logo: the Bisq press-kit mark
  ([`bisq_mark.svg`](https://github.com/bisq-network/bisq-website/blob/fa1b9c86d28ea00a813b171ab9398fe4c8517ccc/resources/bisq-logo/bisq_mark.svg)),
  unmodified, identifying the import source only.
- **Status:** implemented from Bisq's public source (`bisq-network/bisq` at
  `7d6f5b089f2ef0e53d182805c74c3899f43f44ea`); no real export seen yet.
  Synthetic samples: [`samples/bisq/`](samples/bisq/).
- **Inputs:** Portfolio > History (`tradeHistory.csv`,
  `ClosedTradesView.java`) and Funds > Transactions (`transactions.csv`,
  `TransactionsListItem.java`), English or German, plus the exporting
  computer's IANA zone (`--timezone`): Bisq writes local time without an offset
  (`DisplayUtils.formatDateTime`). There is no UTC default. Without system
  tzdata (Windows) only the tax countries' zones are known, each with its own
  summer-time rule (the EU's, or a southern one such as Sydney's);
  `PYTHONTZPATH` points at a zoneinfo directory for others.
- **Watch-only wallet first:** add the xpubs from Account > Wallet info
  (`BTC segwit wallet`, and `BTC legacy wallet` for old wallets) and sync.

## Model (owner decision 2026-11-12)

Bisq is non-custodial: each trade locks deposits in a 2-of-2 escrow, so no
owned on-chain row equals the traded BTC. Each completed BTC/fiat trade becomes
a full-mode escrow ledger in a `bisq` wallet (another wallet kind is refused),
ids `bisq:<trade id>:deposit|trade|payout`, netting to exactly zero:

| Side | `deposit` (inbound) | `trade` | `payout` (outbound) |
|---|---|---|---|
| Buyer | buyer deposit | `buy`, inbound, amount | amount + buyer deposit |
| Seller | amount + seller deposit | `sell`, outbound, amount | seller deposit |

Only the trade leg is priced: the export's `Amount` (fiat volume) and `Price`,
`exchange_execution`, quality `exact`, contract time as `pricing_timestamp`.
A zero deposit writes no leg. No fee leg: Bisq's BTC trade fee is an on-chain
payment the watch-only wallet observes; trade and mining fees stay in
`raw_json` (`bisq_fees`).

**Dates.** Deposit leg: the later of Bisq's `Multisig deposit` date and the
wallet's own observation of that txid. Trade leg: one second later. Payout leg:
the earlier of Bisq's `Multisig payout` date and the wallet's receipt. The
contract time, zone and both txids stay in `raw_json`.

**Pairing.** When exactly one owned row has the txid Bisq names, the import
pairs it with the escrow leg (`kassiber_transfers::create_evidenced`,
`provider_txid`, authored source `import`). The observed deposit debit must
equal Bisq's net amount; the difference to the escrow share is Bisq's stated
miner fee, booked as a reviewed network fee, never suspense. Edges the owner
unpaired or dismissed, and legs another review covers, are skipped
(`bisq_pairs_skipped`); the rest wait for `transfers suggest`. A pair with a
stated fee cannot be revised or grouped; unpair it instead.

## Holds and refusals

A trade the exports do not settle writes nothing and is **held** (listed in
`bisq_review_required`, persisted in the book's opt-in `provider_trade_holds`
table with the txids its exports name). Journals quarantine every live
transaction a hold names (`bisq_trade_review_required`) until a custody review
covers it or it is excluded, so a deposit never books as a disposal. Txids only
accumulate; a hold clears when a later export completes the trade with escrow
transactions covering all of them. Reasons: `not_completed` (mediated,
arbitrated, `N/A` or unknown status), `missing_deposit_tx`,
`missing_payout_tx`, `conflicting_escrow_txs` (also Funds rows that name one
txid with different amounts or dates; identical rows count once),
`deposit_amount_mismatch`,
`payout_amount_mismatch`, `deposit_not_in_book` (sync, then import again),
`deposit_debit_mismatch` and `payout_receipt_mismatch` (also when several
owned rows observe one escrow transaction), `payout_precedes_deposit`,
`tax_year_boundary` (contract and trade leg in different UTC or book tax
years). Canceled offers are skipped (`bisq_canceled_skipped`).

The whole import is **refused, naming the trade**, for: altcoin and BSQ
markets (`<coin>/BTC`: altcoin-market trades are not supported yet); a quote
that is not a fiat currency Kassiber knows; an unknown offer type; inexact or
missing amounts, deposits, price, volume or date; a date the zone skipped or
repeated; two trade ids sharing a short id or one id with different rows; an
escrow transaction another trade of the export or of the book already names;
and any difference from legs an earlier import stored (amounts, prices,
wallet, escrow txids, moved escrow dates or another `--timezone`), including a
stored trade this
export would hold. Roll back the earlier import (`imports rollback`) to
re-import such a trade. Nothing is written for a refused file.

Older Rust builds refuse a book with the holds table; the released Python app
ignores it ([opt-in schema groups](../reference/database-compatibility.md#opt-in-schema-groups)).
A book with holds cannot be split by network partition yet.

## Follow-ups

Real export check; Bisq Easy (Bisq 2) as order evidence; reviewed flows for
mediated/arbitrated payouts (`Dispute payout`, `Refund from arbitration`),
redating moved legs, and partitioning held trades; altcoin-market swap legs
once crypto scope lands. See [TODO.md](../../TODO.md).
