# United Kingdom (`gb`): tax country specification

**Status:** implemented in the Rust backend as a **preview** country
(2026-11-07). Not reviewed by a UK tax adviser: every UK report and screen
carries the preview notice until one does. Python stays `generic`/`at` only
and refuses UK books. Nothing here is tax advice.

**Registry value:** the United Kingdom uses the ISO 3166-1 code `gb`
(`tax_country = "gb"`), never `uk`. User-facing text says "United Kingdom".
The CLI may accept `uk` as input only if it normalizes it to `gb` before
storage; stored books, exports and the registry only ever hold `gb`.

This document says what a UK book must compute and report. The worked cases
are in [tax-country-gb-vectors.json](tax-country-gb-vectors.json); their
expected values come from the reference model in
[tax-country-reference/](tax-country-reference/) (`gb.py`). Regenerate them
with `python3 -I docs/reference/tax-country-reference/build.py . gb`. Vectors
gb-01 to gb-08 replay HMRC's own worked examples and assert HMRC's published
results to the nearest pound.

Scope: an individual resident and domiciled in the UK (or under the FIG
regime without claims), holding exchange tokens as an investor, so Capital
Gains Tax applies. Financial trading (CRYPTO20250: "only in exceptional
circumstances"), mining or staking as a trade, DeFi lending and staking
returns (CRYPTO61000+), non-resident periods and the remittance basis are out
of scope and fail closed or are flagged.

## Sources

All retrieved 2026-11-07.

| Ref | Source |
| --- | --- |
| TCGA92/S104, S105, S106A | Taxation of Chargeable Gains Act 1992: [s104](https://www.legislation.gov.uk/ukpga/1992/12/section/104), [s105](https://www.legislation.gov.uk/ukpga/1992/12/section/105), [s106A](https://www.legislation.gov.uk/ukpga/1992/12/section/106A); s17 (market value), s38 (allowable costs), s43, s52(4), s58 |
| CRYPTO22100 | [What is a disposal](https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22100) |
| CRYPTO22110 | [Transferring tokens between distributed ledgers](https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22110) |
| CRYPTO22150 | [Allowable expenses](https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22150) |
| CRYPTO22200 | [Pooling](https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22200) (updated 17 Nov 2025) |
| CRYPTO22251–22257 | [Pooling examples 1–7](https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22250) |
| CRYPTO22280 | [Fees satisfied in tokens](https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22280) |
| CRYPTO22300, 22350 | [Blockchain forks](https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22300), [airdrops (CGT)](https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22350) |
| CRYPTO22400, 22500 | [Losing private keys](https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22400), [negligible value](https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto22500) |
| CRYPTO21150, 21200, 21250 | Income Tax: [mining](https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto21150), [staking](https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto21200), [airdrops](https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto21250) |
| CRYPTO23000 | [Valuation](https://www.gov.uk/hmrc-internal-manuals/cryptoassets-manual/crypto23000) |
| CGT rates and AEA | [Capital Gains Tax rates and annual tax-free allowances](https://www.gov.uk/government/publications/rates-and-allowances-capital-gains-tax/capital-gains-tax-rates-and-annual-tax-free-allowances) (page updated 13 Apr 2026); earlier years: [2018-19](https://www.gov.uk/government/publications/capital-gains-tax-annual-exempt-amount-for-tax-year-2018-to-2019/capital-gains-tax-annual-exempt-amount-for-tax-year-2018-to-2019), [2019-20](https://www.gov.uk/government/publications/capital-gains-tax-annual-exempt-amount-for-tax-year-2019-to-2020/capital-gains-tax-annual-exempt-amount-for-tax-year-2019-20), [2020-21](https://www.gov.uk/government/publications/changes-to-the-annual-exempt-amount-for-capital-gains-tax-for-the-tax-year-2020-to-2021/changes-to-the-annual-exempt-amount-for-capital-gains-tax-for-the-tax-year-2020-to-2021) |
| CGT guide | [Capital Gains Tax: what you pay it on, rates and allowances](https://www.gov.uk/capital-gains-tax) (reporting thresholds) |
| SA108 notes | Capital Gains Tax summary notes: [2023-24](https://assets.publishing.service.gov.uk/media/660bbb5e91a3206e4382b06b/SA108_Notes_2024.pdf), [2024-25](https://assets.publishing.service.gov.uk/media/67e1612564220b68ed6a7009/SA108-Notes-2025.pdf), [2025-26](https://assets.publishing.service.gov.uk/media/6a02e5df4fb0713aa63ea77c/SA108-Notes-2026.pdf); form [SA108 2026](https://assets.publishing.service.gov.uk/media/69bd8990cfa346b9d47049e4/SA108-2026.pdf) |
| HS325 | [Other taxable income (HS325) 2026](https://www.gov.uk/government/publications/other-taxable-income-hs325-self-assessment-helpsheet/other-taxable-income-2026-hs325): SA100 boxes 17 and 18, the £1,000 allowance |

## Rules

### Disposals

- Disposals include selling for money, exchanging for another token, paying
  for goods or services, and giving away (except to a spouse or civil partner)
  (CRYPTO22100). Gifts are disposals at market value (s17).
- **GB-SWAP** — A token-to-token exchange is a disposal of one pool and an
  acquisition into another (CRYPTO22257).
- **GB-SWAP-VALUES** — The proceeds of the given token are the market value of
  the received token; the cost of the received token is the market value of
  the given token. HMRC's example 7 uses exactly this split (31 August: G
  disposal consideration £4,850 = H received; G acquisition cost £1,700 = H
  given). If only one leg has a value, both use it.
- **GB-TRANSFER** — Moving tokens between addresses the individual
  beneficially owns is not a disposal; neither is a mixer that returns the
  same token (CRYPTO22100).
- **GB-GIFT** — A gift to a spouse or civil partner living together is no gain,
  no loss (s58); a gift to charity is relieved (CG66620P); other gifts are
  disposals at market value. Kassiber requires the recipient class on every
  outgoing gift and fails closed without it (vector gb-23).
- Losing a private key is not a disposal; a negligible value claim on the
  whole pool can crystallise a loss (CRYPTO22400, 22500). Kassiber does not
  file claims; the user records a reviewed claim as a disposal at the stated
  amount and date (not modelled in this version: fail closed).

### Matching (cost basis)

The method is fixed by statute; there is no choice of FIFO, LIFO or average.

- **GB-POOL-GLOBAL** — One pool per token type for the person, across every
  wallet and exchange (CRYPTO22200: "a single pooled asset"). Pool scope
  `global` only (vector gb-12).
- Matching runs per token and per UK calendar day, in this order:
  1. **GB-SAME-DAY** — All acquisitions on a day are one acquisition, all
     disposals one disposal; they are matched first (TCGA92/S105,
     CRYPTO22200).
  2. **GB-30-DAY** — A disposal's remainder is matched with acquisitions in the
     **30 days after** the disposal day (TCGA92/S106A(5)), in the same
     capacity.
  3. **GB-POOL-S104** — What remains comes from the section 104 pool at its
     average cost: cost = pool cost × disposed ÷ pool quantity. Unmatched
     acquisitions enter the pool on their day.
- **GB-30-DAY-ORDER** — Earlier disposals are matched before later ones
  (S106A(4)); within the window, earlier acquisitions first (S106A(5)(b);
  CRYPTO22253).
- **GB-30-DAY-WINDOW** — The window is the 30 calendar days after the disposal
  day: day +1 to day +30 inclusive (vector gb-10).
- Acquisitions made while not UK resident are not matched under the 30-day
  rule (S106A(5A)). Kassiber does not model residence periods: it assumes UK
  residence throughout and says so (Q5).
- A 30-day match may reach into the next tax year: the gain belongs to the
  disposal's year, using the later acquisition's cost (vector gb-03).

### Costs and fees

- Allowable costs are the purchase consideration, transaction fees, and
  incidental costs of acquisition and disposal (s38; CRYPTO22150).
- Exchange fees for buying tokens with fiat raise the acquisition cost; fees
  for selling are incidental disposal costs (CRYPTO22150 table).
- **GB-FEE-SWAP** — A fee on a token-to-token swap belongs to both assets;
  HMRC accepts a 50/50 split between the disposal and the acquisition
  (CRYPTO22150, s52(4)). Kassiber uses 50/50 (vector gb-13).
- **GB-FEE-TOKENS** — A fee paid in tokens is a disposal of those tokens at
  market value (s17(1)(b)), and that market value is also an allowable cost of
  the computation it serves (CRYPTO22280). For the same token on the same day
  it merges into the same-day disposal (vector gb-08 reproduces HMRC's figure
  of £2,998).
- **GB-FEE-TRANSFER** — A network fee on an own-wallet transfer is a disposal
  of the fee tokens at market value with no matching allowable cost, because
  there is no acquisition or disposal it is incidental to (vector gb-14; Q2).
- Mining equipment and electricity are not allowable costs of the tokens
  (CRYPTO22150).

### Income

- **GB-INC-MISC** — Mining and staking that are not a trade are taxed as
  miscellaneous income at the sterling value when received (CRYPTO21150,
  CRYPTO21200). Lightning routing income is treated the same way (a service;
  Q3). Lending interest from a centralised lender is reported in the same
  bucket with a review note; DeFi returns (CRYPTO61000+) are not classified
  by Kassiber (Q4).
- **GB-INC-ACQ** — Coins received as income enter the pool at the value taxed
  as income (vector gb-15).
- **GB-INC-AIRDROP** — An airdrop is income only if received in return for, or
  in expectation of, a service (CRYPTO21250); otherwise it is not income but
  still enters a pool (CRYPTO22350). Kassiber requires a reviewed
  classification per airdrop and fails closed without one (vector gb-22);
  the cost of a non-service airdrop is open (Q6).
- **GB-HARDFORK** — After a hard fork the pooled cost is split between the old
  and new pools on a just and reasonable basis (s43, s52(4); CRYPTO22300). Not
  modelled: fork rows fail closed.
- Miscellaneous income goes on SA100 page TR 3, box 17 (and expenses box 18);
  receipts up to £1,000 can be covered by the trading and miscellaneous income
  allowance (HS325). Kassiber reports gross receipts and states the
  allowance; it does not apply it, because the allowance is shared with any
  trading income outside the book.
- Wages paid in tokens stay a basis acquisition at the reviewed value; PAYE is
  outside Kassiber.

### Exempt amount, losses, rates

- **GB-AEA** — Annual exempt amount for individuals:

  | Tax year | AEA |
  | --- | ---: |
  | 2018-19 | £11,700 |
  | 2019-20 | £12,000 |
  | 2020-21 to 2022-23 | £12,300 |
  | 2023-24 | £6,000 |
  | 2024-25 onwards (2026-27 published) | £3,000 |

  A year after the last published one has no AEA in the registry: the report
  shows the gains and states the AEA is not on file (never assumes one).
- **GB-LOSS** — Same-year losses are set against same-year gains first, in
  full; the AEA then applies to the net gain. Losses brought forward only
  reduce gains down to the AEA. Kassiber reports gains and losses separately
  and the year's net; brought-forward losses are not tracked. Allowable
  losses must be reported (claimed) to be usable later.
- **GB-RATE-SPLIT** — For disposals on or after **30 October 2024** the main
  CGT rates rose from 10%/20% to 18%/24%. The 2024-25 return's automatic
  calculation does not split the year; the taxpayer adjusts in SA108 box 51
  (SA108 notes 2024-25). Kassiber reports 2024-25 gains and losses before and
  from 30 October 2024 (vector gb-19). Kassiber does not compute tax.
- **GB-REPORT-THRESHOLD** — The CGT summary pages are needed if disposal
  proceeds of chargeable assets exceed £50,000 (2023-24 onwards; four times
  the AEA before), if gains before losses exceed the AEA, or if a loss is
  claimed (SA108 notes 2025-26). Kassiber shows each test for the crypto
  figures and labels it indicative: other chargeable assets count too.

### Tax year, time, currency, rounding

- **GB-TAX-YEAR** — 6 April to 5 April; label `2024-25`.
- **GB-TZ** — Every date rule (same day, 30 days, tax year, the 30 October
  split) uses the local date in Europe/London: 2025-04-05T23:30Z is 00:30 BST
  on 6 April, tax year 2025-26 (vector gb-09).
- Values in sterling. A transaction without a sterling value needs a
  reasonable, consistently applied valuation, with the method recorded
  (CRYPTO23000); the report states the price source and granularity.
- Kassiber computes exactly. SA108 boxes are whole pounds (the form pre-prints
  `.00`); Kassiber fills them by dropping the pence of each total
  (Q1). HMRC's examples round individual computations to the pound, so the
  vectors compare HMRC's figures to the nearest pound with ±£1.

## Bitcoin rails

A BTC ↔ L-BTC peg returns a different token on another ledger. A one-way
bridge falls under s43 with the cost carried over (CRYPTO22110); a two-way
peg is "dependent on the facts". As for Germany, the book must record an
explicit decision before a rail pair is booked; otherwise the pair is held
with `rail_policy_required`. Lightning channels stay BTC.

## What the engine and registry need

| Need | Today | For `gb` |
| --- | --- | --- |
| Method | lot methods and moving averages, choosable | one statutory method, `uk_share_matching` (same day → 30 days → s104), not choosable |
| Look-ahead | a disposal is computed when walked | the 30-day rule needs acquisitions up to 30 days later: the computation for a disposal is final only once 30 days have passed, so a report for a year whose last disposal is within 30 days of "now" must say figures may change |
| Day grouping | per event | per token per UK day |
| Holding period | `long_term_days` | none (no short/long distinction) |
| Fees | fee entries are not taxed | fee tokens are disposals (GB-FEE-TOKENS, GB-FEE-TRANSFER); swap fees split 50/50 |
| Tax year | calendar | 6 April – 5 April |
| Swaps | one fiat value per swap | two values (GB-SWAP-VALUES) |

## Registry defaults

| Field | Value |
| --- | --- |
| `tax_country` | `gb` (display: United Kingdom; input `uk` normalizes to `gb`) |
| `preview` | `true` |
| Fiat currency | `GBP`, required |
| Timezone | `Europe/London` |
| Tax year | 6 April – 5 April |
| Holding rule | none (`long_term` not applicable) |
| Accounting methods | `uk_share_matching` (default and only) |
| Cost-basis pool scopes | `global` (default and only) |
| Report language | English, United Kingdom (`en-GB`: `£`, `1,234.56`; dates ISO as in the shared design) |
| Report generators | `open_positions`, `gb_sa108` |
| Income categories | `misc_income` (mining, staking, routing_income, airdrop_service, lending_interest/interest with a review note); refused: unclassified airdrops, hardfork, unclassified gifts |
| Rail pairs | explicit decision required |
| Annual tables | AEA per year (above); reporting threshold per year; 2024-25 rate-change date |
| Legal sources | the table above, retrieved 2026-11-07 |

## Report

### What the taxpayer files

Self Assessment **SA108, Capital Gains Tax summary**:

| Tax year | Where crypto goes | Boxes |
| --- | --- | --- |
| up to 2023-24 | Other property, assets and gains | 14 number of disposals, 15 disposal proceeds, 16 allowable costs, 17 gains before losses, 19 losses |
| 2024-25, 2025-26 | Cryptoassets | 13.1 number of disposals, 13.2 disposal proceeds, 13.3 allowable costs (including purchase price), 13.4 gains before losses, 13.5 losses, 13.6 claim code, 13.7 and 13.8 real-time CGT returns |

Plus, for 2024-25, box 51 (adjustment for the 30 October rate change), and
SA100 box 17 for miscellaneous income. A computation is attached (box 54
or a separate document).

### What Kassiber outputs

One English document in the [shared report design](report-design.md), per
tax year:

1. **Scope** adds the tax year (6 April – 5 April), the matching rules, the
   residence assumption and the preview notice.
2. **SA108** section (after Income): the box table for the year's section with
   the exact totals and the whole-pound box value; the AEA, net gains, gains
   above the AEA, the loss available; the three reporting tests; for 2024-25
   the before/from 30 October split.
3. **Disposals** table, one row per computation (token and UK day): date,
   token, quantity, proceeds, allowable costs, gain or loss, with country
   columns `same_day_quantity`, `same_day_cost`, `thirty_day_quantity`,
   `thirty_day_cost`, `s104_quantity`, `s104_cost`, `incidental_costs` and
   `tax_year`. Its detail rows (one per match, with the acquisition date of
   30-day matches) form the attached computation HMRC asks for.
4. **Section 104 pools** table: per token, the pool quantity and pooled cost
   at the end of the tax year, with the movements (CRYPTO22200 asks to keep
   the pooled cost per token).
5. **Income** table: date, kind, quantity, sterling value; SA100 box 17
   total and the £1,000 allowance note.
6. **Method and provenance**: matching order, fee rules, valuation source,
   review notes.

XLSX sheets and CSV files use the ids `sa108`, `disposals`, `matches`,
`pools`, `income`.

## Test vectors

All 23 cases are in [tax-country-gb-vectors.json](tax-country-gb-vectors.json).
HMRC writes "20XX"; the vectors use 2024 (and 2025 for Daniel), which puts
some examples across a tax-year end on purpose. Token names `TOKA`…`TOKT`
stand for HMRC's "token A"…; amounts in GBP.

**gb-01 Victoria (CRYPTO22251).** Pool 100 for £1,000; buy 50 for £125,000
→ 150 at £126,000. Sell 50 for £300,000: cost 126,000 × 50/150 = 42,000;
gain **£258,000**. Pool 100 at £84,000.

**gb-02 Martyn (CRYPTO22252).** Pool 5,000 at £500. On 23 June: sell 1,000
(£800), buy 1,600 (£1,000), sell 500 (£600). Same day: one disposal of 1,500
for £1,400, matched with 1,500 of the 1,600 bought: cost 1,000 × 1,500/1,600
= 937.50; gain **£462.50** (HMRC £462). The other 100 at £62.50 join the pool:
5,100 at £562.50 (HMRC £562).

**gb-03 Rachel (CRYPTO22253).** Pool 2,000 at £1,000. Sell 1,000 on 31 March
(£400) and 500 on 20 April (£150); buy 700 on 21 April (£175), 500 on 28 April
(£100), 500 on 1 May (£150). 31 March: 700 @ 21 April (£175) + 300 @ 28 April
(£60) → gain **£165**, tax year 2023-24 (the matches are in 2024-25). 20 April:
200 @ 28 April (£40) + 300 @ 1 May (£90) → gain **£20**, 2024-25. 200 from
1 May (£60) join the pool: 2,200 at £1,060.

**gb-04 Daniel (CRYPTO22254).** Pool 8,000 at £1,000. On 31 January: sales
5,000 (£500) and 2,000 (£142); buys 4,000 (£320), 1,000 (£75), 1,000 (£70),
500 (£35). Same day: disposal 7,000 for £642 vs acquisition 6,500 for £500;
the other 500 from the pool: 1,000 × 500/8,000 = 62.50. Gain 642 − 500 −
62.50 = **£79.50** (HMRC £79). Pool 7,500 at £937.50.

**gb-05 Melanie (CRYPTO22255).** Pool 14,000 at £200,000. Sell 4,000 on
30 August for £160,000; buy 500 on 11 September for £17,500. 30 days: 500 at
£17,500; pool: 200,000 × 3,500/14,000 = 50,000. Gain **£92,500**. Pool
10,500 at £150,000.

**gb-06 Gulferaz (CRYPTO22256).** Pool 100,000 at £300,000. 31 July buy
10,000 (£45,000) and sell 30,000 (£150,000); 5 August sell 20,000
(£100,000); 6 August buy 50,000 (£225,000); 7 August sell 100,000
(£150,000). 31 July: same day 10,000 (£45,000) + 30 days 20,000 of 6 August
(£90,000) → gain **£15,000**. 5 August: 20,000 of 6 August (£90,000) → gain
**£10,000**. 6 August: the other 10,000 (£45,000) join the pool: 110,000 at
£345,000. 7 August: 345,000 × 100,000/110,000 = 313,636.36 → loss
**£163,636.36** (HMRC £163,637). Pool 10,000 at £31,363.64.

**gb-07 Elina (CRYPTO22257).** Pool G 100,000 at £300,000. Swaps (given
value / received value):
31 Aug G 1,000 → H 10,000 (3,200 / 3,200); H 5,000 → G 600 (1,700 / 1,920);
G 550 → H 5,000 (1,760 / 1,650); 4 Sep H 2,000 → G 180 (560 / 558);
16 Sep G 400 → H 4,000 (1,080 / 1,080); 27 Oct H 12,000 → G 900
(2,400 / 2,430).
- G, 31 Aug: disposal 1,550, proceeds 3,200 + 1,650 = 4,850. Same day 600 G
  (cost 1,700, the H given); 30 days 180 G of 4 Sep (cost 560); pool 770:
  300,000 × 770/100,000 = 2,310. Gain **£280**. Pool 99,230 at £297,690.
- H, 31 Aug: acquisitions 15,000 for 3,200 + 1,760 = 4,960; disposal 5,000
  for 1,920: cost 4,960 × 5,000/15,000 = 1,653.33 → gain **£266.67** (HMRC
  £267); 10,000 at 3,306.67 join the H pool.
- H, 4 Sep: 2,000 for 558, matched to 2,000 of the 4,000 bought 16 Sep (cost
  1,080 × ½ = 540) → gain **£18**.
- G, 16 Sep: 400 for 1,080 from the pool: 297,690 × 400/99,230 = 1,200 →
  loss **£120**. H pool gets the other 2,000 of 16 Sep at 540: 12,000 at
  3,846.67.
- H, 27 Oct: 12,000 for 2,430, cost 3,846.67 → loss **£1,416.67** (HMRC
  £1,417). G pool gets 900 at 2,400: 99,730 at **£298,890**.
- HMRC's prose says the 180 G of 4 September "go into the section 104 pool",
  but its own computation matches them to the 31 August disposal under the
  30-day rule. The computation is right (S106A) and the vector follows it.

**gb-08 Terri (CRYPTO22280).** Pool 10,000 at £20,000. Sell 1,000 for
£5,000 paying 1 token (£5) as fee. Same-day disposal 1,001 for 5,005; costs
20,000 × 1,001/10,000 = 2,002 plus the fee's £5 → gain **£2,998**. Pool
8,999 at £17,998.

**gb-09 tax-year-boundary-bst.** Pool 1 BTC at £30,000. Sell 0.1 at
2025-04-05T23:30Z = 6 April 00:30 BST for £7,000 → tax year **2025-26**,
cost 3,000, gain 4,000; above the £3,000 AEA by £1,000.

**gb-10 thirty-day-window-edge.** Pool 1 BTC at £30,000. Sell 0.5 on
1 June 2024 for £35,000; buy 0.2 on 1 July (day 30, £12,000) and 0.2 on
2 July (day 31, £12,500). Match 0.2 at 12,000 (30 days); 0.3 from the pool
at 9,000. Gain **£14,000**. Pool 0.9 at 33,500.

**gb-11 bed-and-breakfast-loss.** Buy 1 BTC for £50,000; sell on 5 August
for £40,000; buy back on 20 August for £41,000. The sale matches the
buy-back: loss **£1,000**, not £10,000; the pool keeps 1 BTC at £50,000.

**gb-12 one-pool-across-wallets.** Wallet A buys 0.5 for £10,000, wallet B
0.5 for £20,000. Selling 0.4 from B for £24,000 costs 30,000 × 0.4/1 =
12,000 from the single pool; gain **£12,000** (not B's own £16,000).

**gb-13 swap-fee-split.** Pool 1 BTC at £30,000. Swap 0.5 BTC (value
£24,850) for 31,000 USDT (value £24,800), fee £50. BTC disposal: proceeds
24,800; cost 15,000 + 25 (half the fee) → gain **£9,775**. USDT pool: 24,850
+ 25 = 24,875. Sell the USDT for £24,900 → gain **£25**.

**gb-14 own-transfer-fee-disposal.** Pool 1 BTC at £30,000. Move 0.5 to
wallet B with a 0.0001 BTC fee (£4.50): disposal of 0.0001, cost 3.00, gain
**£1.50** (tax year 2023-24). Pool 0.9999 at £29,997.

**gb-15 misc-income-into-pool.** Mining 0.01 BTC (£400) and routing income
0.002 BTC (£90): box 17 **£490**. Selling the 0.012 for £600 costs 490 from
the pool; gain **£110**.

**gb-16 aea-2024-25-exactly.** 2024-25 gain £3,000.00 = AEA → nothing above
the AEA; gains are not *more than* the AEA, so that test does not trigger the
SA108 pages.

**gb-17 aea-2023-24-crossed.** 2023-24 gain £6,000.01 → £0.01 above that
year's £6,000; the pages are required. Boxes (other property section, the
crypto boxes did not exist yet): 14 = 1, 15 = 10,000, 16 = 4,000, 17 = 6,000,
19 = 0.

**gb-18 losses-offset-before-aea.** 2024-25: BTC gain £5,000, TOKX loss
£4,000. Net £1,000, under the AEA; still gains before losses (£5,000) exceed
the AEA and a loss is claimed, so the pages are required. Boxes 13.4 = 5,000,
13.5 = 4,000.

**gb-19 rate-change-split-2024-25.** Pool 1 BTC at £40,000. Sell 0.1 at
2024-10-29T23:30Z (29 October, GMT) for £6,000 → gain 2,000 **before**; sell
0.1 at 2024-10-30T00:30Z for £5,500 → gain 1,500 **from** 30 October.

**gb-20 proceeds-threshold.** Proceeds £50,000.01 with a £100.01 gain: the
pages are required because proceeds exceed £50,000.

**gb-21 … gb-23 (fail closed).** A disposal larger than every matching source
(`insufficient_holdings`), an unclassified airdrop
(`airdrop_classification_required`), a gift without a recipient class
(`gift_recipient_classification_required`).

## Fail closed

**GB-FAILCLOSED** — Kassiber refuses or holds instead of guessing for:
unclassified airdrops; outgoing gifts without a recipient class; hard forks;
negligible value claims; BTC ↔ L-BTC pairs without the book's rail decision;
book currency other than GBP; a pool scope other than `global`; any method
other than `uk_share_matching`; disposals exceeding every matching source;
an AEA for a year not in the table (reported as unknown, never assumed);
trading, DeFi or non-resident cases (out of scope).

## Implementation

What the Rust backend does, and where it differs from the rules above.

- **Registry** (`crates/tax-countries`, entry `gb`): GBP required, the one
  method `UK_SHARE_MATCHING`, the `global` pool scope only, no holding
  period, Europe/London, tax years from 6 April (`2024-25`), the generators
  `open_positions` and `gb.sa108`, the AEA, proceeds-threshold and main-rate
  tables, and the sources above. `uk` is not accepted as input: the
  registry knows only `gb`, so `--tax-country uk` is refused as an unknown
  country.
- **Engine** (`uk_share_matching`, `tax-engine/core/src/share_matching.rs`):
  per token and London day, same day, then the 30 days after, then the
  section 104 pool, exactly as GB-SAME-DAY, GB-30-DAY, GB-30-DAY-ORDER,
  GB-30-DAY-WINDOW and GB-POOL-S104 state. Acquisitions include income
  receipts and swap in-legs (a mining receipt within 30 days after a sale is
  matched with it). Fee tokens are disposals before the sale they pay for,
  their value deducted from the sale (`disposal_before_sale`, GB-FEE-TOKENS);
  a move's fee is a disposal with no other cost (GB-FEE-TRANSFER); a taxable
  swap's proceeds are the received token's value and the received token's
  cost the given one's (GB-SWAP-VALUES). A disposal the pool cannot cover
  fails; the walk's wallet balance check holds it first as
  `insufficient_holdings`.
- **Refusals** (GB-FAILCLOSED): the engine holds `airdrop`
  (`airdrop_classification_required`), `hardfork`/`hard_fork`
  (`hardfork_cost_split_unsupported`) and `income`
  (`income_kind_unsupported`) receipts before pricing them, and quarantines
  every outgoing gift and donation, which the layer reports as
  `gift_recipient_classification_required`. Kassiber has no kind for a
  reviewed service airdrop or a gift's recipient class yet, so every airdrop
  and gift is refused. A book with any refusal shows the refusals and
  withholds every SA108 figure.
- **Country layer** (`crates/reports/src/gb_tax.rs`, sections in
  `report_design/gb.rs`): the engine's journal entries grouped per token and
  London day, each fragment's rule read from its lot's London date (same
  day, later: 30 days, earlier: the pool), the pools replayed from the
  journal's acquisitions to give each pool match's pool before and the pools
  at each tax year's end, and the year figures, tests and 2024-25 split.
  Box values drop the pence after removing the engine's 32-digit dust (below
  1E-10). An entry's expenses (the fee tokens' value) are read as its
  proceeds less its cost and gain; the journal does not store them apart.
  A year whose last disposal is no more than 30 days before the report's
  time is marked provisional.
- **Fiat fees:** a Kassiber book stores no fiat fee apart from a
  transaction's value. A swap's fiat fee (GB-FEE-SWAP) can therefore only be
  booked inside the legs' values; the vector runner puts half into each leg
  and adds the disposal's half back as an incidental cost (gb-13).
- **Rail pairs:** the registry entry sets `rail_decision_required`, as
  Germany's does: a BTC/L-BTC rail pair is held (`rail_policy_required`)
  until the book decides it with `transfers pair --policy`; the profile's
  `bitcoin_rail_carrying_value` default does not decide it.
- **Calendars:** the report readers (desktop years, tax summary, period
  presets) date a UK book in Europe/London; their year views are calendar
  years, while the UK layer and the engine's tax summary use tax years.
- **Wallet scope:** matching runs over the whole book, so a wallet-scoped
  report shows the book's computations and says so.

Every vector passes through the engine (`ledger` operation under the
registry's rules) and the layer (`crates/reports/tests/tax_country_vectors.rs`
with `country_vectors/gb_ie.rs`); gb-21 to gb-23 are refused with their
codes. `tests/test_rust_gb_ie_tax_countries.py` builds a two-wallet book with
the CLI (a 30-day match, a move's fee, a sale across the 5 April year end,
mining matched by the 30-day rule), processes its journal and checks the
journal entries, SA108 boxes, matches, pools and income in the CSV, XLSX
and PDF exports (two PDFs of one book are byte-identical), and a gift's
refusal. RP2 has no UK plugin, so there is no RP2 comparison.

### Using it

`kassiber profiles create --workspace Main --tax-country gb Book` creates a
UK book (GBP, UK share matching, one pool); the desktop's book picker lists
the United Kingdom with a Preview badge. `kassiber journals process`, then
`kassiber reports export-pdf|export-xlsx|export-csv` write the full report
with the UK sections; the CSV bundle's UK files are `gb_sa108.csv`,
`gb_years.csv`, `gb_rate_split.csv`, `gb_computations.csv`,
`gb_matches.csv`, `gb_pools.csv` and `gb_income.csv` (`gb_refusals.csv`
instead when a transaction is refused). The specification's sheet ids
`sa108`, `disposals`, `matches`, `pools` and `income` carry a `gb_` prefix so
they cannot collide with the shared tables.

## Open questions for a UK tax adviser

1. **Q1** SA108 whole pounds: drop the pence of each total (this spec), or
   round proceeds and gains down and costs and losses up?
2. **Q2** Network fee on an own-wallet transfer: a disposal of the fee tokens
   at market value with no allowable cost besides their pooled cost (this
   spec)?
3. **Q3** Lightning routing fees: miscellaneous income (this spec), or could
   they be capital or trading receipts?
4. **Q4** Interest from a centralised BTC lender: miscellaneous or savings
   income; and where on SA100?
5. **Q5** Is assuming UK residence for every 30-day match acceptable when the
   book has no residence history (S106A(5A))?
6. **Q6** Airdrops not received for a service: allowable cost nil (s17(2)
   excludes market value where there is no consideration and no corresponding
   disposal) or market value?
7. **Q7** Box 13.1 "number of disposals": one per token per day after
   same-day aggregation (this spec), or one per transaction?
8. **Q8** BTC ↔ L-BTC pegs: disposal, or s43-style continuation?
9. **Q9** Acquisitions by income (mining, staking) within 30 days after a
   disposal: Kassiber matches them under S106A like purchases. Correct?
10. **Q10** Box values: Kassiber drops the pence of the exact total (after
    removing arithmetic dust below 1E-10). Acceptable?

## Fonts and language

English with `£`; DM Sans covers every needed glyph. No new font is needed.
