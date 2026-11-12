# Ireland (`ie`): crypto tax rules and report specification

**Status:** implemented in the Rust backend as a **preview** country
(2026-11-07). No Irish tax professional has reviewed it; every Irish report
and screen carries the preview notice until one does. Sources were read on
2026-11-07. Python stays `generic`/`at` only and refuses Irish books
(`tests/test_python_rust_only_countries.py`). Nothing here is tax advice.

Test vectors: [tax-country-ie-vectors.json](tax-country-ie-vectors.json)
(18 cases, same format as the
[Spanish vectors](tax-country-es.md#test-vectors)).
[`scripts/tax-country-vectors.py`](../../scripts/tax-country-vectors.py)
recomputes them and runs RP2's IE plugin on the same inputs.

## Summary

| Item | Ireland |
| --- | --- |
| Tax | Capital Gains Tax (CGT) for disposals outside a trade. Revenue: "no special tax rules" for crypto |
| Tax year | Calendar year, Europe/Dublin civil date |
| Rate | **33 %** for disposals 6 Dec 2012 – 6 Oct 2026; **31 %** from **7 Oct 2026** (Budget 2027, TDM 02-03-01A) |
| Exemption | €1,270 per individual per year, after losses, non-transferable (s.601) |
| Currency | EUR, required |
| Cost basis | **FIFO** (s.580(1), applied to fungible assets by s.580(4)); one pool per person across all wallets |
| Four-week rule | s.581 applies to shares and securities; not applied to crypto by default, flagged |
| Crypto-to-crypto | Disposal; consideration = market value of the coins received (Kassiber convention, IE-Q2) |
| Income | No Revenue guidance for staking, lending or mining. Kassiber books them as Case IV miscellaneous income at market value (IE-Q4); airdrops and forks fail closed |
| Losses | Set against the year's gains, then carried forward without limit; no carry-back (s.31) |
| Payment | Disposals 1 Jan – 30 Nov: pay by 15 Dec; 1 – 31 Dec: pay by 31 Jan. Return by 31 Oct of the next year |
| Form | Form 11 (chargeable persons, ROS) or **Form CG1**; the same CGT lines 1(k), 10–25 |
| Report language | English |

## Sources

All retrieved 2026-11-07. The Irish Statute Book refused the fetcher (HTTP
403), so statute text was read through Revenue's Notes for Guidance.

- **[IE-S1]** Revenue, Tax and Duty Manual Part 02-01-03, *Taxation of
  Crypto-Asset Transactions* (last reviewed January 2026):
  <https://www.revenue.ie/en/tax-professionals/tdm/income-tax-capital-gains-tax-corporation-tax/part-02/02-01-03.pdf>.
  Covers disposals, losses, the €1,270 exemption, examples 1–4 (John, Mary,
  Dave, payment dates), valuation ("a reasonable effort"), CAT, records and
  the remittance basis.
- **[IE-S2]** Revenue, *Notes for Guidance – Taxes Consolidation Act 1997*,
  Finance Act 2025 edition, Part 19 (December 2025):
  <https://www.revenue.ie/en/tax-professionals/documents/notes-for-guidance/tca/part19.pdf>.
  Sections used: 538 (loss, destruction, negligible value), 547 (market
  value), 552 (allowable expenditure; foreign
  currency at the date incurred, (1A)), 580 (FIFO; (4) extends it to assets
  dealt in without identifying the particular item), 581 (four weeks;
  shares and securities), 601 (annual exemption; (3) highest rate first).
- **[IE-S3]** Revenue, TDM Part 02-03-01A, *Capital Gains Tax – rate of
  charge (S.28)*, last updated October 2026:
  <https://www.revenue.ie/en/tax-professionals/tdm/income-tax-capital-gains-tax-corporation-tax/part-02/02-03-01a.pdf>.
- **[IE-S4]** Revenue eBrief No. 142/26, 6 October 2026 (31 % for
  disposals from 7 October 2026; development land stays 33 %):
  <https://www.revenue.ie/en/tax-professionals/ebrief/2026/no-1422026.aspx>.
- **[IE-S5]** Revenue, *How to calculate CGT*:
  <https://www.revenue.ie/en/gains-gifts-and-inheritance/transfering-an-asset/how-to-calculate-cgt.aspx>.
- **[IE-S6]** Revenue, *When and how do you pay and file CGT?*:
  <https://www.revenue.ie/en/gains-gifts-and-inheritance/transfering-an-asset/when-and-how-do-you-pay-and-file-cgt.aspx>.
- **[IE-S7]** Form CG1 2025 and its helpsheet:
  <https://www.revenue.ie/en/gains-gifts-and-inheritance/documents/formcg1.pdf>,
  <https://www.revenue.ie/en/gains-gifts-and-inheritance/documents/form-cg1-helpsheet.pdf>.
- **[IE-S8]** Revenue, *Cryptocurrencies and crypto-assets*:
  <https://www.revenue.ie/en/companies-and-charities/financial-services/cryptocurrencies/index.aspx>.

## Rules

**IE-R1 Disposals.** The sale, transfer or redemption of crypto-assets is
most likely a CGT disposal unless a trade of dealing is carried on
[IE-S1 §1]. Paying for goods is a disposal (example 3, Dave) [IE-S1].
Crypto-to-crypto is a disposal; its consideration is the market value of
what is received (IE-Q2). A gift is a disposal at market value, because a
bargain not at arm's length uses market value (s.547) [IE-S2]. Gifts
between spouses and civil partners are no-gain/no-loss and out of scope
(IE-Q6). A move between one's own wallets is not a disposal. Trading
(Case I) is out of scope: Kassiber does not judge "trade" (IE-Q5).

**IE-R2 Gain.** Consideration minus allowable expenditure: acquisition
cost with incidental acquisition costs, enhancement expenditure, and
incidental disposal costs (s.552(1)) [IE-S2, IE-S5]. Kassiber adds EUR
purchase fees to the lot, deducts EUR sale fees from the gain, and reports
consideration *before* costs. Form CG1 line 1 asks for consideration before
incidental costs [IE-S7].

**IE-R3 FIFO.** Shares of the same class are identified first-in,
first-out (s.580(1)). The rule also applies "to such assets as can be dealt
in without identifying individual items" (s.580(4)) [IE-S2]. Fungible
crypto fits that description. RP2's IE plugin chose FIFO too. The pool is
the person's: all wallets and exchanges, per coin type. Kassiber: method
`fifo` only, pool scope `global` only.

**IE-R4 Four-week rule.** s.581 sets FIFO aside for shares or securities
acquired within four weeks before a disposal. It also restricts a loss when
the same shares are reacquired within four weeks after (s.581(3))
[IE-S2]. s.580(4) extends s.580 to fungible assets, but s.581 names
shares and securities only. Kassiber does **not** apply s.581. It flags
each loss disposal with a same-asset acquisition within 28 days before or
after (`ie_s581_four_week_review`, vector ie-15) (IE-Q1).

**IE-R5 Rate.** 33 % for disposals from 6 December 2012 to 6 October
2026; 31 % for disposals on or after 7 October 2026 [IE-S3, IE-S4]. The
rate follows the disposal date in Irish time: 2026-10-06T23:30Z is
7 October in Dublin, so 31 % (vector ie-11). The 31 % was announced in
Budget 2027 and already appears in Revenue's manual. Kassiber flags 31 %
rows (`ie_rate_31_pending_finance_act`) until the Finance Act 2026 is
confirmed (IE-Q3). Rates before 2012-12-06 are not modelled, and such
years fail closed.

**IE-R6 Annual exemption.** The first €1,270 of an individual's net
chargeable gains in a year is exempt. Losses are deducted first, and the
exemption cannot be used to preserve losses for carry-forward (s.601(1);
example 1) [IE-S1, IE-S2]. When gains are taxed at more than one rate,
the exemption goes against the highest-rate gains first (s.601(3)). It is
per individual and not transferable [IE-S7]. Kassiber applies the full
€1,270 against crypto gains by default. A book setting
(`ie_exemption_available`, 0–1,270) lets a user who uses part of the
exemption on other assets reduce it.

**IE-R7 Losses.** Losses of the year are set against gains of the year.
An excess carries forward to later years without time limit and is used
before the exemption [IE-S1, example 2]. There is no carry-back. Kassiber
carries crypto losses forward between years of the book. A book setting
`losses_brought_forward_opening` (EUR) adds losses from before the book's
first year or from other assets (vector ie-02).

**IE-R8 Payment periods.** Disposals from 1 January to 30 November form
the *initial period*, with tax due by 15 December. December disposals form
the *later period*, with tax due by 31 January of the next year. The return
is due by 31 October of the next year (ROS extends this) [IE-S1 §1.4,
IE-S6, IE-S7]. Kassiber computes the initial-period payment as if the year
ended on 30 November: initial-period gains less initial-period losses,
losses brought forward and the exemption. The later-period payment is the
year's total tax less that amount. A negative later amount is an
overpayment of the 15 December payment, reported as
`overpaid_initial` (vector ie-12). This mirrors s.958 TCA's split (IE-Q7).

**IE-R9 Income.** Revenue publishes no treatment for staking, lending or
mining receipts outside a trade [IE-S1, IE-S8]. Kassiber books `staking`,
`lending_interest`, `interest` and non-trade `mining` as **Case IV**
miscellaneous income at EUR market value when received. The coins take
that value as their CGT base cost (IE-Q4). `airdrop`, `hardfork`,
`routing_income` and unlabelled `income` fail closed
(`ie_<kind>_requires_review`). `wages` are PAYE emoluments valued in euro
at payment [IE-S1 §3], outside Kassiber; the coins enter at that value as
basis, as today.

**IE-R10 Fees paid in crypto.** A fee paid in the coin is a disposal of the
fee units at market value. On a self-transfer the fee is not incidental to
any disposal, so nothing is deducted (vector ie-18). On a sale the fee
units go out with the sold units and the consideration is the gross price,
which is the same total as deducting the fee's value as a disposal cost.

**IE-R11 Currency and valuation.** EUR only. Expenditure in a foreign
currency converts at the rate on the date incurred (s.552(1A)) [IE-S2].
Crypto has no single exchange rate, and "a reasonable effort should be
made to use an appropriate valuation" [IE-S1 §4]. Kassiber's rates cache
and reviewed pricing serve, and the pricing source is shown on every row.

**IE-R12 Dates.** Calendar year, Europe/Dublin civil date, for the year,
the payment period and the rate (IE-R5, IE-R8).

**IE-R13 Lost coins.** The entire loss, destruction or extinction of an
asset is a disposal, and relief may be claimed (s.538(1)). An asset that
still exists but has negligible value needs a claim the inspector accepts
(s.538(2)) [IE-S2]. Whether lost keys or a failed platform are an
"entire loss" is a question of fact. Kassiber does not decide it: `lost`,
`stolen` and `theft` outflows fail closed
(`ie_lost_requires_negligible_value_claim`, vector ie-17) until a reviewed
marker says which subsection applies.

**IE-R14 Remittance basis.** A non-domiciled resident is taxed on foreign
gains only when remitted. Revenue holds that crypto "on the cloud" is not
situated outside the State, so the remittance basis rarely applies [IE-S1
§8]. Kassiber assumes the ordinary basis and says so in Method.

**IE-R15 Rounding.** The engine computes exactly. Rows and totals show
euro and cent, half-up. Tax per year is `round_half_up(Σ net × rate, 2)`
(vector ie-10). Form CG1 boxes are whole euros (".00" printed). The
"Form lines" section drops the cents. That is a Kassiber convention
(IE-Q8).

## Kassiber mapping

| Kassiber row | IE event |
| --- | --- |
| inbound purchase | FIFO lot, EUR value + EUR fees |
| outbound sale | disposal; consideration = gross EUR price; EUR fees are disposal costs |
| cross-asset taxable pair | disposal; consideration = market value of the in leg, which is also its base cost |
| outbound payment (`spend`) | disposal; consideration = goods' value if reviewed, else the coins' market value |
| reviewed `gift` | disposal at market value, CG1 connected-party flag (`ie_gift_connected_party`) |
| self-transfer with fee | disposal of the fee units |
| `staking`, `lending_interest`, `interest`, `mining` | Case IV income + lot at that value |
| `lost`, `stolen`, `airdrop`, `hardfork`, `routing_income`, `income` | quarantine |
| BTC↔LBTC carrying-value pair | carry basis, no disposal (IE-Q9) |

As for Spain, reviewed gifts need a country hook in place of today's
`non_sale_disposal_kind` quarantine.

## Report

[Shared report design](report-design.md), Irish country layer:

- **Method:** FIFO (s.580); EUR; Europe/Dublin; rate table; exemption used;
  losses brought forward (opening value and source); preview notice;
  "Kassiber does not file the return".
- **Disposals:** shared columns plus `consideration`, `allowable_cost`,
  `period` (initial/later), `rate`.
- **Income:** shared columns plus `schedule` (Case IV).
- **CGT computation** (after Income), one block per year:

| Line | Meaning |
| --- | --- |
| 1(k) | Other assets: number of crypto disposals and aggregate consideration |
| 10 | Chargeable gains (sum of gains) |
| 11 | Losses in the year |
| 14 | Gains net of current-year losses |
| 17 | Losses brought forward used |
| 18 | Personal exemption used |
| 19 | Net chargeable gain |
| 21, 22, 23 | Current-year losses carried forward, unused prior losses carried forward, total |
| 24 (a/…) | Initial period net gain by rate (33 %, and 31 % for 2026 onwards) |
| 25 (a/…) | Later period net gain by rate |
| Tax | Total CGT on crypto; payment due 15 Dec (initial), 31 Jan (later), and any initial-period overpayment |

  Lines 2–9, 12–13, 15–16, 20 and 26 (reliefs, foreign life policies,
  connected-party details, double taxation) are listed as "not modelled".
  The return is per individual. Line 12 needs the gift flags.
- **Case IV income:** total for the Form 11 / myAccount "other income"
  entry, with the caveat of IE-Q4.
- **Issues:** quarantines and flags (`ie_s581_four_week_review`,
  `ie_rate_31_pending_finance_act`, `ie_gift_connected_party`).

English labels. Numbers `1,234.56`; dates ISO in the PDF (the shared
English convention), even though Revenue forms use DD/MM/YYYY.

## Registry defaults

| Field | Value |
| --- | --- |
| `tax_country` | `ie` |
| `preview` | `true` |
| `fiat_currency` | `EUR` (required) |
| `accounting_methods` | `fifo` |
| `default_accounting_method` | `fifo` |
| `cost_basis_pool_scopes` | `global` |
| `long_term_days` | none (`sys.maxsize`, as RP2's IE plugin) |
| `timezone` | `Europe/Dublin` |
| `tax_year` | calendar |
| `generation_language` | `en` (RP2 uses `en_IE`; Kassiber's English formats suffice) |
| `report_generators` | shared full/summary reports + `ie_cgt` |
| country settings | `losses_brought_forward_opening` (EUR, default 0), `ie_exemption_available` (EUR, default 1,270) |
| rates | `[(..2012-12-05, unsupported), (2012-12-06..2026-10-06, 0.33), (2026-10-07.., 0.31)]`, keyed by Dublin date |
| income categories | per IE-R9 |
| sources | IE-S1 … IE-S8 with retrieval date 2026-11-07 |

## Test vectors

Step by step (EUR; "net" = net chargeable gain, line 19):

- **ie-01** (Revenue example 1, John). Bought 4,500 (2023), sold 5,000
  (2024): gain 500. Exemption limited to 500 (the other 770 is lost for
  crypto). Net 0, tax 0.
- **ie-02** (example 2, Mary). Losses brought forward 2,500 (book setting).
  Gain 15,000 − 4,500 = 10,500; less 2,500 = 8,000; less 1,270 = **6,730**;
  tax 33 % = **2,220.90**, due 15 Dec 2024.
- **ie-03** (IE-R3). Lots 20,000 (WalletA, 2022) and 50,000 (WalletB,
  2023). Selling 1 BTC from WalletB takes the 2022 lot: 60,000 − 20,000 =
  40,000; net 38,730; tax **12,780.90**.
- **ie-04** (s.580 Notes for Guidance example, without indexation). Lots
  0.2 BTC 1,000 and 0.3 BTC 1,800. Sale of 0.3 for 3,000: cost 1,000 +
  1,800 × 0.1/0.3 = 1,600; gain 1,400; net 130; tax **42.90**.
- **ie-05** (IE-R2). Cost 30,000 + 150; sale 40,000 with 200 costs. Gain
  40,000 − 30,350 = 9,650; net 8,380; tax **2,765.40**. Line 1(k)
  consideration 40,000 (before costs).
- **ie-06** (IE-R1). 1 BTC (cost 25,000) swapped for 15 ETH worth 61,000
  (the BTC was worth 60,800). Consideration 61,000; gain 36,000; ETH base
  cost 61,000; net 34,730; tax **11,460.90**.
- **ie-07** (IE-R7). 2024: sales gain 3,000 and lose 5,000. Line 14 is 0,
  the exemption is unused, and 2,000 carries forward (line 21). 2025: gain
  4,000, less 2,000 b/f (line 17), less 1,270 = **730**; tax **240.90**.
- **ie-08** (s.601 note). Gain 1,270 and loss 270 in one year: net gains
  1,000 use 1,000 of the exemption. Nothing is taxed and nothing carries
  forward.
- **ie-09** (IE-R6). Gain exactly 1,270.00: fully exempt.
- **ie-10** (IE-R15). Gain 1,272.50: net 2.50; tax 0.825 → **0.83**
  (half-up). Form line 19 shows 2.
- **ie-11** (IE-R5/R6). 2026 gains: 2,000 on 10 March (33 %) and 3,000 at
  2026-10-06T23:30Z, which is 7 October in Dublin (31 %). The exemption goes
  to the 33 % gains first (s.601(3)): 33 % net 730 → 240.90; 31 % net 3,000
  → 930.00; tax **1,170.90**, all in the initial period (lines 24 at 33 %
  and 31 %).
- **ie-12** (IE-R8). June gain 10,000; December loss 4,000. Initial-period
  payment as if the year ended on 30 November: (10,000 − 1,270) × 33 % =
  **2,880.90** due 15 Dec 2025. Year: (10,000 − 4,000 − 1,270) = 4,730 →
  **1,560.90**. Later-period payment 0, initial overpaid **1,320.00**.
  Line 24a 4,730.
- **ie-13** (IE-R8). Only a December gain of 5,000: net 3,730, tax
  **1,230.90** due 31 Jan 2026 (line 25a).
- **ie-14** (IE-R9). Lending interest 0.01 BTC at 400 → Case IV 400, base
  cost 400. Spending the 0.01 BTC on goods worth 500: gain 100, covered by
  the exemption.
- **ie-15** (IE-R4). 1 BTC at 60,000 sold for 50,000 (loss 10,000) and
  bought back 19 days later for 51,000. The loss is allowed and carried
  forward (line 21 10,000), with the s.581 review flag.
- **ie-16** (IE-R1). Gift of 0.5 BTC worth 30,000 (cost 5,000): gain
  25,000; net 23,730; tax **7,830.90**. Connected-party flag.
- **ie-17** (IE-R13). Lost 0.1 BTC → quarantine, no loss.
- **ie-18** (IE-R10). Network fee 0.0001 BTC worth 6.00 on a self-transfer
  (cost 3.00): gain 3.00, inside the exemption.

## RP2 comparison

RP2's IE plugin (pinned fork) is FIFO-only, EUR, `en_IE`, with an infinite
long-term period. Its `tax_report_ie` generator writes one sheet per
transaction type (Capital Gains, Income, Staking, Interest, Mining,
Airdrops, Hard Forks, Gifts, Donations, Investment Expenses, Wages) with
the lot rows. It has **no** annual exemption, rate, payment period, loss
carry-forward or CG1 line. Its numbers on the vectors (`rp2.computed`,
explained in `rp2.differences`):

| Rule | RP2 | Spec | Vectors |
| --- | --- | --- | --- |
| Disposal costs in EUR (s.552) | ignored on sales | deducted | ie-05 (+200) |
| Crypto-to-crypto consideration | the out leg's own value | the received coins' value (IE-Q2) | ie-06 (−200) |
| Lost coins | full capital loss | quarantine | ie-17 |
| Interest/staking | a taxable event in the gain list | Case IV income, separate | ie-14 |
| Exemption, rates, periods, losses forward, form lines | absent | IE-R5…R8 | every year block |

All other disposal rows agree, including FIFO across wallets (ie-03) and
partial lots (ie-04). RP2 is a correct FIFO lot engine without the Irish
year computation.

## Fail closed

- `ie_lost_requires_negligible_value_claim` (IE-R13).
- `ie_<kind>_requires_review` for airdrops, forks, routing income and
  unlabelled income (IE-R9).
- `ie_rate_before_2012_12_06_not_modelled`: a year with older disposals
  refuses its CGT block (IE-R5).
- A non-EUR book refuses. Unpriced rows quarantine as today.
- Gifts to a spouse or civil partner (no gain/no loss) need a reviewed
  marker. Until one exists, a gift is a market-value disposal with the
  connected-party flag, and the report says so.

## Implementation

What the Rust backend does, and where it differs from the rules above.

- **Registry** (`crates/tax-countries`, entry `ie`): EUR required, FIFO
  only, the `global` pool scope only, no holding period, Europe/Dublin, the
  calendar year, the generators `open_positions` and `ie.cgt`, the
  `ie.cgt_rate` and `ie.annual_exemption` tables, and the sources above.
  Engine rules: fee units are disposals at their value with a sale's fee
  units staying in the sale (IE-R10), a taxable crypto-to-crypto exchange is
  valued at the received coins' value for both legs (IE-R1, IE-Q2), and an
  outbound gift is a disposal at its value (IE-R1). Donations, gifts to a
  spouse and lost or stolen coins stay quarantined.
- **Income** (IE-R9): `staking`, `interest`, `lending_interest`, `mining` and
  `mining_reward` are Case IV income at their value. The engine holds
  `airdrop`, `hardfork`/`hard_fork`, `routing_income` and `income` receipts
  before pricing them, with the codes `ie_<kind>_requires_review`
  (`refused_receipts`), and they taint the asset's later disposals.
- **Lost coins** (IE-R13): quarantined; whatever the engine held them for
  (an unpriced loss is held for its price first), the layer reports
  `ie_lost_requires_negligible_value_claim`.
- **Country layer** (`crates/reports/src/ie_tax.rs`, sections in
  `report_design/ie.rs`): rows rounded half-up to cents with the gain their
  difference, the payment period and rate by Dublin date, the year blocks,
  allocation of losses, losses brought forward and the exemption to the
  highest rate first and the initial period before the later one, tax
  half-up to cents, and the flags. A year after the last table date (2027
  on) reads "unknown" and shows no tax; a year with a disposal before
  6 December 2012 is refused with `ie_rate_before_2012_12_06_not_modelled`.
  Form lines drop the cents.
- **Fiat disposal costs:** a Kassiber book stores no fiat commission apart
  from a transaction's value, so a sale's commission is netted into the
  value. In a book, consideration (line 1(k)) is after the commission and
  the allowable cost excludes it, with the same gain. The vector runner
  nets ie-05's commission the same way and adds it back as a disposal cost.
- **Book settings not stored yet:** `losses_brought_forward_opening` (taken
  as 0) and `ie_exemption_available` (taken as the full €1,270). The layer
  accepts both (`ie_tax::Settings`) and the vectors set the first.
- **s.581 flag:** the vectors count purchases (`buy` events); a book counts
  every acquisition that is not income, swap in-legs included, so it flags
  at least as often.
- **Disposal kinds** in a book come from the journal entry and the
  transaction kind: fees are `network_fee`, `gift`, `spend`/`payment` and
  `swap` kinds are labelled as such, and every other disposal is a `sale`.
  Only the s.581 flag depends on it.
- **Calendars:** the report readers (desktop years, tax summary, period
  presets) date an Irish book in Europe/Dublin.

Every vector passes through the engine (`ledger` operation under the
registry's rules) and the layer: `crates/reports/tests/tax_country_vectors.rs`
with `country_vectors/gb_ie.rs`, which also checks RP2's computed rows lot
by lot and allows only the four recorded differences (ie-05, ie-06, ie-14,
ie-17). `tests/test_rust_gb_ie_tax_countries.py` builds a book with the CLI,
processes its journal and checks the CG1 lines, payments and rates in the
CSV, XLSX and PDF exports (two PDFs of one book are byte-identical).

### Using it

`kassiber profiles create --workspace Main --tax-country ie Book` creates an
Irish book (EUR, FIFO, global pool); the desktop's book picker lists
Ireland with a Preview badge. `kassiber journals process`, then
`kassiber reports export-pdf|export-xlsx|export-csv` write the full report
with the Irish sections; the CSV bundle's Irish files are `ie_cg1.csv`,
`ie_rate_lines.csv`, `ie_payments.csv`, `ie_disposals.csv`, `ie_income.csv`
and `ie_review.csv`.

## Open questions for an Irish tax adviser

- **IE-Q1** Does s.581 (four weeks) apply to crypto via s.580(4), or only
  to shares and securities? Kassiber only flags.
- **IE-Q2** Crypto-to-crypto: is the consideration the market value of the
  coins received or of those given? Kassiber uses received, the same value
  as the new coins' cost. What applies when the two differ materially?
- **IE-Q3** The 31 % rate from 7 October 2026: is it enacted (Finance Act
  2026), and how should losses be allocated between 33 % and 31 % gains?
  Kassiber sets losses and the exemption against the highest rate first,
  which s.601(3) states only for the exemption.
- **IE-Q4** Staking, lending interest and non-trade mining: Case IV income
  at receipt (Kassiber's default), or CGT-only with nil or market-value
  base cost? Revenue's manual is silent.
- **IE-Q5** Trading: at what activity level should Kassiber warn that
  Case I may apply (TDM 02-02-06 badges of trade)?
- **IE-Q6** Gifts between spouses or civil partners (s.1028(5)) and
  disposals to connected persons (s.549 loss restriction): what markers
  are needed?
- **IE-Q7** The initial-period computation "as if the year ended on
  30 November" (s.958): is this how Revenue expects December losses to be
  treated (as an overpayment refunded on assessment)?
- **IE-Q8** Rounding to whole euros on Form CG1: truncate, or round half-up?
- **IE-Q9** BTC↔LBTC pegs and Lightning: one asset for s.580 or a disposal
  at each peg?
- **IE-Q10** Network fees on self-transfers: are they disposals Revenue
  expects to see, or is a de-minimis practice accepted?
- **IE-Q11** Consideration on line 1(k): Kassiber books net a sale's fiat
  commission into its value. Is the net consideration acceptable when the
  commission is not recorded apart?
- **IE-Q12** s.581 review flag: which acquisitions should count as a
  repurchase (purchases only, or swaps into the asset too)?

## Fonts and language

English only. The bundled fonts cover every character the report needs,
including á é í ó ú for Irish names and €; checked on 2026-11-07. No
font change.
