# Japan (`jp`): crypto tax rules and report specification

**Status:** implemented in the Rust backend as a **preview** country
(2026-11-07). No Japanese tax professional (税理士) has reviewed it; every
Japanese report and screen carries the preview notice until one does.
Sources were read on 2026-11-07. Nothing here is tax advice.

Test vectors: [tax-country-jp-vectors.json](tax-country-jp-vectors.json)
(19 cases, same format as the
[Spanish vectors](tax-country-es.md#test-vectors), with per-asset year
blocks that mirror the NTA calculation sheet).
A Python script (`scripts/tax-country-vectors.py`) computed them and
recorded RP2's JP behaviour: the country refuses, and its report template
was evaluated separately (`rp2-jp-template`). The script was removed with
Python ([plan 22](../plan/22-python-removal.md), D6), so the file is now a
frozen, reviewed golden.

## Summary

| Item | Japan |
| --- | --- |
| Tax | Income tax: crypto gains are **雑所得 (その他雑所得)**, miscellaneous income taxed together with other income at progressive rates. From the 適用開始日 of the 令和8年度 reform, 特定暗号資産 move to 20 % separate taxation (not yet in force; see JP-R16) |
| Tax year | Calendar year, Asia/Tokyo civil date |
| Currency | JPY, required |
| Cost method | **総平均法 (total average)**, the statutory default, or **移動平均法 (moving average)** by notification; chosen per crypto type; one pool per taxpayer across all exchanges and wallets |
| Holding period | None |
| Allowance | None in the crypto computation. Salaried taxpayers with ≤ ¥200,000 of other income need not file an income-tax return (所得税法121条) |
| Crypto-to-crypto | Taxable; revenue = market value of what is received |
| Income | Mining, staking and lending: income at market value when received, which is also the acquisition cost. Hard fork: no income, cost ¥0. Airdrops: fail closed |
| Losses | Net inside その他雑所得 only. A net 雑所得 loss offsets no other income and does not carry forward |
| Rounding | NTA sheets: revenue rounded down and expenses rounded up to the yen (total average); book value after each sale rounded up (moving average) |
| Form | 確定申告書 第一表 収入金額等「雑・その他」(区分 2 = 暗号資産) and 所得金額等「雑・その他」; 第二表「所得の内訳」 (種目: 暗号資産); the NTA 暗号資産の計算書 as a supporting schedule |
| Report language | Japanese (`ja-JP` string table) in PDF, XLSX and CSV labels; the PDF sets kana and kanji in the bundled Noto Sans JP |

## Sources

All retrieved 2026-11-07.

- **[JP-S1]** 国税庁「暗号資産等に関する税務上の取扱いについて（ＦＡＱ）」
  令和7年12月 (law as of 1 December 2025):
  <https://www.nta.go.jp/publication/pamph/pdf/virtual_currency_faq_03.pdf>.
  Questions used: 1-1 (sale), 1-2 (paying for goods), 1-3 (crypto-to-crypto),
  1-4 (donation), 1-5 (acquisition cost and fees), 1-6 (forks), 1-7
  (mining, staking, lending), 2-1 (timing), 2-2 (income category, updated
  令和7年12月), 2-3 (necessary expenses), 2-4 (cost of sales, both methods,
  worked example), 2-5/2-6 (method notification and change), 2-7 (unknown
  prices; the 5 % rule), 2-8/2-9 (annual exchange reports), 2-10 (low-value
  transfers and gifts), 2-11 (losses), 2-12/2-13 (margin and credit
  trading).
- **[JP-S2]** 国税庁「暗号資産等に関する税務上の取扱い及び計算書について（令和7年12月）」:
  <https://www.nta.go.jp/publication/pamph/shotoku/kakuteishinkokukankei/kasoutuka/>.
- **[JP-S3]** NTA calculation sheets 暗号資産の計算書（移動平均法用）
  <https://www.nta.go.jp/taxes/shiraberu/shinkoku/yoshiki/02/xlsx/001.xlsx>
  and （総平均法用）
  <https://www.nta.go.jp/taxes/shiraberu/shinkoku/yoshiki/02/xlsx/002.xlsx>.
  Their formulas were read: total average `F42 = (N42+W42)/(N41+W41)`,
  income `ROUNDDOWN(revenue,0) − ROUNDUP(expenses,0)`; moving average,
  after each sale `book = ROUNDUP(unit × remaining, 0)`.
- **[JP-S4]** 所得税法施行令 (昭和40年政令第96号) 第119条の2（評価の方法）,
  第119条の3（選定）, 第119条の5（法定評価方法＝総平均法）, 第119条の6
  （取得価額）, via the e-Gov law API:
  <https://laws.e-gov.go.jp/law/340CO0000000096>.
- **[JP-S5]** 国税庁 確定申告の手引き 令和7年分「手順2 収入金額等、所得金額等を計算する」
  (雑所得): <https://www.nta.go.jp/taxes/shiraberu/shinkoku/tebiki/2025/03/order2/3-2_07.htm>.
  It gives the その他雑所得 computation G − H = I, the 区分 code 2 for
  crypto, and that total 雑所得 is floored at ¥0. The box letters are
  images; ク and ⑨ are taken from the guide's worked example (JP-Q9).
- **[JP-S6]** 国税庁 タックスアンサー No.1900 (給与所得者で確定申告が必要な人):
  <https://www.nta.go.jp/taxes/shiraberu/taxanswer/shotoku/1900.htm>.
- **[JP-S7]** 財務省「令和８年度税制改正の大綱」(閣議決定 2025-12-26), 一 3 (2):
  <https://www.mof.go.jp/tax_policy/tax_reform/outline/fy2026/08taikou_01.htm>.
  Secondary reports (not relied on for rules) say the amended 所得税法 was
  enacted on 2026-03-31 and the amended 金融商品取引法 was promulgated on
  2026-07-23, with its crypto part effective within a year.

## Rules

**JP-R1 Taxable events.** Selling for yen (FAQ 1-1), paying for goods
(1-2) and exchanging for another crypto (1-3) all realise income: revenue
minus the cost of the coins given. A gift to an individual or company puts
the market value at the time into revenue (所得税法40条; FAQ 1-4, 2-10). So
does a donation (1-4), and a donation to a qualifying body may also give a
deduction, which is outside Kassiber. A move between one's own wallets is
not taxable. Revenue in a payment for goods is the goods' price (FAQ 1-2
uses ¥403,000, not the coins' ¥405,000). Kassiber uses
`consideration_value` when a reviewed one exists, otherwise the coins'
market value, with a note. In crypto-to-crypto, revenue is the market
value of the coins received (FAQ 1-3).

**JP-R2 Income category.** Gains are 雑所得 (その他雑所得) by default.
If the year's crypto revenue exceeds ¥3,000,000, they are 事業所得 when
books are kept, or 業務に係る雑所得 when not (FAQ 2-2, updated 令和7年12月).
Crypto used in a business is business income. Kassiber computes the same
amounts either way. It flags the year (`jp_revenue_over_3m_classification`)
and lets the user state the category; the report labels follow that
choice.

**JP-R3 Cost method.** The year-end value, and with it the cost of sales,
uses 総平均法 or 移動平均法, chosen per crypto type (所令119の2, 119の3).
Without a notification the method is 総平均法 (所令119の5; FAQ 2-5). A
change needs an application by 15 March of the year and approval (FAQ
2-6), and one is normally refused within three years of the last choice.
The pool covers every exchange and wallet: FAQ 2-8 adds exchanges A and B
into one sheet. Kassiber adds two methods:

- `jp_total_average` (default) and `jp_moving_average`, distinct from
  `fifo` and the generic `moving_average` because of the NTA rounding
  (JP-R12);
- a per-asset method with an effective-from year. Changing a past year's
  method is refused; the report names the method per coin.

**JP-R4 Total average (総平均法).** Per coin type and year: unit cost =
(value at 1 January + acquisition cost of the year) ÷ (quantity at
1 January + quantity acquired in the year). Cost of sales = unit cost ×
quantity disposed. Year-end value = unit cost × quantity held at
31 December (所令119の2①一; FAQ 2-4). Every disposal of the year shares
one unit cost, so the year's figures are final only after 31 December.
Earlier reports are provisional, and the report says so. Kassiber computes
`cost = total_value × qty_disposed ÷ total_qty` in the engine's 32-digit
context. It carries the year-end value forward **unrounded** as
`total_value − cost` (the NTA sheet carries its unrounded cell; JP-Q4).

**JP-R5 Moving average (移動平均法).** The unit cost is revised at every
acquisition: (book value + acquisition cost) ÷ (quantity) (所令119の2①二).
A disposal leaves the unit cost unchanged. The NTA sheet rounds the book
value left after each disposal **up** to the yen:
`book = ROUNDUP(unit × remaining, 0)`. The disposal's cost is the drop in
book value [JP-S3]. Kassiber follows the sheet (vectors jp-05, jp-08).

**JP-R6 Acquisition cost.** Purchase: price + purchase fees (所令119の6①一;
FAQ 1-5: ¥2,000,000 + ¥550 = ¥2,000,550). Anything else (crypto-to-crypto,
mining, staking, lending): market value at acquisition (所令119の6①三; FAQ
1-5). Hard fork: ¥0 (FAQ 1-6). Gift received: market value at the time
(FAQ 1-5 ③). Acquisitions made only momentarily to complete a purchase or
an exchange (a bridge coin) are excluded from the averages (所令119の2②);
see JP-R7.

**JP-R7 Bridge acquisitions.** A coin bought only to be spent at once on
another (yen → USDT → BTC inside one order) is left out of the averages
(所令119の2②). Kassiber cannot tell this from the data. A reviewed
`bridge_acquisition` marker on a cross-asset pair excludes the bridge coin.
Without the marker, the acquisition counts normally (JP-Q3).

**JP-R8 Necessary expenses.** The cost of the coins sold, sale fees, and
other costs directly needed for the sale, such as the clearly separable
share of line fees and depreciation of equipment (FAQ 2-3). Kassiber
deducts the cost of sales and the JPY fees on its rows. It never
apportions household costs. A network fee paid in the coin on a
self-transfer is a disposal of the fee units: revenue = their market
value, plus an equal expense (`other_expenses`). The net effect is minus
their cost (vector jp-14; JP-Q5).

**JP-R9 Income receipts.** Mining, staking and lending: the market value at
acquisition is revenue (雑所得), costs of earning it are expenses, and the
value becomes the coins' acquisition cost (FAQ 1-7). Kassiber kinds
`mining`, `staking`, `lending_interest` and `interest` → income receipts
(report section "Income receipts", part of その他雑所得 revenue) and
acquisitions at that value. `hardfork` → acquisition at ¥0, no income.
`airdrop`, `routing_income` and unlabelled `income` fail closed
(`jp_<kind>_requires_review`): the FAQ does not cover airdrops (JP-Q2).

**JP-R10 Timing.** Revenue belongs to the year the coins are delivered.
The contract date may be elected instead (FAQ 2-1). Kassiber uses the row
time, which for exchange trades is the execution time, and the Asia/Tokyo
civil year: 2025-12-31T15:30Z is 2026-01-01 00:30 JST (vector jp-16).

**JP-R11 Losses.** A loss inside 雑所得 cannot be deducted from salary or
any other category (所得税法69条; FAQ 2-11). The annual crypto computation
still nets coins against each other within その他雑所得 (vector jp-15).
Total 雑所得 on the return is floored at ¥0 [JP-S5], and nothing carries
forward. Kassiber shows the negative crypto total, floors the 第一表
figure at 0 and flags `jp_zatsu_loss_no_offset`.

**JP-R12 Rounding.** As the NTA sheets [JP-S3]: total average — per coin,
revenue `ROUNDDOWN(Σ, 0)` and expenses (cost of sales + fees)
`ROUNDUP(Σ, 0)`; moving average — book value after each disposal
`ROUNDUP(…, 0)`, then the same year-end rounding of revenue and expenses.
Income receipts are summed and rounded down. Per-coin incomes add up to
the 第一表 figure.

**JP-R13 Low-value transfers and gifts.** A transfer to another person for
less than 70 % of market value puts 70 % of market value into revenue
(FAQ 2-10: sold for ¥450,000 when worth ¥1,000,000 → revenue ¥700,000).
"Significantly low" means below 70 % (70 %未満), so exactly 70 % is not
caught (vector jp-13). A gift puts the full market value into revenue.
Kassiber needs the market value of every outbound sale (it has it from
rates) and flags `jp_low_value_transfer_70pct`.

**JP-R14 Currency.** JPY. A trade in a foreign currency converts at the
TTM (telegraphic transfer middle rate) of the trade date, as exchanges'
annual reports do (FAQ 2-9). Kassiber prices rows with its JPY rates. A
row whose fiat leg is a foreign currency is flagged until a TTM-based
reviewed value replaces it (JP-Q6).

**JP-R15 Unknown acquisition cost.** NTA accepts 5 % of the sale price as
the cost of coins whose price cannot be traced (所基通48の2-4; FAQ 2-7).
Kassiber never applies it silently. A disposal without lots quarantines
the coin's year (`missing_cost_basis`, vector jp-18). The 5 % rule may
later become an explicit, reviewed election per disposal.

**JP-R16 Separate taxation from the 適用開始日.** The 令和8年度 outline
[JP-S7] taxes the transfer of 特定暗号資産 (coins registered in the FIEA
register) to a 暗号資産取引業 operator at 20 % (15 % national + 5 % local),
separately from other income. Losses carry forward three years. Crypto that
stays in comprehensive taxation becomes 譲渡所得, without the special
deduction, the 5-year halving or loss offset against other income. Both
start on 1 January of the year after the amended FIEA takes effect, a date
not yet fixed. It is 2027-01-01 at the earliest. Kassiber **refuses tax
years from 2027** (`jp_separate_taxation_regime_unconfirmed`, vector jp-17)
until the start date and the implementing rules (cost method under the new
regime, transition) are confirmed.

**JP-R17 Margin and credit trading.** Crypto margin trading is
comprehensive taxation, not separate (FAQ 2-12). Credit trading uses
specific identification (個別法) at settlement (FAQ 2-13). Kassiber has no
derivative rows and fails closed on any import that implies them.

**JP-R18 Year-end holdings.** For information: quantity and value per coin
at 31 December. 財産債務調書 and 国外財産調書 use year-end holdings (FAQ 7-1…
7-3). Kassiber does not prepare them.

## Implementation

What Kassiber does today, and where (2026-11-07).

| Piece | Where | Notes |
| --- | --- | --- |
| Registry entry | `crates/tax-countries/src/lib.rs` (`JAPAN`) | `jp`, preview, JPY required, `JP_TOTAL_AVERAGE` (default) and `JP_MOVING_AVERAGE`, `global` pool only, no holding period, Asia/Tokyo, calendar year, names Japan / Japan / 日本, the JP-S sources, the `jp.tax_regime` table, generators `open_positions` and `jp.crypto_keisansho` |
| Averages | `tax-engine/core/src/matching.rs` (`JpAverage`) on the `Matching` hook | total average per Tokyo tax year (`total × quantity ÷ total quantity`, closing value carried unrounded) and moving average (book value left after a disposal `ROUNDUP` to the yen); fragments take FIFO portions of the lots for open positions, each with its share of the disposal's cost (`cost_override`), so a disposal's fragments add up to its cost exactly; the global scope only |
| Engine rules | registry `engine` | `fee_units: disposal` (JP-R8), `swap_values: received` (FAQ 1-3), gifts and donations at market value (所得税法40条), `below_market: seventy_percent` (FAQ 2-10), `zero_cost_kinds: hardfork` (FAQ 1-6), and the refusals below |
| Refusals | engine `refused_receipts` (the shared refusal path, [rules countries](tax-engine.md#rules-countries)) | inbound `airdrop`, `routing_income`, `income` are held as `acquisition_valuation_unsupported`; outbound kinds containing `lost`, `stolen` or `theft` (`direction: outbound`) as `non_sale_disposal_kind`; each before pricing, with Japan's code in the quarantine detail |
| Sale fees | the sale's value | a Kassiber transaction has one fiat value: an exchange's yen commission is netted into it, as for every country, which gives the same income (revenue and その他の必要経費 are each lower by the commission; JP-Q12). The vector runner restates jp-14 that way |
| Journal facts | `journal_entries.calculation_json` (`valuation` beside the fragments; a fragment's `expenses_exact` as the tip stores them) | present only on entries that have them |
| Country layer | `crates/reports/src/jp_tax.rs` (figures), `crates/reports/src/report_design/jp.rs` (sections) | the calculation sheet per coin and year (A–I, revenue, expenses, income, rounded as the NTA sheet), disposals, income receipts, the return figures (第一表 雑・その他 区分2, floored at 0; 第二表 venues), flags and refused years |
| Strings | `crates/reports/src/report_design/strings_ja.rs` | the shared report strings and Japan's layer in Japanese |
| Font | `crates/pdf/fonts/NotoSansJP-Regular.otf` | Noto Sans JP 2.004 Regular (SIL OFL), the last glyph fallback of the sealed PDF engine; no system fonts |
| Calendar | registry Asia/Tokyo; report readers through `kassiber_reports::country::zone`, `calendar::local` and `export_range::book_zone` (a fixed +09:00 without tzdata) | as for Spain |
| Vectors | `crates/reports/tests/country_vectors/es_jp.rs` (`jp_compute`) | all 19 pass through the `ledger` operation under the registry's rules and the layer above; jp-14's commission is restated as netted into the price (`net_jp_commissions`) |
| End to end | none in Rust yet | the Python black-box test (`tests/test_rust_jp_tax_country.py`: a CLI-built book through journals to the Japanese PDF, XLSX and CSV, repeat PDFs byte-identical) went with Python ([plan 22](../plan/22-python-removal.md)) |

### Using it

- CLI: `kassiber profiles create --workspace <ws> --tax-country jp <name>`
  sets JPY, `JP_TOTAL_AVERAGE` and the global pool;
  `--gains-algorithm JP_MOVING_AVERAGE` chooses the moving average (it
  needs a notification to the tax office, FAQ 2-5). `journals process`
  books the averages; `reports export-pdf|export-xlsx|export-csv` write the
  Japanese full report with the sections 暗号資産の計算書 (`jp_keisansho`),
  譲渡と収入 (`jp_disposals`, `jp_income_receipts`), 確定申告書の金額
  (`jp_return`) and 確認事項 (`jp_review`).
- Desktop: Japan appears in onboarding and the Books picker with the
  **Preview** badge, and both methods in the method picker.

### Not implemented (fail closed or documented)

- One method per book, not per coin with an effective-from year (JP-R3);
  changing the profile's method recomputes every year with the new one.
  A reviewer must check that the book's method matches the notification.
- A sale's market value apart from its price: the engine applies the 70 %
  rule when a row carries `market_value`, but no importer records one yet
  (vectors jp-12 and jp-13 exercise it through the engine).
- `consideration_value` for payments: a payment's recorded value is its
  revenue (FAQ 1-2 uses the goods' price).
- Bridge acquisitions (JP-R7) are averaged like any acquisition; there is
  no reviewed marker yet.
- The ¥3,000,000 classification flag is per coin, as the vectors define
  it; the FAQ speaks of the year's crypto revenue (open question below).
- The year-handoff workbook, the capital-gains CSV, the cross-check and the
  exit-tax estimate still cut years in UTC for Japanese books.

### Refusals

| Case | Code |
| --- | --- |
| tax years from 2027 | the year is refused, `jp_separate_taxation_regime_unconfirmed` (its figures are withheld; the review lists the year) |
| tax years before 2009 | refused, `jp_tax_year_unknown` |
| a disposal without known acquisitions | quarantine (`insufficient_lots` or `missing_cost_basis`), the coin's year withheld as `missing_cost_basis`, the year's total `null`; no automatic 5 % rule |
| airdrop, routing income, unlabelled income | quarantine `jp_airdrop_requires_review`, `jp_routing_income_requires_review`, `jp_income_requires_review` |
| lost, stolen or theft outflow | quarantine `jp_lost_requires_review` |
| the wallet pool scope, a lot method | refused by the profile; the engine refuses the averages on the wallet scope |
| non-JPY currency | the profile replaces it with JPY |

## Kassiber mapping

| Kassiber row | JP event |
| --- | --- |
| inbound purchase | acquisition at JPY price + JPY fees |
| outbound sale | disposal; revenue = price (or 70 % of market value if below 70 %) |
| cross-asset taxable pair | disposal of the out leg; revenue = in leg's market value = its acquisition cost |
| outbound payment | disposal; revenue = goods' price if reviewed, else coins' market value |
| reviewed `gift` | disposal; revenue = market value |
| self-transfer with fee | fee units disposed; revenue and expense = their value |
| `mining`, `staking`, `lending_interest`, `interest` | income receipt + acquisition at market value |
| `hardfork` | acquisition at ¥0 |
| `airdrop`, `routing_income`, `income`, `lost`, `stolen` | quarantine |
| BTC↔LBTC carrying-value pair | no disposal (as generic; JP-Q7) |

Reviewed gifts need the country hook that also Spain and Ireland need,
replacing today's `non_sale_disposal_kind` quarantine.

The JP methods are not lot methods. The engine needs an
average-pool computation per coin and year. Total average needs the whole
year before it can price any disposal, so a disposal's cost changes when a
later acquisition of the same year arrives. Journals therefore re-derive
the year's rows whenever an input of that year changes, and the readiness
barrier marks the current year provisional.

## Report

[Shared report design](report-design.md), Japanese country layer:

- **Method:** 総平均法 or 移動平均法 per coin, with the effective year; JPY;
  Asia/Tokyo; preview notice; "Kassiber does not file the return"; the
  ¥200,000 filing threshold for salaried taxpayers (所得税法121条, [JP-S6])
  as information only.
- **Calculation sheet per coin and year** (after Income), mirroring the NTA
  暗号資産の計算書:
  1. coin name;
  2. per exchange or wallet: purchase quantity and amount, sale quantity and
     amount (the annual-report section);
  3. other acquisitions and disposals (receipts, gifts, fees, swaps);
  4. cost of sales: opening quantity and value (A, B), acquisitions (C, D),
     unit cost (E, total average only), quantity disposed and cost (F, G),
     closing quantity and value (H, I);
  5. income: revenue, cost of sales, fees, income.
- **Income receipts:** mining, staking and lending at market value.
- **Return figures:** 第一表 収入金額等 雑・その他 = revenue (rounded) + income
  receipts, with 区分 2; 第一表 所得金額等 雑・その他 = that less expenses,
  floored at 0 on the return; 第二表 所得の内訳: 雑 / 暗号資産 / the exchanges
  named / revenue. A category other than その他雑所得, chosen under JP-R2,
  moves the figures to 雑・業務 or 事業.
- **Disposals table:** one row per disposal with date, wallet, coin,
  quantity, revenue and cost. Under total average a row's cost is
  `unit × qty`; under moving average it is the book-value drop.
- **Issues:** quarantines, flags (`jp_revenue_over_3m_classification`,
  `jp_low_value_transfer_70pct`, `jp_zatsu_loss_no_offset`), and
  "provisional until 31 December" for the current year.
- No indicative tax: rates are progressive over total income (5–45 %
  national, plus 10 % local and the 2.1 % reconstruction surtax), and
  Kassiber does not know the other income.

Yen amounts are integers in the return figures. Unrounded unit costs and
values are shown to two decimals in the PDF and exactly in XLSX/CSV.

## Registry defaults

| Field | Value |
| --- | --- |
| `tax_country` | `jp` |
| `preview` | `true` |
| `fiat_currency` | `JPY` (required) |
| `accounting_methods` | `jp_total_average`, `jp_moving_average` |
| `default_accounting_method` | `jp_total_average` (法定評価方法) |
| method scope | per asset, with effective-from year |
| `cost_basis_pool_scopes` | `global` |
| `long_term_days` | none (`sys.maxsize`, as RP2's JP plugin) |
| `timezone` | `Asia/Tokyo` |
| `tax_year` | calendar; years ≥ 2027 refused (JP-R16) |
| `generation_language` | `en` (PDF), with Japanese form labels in XLSX; `ja` once a CJK font is bundled |
| `report_generators` | shared full/summary reports + `jp_crypto_keisansho` |
| income categories | per JP-R9 |
| sources | JP-S1 … JP-S7 with retrieval date 2026-11-07 |

## Test vectors

Step by step (yen):

- **jp-01** (FAQ 1-1). 4 BTC for 4,000,000; 0.2 BTC sold for 210,000.
  Unit 1,000,000; cost 200,000; income **10,000**.
- **jp-02** (FAQ 1-2). 0.3 BTC pays for goods priced 403,000 (coins worth
  405,000): revenue 403,000 − cost 300,000 = **103,000**.
- **jp-03** (FAQ 1-3). 1 BTC for 40 XRP at 30,000 each: revenue 1,200,000 −
  cost 1,000,000 = **200,000**. XRP acquisition cost 1,200,000.
- **jp-04** (FAQ 2-4, 2-8; total average). Purchases 4 BTC 1,845,000;
  2 BTC 1,650,000; 0.5 BTC 542,800 → 6.5 BTC, 4,037,800. Unit 4,037,800 ÷ 6.5
  = 621,200. 5 BTC sold: cost 3,106,000. Year-end 1.5 BTC, 931,800. Revenue
  2,400,000 + 2,895,000 = 5,295,000. Income **2,189,000** (the FAQ's
  figure). Flagged over ¥3,000,000 of revenue.
- **jp-05** (FAQ 2-4; moving average). Units 461,250 → 582,500 after 20 June.
  The 10 July sale leaves 4 BTC at ROUNDUP(2,330,000) = 2,330,000 (cost
  1,165,000). After 15 September: 2,872,800 ÷ 4.5 = 638,400. The 30 November
  sale leaves 1.5 BTC at 957,600 (cost 1,915,200). Cost 3,080,200 (the
  FAQ's figure); income **2,214,800**.
- **jp-06** (JP-R4). 2024 as jp-04, ending 1.5 BTC at 931,800. 2025:
  + 0.5 BTC for 5,000,000 → unit (931,800 + 5,000,000) ÷ 2 = 2,965,900; 1 BTC
  sold for 15,000,000 → income **12,034,100**; year-end 1 BTC at 2,965,900.
- **jp-07** (JP-R12, total average). 3 BTC for 1,000,000; 1 BTC sold for
  500,000.70. Cost 333,333.33…; revenue ROUNDDOWN → 500,000; expenses
  ROUNDUP → 333,334; income **166,666**. Year-end value carried exactly:
  666,666.666…67 (32 digits).
- **jp-08** (JP-R12, moving average). Same trades. Book after the sale
  ROUNDUP(333,333.33… × 2) = 666,667, so cost = 1,000,000 − 666,667 = 333,333;
  income **166,667**.
- **jp-09** (JP-R9). Purchase 1 BTC 5,000,000; mining 0.1 BTC (600,000);
  lending 0.01 BTC (70,000) → 1.11 BTC, 5,670,000. Sale of 0.5 BTC for
  4,000,000: cost 5,670,000 × 0.5 ÷ 1.11 = 2,554,054.05… → ROUNDUP 2,554,055;
  sale income 1,445,945. Receipts 670,000. その他雑所得 **2,115,945**
  (revenue 4,670,000, expenses 2,554,055).
- **jp-10** (FAQ 1-6). BCH from a fork at ¥0; sold for 50,000: income
  **50,000**.
- **jp-11** (所得税法40条). Gift of 0.5 BTC worth 5,000,000 (cost 2,000,000):
  income **3,000,000**.
- **jp-12** (FAQ 2-10). Bought 450,000; sold for 450,000 when worth
  1,000,000. 450,000 < 700,000 = 70 %, so revenue 700,000 and income
  **250,000**.
- **jp-13** (JP-R13 boundary). Sold for exactly 700,000 when worth
  1,000,000: not below 70 %, revenue 700,000, income **250,000**, no flag.
- **jp-14** (JP-R6, R8). 2 BTC for 10,000,000 + 550 fee → 10,000,550, unit
  5,000,275. A self-transfer pays a 0.0001 BTC fee worth 600; 1 BTC sold for
  7,000,000 with a 7,000 fee. Disposed 1.0001 BTC: cost 5,000,775.0275.
  Revenue 7,000,000 + 600; expenses 5,000,775.0275 + 7,000 + 600 →
  ROUNDUP 5,008,376. Income **1,992,224**. Year-end 0.9999 BTC at
  4,999,774.9725.
- **jp-15** (JP-R11). BTC +300,000, ETH −500,000 → その他雑所得 −200,000:
  reported, floored to 0 on the return, nothing carried forward, flagged.
- **jp-16** (JP-R10). A sale at 2025-12-31T15:30Z is 1 January 2026 in
  Tokyo: 2025 has no disposal; 2026 income **1,500,000** (0.5 BTC for
  4,000,000, cost 2,500,000).
- **jp-17** (JP-R16). A 2027 sale: year 2027 refused; 2026 computes.
- **jp-18** (JP-R15). A sale with no acquisitions: quarantine
  `missing_cost_basis`; the year's total is withheld.
- **jp-19** (JP-R9). An airdrop: quarantine `jp_airdrop_requires_review`.

## RP2 comparison

In the pinned fork (`bitcoinaustria/rp2@3d2e03ef`) the JP country is
**disabled**. `get_accounting_methods()` is empty, `rp2_entry` exits with
"Japan support is disabled: the JP country plugin still needs a
Japan-specific accounting method". The plugin advertises `ja` as its
language but ships only `en` (and a `kl` test) templates. Every vector's
`rp2.computed` is that refusal.

RP2's `tax_report_jp` template still implements the total average in
spreadsheet formulas, the same as the NTA sheet. The removed
`scripts/tax-country-vectors.py rp2-jp-template` ran RP2's lot engine under a
JP subclass that allows FIFO (the template reads transactions, not lots),
rendered the template, recalculated it with headless LibreOffice and stored
each year's summary in `rp2.template`. Against the spec:

| Rule | RP2 template | Spec | Vectors |
| --- | --- | --- | --- |
| Total average, NTA rounding | same | same | jp-01, 03, 04, 06, 07, 09, 11, 13, 15 agree |
| Moving average | absent (total average only) | `jp_moving_average` | jp-05 (2,189,000 vs 2,214,800), jp-08 |
| Payment for goods | coins' market value | goods' price (FAQ 1-2) | jp-02 (105,000 vs 103,000) |
| Purchase fee | expensed in the year | added to acquisition cost (FAQ 1-5) | jp-14 |
| Network fee on a self-transfer | revenue without the matching expense | revenue and equal expense | jp-14 (1,992,550 vs 1,992,224) |
| Hard fork at ¥0 | impossible (RP2 needs a positive spot price) | ¥0 cost, no income | jp-10 |
| Low-value transfer (70 %) | absent | FAQ 2-10 | jp-12 (0 vs 250,000) |
| Tax year | year in the timestamp's own offset | Asia/Tokyo | jp-16 |
| 2027 onwards | computed as 雑所得 | refused | jp-17 |
| Airdrop | income at market value | quarantine | jp-19 |

RP2 is right about the total average and incomplete everywhere else. The
fork's own message agrees that its JP plugin must not be used for results.

## Fail closed

- `jp_separate_taxation_regime_unconfirmed` — tax years from 2027 (JP-R16).
- `missing_cost_basis` — disposals beyond known holdings; no automatic 5 %
  rule (JP-R15).
- `jp_<kind>_requires_review` — airdrops, routing income, unlabelled income
  (JP-R9).
- `jp_lost_requires_review` — lost or stolen coins. No published FAQ answer;
  a casualty loss under 雑損控除 may apply only to some cases (JP-Q8).
- Margin/credit trading rows (JP-R17).
- A non-JPY book refuses. Foreign-currency fiat legs are flagged (JP-R14).

## Open questions for a Japanese tax adviser

- **JP-Q1** Total average and in-year reports: is a provisional mid-year
  figure useful, or should Kassiber only report closed years?
- **JP-Q2** Airdrops: income at market value on receipt (like FAQ 1-7), or
  no income and ¥0 cost (like a fork, FAQ 1-6)? It probably depends on
  whether a market exists and whether anything was done for the coins.
- **JP-Q3** Bridge acquisitions (所令119の2②): which patterns qualify, and
  is a reviewed marker per swap acceptable?
- **JP-Q4** Year-end value carried forward: exact, or rounded to the yen?
  The NTA total-average sheet carries the unrounded cell; the moving-average
  sheet rounds up after each sale.
- **JP-Q5** Network fee paid in the coin on a self-transfer: is the
  revenue-plus-expense treatment (net: minus the fee coins' cost) right, or
  should only the cost be expensed without revenue?
- **JP-Q6** Foreign-currency trades: must Kassiber use TTM rather than its
  JPY market rates? Which TTM source is acceptable?
- **JP-Q7** BTC↔LBTC pegs and Lightning: one coin type (no disposal) or an
  exchange (FAQ 1-3)?
- **JP-Q8** Lost keys, exchange failures, hacks: 雑損控除 (casualty loss),
  雑所得 loss, or nothing?
- **JP-Q9** Return boxes: confirm 第一表 ク (収入金額等 雑・その他) and ⑨
  (所得金額等 雑・その他) for the 令和7年分 form, and the 第二表 entries.
- **JP-Q10** Under the new regime (JP-R16): which cost method applies to
  特定暗号資産, and how are holdings acquired before the 適用開始日 carried in?
- **JP-Q11** The ¥3,000,000 classification line (FAQ 2-2): the year's crypto
  revenue in total, or per coin as the vectors and Kassiber's flag read it?
- **JP-Q12** Kassiber's importers net an exchange's yen commission into the
  sale's value; the NTA sheet lists it as an expense. The income is the
  same up to the sheet's yen rounding of revenue (down) and expenses (up).
  Is the netted form acceptable, or must the commission be shown apart?
- **JP-Q13** Which source should the 70 % check use for a sale's market
  value (Kassiber's rate cache at the sale's time, the exchange's own
  rate)? Until one is chosen, the check runs only on rows that record a
  market value.

## Fonts and language

DM Sans, JetBrains Mono and Liberation Sans have no kana or kanji. With the
owner's approval (2026-11-07) Kassiber bundles **Noto Sans JP 2.004
Regular** (SIL OFL 1.1, `crates/pdf/fonts/NotoSansJP-Regular.otf`, from the
notofonts/noto-cjk release `Sans2.004`, checked against the release tree's
git blob hash; see `crates/pdf/fonts/README.md`). It is the last face of
both font fallback lists, so text the other faces draw is unchanged, and
the PDF engine stays sealed (no system fonts). The full report of a
Japanese book is in Japanese in every format; the CSV keeps machine
numbers and dates, as for every country.
