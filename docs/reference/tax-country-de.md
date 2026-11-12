# Germany (`de`): tax country specification

**Status:** implemented in the Rust backend as a **preview** country
(2026-11-07; [implementation](#implementation)). Not reviewed by a German
tax adviser (Steuerberater). Every German report and screen carries the
preview notice until one does. Nothing here is tax advice.

This document says what a German book must compute and report so that an
implementation on the Rust country registry can be written and tested without
repeating the research. The worked cases are in
[tax-country-de-vectors.json](tax-country-de-vectors.json); their expected
values come from the reference model in
[tax-country-reference/](tax-country-reference/) (`de.py`), which implements
the rules below in plain Python and can regenerate the file:
`python3 -I docs/reference/tax-country-reference/build.py . de`.

Scope: an individual resident in Germany holding Bitcoin (and, rarely, other
tokens) as **private assets** (Privatvermögen). Business assets
(Betriebsvermögen), commercial trading (§ 15 EStG) and commercial mining are
out of scope and fail closed or are flagged (see [fail closed](#fail-closed)).

## Sources

All retrieved 2026-11-07.

| Ref | Source |
| --- | --- |
| EStG § 23 | [§ 23 EStG, private Veräußerungsgeschäfte](https://www.gesetze-im-internet.de/estg/__23.html) |
| EStG § 22 | [§ 22 EStG, Nr. 2 and Nr. 3](https://www.gesetze-im-internet.de/estg/__22.html) |
| AO § 108, BGB §§ 187, 188 | [§ 108 AO](https://www.gesetze-im-internet.de/ao_1977/__108.html) applies [§ 187](https://www.gesetze-im-internet.de/bgb/__187.html) and [§ 188 BGB](https://www.gesetze-im-internet.de/bgb/__188.html) to tax periods |
| BMF 2025 | BMF-Schreiben vom 6. März 2025, „Einzelfragen zur ertragsteuerrechtlichen Behandlung bestimmter Kryptowerte“, IV C 1 - S 2256/00042/064/043, BStBl I S. 658; [BMF page](https://www.bundesfinanzministerium.de/Content/DE/Downloads/BMF_Schreiben/Steuerarten/Einkommensteuer/2025-03-06-einzelfragen-kryptowerte.html). It replaces the letter of 10 May 2022. Paragraph numbers below are its Randnummern (Rn.). The BMF site refused automated downloads on the retrieval date; the text was read from a verbatim copy of the signed PDF ([steuerrat24 mirror](https://www.steuerrat24.de/images/bestand-pdf/Kryptowerte-Einzelfragen-BMF-2025-03-06.pdf)), header, GZ and DOK checked |
| BFH IX R 27/04 | BFH, Urteil vom 11.01.2005, IX R 27/04, BStBl II 2005, 433: the Freigrenze is applied before a loss carry-back ([dejure](https://dejure.org/dienste/vernetzung/rechtsprechung?Text=IX+R+27%2F04)) |
| ELSTER 2024, 2025 | ELSTER help for Einkommensteuer, Anlage SO: [2025](https://www.elster.de/eportal/helpGlobal?themaGlobal=help_est_ufa_10_2025), [2024](https://www.elster.de/eportal/helpGlobal?themaGlobal=help_est_ufa_10_2024) |

## Rules

Every rule has an id; the vectors list the ids they exercise.

### Taxable events

- **DE-EVENT-SELL** — Selling crypto for euros or other fiat is a disposal
  (Veräußerung) under § 23 Abs. 1 Satz 1 Nr. 2 EStG; crypto are „andere
  Wirtschaftsgüter“ (BMF 2025 Rn. 53, BFH IX R 3/22). Proceeds are the agreed
  price (Rn. 58).
- **DE-EVENT-SWAP** — Exchanging one token for another is a disposal of the
  given token and an acquisition of the received token (Rn. 54).
- **DE-SWAP-VALUES** — Proceeds of the given token are the market value of the
  received token at the time of the swap (Rn. 58 Satz 2). The acquisition cost
  of the received token is the market value of the given token plus any
  incidental costs (Rn. 59 Satz 1). The two values can differ; Kassiber keeps
  both.
- **DE-SWAP-FALLBACK** — If the received token has no ascertainable market
  value, the given token's value may be used as proceeds (Rn. 58 Satz 3).
  If neither leg has a value the swap is unpriced and quarantined (existing
  `missing spot price` behaviour).
- **DE-EVENT-SPEND** — Paying for goods or services is a disposal; proceeds
  are the agreed euro price, or the given token's market value if no price was
  stated (Rn. 60).
- **DE-HOLD-RESTART** — The holding period starts again after every swap
  (Rn. 55 Satz 1): the received token is acquired at the swap.
- **DE-TRANSFER** — Moving coins between the taxpayer's own wallets is neither
  a disposal nor an acquisition. The moved units keep their acquisition date
  and cost (Rn. 56 for UTXO change; the per-wallet rule in Rn. 62 needs the
  moved lots to arrive with their history).
- **DE-GIFT-IN** — A gift received is not an acquisition; the donor's
  acquisition date and cost carry over (§ 23 Abs. 1 Satz 3 EStG, Rn. 75 last
  sentence). Without the donor's data the gift fails closed.

### Holding period and exemption

- **DE-HOLD-1Y** — A disposal is taxable only if no more than one year passed
  between acquisition and disposal (§ 23 Abs. 1 Satz 1 Nr. 2 EStG). Gains
  outside the period are not taxable, and losses outside it are not
  deductible.
- **DE-HOLD-CAL** — The year is counted in calendar dates, not days: the period
  starts the day after the acquisition (§ 187 Abs. 1 BGB) and ends at the end of
  the anniversary date (§ 188 Abs. 2 BGB, via § 108 AO). A disposal on the
  anniversary date is still taxable; from the following day it is tax-free.
  Rule: `taxable ⇔ disposal_date ≤ anniversary(acquisition_date)`.
- **DE-HOLD-LEAP** — If the anniversary date does not exist (29 February), the
  period ends on 28 February (§ 188 Abs. 3 BGB).
- **DE-TZ** — Dates are the local dates in Europe/Berlin. Times are those the
  platform or wallet records (Rn. 55 Sätze 2–3); Kassiber uses the event time
  it already prices from (confirmation time when known, otherwise the
  observed time).
- **DE-ENGINE-DAYCOUNT** — The engine's current long-term rule ("held at least
  `long_term_days` whole 24-hour days", [tax engine](tax-engine.md)) does not
  express this: 365 days that cross 29 February are still within the year
  (vector de-05). Germany needs a calendar-date rule in the registry; a
  `long_term_days` value must not be used for `de`.
- No ten-year extension for currency or payment tokens (§ 23 Abs. 1 Satz 1
  Nr. 2 Satz 4 EStG does not apply: Rn. 63), including coins used for lending
  or staking.

### Cost basis

- **DE-POOL-WALLET** — The order of use is determined per wallet
  (walletbezogene Betrachtung, Rn. 62). Each Kassiber wallet (including an
  exchange account) is one wallet. Pool scope `wallet` is mandatory for `de`;
  the global pool is wrong (vector de-17).
- **DE-METHOD-FIFO** — Where units cannot be individually identified, the first
  acquired units of a token in a wallet are treated as disposed first for the
  holding period, and FIFO may also be used for the value as a simplification
  (Rn. 61 Sätze 2–3). This is Kassiber's default (`fifo`).
- **DE-METHOD-AVG** — The main rule in Rn. 61 is FIFO for the holding period
  and the average method (Durchschnittsmethode, BFH X R 49/90) for the value.
  Kassiber offers it as `moving_average_fifo_period`: the taxable share is
  decided by the FIFO lots' dates, the cost of every disposed unit is the
  wallet's moving average cost at the time (vector de-19).
- Individual identification (Einzelbetrachtung, Rn. 61 Satz 1) of specific
  UTXOs is the legal starting point, but Kassiber does not offer it. LIFO,
  HIFO and LOFO are not permitted for `de`.
- The chosen method must be kept per wallet and token until that wallet's
  holding of the token is fully disposed of; only then may it change (Rn. 62).
  Kassiber has one method per book, applied to the whole history; changing it
  recomputes every year, so the report says so (see open question Q4).
- **DE-FEE-ACQ** — Acquisition fees paid in fiat are incidental acquisition
  costs (Anschaffungsnebenkosten, Rn. 59, Rn. 103) and raise the lot's cost.
- **DE-FEE-DISP** — Fees connected with a disposal are Werbungskosten of that
  disposal (Rn. 59 Satz 2) and are shown in their own column.
- **DE-FEE-SPLIT** — Werbungskosten are split between taxable and non-taxable
  parts of a disposal (Rn. 57 Satz 2). Kassiber splits them by quantity over
  the consumed lots.
- **DE-FEE-NETWORK** — A network fee paid in the disposed token is itself a
  disposal: the fee units are given for a service (Rn. 54, Rn. 60), at their
  market value. That value is also a Werbungskosten item of the disposal the
  fee belongs to (Rn. 59). Within one transaction the fee units are taken
  first, then the principal, matching how the engine already pays a move's
  fee.
- **DE-FEE-TRANSFER** — A network fee on an own-wallet transfer is a disposal
  of the fee units at market value. It is not Werbungskosten of any disposal,
  because the transfer is not one (open question Q2).

### Income

- **DE-INC-LENDING** — Lending returns are other income from services
  (§ 22 Nr. 3 EStG, Rn. 65), valued at market value when received.
- Passive staking (delegation, pool or platform staking) is likewise § 22 Nr. 3
  income (Rn. 48); rewards may be dated when claimed and credited in the
  wallet, and unclaimed rewards at the latest at year end (Rn. 48a).
- **DE-INC-ROUTING** — Lightning routing fees are not named by the BMF. They
  are compensation for a service, so the spec books them as § 22 Nr. 3 income
  (Rn. 46: any conduct for which consideration is accepted). Open question Q3.
- **DE-INC-ACQ** — Coins received as income are acquired at that value on that
  date (Rn. 54: block rewards, lending and staking returns are acquisitions);
  their § 23 period starts there.
- **DE-INC-MINING** — Block creation (mining, forging) is never private asset
  management (Rn. 39). It is commercial (§ 15) if sustained, otherwise
  § 22 Nr. 3 (Rn. 45). Kassiber cannot decide which: mining income is reported
  in its own section, „Blockerstellung – Einordnung erforderlich“, and is not
  added to the § 22 Nr. 3 total (vector de-22).
- **DE-INC-AIRDROP** — An airdrop is § 22 Nr. 3 income only if the recipient
  provided a service (Leistung, Rn. 70–71), for example personal data beyond
  what allocation needs or promotional posts; a chance element breaks the link
  (Rn. 72). Without a service it may be a gift (Rn. 74). Kassiber requires a
  reviewed classification per airdrop: `airdrop_service` (income at market
  value, 0 € if no market value can be found, Rn. 73) or `airdrop_gift`.
  Unclassified airdrops fail closed (vector de-23). `airdrop_gift` also fails
  closed in this version, because the acquisition date and cost of a gift from
  a project are unsettled (Q5).
- **DE-HARDFORK** — A hard fork is not income (Rn. 68); the new coins share the
  original acquisition date, and the original cost is split by the market-value
  ratio at the fork (Rn. 67). Kassiber does not model the split; fork rows fail
  closed (vector de-25).
- Wages paid in crypto stay a basis acquisition at the reviewed value
  (Kassiber's existing `wages` behaviour); wage tax is outside Kassiber.

### Allowances and losses

- **DE-FG-23** — § 23 gains are tax-free if the year's total gain from all
  private disposals is **less than** the Freigrenze (§ 23 Abs. 3 Satz 5 EStG,
  Rn. 53). It is a Freigrenze, not a Freibetrag: at or above it the whole
  amount is taxable.
- **DE-FG-23-2023** — Up to and including 2023: less than 600 €.
- **DE-FG-23-2024** — From 2024: less than 1,000 € (Wachstumschancengesetz;
  ELSTER help 2024 and 2025). The limit applies per person, also under joint
  assessment.
- The Freigrenze covers every private disposal of the year, including assets
  outside Kassiber (gold, foreign currency, other assets). Kassiber's verdict
  is therefore marked indicative.
- **DE-LOSS-23** — § 23 losses offset only § 23 gains of the same year; the
  remainder can be carried back one year or forward, again only against § 23
  gains (§ 23 Abs. 3 Sätze 7–8). The Freigrenze is tested on the year's net
  figure **before** any carry-back or carry-forward (BFH IX R 27/04), so a year
  under the limit consumes no carried loss. Kassiber reports the year's net
  figure and leaves the carry to the tax office (Anlage SO loss line).
- **DE-FG-22-3** — § 22 Nr. 3 income is not taxable if it totals **less than**
  256 € in the calendar year (§ 22 Nr. 3 Satz 2 EStG, Rn. 45); again a
  Freigrenze per person, covering all § 22 Nr. 3 income. § 22 Nr. 3 losses
  follow § 22 Nr. 3 Sätze 3–4; Kassiber has no expenses to set against this
  income, so it reports gross receipts.

### Currency, prices and rounding

- Book currency must be EUR. Rows in other fiat are converted at the
  transaction's stored rate; an unconvertible row is quarantined.
- Market values come from a trading platform or a web list (Rn. 43). Exact
  prices or documented daily prices (daily average, fixed time of day or
  close) are accepted if applied consistently for acquisitions and disposals
  (Rn. 91; Rn. 106 tolerates the old practice until 2024). Kassiber's accepted
  coarse fallback is a daily price; the report must state the price source and
  how many rows used exact and daily prices (Rn. 90 asks for the report
  settings). See Q6 on mixing.
- Kassiber computes exactly and rounds only for display: fiat to cents,
  half-even, after summing the exact values. The form takes euros and cents.

### Tax year

Calendar year (Veranlagungszeitraum), Europe/Berlin local dates.

## Bitcoin rails (DE-RAIL)

BTC ↔ L-BTC pegs and swaps: whether L-BTC (a claim on the Liquid federation)
is the same Wirtschaftsgut as BTC is not settled. If it is not, a peg is a
Tausch: a disposal that restarts the one-year period. Kassiber's generic
default (`bitcoin_rail_carrying_value` on) would silently assume carrying
value. For `de` the book must record an explicit decision (carrying value or
taxable) before a rail pair can be booked; otherwise the pair is held with
`rail_policy_required` (vector de-24). Lightning channel opens and closes stay
BTC and are own-wallet movements.

## What the engine and registry need

| Need | Today | For `de` |
| --- | --- | --- |
| Holding rule | `long_term_days` (24-hour days) | calendar rule "anniversary date passed", Europe/Berlin (DE-HOLD-CAL) |
| Pool scope | `global` only | `wallet` only (DE-POOL-WALLET): lots move with own-wallet transfers, keeping acquisition dates and cost |
| Methods | fifo, lifo, hifo, lofo, moving_average | `fifo` (default), `moving_average_fifo_period` (new) |
| Fees | fee entries are excluded from the yearly tax summary | fee units are disposals with gain or loss (DE-FEE-NETWORK, DE-FEE-TRANSFER) |
| Freigrenzen | none | § 23 (600 € / 1,000 €) and § 22 Nr. 3 (256 €) yearly verdicts |
| Income | earn-like kinds are one income bucket | § 22 Nr. 3 vs mining (needs classification); airdrops need review |
| Rails | `bitcoin_rail_carrying_value` default on | explicit decision required |

A swap keeps two values (Rn. 58, 59); if the lot engine stores one fiat value
per swap, `de` needs both legs' values.

## Registry defaults

| Field | Value |
| --- | --- |
| `tax_country` | `de` |
| `preview` | `true` |
| Fiat currency | `EUR`, required |
| Timezone | `Europe/Berlin` |
| Tax year | calendar year |
| Holding rule | calendar: taxable while `disposal_date ≤ anniversary(acquisition_date)` (29 Feb → 28 Feb); no `long_term_days` |
| Accounting methods | `fifo` (default), `moving_average_fifo_period` |
| Cost-basis pool scopes | `wallet` (default and only) |
| Report language | German, Germany (`de-DE`) |
| Report generators | `open_positions`, `de_anlage_so` |
| Income categories | `p22_3` (staking, lending_interest, interest, routing_income, airdrop_service), `mining_classification_required` (mining), refused: unclassified airdrops, hardfork, airdrop_gift |
| Rail pairs | explicit decision required |
| Legal sources | the table above, retrieved 2026-11-07 |

## Report

### What the taxpayer files

Private disposals and § 22 Nr. 3 income go on **Anlage SO** of the income tax
return (ESt). The line numbers move between years:

| Year | Leistungen (§ 22 Nr. 3) | Kryptowerte (§ 23) | Loss deduction |
| --- | --- | --- | --- |
| 2024 | Zeilen 10–16 | Zeilen 41–47, 54–55 („Einheiten virtueller Währungen und/oder sonstige Token“) | Zeile 62 |
| 2025 | Zeilen 14–21 | Zeilen 43/45–49, 56–57 („Kryptowerte“) | Zeile 66 |

The 2025 ELSTER help names both line 43 and line 45 as the start of the crypto
block (Q1). The crypto block asks, per entry, for a description
(Bezeichnung), the acquisition and disposal dates, the proceeds
(Veräußerungspreis), acquisition costs (Anschaffungskosten), Werbungskosten
and the gain or loss; the total carries to the private-disposals sum line.
Many disposals are entered as one aggregated entry with an attached list
(Rn. 102 sets what the list must show). Kassiber prints line numbers only for
years in its table; for any other year it prints the field names and says the
line numbers are not on file, rather than guessing.

### What Kassiber outputs

One German document (`de-DE`) in the [shared report design](report-design.md),
with the country layer below. Month names are German (Januar, not the
Austrian Jänner); numbers `1.234,56`, dates `15.03.2025`, as for Austria.

1. **Scope** adds: method („FIFO je Wallet“ or „Durchschnittsmethode,
   Haltefrist nach FIFO, je Wallet“), the price source and the count of rows
   priced exactly vs with daily prices (Rn. 90, 91), and the preview notice.
2. **Anlage SO** section (after Income), per tax year:
   - Kryptowerte (§ 23): Bezeichnung „Kryptowerte lt. Aufstellung“, period,
     Veräußerungspreis, Anschaffungskosten, Werbungskosten and Gewinn/Verlust,
     summed over **taxable** fragments only, with the year's line numbers;
   - Freigrenze verdict: the net gain, the year's limit (600 € / 1,000 €) and
     one of „steuerpflichtig“, „unter der Freigrenze (indikativ)“, „Verlust
     (nur mit § 23-Gewinnen verrechenbar)“, „keine“;
   - Leistungen (§ 22 Nr. 3): total and the 256 € verdict;
   - Blockerstellung – Einordnung erforderlich: mining total, not added.
3. **Disposals** table, one row per consumed lot (fragment). Country columns
   appended: `acquired_at`, `holding` („≤ 1 Jahr“ / „> 1 Jahr“), `taxable`
   (ja/nein), `expenses` (Werbungskosten), `price_source`. Non-taxable rows
   stay in the table so the list proves the holding periods (Rn. 102).
4. **Income** table with a `section` column (`§ 22 Nr. 3` /
   `Blockerstellung`).
5. **Holdings** per wallet at 31 December of the year and the year before
   (Rn. 104).
6. **Method and provenance**: the rules above in one paragraph each, the
   Freigrenze caveat (other private disposals count), and the open
   classifications.

XLSX sheets and CSV files use the table ids `anlage_so`, `disposals`,
`income`, `holdings`; CSV headers stay English `snake_case`.

## Test vectors

All 26 cases are in [tax-country-de-vectors.json](tax-country-de-vectors.json)
(wallet `A` unless stated; times as given, dates are Berlin dates; amounts in
EUR). "Taxable" means inside the one-year period.

**de-01 fifo-within-year.** Buy 1 BTC on 10.01.2024 for 40,000 + 40 fee →
lot cost 40,040. Sell 0.5 on 15.06.2024 for 30,000 with a 30 € fee. Cost
0.5 × 40,040 = 20,020; gain 30,000 − 20,020 − 30 = **9,950**, taxable.
2024: 9,950 ≥ 1,000 → taxable 9,950. Remaining 0.5 BTC at 20,020.

**de-02 anniversary-day-taxable.** Buy 15.03.2023 for 20,000; sell
15.03.2024 for 60,000. Anniversary 15.03.2024, disposal date not after it →
taxable gain **40,000**.

**de-03 day-after-anniversary-exempt.** Same buy; sell 16.03.2024 00:05 →
after the anniversary → not taxable; gain 40,000 listed as outside the period;
2024 result „keine“.

**de-04 leap-day-acquisition.** Buy 0.1 on 29.02.2024 for 5,000. The
anniversary is 28.02.2025 (§ 188 Abs. 3). Sell 0.05 on 28.02.2025 for 4,500:
taxable, cost 2,500, gain **2,000**. Sell 0.05 on 01.03.2025 for 4,600:
not taxable, gain 2,100. 2025 taxable 2,000.

**de-05 365-days-still-taxable.** Buy 01.03.2023 10:00, sell 29.02.2024 10:00:
exactly 365 × 24 h, which the generic rule (`long_term_days` 365) treats as
long-term. The anniversary is 01.03.2024, so the sale is inside the year:
taxable gain 55,000 − 21,000 = **34,000**.

**de-06 berlin-calendar-day.** Buy 2023-06-10T22:30Z = 11.06.2023 00:30
Berlin; sell 2024-06-11T21:30Z = 11.06.2024 23:30 Berlin. Anniversary
11.06.2024 → taxable gain 13,000 − 5,000 = **8,000**. Using UTC dates
(10.06.2023 → anniversary 10.06.2024) would wrongly exempt it.

**de-07 mixed-lots-expense-split.** Lots: 0.3 BTC on 01.05.2022 for 9,000;
0.3 BTC on 20.01.2024 for 12,000. Sell 0.4 on 01.09.2024 for 24,000, fee 40.
FIFO: 0.3 from the 2022 lot (not taxable) and 0.1 from the 2024 lot (taxable).
Proceeds and fee by quantity: 18,000 / 6,000 and 30 / 10. Taxable: 6,000 −
4,000 − 10 = **1,990**; outside the period: 18,000 − 9,000 − 30 = 8,970.
Remaining 0.2 at 8,000.

**de-08 freigrenze-2024-below.** Gain 4,999.99 − 4,000 = 999.99 < 1,000 →
„unter der Freigrenze“, taxable amount 0.

**de-09 freigrenze-2024-at-limit.** Gain 5,000 − 4,000 = 1,000.00, not below
1,000 → the whole **1,000.00** is taxable.

**de-10 freigrenze-2023-600.** 2023 gain 2,600 − 2,000 = 600.00; the 2023
limit is 600, not below → taxable **600.00** (it would be exempt under the
2024 limit).

**de-11 loss-netting-under-freigrenze.** Lots 0.1 at 4,000 (05.01.2024) and
0.1 at 6,500 (01.03.2024). Sell 0.1 on 01.04.2024 for 5,500 → FIFO first lot,
gain 1,500. Sell 0.1 on 01.08.2024 for 5,800 → second lot, loss −700. Net
800 < 1,000 → exempt; the loss is used up in the year, nothing carries.

**de-12 net-loss-year.** Sell 0.1 for 6,200 bought for 6,500 → **−300**,
reported as a § 23 loss.

**de-13 swap-btc-usdt.** Buy 0.5 BTC for 20,000. On 10.04.2024 swap 0.25 BTC
(market value 14,250) for 15,500 USDT (market value 14,300). BTC disposal:
proceeds 14,300 (received value), cost 10,000 → gain 4,300. USDT lot: cost
14,250 (given value), acquired 10.04.2024. Sell the USDT on 10.05.2024 for
14,400 → gain 150. 2024 taxable **4,450**.

**de-14 swap-received-value-unknown.** Same swap, USDT value unknown →
proceeds 14,250 (given value), gain **4,250**; USDT cost 14,250.

**de-15 lending-income-under-256.** Lending interest 0.001 (60.00), 0.002
(130.00), 0.001 (65.99) → § 22 Nr. 3 total 255.99 < 256 → exempt. Selling
the 0.004 BTC on 01.12.2024 for 380: FIFO costs 60 + 130 + 65.99 = 255.99,
proceeds split 95 / 190 / 95, gain 124.01, taxable but under the § 23
Freigrenze.

**de-16 routing-income-at-256.** Routing income 120 + 136 = 256.00, not below
256 → **256.00** taxable § 22 Nr. 3 income.

**de-17 wallet-based-fifo.** Wallet A: 1 BTC bought 10.01.2023 for 15,000.
Wallet B: 1 BTC bought 01.02.2024 for 38,000. Sell 0.5 from B on 01.07.2024
for 28,000 → B's lot: cost 19,000, gain **9,000** taxable. A global FIFO
pool would take A's 2023 lot (outside the period) and report 0.

**de-18 own-transfer-keeps-dates-fee-is-disposal.** Buy 1 BTC on 10.01.2024
for 40,000 in A. On 01.03.2024 move 0.5 to B with a 0.0001 BTC fee (market
value 5.80). Fee first: 0.0001 from the lot, cost 4.00, proceeds 5.80, gain
1.80 (taxable, not Werbungskosten). The 0.5 arrives in B with acquisition
date 10.01.2024 and cost 20,000. Sell it on 01.05.2024 for 31,000 → gain
11,000. 2024 taxable **11,001.80**; A keeps 0.4999 at 19,996.

**de-19 average-method.** `moving_average_fifo_period`. Lots 0.5 (10.01.2023,
8,000) and 0.5 (01.02.2024, 20,000): average 28,000/BTC. Sell 0.6 on
01.06.2024 for 36,000 (60,000/BTC). FIFO dates: 0.5 from 2023 (outside),
0.1 from 2024 (taxable). Cost at the average: 0.5 × 28,000 = 14,000 and
0.1 × 28,000 = 2,800. Taxable gain 6,000 − 2,800 = **3,200** (FIFO value
would give 6,000 − 4,000 = 2,000). Remaining 0.4 at 11,200.

**de-20 spend-agreed-price.** Pay 0.01 BTC for a laptop priced 650 €: gain
650 − 400 = 250, taxable, under the Freigrenze.

**de-21 network-fee-on-sale.** Lots 1 BTC (10.01.2023, 15,000) and 0.5 BTC
(01.02.2024, 20,000). Sell 1 BTC on 01.06.2024 for 60,000; network fee 0.0002
BTC worth 12.00. Fee first: 0.0002 from the 2023 lot, cost 3.00, proceeds
12.00, gain 9.00 outside the period. Principal: 0.9998 from the 2023 lot
(cost 14,997, proceeds 59,988, Werbungskosten 11.9976, outside) and 0.0002
from the 2024 lot (cost 8.00, proceeds 12.00, Werbungskosten 0.0024,
gain 3.9976 → **4.00** taxable). Remaining 0.4998 at 19,992.

**de-22 mining-needs-classification.** Mining 0.01 BTC worth 600 →
reported under „Blockerstellung – Einordnung erforderlich“: 600.00; § 22
Nr. 3 total 0.

**de-23 … de-26 (fail closed).** An unclassified airdrop
(`airdrop_classification_required`), a BTC → L-BTC peg without a book decision
(`rail_policy_required`), a hard fork (`hardfork_cost_split_unsupported`),
a received gift without the donor's data (`gift_predecessor_data_required`).
Each names the row; nothing after it is booked as if the input were known.

## Fail closed

**DE-FAILCLOSED** — Kassiber must refuse or hold instead of guessing for: unclassified airdrops
and `airdrop_gift`; hard forks; received gifts without the donor's acquisition
data; BTC ↔ L-BTC pairs without the book's explicit rail decision; a book
currency other than EUR; a wallet pool scope other than `wallet`; LIFO, HIFO
and LOFO; a disposal larger than the wallet's lots (insufficient lots, as
today); Anlage SO line numbers for a year not on file (print field names
only). Mining is reported but never totalled into § 22 Nr. 3.

## Implementation

What Kassiber does for a `de` book, and where.

| Rule | Where | Test |
| --- | --- | --- |
| Registry entry (EUR, Europe/Berlin, `MoreThanOneYear`, FIFO and `MOVING_AVERAGE_FIFO_PERIOD`, `wallet` only, `de-DE`, `de.anlage_so`, the § 23 and § 22 Nr. 3 Freigrenzen, sources) | `crates/tax-countries/src/lib.rs` (`GERMANY`) | `germany_is_a_preview_eur_country_with_wallet_fifo_and_its_tables` |
| DE-HOLD-CAL, DE-HOLD-LEAP, DE-TZ | engine `holding: more_than_one_year` in Europe/Berlin; a fragment is taxable while not long-term | vectors de-02 … de-06 |
| DE-POOL-WALLET, DE-METHOD-FIFO, DE-METHOD-AVG, DE-TRANSFER | engine wallet pass and `moving_average_fifo_period` | de-07, de-17, de-18, de-19 |
| DE-FEE-NETWORK, DE-FEE-TRANSFER, DE-FEE-SPLIT | engine `fee_units: disposal_before_sale`; the journal stores each fragment's Werbungskosten (`expenses_exact`) | de-18, de-21; `a_fragment_with_expenses_keeps_them_in_the_legacy_shape` |
| DE-SWAP-VALUES, DE-SWAP-FALLBACK, DE-HOLD-RESTART | engine `swap_values: received_given` | de-13, de-14 |
| DE-GIFT-IN | engine `gifts_received: quarantine` (held `acquisition_valuation_unsupported`, `valuation_mode: donor_basis`; the vectors' `gift_predecessor_data_required`) | de-26 |
| DE-INC-AIRDROP, DE-HARDFORK | engine `refused_receipts`: held `acquisition_valuation_unsupported` with code `airdrop_classification_required` or `hardfork_cost_split_unsupported` | de-23, de-25; `a_refused_receipt_is_held_with_the_countrys_code_before_its_price` |
| DE-RAIL | `rail_decision_required`: `transfers pair` of a BTC/LBTC change without `--policy` fails with `rail_policy_required`; a native transition or a reviewed rail pair without its own policy is held `rail_policy_required` | de-24 (not the engine's): `rail_pairs_need_an_explicit_policy_where_the_country_requires_one`, `a_rail_pair_without_a_decided_policy_is_held_where_the_country_requires_one` |
| DE-FG-23, DE-FG-23-2023/2024, DE-LOSS-23, DE-FG-22-3, DE-INC-MINING | `crates/reports/src/de_tax.rs` (`years`, `section`) with `tables::DE_P23_FREIGRENZE`, `DE_P22_3_FREIGRENZE`; a year without a table is `unknown`, never the neighbouring year's | de-08 … de-12, de-15, de-16, de-22 |
| Report | `crates/reports/src/report_design/de.rs`, strings `DE_DE` in `report_design/strings.rs` | `anlage_so_rows_carry_the_lines_and_verdicts_in_german`, `tests/test_rust_de_tax_country.py` |

All 26 vectors run in `crates/reports/tests/tax_country_vectors.rs`: 25
through the engine under the registry's rules and `de_tax` (the full
expected document: fragments, Werbungskosten, income sections, years,
holdings, or the refusal code), and de-24 by the custody and transfers
tests named above, because the rail decision is made before the engine
sees a pair.

### The report

`reports export-pdf|export-xlsx|export-csv` of a German book add three
sections after Income, in German (`Januar`, `1.234,56`, `15.03.2025`):

- **Deutschland: Anlage SO** — table `anlage_so` per tax year: lines of the
  Kryptowerte block (2024 `41–47, 54–55`, 2025 `43/45–49, 56–57`, `—` for a
  year not on file), taxable Veräußerungspreis, Anschaffungskosten,
  Werbungskosten and Gewinn/Verlust, the Freigrenze and the verdict
  (`steuerpflichtig`, `unter der Freigrenze (indikativ)`, `Verlust (nur mit
  § 23-Gewinnen verrechenbar)`, `keine`), the taxable amount, gains outside
  the period and the loss line; table `anlage_so_leistungen`: § 22 Nr. 3
  total, its Freigrenze and verdict, and the block rewards apart. The
  Freigrenze caveat (other private disposals count) follows.
- **Deutschland: Aufstellung der Veräußerungen** — table `p23_disposals`,
  one row per consumed lot (Rn. 102): disposal and acquisition dates
  (Berlin), kind (fee units as `Gebühr`, before their sale), wallet, asset,
  quantity, `≤ 1 Jahr`/`> 1 Jahr`, taxable, Veräußerungspreis,
  Anschaffungskosten, Werbungskosten, gain and transaction.
- **Deutschland: Einkünfte und Bestände** — table `income_sections` (each
  receipt with its section, `§ 22 Nr. 3` or `Blockerstellung – Einordnung
  erforderlich`) and `year_end_holdings` (each wallet's quantity of each
  asset at the end of 31 December of every year with figures and the year
  before, Rn. 104).

The Method section states the holding rule, the per-wallet method, the fee
treatment, how many rows were priced at which granularity (Rn. 90, 91) and
what stays unbooked. The preview notice is on every page, the XLSX summary
and the CSV summary and manifest.

### Using it

CLI: `kassiber profiles create --tax-country de <label>` (EUR, FIFO,
`wallet`; `--gains-algorithm MOVING_AVERAGE_FIFO_PERIOD` for the average
method; LIFO, HIFO, LOFO and the global pool are refused), import or sync
the wallets, pair moves between own wallets (`transfers pair`; a BTC/LBTC
rail change needs `--policy carrying-value` or `--policy taxable`),
`journals process`, then the report exports. The desktop's country picker
lists Germany with the Preview badge from the registry
(`ui-tauri/src/lib/taxCountries.generated.ts`).

### Differences from the specification

- **Fiat commissions.** A Kassiber transaction has one fiat value and a fee
  in the asset; an exchange's euro commission is netted into the sale's
  value (and a purchase's into its cost). The report's Veräußerungspreis is
  therefore net of a euro commission, which then does not appear as
  Werbungskosten; the gain is the same. The runner restates the expected
  figures of de-01 and de-07 that way (`net_fiat_commissions`). Network fee
  units are Werbungskosten as specified.
- **Airdrops.** Kassiber has one `airdrop` kind and no reviewed
  `airdrop_service` classification, so every airdrop is held
  (`airdrop_classification_required`) until such a classification exists.
- **Received gifts** are held with the engine's shared reason
  (`acquisition_valuation_unsupported`, `valuation_mode: donor_basis`)
  rather than the code `gift_predecessor_data_required`, which the vector
  runner maps.
- **Holdings** at the year ends are quantities per wallet; their cost at
  that date is not stated.
- **Table ids** are `anlage_so`, `anlage_so_leistungen`, `p23_disposals`,
  `income_sections` and `year_end_holdings`: the shared report already has
  `income`, `disposals` and `holdings` sheets.
- **Price source per row** is not a column; the Method section counts the
  rows per pricing granularity instead.

## Open questions for a German tax adviser

1. **Q1** Anlage SO 2025: does the crypto block start at line 43 or 45 (the
   ELSTER help names both), and how should many disposals be entered:
   one aggregated entry with an attached list, or one entry per token?
2. **Q2** Network fees on own-wallet transfers: a § 23 disposal of the fee
   units at market value (this spec), a non-deductible cost without a
   disposal, or nothing? The BMF letter is silent.
3. **Q3** Lightning routing fees: § 22 Nr. 3 income (this spec) or something
   else? Is running a routing node block-creation-like (Rn. 39)?
4. **Q4** Rn. 62 lets the method change per wallet once that wallet's token
   holding is empty. Is a single book-wide method for the whole history
   acceptable, and must a later method change leave filed years untouched?
5. **Q5** Airdrops received without a service (Rn. 74): acquisition date and
   cost for a later § 23 disposal?
6. **Q6** Rn. 91 consistency: is mixing exact provider prices with a daily
   fallback "gleichmäßig" if each row records its granularity, or must one
   method apply to the whole year?
7. **Q7** BTC ↔ L-BTC: same Wirtschaftsgut (carrying value) or Tausch?
8. **Q8** Transfers under FIFO: does a moved lot keep its original
   acquisition date for the destination wallet's FIFO order (this spec), or
   does it queue at the transfer date?
9. **Q9** The order within a transaction: fee units before the principal
   (this spec, as the engine does) or after?
10. **Q10** Euro trading commissions: is listing the sale at its net
    proceeds (commission deducted, not shown as Werbungskosten) acceptable
    for Anlage SO and the Rn. 102 list, or must the commission be shown
    separately?
11. **Q11** Holdings at 31 December (Rn. 104): are quantities per wallet
    enough, or must the list state each holding's acquisition cost?
12. **Q12** Should a received gift's donor data (date and cost per lot) be
    entered so the gift can be booked with carried-over basis, rather than
    held?

## Fonts and language

German needs ä, ö, ü, ß, §, € and the en dash; DM Sans has all of them
(checked against `crates/pdf/fonts/` with `fc-query`), so no font is
missing. The report needs a `de-DE` string table distinct from today's
Austrian German (Jänner → Januar; Kennzahlen → Anlage SO lines; Austrian
terms such as Altvermögen do not apply).
