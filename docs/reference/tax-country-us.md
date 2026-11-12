# United States (`us`): tax country specification

**Status:** implemented in the Rust backend as a **preview** country
(2026-11-07). Not reviewed by a US tax professional (CPA or enrolled agent).
Every US report and screen carries the preview notice until one does.
Nothing here is tax advice.

Scope: an individual US taxpayer holding bitcoin (and Liquid bitcoin) as a
**capital asset** for investment. Dealers, traders with a mark-to-market
election, businesses, and mining or staking as a trade or business
(Schedule C, self-employment tax) are out of scope: Kassiber reports the
income figures and flags the classification for review.

The worked cases are in [tax-country-us-vectors.json](tax-country-us-vectors.json)
(schema `kassiber.tax-country-vectors/1`, shared with the other countries);
their expected values came from an independent Python reference model
(`us.py`), which also checked the hand-worked figures each vector carries.
That model was removed with Python ([plan 22](../plan/22-python-removal.md),
D6; it remains at the `archive/python-backend` tag), so the file is now a
frozen, reviewed golden: change an expected value only by hand, with the
reason.

## Sources

All retrieved 2026-11-07.

| Ref | Source |
| --- | --- |
| Pub 544 | [IRS Publication 544, Sales and Other Dispositions of Assets](https://www.irs.gov/publications/p544): holding period ("start counting on the day following the day you acquired the property … The day you disposed of the property is part of your holding period"; bought 2024-06-17, sold 2025-06-17 is not longer than 1 year), tacked holding periods, Form 8949 Part I/II, Schedule D |
| Pub 550 | [IRS Publication 550, Investment Income and Expenses](https://www.irs.gov/publications/p550): holding period example (bought 2024-01-31; sold 2025-01-29 short-term, 2025-02-06 long-term), trade-date rule, digital assets are property, gifts |
| VC FAQ | [IRS FAQs on virtual currency transactions](https://www.irs.gov/individuals/international-taxpayers/frequently-asked-questions-on-virtual-currency-transactions): FAQ 6 and 29 (holding period begins the day after acquisition or receipt), 8 (basis includes fees and commissions), 9, 12, 13 (income at fair market value, which is the basis), 22–25 (hard forks and airdrops), 26–28 (fair market value), 38 (transfers between own wallets are not taxable), 39–41 (specific identification, FIFO default) |
| Rev. Proc. 2024-28 | [Rev. Proc. 2024-28](https://www.irs.gov/pub/irs-drop/rp-24-28.pdf): safe harbor for allocating unused basis to wallets or accounts as of 1 January 2025; background on § 1.1012-1(j) (specific identification or FIFO per wallet or account for acquisitions and dispositions on or after 1 January 2025; FIFO "in order of time from the earliest purchase date of the units"; transfers are identified like sales), specific-unit and global allocations, standing orders |
| Rev. Rul. 2023-14 | [Rev. Rul. 2023-14](https://www.irs.gov/pub/irs-drop/rr-23-14.pdf): staking rewards are gross income at fair market value when the taxpayer gains dominion and control |
| Rev. Rul. 2019-24 | Cited by VC FAQ 25: an airdrop after a hard fork is income when received; its basis is the amount included in income |
| i8949 | [Instructions for Form 8949 (2025)](https://www.irs.gov/instructions/i8949): boxes G/H/I and J/K/L for digital assets with or without Form 1099-DA; "Digital asset transactions should not be reported using box C or F"; columns (a)–(h); "VARIOUS" |
| i1040sd | [Instructions for Schedule D (2025)](https://www.irs.gov/instructions/i1040sd): lines 1b, 2, 3, 8b, 9, 10 take Form 8949 totals; codes G–L added to those lines |

The Notice 2014-21 and Rev. Rul. 2019-24 texts were not opened directly
(the FAQ quotes and cites them); a reviewer should confirm them.

## Rules

Every rule has an id; the vectors list the ids they exercise.

### Events

- **US-EVENT-SELL** — Selling bitcoin for dollars is a disposal of property;
  gain is proceeds minus basis.
- **US-EVENT-SPEND** — Paying for goods or services disposes of the bitcoin at
  the value received (VC FAQ 14, 16).
- **US-TRANSFER** — A move between the taxpayer's own wallets is not taxable
  (VC FAQ 38). The moved units keep their basis and acquisition date
  (**US-HOLD-TACK**, Pub 544's tacked holding period for carried-over basis).
- **US-TRANSFER-FEE** — A move's network fee is paid in bitcoin. Its units
  leave the source wallet first, by the book's method, then the moved units.
  Choice: the fee units realize no gain or loss; their basis leaves the book
  with them. This is how Kassiber composes every fee row (a fee-only spend,
  a channel open, a move's fee: proceeds equal to basis), and the US report
  lists these fees with their basis and dollar value instead of on Form
  8949. Whether the fee units are a disposal at their value, or their basis
  belongs to the moved units, is not settled in the sources (open question 2).
- **US-FEE-SALE** — A sale's network fee units are part of the disposal: the
  row's quantity includes them and its proceeds are the sale's dollar amount.
  The final broker regulations split this into a sale (proceeds reduced by
  the fee's value) and a fee disposition (proceeds equal to the fee's value);
  the totals of proceeds, basis and gain are the same.
- **US-INCOME** — Staking (Rev. Rul. 2023-14), mining, interest, income and
  airdrops (Rev. Rul. 2019-24) are ordinary income at their dollar fair
  market value when received (VC FAQ 9, 12, 23); **US-INCOME-BASIS** that value
  is their basis (VC FAQ 13, 25) and their holding period starts the next
  day (FAQ 29). Kassiber's inbound kinds map to these as in every country
  (the engine's kind tables, `tax-engine/core/src/manifest.rs`); `wages` are an acquisition at
  their value, since the employer reports the wage. **US-INCOME-AIRDROP**
  Kassiber books an airdrop as income; a reviewer should confirm that no
  airdrop kind needs a separate review (open question 5).

### Holding period

- **US-HOLD-1Y** — Long-term when held more than one year, short-term
  otherwise (Pub 544, Pub 550).
- **US-HOLD-CAL** — Counted in calendar dates: the period starts the day
  after acquisition and includes the disposal date, so a disposal is
  long-term exactly when its date is after the acquisition's anniversary.
  A disposal on the anniversary is short-term.
- **US-HOLD-LEAP** — A lot acquired on 29 February has 28 February as its
  anniversary in a common year: long-term from 1 March. (Choice: the
  publications have no 29 February example; this follows the last-day-of-
  month reading.)
- **US-CAL-UTC** — Dates are UTC calendar dates. Choice: Kassiber has no
  per-book time zone, and the US spans several; an evening trade in New York
  can fall on the next UTC day (vector `us-06`). A reviewer should decide
  whether a book needs its own zone (open question 1).
- **US-ENGINE-DAYCOUNT** — RP2's US plugin counts whole elapsed days
  (`days >= 365`), which calls a sale on the anniversary, or 365 days across
  29 February, long-term. Kassiber follows the publications instead; the RP2
  comparison checks every other value and that every disagreement is one of
  these boundary days.

### Identification and pools

- **US-METHOD-FIFO** — Without specific identification, units are disposed of
  first in, first out (VC FAQ 41), "in order of time from the earliest
  purchase date" (Rev. Proc. 2024-28 § 2). FIFO is the default.
- **US-METHOD-SPECID** — Specific identification is allowed when the units are
  identified no later than the disposal and the records support it (VC FAQ
  39–40). Kassiber offers LIFO (**US-METHOD-LIFO**), HIFO (**US-METHOD-HIFO**)
  and LOFO (**US-METHOD-LOFO**) as standing orders: a rule recorded in the
  book that identifies the units of every disposal. Ties are broken by
  acquisition time, then the row.
- **US-POOL-WALLET** — From 1 January 2025, § 1.1012-1(j) identifies units per
  wallet or account: a disposal or transfer takes units only from the wallet
  that holds them (`cost_basis_pool_scope = wallet`, the US default). Kassiber
  applies this to the whole history, which amounts to a specific unit
  allocation (Rev. Proc. 2024-28 § 5.02(2)(a)) of each wallet's actual units
  on 1 January 2025 (**US-REVPROC-2024-28**).
- **US-POOL-GLOBAL** — Before 2025 many taxpayers identified units across all
  wallets (universal application). The book may choose
  `cost_basis_pool_scope = global` for that; **US-POOL-2025** such a book
  refuses to process once it has a disposal or transfer fee on or after
  1 January 2025 UTC, because the transition (a reasonable allocation of
  unused basis to wallets as of that day, Rev. Proc. 2024-28 § 5) is not
  modelled (open question 3).
- **US-RP2-PER-WALLET** — RP2's own per-wallet application runs a transfer
  analysis and then a separate pass per wallet; the second pass can dispose
  of a lot the first pass already moved away (vector `us-10`: FIFO sells b1
  in both wallets; basis 200,000 instead of 300,000). Kassiber's engine
  holds every wallet's portions in one chronological pass instead
  (`tax-engine/core/src/wallets.rs`). The RP2 comparison used RP2's
  per-wallet results only on books where its passes agree (no disposal from
  a wallet after it sent units away).
- **US-ORDER** — At one instant: acquisitions, then moves, then disposals.

### Reporting

- **US-8949** — Each disposal's units of one holding period are a Form 8949
  row (the journal books a disposal that spans both terms as two entries):
  (a) description (quantity and asset), (b) date acquired (UTC; `VARIOUS`
  when its units come from several acquisition dates, as the instructions
  allow), (c) date sold, (d) proceeds, (e) cost basis, (h) gain or loss.
  Short-term rows go to Part I, long-term rows to Part II. Rows read by date
  sold, short-term first, then transaction, so a report repeats.
- **US-8949-BOX** — Kassiber has no Form 1099-DA data, so its rows use the
  box for transactions without a broker statement: C and F for tax years
  before 2025, I and L (digital assets) for 2025 and 2026. A row a broker
  reported on Form 1099-DA belongs in G/H or J/K instead; the report says so.
- **US-SCHED-D** — The box I/L (C/F) totals feed Schedule D lines 3 and 10.
  Kassiber states only those two lines: carryovers, other sales and the rest
  of Schedule D are outside the book.
- **US-YEAR-UNKNOWN** — A tax year after the last year Kassiber has forms
  for (2026) has no boxes or lines: the year reads `unknown` and its totals
  are shown without form placement.
- **US-DA-QUESTION** — Form 1040's digital asset question is "yes" for a year
  with any disposal, income or network-fee row.
- **US-INCOME-REPORT** — Ordinary income per kind and in total. Choice: no
  form line is assigned (Schedule 1 "other income" for most individuals,
  Schedule C when the activity is a trade or business, open question 4).

## Fail closed

| Case | Code |
| --- | --- |
| global scope with a disposal or transfer fee on or after 2025-01-01 | `wallet_scope_required` |
| a wallet disposes of or moves more than it holds | `insufficient_lots` (engine: lots exhausted in that wallet) |
| a move's fee without a dollar value under wallet scope | the engine refuses rather than drop its basis |
| the moving averages | not offered for US books; the engine refuses them with wallet scope |

## Fiat

US books are in dollars (`fiat_currency = USD`, required). Values come from
the rates Kassiber already uses (VC FAQ 26–28: the exchange's recorded value,
else the market value when the transaction is recorded on chain).

## Open questions for a US tax reviewer

1. Should the holding period and tax year use the taxpayer's local date
   (a per-book US time zone) instead of UTC?
2. A network fee on a transfer between own wallets: Kassiber removes the fee
   units and their basis without a gain or loss. Should it instead be a
   disposal at the fee's dollar value (the engine computes that gain; the
   final broker regulations treat fee units spent on a sale that way), or
   an addition to the moved units' basis?
3. Should Kassiber support the 2025 transition in one book: universal
   identification through 2024, then a Rev. Proc. 2024-28 allocation
   (specific-unit or global) and per-wallet identification from 2025?
4. Mining and staking: report as Schedule 1 other income by default, with a
   per-book "trade or business" choice for Schedule C?
5. Airdrops and hard forks: is every Kassiber `airdrop` receipt income at
   receipt, or do promotional airdrops need their own review?
6. BTC↔L-BTC pegs: Kassiber books a reviewed peg as a carrying-value move
   when the book's `bitcoin_rail_carrying_value` is on (the default). Is a
   peg a non-taxable transfer, or an exchange of property?
7. Gifts received: Kassiber books an inbound gift kind as an acquisition at
   its value; § 1015 carries over the donor's basis and holding period.
   Should US books refuse gifts until the donor's data is entered?
8. HIFO/LIFO/LOFO as standing orders: are they adequate identification for
   self-custody wallets, and how must the book record the standing order?
9. Form 1099-DA reconciliation: rows from custodial accounts in 2025+ belong
   in boxes G/H or J/K; should Kassiber import 1099-DA data to place them?
