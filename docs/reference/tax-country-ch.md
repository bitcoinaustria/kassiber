# Switzerland (`ch`): tax country specification

**Status:** implemented in the Rust backend as a **preview** country
(2026-11-07; [implementation](#implementation)). Not reviewed by a Swiss
tax adviser (Steuerberater/Treuhänder). Every Swiss report and screen
carries the preview notice until one does. Nothing here is tax advice.

Switzerland is different from the other countries: a private investor's
capital gains are **tax-free** and losses are not deductible. What a Swiss
book must deliver is the **income** from crypto (staking, lending, airdrops,
mining), the **wealth** at 31 December for the cantonal wealth tax, and an
honest check of whether the investor might be treated as a **professional
trader** (gewerbsmässiger Wertschriftenhandel), in which case gains become
taxable and Kassiber's figures stop being the answer.

The worked cases are in [tax-country-ch-vectors.json](tax-country-ch-vectors.json);
their expected values come from the reference model in
[tax-country-reference/](tax-country-reference/) (`ch.py`); regenerate with
`python3 -I docs/reference/tax-country-reference/build.py . ch`.

Scope: an individual resident in Switzerland holding crypto as private
assets (Privatvermögen). Business assets, professional trading accounting,
legal entities and canton-specific tax rates are out of scope.

## Sources

All retrieved 2026-11-07.

| Ref | Source |
| --- | --- |
| DBG | Bundesgesetz über die direkte Bundessteuer, SR 642.11 ([fedlex](https://www.fedlex.admin.ch/eli/cc/1991/1184_1184_1184/de)): Art. 16 (Abs. 1 general clause, Abs. 3 private capital gains tax-free), Art. 17, Art. 18, Art. 20 (income from movable assets), Art. 24 lit. a (gifts) and lit. j (small lottery gains) |
| StHG | Steuerharmonisierungsgesetz, SR 642.14 ([fedlex](https://www.fedlex.admin.ch/eli/cc/1991/1256_1256_1256/de)): Art. 7 Abs. 4 lit. b, Art. 13, Art. 14 (wealth at market value) |
| ESTV AP | ESTV, Arbeitspapier „Kryptowährungen und Initial Coin/Token Offerings (ICOs/ITOs) als Gegenstand der Vermögens-, Einkommens- und Gewinnsteuer, der Verrechnungssteuer und der Stempelabgaben“, 14. Dezember 2021 ([PDF](https://www.estv.admin.ch/dam/de/sd-web/HgzNhzz7q7YD/dbst-arbeitspapier-kryptowaehrungen-de.pdf), linked from the [ESTV crypto page](https://www.estv.admin.ch/de/kryptowaehrungen-besteuerung)) |
| ESTV Dossier | ESTV Dossier Steuerinformationen B „Kryptowährung“, Oktober 2023 ([PDF](https://www.estv2.admin.ch/stp/ds/b-kryptowaehrung-de.pdf)): staking, validators, airdrops, wealth tax |
| KS 36 | ESTV Kreisschreiben Nr. 36 „Gewerbsmässiger Wertschriftenhandel“, 27. Juli 2012 ([PDF](https://www.estv2.admin.ch/dvs/kreisschreiben/dbst-ks-2012-1-036-d-de.pdf)), applied to crypto by the ESTV AP and Dossier |
| Kursliste | ESTV Kursliste / ICTax: official year-end tax values for the most common cryptocurrencies ([ICTax](https://www.ictax.admin.ch/extern/de.html)), for completed calendar years only |

## Rules

### Capital gains

- **CH-CG-PRIVATE** — Gains on movable private assets are tax-free and losses
  are not deductible (Art. 16 Abs. 3 DBG, Art. 7 Abs. 4 lit. b StHG; ESTV AP
  2.2.2, Dossier 3.1). Buying and selling payment tokens is treated like
  foreign-currency transactions. Kassiber reports `taxable_capital_gains = 0`
  and lists realized gains only for information and for KS 36 criterion 3.
- Informational gains use global FIFO in CHF (default `fifo`); the method has
  no tax effect for a private investor.
- **CH-PROFESSIONAL** — If the activity goes beyond private asset management,
  gains are taxable self-employment income and booked losses are deductible
  (Art. 18 DBG; ESTV AP 2.2.2). Kassiber does not offer business-asset
  accounting: a book marked professional is refused
  (`professional_trading_unsupported`, vector ch-20).

### Professional-trader safe harbour (KS 36)

The ESTV always assumes private asset management if **all five** criteria are
met (KS 36 Ziff. 3); otherwise professional trading "cannot be excluded" and
the case is judged on all facts. Kassiber shows each criterion as `met`,
`not_met` or `not_assessable` and an overall verdict (`met`, `not_met`,
`inconclusive`). It never concludes that the user *is* a professional trader.

- **CH-KS36-C1** — Disposed assets were held at least **6 months**. Checked per
  consumed FIFO lot: `disposal_date ≥ acquisition_date + 6 calendar months`
  (month-end clamped: bought 31 August, 28 February counts). Any shorter lot
  fails the criterion (vectors ch-10, ch-11).
- **CH-KS36-C2** — Transaction volume (all purchase prices plus all sale
  proceeds of the calendar year; a swap counts both legs) is at most **five
  times** the securities-and-cash holdings at the start of the tax period.
  Kassiber only knows the crypto part, so it computes the limit from the
  crypto wealth at the previous 31 December (`basis: crypto_only`) and marks
  it indicative; with no start value it is `not_assessable` (vectors ch-12,
  ch-13).
- **CH-KS36-C3** — Capital gains are not needed to replace missing income:
  realized gains are **less than 50%** of the period's net income. Needs the
  net income, which the user may enter; otherwise `not_assessable` (vector
  ch-14).
- **CH-KS36-C4** — Not debt-financed, or taxable investment income exceeds the
  share of debt interest. User declaration only.
- **CH-KS36-C5** — Derivatives only to hedge own positions. User declaration
  only.

### Income

All income is valued in CHF at the time it is received (Zufluss: receipt or
acquisition of a firm claim, ESTV AP 2.2.2).

- **CH-INC-VALUE** — Value when received, converted to CHF; the coins then
  form part of the wealth.
- **CH-INC-MOVABLE** — Staking rewards of a delegator and lending interest are
  income from movable assets (Art. 20 Abs. 1 DBG; ESTV AP 2.2.2; Dossier
  4.2.1). Fees kept by a validator or custodian for managing the stake are
  mostly deductible asset-management costs; transaction costs are not
  (Dossier 4.2.1). Kassiber reports the rewards as recorded (net if the
  provider pays net) and names the treatment.
- **CH-INC-AIRDROP** — Airdrops are taxable at market value when allocated
  (ESTV AP 2.2.2; Dossier 11.2.1). The CHF 1,000 exemption for lottery-like
  promotional draws (Art. 24 lit. j DBG) needs four cumulative conditions,
  including an organiser seated in Switzerland; Kassiber cannot verify them,
  so it does not apply the exemption and notes it (vector ch-04).
- Lightning routing income: income under the general clause (Art. 16 Abs. 1
  DBG); if run like a business, self-employment income (Q3).
- **CH-INC-MINING** — Mining rewards are taxable income (Art. 16 Abs. 1 DBG);
  if the criteria of self-employment are met, self-employment income (ESTV AP
  2.2.2). Private small-scale mining is usually a hobby (Dossier 4.1.1).
  Kassiber reports mining in its own category with that note (vector ch-05).
- **CH-WAGES** — Salary paid in crypto is employment income reported by the
  employer on the salary certificate (Lohnausweis, ESTV AP 2.2.2). Kassiber
  lists it for information but keeps it out of its own income total (vector
  ch-17); the coins are wealth.
- **CH-GIFT** — A gift or inheritance received is not income (Art. 24 lit. a
  DBG); cantonal gift and inheritance taxes are outside Kassiber (vector
  ch-16).
- **CH-HARDFORK** — No ESTV text settles whether coins from a hard fork are
  income or a tax-free split of an existing asset. Fork rows fail closed
  (`hardfork_treatment_unsupported`, vector ch-19; Q2).
- Withholding tax (Verrechnungssteuer) does not apply to payment-token income
  (ESTV AP 2.2.3; Dossier 4.2.2.1.5), so all crypto income belongs in the
  "without withholding tax" column of the securities list.

### Wealth

- **CH-WEALTH** — Crypto is movable capital assets subject to the cantonal
  wealth tax, declared at market value at the end of the tax period
  (Art. 13, 14 StHG; ESTV AP 2.2.1). Kassiber reports holdings per wallet and
  asset at the end of 31 December, Europe/Zurich.
- **CH-WEALTH-ESTV** — For the most common cryptocurrencies the ESTV publishes
  official year-end tax values (Kursliste; Dossier 4.2.2.1.4: averages of
  several platforms). Kassiber makes no network request for them: the user
  enters the published value as a manual rate for 31 December tagged
  `estv_kursliste` (vector ch-06).
- **CH-WEALTH-FALLBACK** — For a coin without an ESTV value, the market value
  of a leading platform at year end may be used (ESTV AP footnote 3). Kassiber
  uses its cached closing quote for 31 December, labelled "market value, not
  the ESTV tax value" (vector ch-07). Only if no current value can be found
  does the ESTV allow the original CHF purchase price (ESTV AP 2.2.1); Kassiber
  never falls back to cost by itself. Without a value the wealth figure stays
  unknown and the report is incomplete (vector ch-08).
- L-BTC has no ESTV value. Kassiber values it at the BTC value and flags the
  row `lbtc_valued_as_btc` (vector ch-18; Q4).

### Rails, transfers, fees

- **CH-RAIL** — Because private gains are tax-free, a BTC ↔ L-BTC peg has no
  income-tax effect for a private investor. No explicit rail decision is
  required for `ch` (unlike `de` and `gb`); the book's existing
  `bitcoin_rail_carrying_value` setting only changes the informational gains
  and the KS 36 holding check.
- Own-wallet transfers are not events. Network and trading fees are
  transaction costs, not deductible for a private investor (ESTV AP 2.2.2).

### Tax year, time, currency, rounding

- **CH-TZ** — Calendar tax period (Steuerperiode) in Europe/Zurich. A receipt
  at 2024-12-31T23:30Z is 1 January 2025 00:30 in Zurich and belongs to 2025
  (vector ch-09).
- Book currency must be CHF.
- Kassiber computes exactly and rounds displayed CHF to two decimals.
  Cantonal forms round to whole francs at assessment; Kassiber does not.

## What the engine and registry need

| Need | Today | For `ch` |
| --- | --- | --- |
| Tax on gains | gains are the report's centre | gains are informational; the report centres on income and wealth |
| Year-end wealth | exit-tax FMV lookup (24 h rule) | wealth at 31 December per wallet from a manual `estv_kursliste` rate, else cached close; unknown stays unknown |
| Holding rule | `long_term_days` | KS 36 six-calendar-month indicator (not an exemption) |
| KS 36 | none | five criteria, user inputs for net income, financing, derivatives, optional start-of-year securities and cash |
| Income | one bucket | movable-assets income, general-clause income, mining; wages and gifts listed outside the total |

## Registry defaults

| Field | Value |
| --- | --- |
| `tax_country` | `ch` |
| `preview` | `true` |
| Fiat currency | `CHF`, required |
| Timezone | `Europe/Zurich` |
| Tax year | calendar year (Steuerperiode) |
| Holding rule | none for tax; KS 36 indicator `6 calendar months` |
| Accounting methods | `fifo` (default, informational), `moving_average` |
| Cost-basis pool scopes | `global` (default and only) |
| Report language | German, Switzerland (`de-CH`); French and Italian are future work |
| Report generators | `open_positions`, `ch_wealth_income` |
| Income categories | `movable_assets_income` (staking, lending_interest, interest, airdrop), `general_clause_income` (routing_income, airdrop_service), `mining_income`; outside the total: wages, gift_in; refused: hardfork |
| Rail pairs | no decision required |
| Legal sources | the table above, retrieved 2026-11-07 |

## Report

### What the taxpayer files

Swiss returns are cantonal; every canton has its own form and software, but
all have a **securities and assets list** (Wertschriften- und
Guthabenverzeichnis) with, per position: description, quantity, tax value at
31 December, and gross income split into income **with** withholding tax
(column A) and **without** (column B). Crypto belongs there with its ESTV
tax value and its income in column B. Mining and other self-employment income
goes in the main form.

### What Kassiber outputs

One German (`de-CH`) document in the [shared report design](report-design.md),
per tax period:

1. **Scope** adds the canton field (free text, informational), the valuation
   sources, and the preview notice.
2. **Wertschriftenverzeichnis** section (after Income): one row per asset
   (and per wallet in the detail table): Bezeichnung, Menge per 31.12.,
   Steuerwert pro Einheit, Steuerwert total, source (ESTV-Kursliste /
   Marktwert), Bruttoertrag ohne Verrechnungssteuer (B). Totals per column.
   Unknown values print `—` with the reason, never 0.
3. **Income** table: date, kind, quantity, CHF value, category (Ertrag aus
   beweglichem Vermögen / Einkommen Art. 16 / Mining), plus the lines kept
   out of the total (Lohnausweis, Schenkung).
4. **KS 36** section: the five criteria with their figures and verdicts,
   the overall verdict, and the KS 36 text that a failed safe harbour does not
   mean professional trading.
5. **Disposals** (informational): FIFO fragments with `acquired_at`,
   `held_6_months`, CHF proceeds, cost, gain; headed „steuerfreie
   Kapitalgewinne/-verluste (Privatvermögen)“.
6. **Method and provenance.**

Swiss German conventions: no ß (ss), thousands separator apostrophe
(`1'234.56`), decimal point, dates `31.12.2024`, month names Januar …
Dezember. These differ from the Austrian German table and need a `de-CH`
locale. XLSX sheets and CSV files: `wertschriften`, `income`, `ks36`,
`disposals`, `holdings`.

## Test vectors

All 20 cases are in [tax-country-ch-vectors.json](tax-country-ch-vectors.json).
Year-end values are illustrative inputs, **not** published Kursliste values;
`source` tells the implementation what kind of value it received. Amounts in
CHF, wallet `A` unless stated.

**ch-01 private-gain-tax-free.** Buy 1 BTC for 40,000, sell 0.5 on
01.09.2024 for 30,000: informational gain 10,000, taxable 0. Held from
10.01.2024: six months reached 10.07.2024 → C1 met. Wealth 0.5 × 80,000 =
40,000. C2 not assessable (no start-of-year value); overall inconclusive.

**ch-02 private-loss-not-deductible.** Sell 1 BTC bought for 40,000 at
35,000: informational −5,000, taxable 0.

**ch-03 lending-interest-income.** 120.40 + 112.10 + 115.75 = **348.25**
income from movable assets; wealth 0.006 × 84,000 = 504.00.

**ch-04 airdrop-income.** 500 TOKZ worth 640.00 → income **640.00**; no ESTV
or market value for TOKZ at year end → wealth unknown, report incomplete.

**ch-05 mining-income.** 420 + 550 = **970.00** mining income (own category);
wealth 0.02 × 84,000 = 1,680.

**ch-06 wealth-estv-value.** Cold 0.5 and Hot 0.3 BTC at the ESTV value
84,000: 42,000 + 25,200 = **67,200**.

**ch-07 wealth-market-value.** 0.5 × 85,210.33 (cached close) = 42,605.165 →
**42,605.16** (half-even), flagged `not_estv_tax_value`.

**ch-08 wealth-missing-value.** 0.5 BTC and no value: wealth `null`,
`complete: false`.

**ch-09 new-year-zurich.** 0.5 BTC held over the year end; interest of 0.001
BTC (88.00) at 2024-12-31T23:30Z = 01.01.2025 00:30 → 2025 income 88.00;
31.12.2024 wealth 0.5 × 84,000 = 42,000; 31.12.2025 wealth 0.501 × 90,000 =
45,090. 2025 C2: no trades, limit 5 × 42,000 = 210,000 → met.

**ch-10 ks36-holding-under-6-months.** Bought 31.01.2024, sold 30.07.2024;
six months end 31.07.2024 → C1 **not met**, overall not met.

**ch-11 ks36-holding-exactly-6-months.** Bought 31.08.2024, sold 28.02.2025;
31 August + 6 months = 28.02.2025 (month end) → C1 **met**.

**ch-12 ks36-volume-at-limit.** 31.12.2023: 1 BTC × 36,000 = 36,000; limit
180,000. 2024: buy 60,000 + sell 120,000 = **180,000** ≤ limit → met.

**ch-13 ks36-volume-over-limit.** Same with a sale of 120,000.01: volume
180,000.01 > 180,000 → **not met**.

**ch-14 ks36-gains-half-income.** Gain 50,000 − 20,000 = 30,000; declared
net income 60,000, half 30,000; 30,000 is not less than 30,000 → C3 **not
met**; C4 and C5 declared met; overall not met.

**ch-15 ks36-all-met.** Start value 38,000 (limit 190,000); 2024 sale of 0.5
BTC (held since 10.01.2023) for 30,000, gain 20,000 < 60,000 (half of
120,000); C4, C5 declared met → overall **met**.

**ch-16 gift-received-not-income.** 0.1 BTC received as a gift (6,100): no
income; listed outside the total; wealth 8,400.

**ch-17 wages-in-btc.** 0.05 BTC salary (2,000): outside Kassiber's income
total (Lohnausweis); wealth 4,200.

**ch-18 lbtc-valued-as-btc.** 0.5 BTC, 0.2 pegged to L-BTC: BTC 0.3 ×
84,000 = 25,200; L-BTC 0.2 × 84,000 = 16,800 flagged; total 42,000. No
refusal for the peg.

**ch-19, ch-20 (fail closed).** A hard fork
(`hardfork_treatment_unsupported`) and a book marked professional trader
(`professional_trading_unsupported`).

## Fail closed

**CH-FAILCLOSED** — Kassiber refuses or holds instead of guessing for: hard
forks; professional-trader (business-asset) books; a book currency other
than CHF; a year-end wealth figure without any value (unknown, never 0 and
never cost unless the user explicitly enters cost as the value); KS 36
criteria without their inputs (`not_assessable`, never `met`).

## Implementation

What Kassiber does for a `ch` book, and where.

| Rule | Where | Test |
| --- | --- | --- |
| Registry entry (CHF, Europe/Zurich, `Never` long-term, FIFO and MOVING_AVERAGE, `global` only, `de-CH`, `ch.wealth_income`, sources) | `crates/tax-countries/src/lib.rs` (`SWITZERLAND`) | `switzerland_is_a_preview_chf_country_with_informational_gains` |
| CH-CG-PRIVATE, CH-TZ | engine rules country in Europe/Zurich, `holding: never`, global FIFO; `taxable_capital_gains_chf` is always 0 and the gains are listed for information | ch-01, ch-02, ch-09 |
| CH-INC-* | `crates/reports/src/ch_tax.rs` (`category`): staking, lending interest, interest and airdrops are income from movable assets; routing and other income the general clause; mining its own category | ch-03, ch-04, ch-05 |
| CH-WAGES, CH-GIFT | salary (`wages`) and received gifts are acquisitions at their value, listed outside the income total | ch-16, ch-17 |
| CH-WEALTH, CH-WEALTH-ESTV, CH-WEALTH-FALLBACK | `ch_tax::wealth` and `ch_tax::valuations`: each wallet's holding at the end of 31 December (Zurich), valued by a `BTC-CHF` (`<ASSET>-CHF`) rate of that day with source `estv_kursliste`, else the day's last rate as the market close (flagged), else unknown; L-BTC without its own value at Bitcoin's (flagged) | ch-06 … ch-08, ch-18 |
| CH-KS36-C1 … C5 | `ch_tax::years`: six calendar months per consumed lot (month ends clamped), volume (purchases and sale proceeds) against five times the previous 31 December's crypto wealth (`crypto_only`), gains against half the declared net income, the two declarations; any `not_met` fails the safe harbour, any `not_assessable` leaves it inconclusive | ch-10 … ch-15 |
| CH-HARDFORK | engine `refused_receipts`: a hard fork is held `acquisition_valuation_unsupported` with code `hardfork_treatment_unsupported` | ch-19 |
| CH-PROFESSIONAL | a book whose KS 36 inputs say `professional_trader` is refused by the report with `professional_trading_unsupported` | ch-20 |
| CH-RAIL | no rail decision required: a peg keeps the book's carrying-value default | ch-18 |
| Report | `crates/reports/src/report_design/ch.rs`, strings `DE_CH` in `report_design/strings.rs` | `the_securities_list_states_values_sources_and_income_in_swiss_german`, `tests/test_rust_ch_tax_country.py` |

All 20 vectors run in `crates/reports/tests/tax_country_vectors.rs`
through the engine under the registry's rules and `ch_tax` (the full
expected document, or the refusal code). Disposals and income are the
engine's; the year-end holdings follow the vector's events and are valued
by its `valuations`, as the product values the journal's holdings by the
rate cache.

### The report

`reports export-pdf|export-xlsx|export-csv` of a Swiss book add four
sections after Income, in Swiss German (no ß, `1'234.56`, `31.12.2024`,
`Januar`):

- **Schweiz: Wertschriftenverzeichnis** — table `wertschriften`, one row
  per wallet and asset at 31 December: Bezeichnung, Menge, Steuerwert pro
  Einheit, Steuerwert total, Quelle (`ESTV-Kursliste`, `Marktwert (nicht
  ESTV-Steuerwert)`, `—`), the year's Bruttoertrag ohne Verrechnungssteuer
  (B) from that holding, and Hinweise; `wertschriften_total` per tax period
  (`unvollständig` when a value is missing, never 0).
- **Schweiz: Einkommen aus Kryptowährungen** — table `ch_income` with each
  receipt's category, and `ch_outside_income` (Lohnausweis, Schenkung).
- **Schweiz: Gewerbsmässiger Wertschriftenhandel (KS 36)** — table `ks36`
  per tax period with each criterion, its figures and the overall verdict,
  and the KS 36 caveat that a failed safe harbour does not establish
  professional trading.
- **Schweiz: Steuerfreie Kapitalgewinne und -verluste (Privatvermögen)** —
  table `ch_disposals` per consumed lot with acquisition date and the
  six-month check.

The Method section states the tax-free gains, the valuation sources, where
the KS 36 inputs come from, and the canton when given. The preview notice
is on every page, the XLSX summary and the CSV summary and manifest.

### Using it

CLI: `kassiber profiles create --tax-country ch <label>` (CHF, FIFO,
`global`; `--gains-algorithm MOVING_AVERAGE` is the other choice), import
or sync the wallets, `journals process`, then the report exports. Enter
each year's ESTV Kursliste value as a rate of 31 December:
`kassiber rates set BTC-CHF 2024-12-31T12:00:00+01:00 84000 --source
estv_kursliste` (Kassiber makes no network request for it). The KS 36
inputs are read from the book's `settings` row
`tax_country.ch.ks36:<profile id>`, a JSON object with any of
`net_income_chf` (`{"2024": "120000"}`), `debt_financed_without_cover`,
`derivatives_only_hedging`, `professional_trader` and `canton`; there is no
CLI or desktop form for it yet, so without it criteria 3 to 5 read `nicht
beurteilbar`. The desktop's country picker lists Switzerland with the
Preview badge from the registry.

### Differences from the specification

- **Network fees** stay Kassiber's generic rule (their basis leaves without
  a gain): they are transaction costs, not deductible, and no event for a
  private investor. The reference model lists a move's fee as an
  informational disposal at its value; no vector has one.
- **KS 36 inputs** have no input form yet (see Using it); the product reads
  a net income per year, the vectors one for every year.
- **Table ids** are `wertschriften`, `wertschriften_total`, `ch_income`,
  `ch_outside_income`, `ks36` and `ch_disposals`; the shared report already
  has `income`, `disposals` and `holdings` sheets.
- **A professional trader's book** is refused when its report is built;
  journal processing still runs, so its figures exist but are not reported.

## Open questions for a Swiss tax adviser

1. **Q1** KS 36 criterion 2 counts all securities and cash at the start of
   the year. Is a crypto-only indicator useful, or should Kassiber require the
   user's total before showing a verdict?
2. **Q2** Hard forks: income at market value, or a tax-free split with no
   income?
3. **Q3** Lightning routing income: income under Art. 16 Abs. 1 DBG for a
   private node, self-employment above some scale?
4. **Q4** L-BTC at the BTC Kursliste value, or at its own market value?
5. **Q5** Staking rewards paid net of the validator's commission: report net,
   or gross with the commission as a deductible asset-management cost?
6. **Q6** Mining: is a home miner's income always taxable income even when it
   is a hobby with no profit (Dossier 4.1.1 calls it Liebhaberei)?
7. **Q7** French- and Italian-speaking cantons: is a German report
   acceptable, or does `ch` need `fr-CH` and `it-CH` before leaving preview?
8. **Q8** Network fees on own-wallet moves: nothing (Kassiber's rule), or an
   informational disposal at their value?
9. **Q9** Criterion 2's volume counts purchases at cost and sales at their
   proceeds; should transfers between own wallets or salary receipts ever
   count?

## Fonts and language

German with Swiss conventions needs no characters beyond Latin-1 (no ß).
French and Italian (é, è, à, ç, …) are covered by DM Sans too (checked with
`fc-query`). No font is missing. A `de-CH` string table is required; the
apostrophe thousands separator must be a typographic choice (U+2019 or
U+0027) fixed in `locale.rs`.
