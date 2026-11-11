# Spain (`es`): crypto tax rules and report specification

**Status:** implemented in the Rust backend as a **preview** country
(2026-11-07). No Spanish tax professional (asesor fiscal) has reviewed it;
every Spanish report and screen carries the preview notice until one does.
Sources were read on 2026-11-07. Nothing here is tax advice.

Test vectors: [tax-country-es-vectors.json](tax-country-es-vectors.json)
(17 cases). A Python script (`scripts/tax-country-vectors.py`) computed them
from the rules below and ran RP2's ES plugin on the same inputs; it was
removed with Python ([plan 22](../plan/22-python-removal.md), D6), so the
file is now a frozen, reviewed golden.

## Summary

| Item | Spain |
| --- | --- |
| Tax | IRPF (personal income tax). Gains from transmitting crypto are *ganancias patrimoniales* in the **base imponible del ahorro** |
| Tax year | Calendar year, Europe/Madrid civil date (see the Canary Islands question) |
| Currency | EUR, required |
| Cost basis | **FIFO** per coin type, one pool per taxpayer across every exchange and wallet. No other method is allowed |
| Holding period | None: no short/long distinction, no exemption by holding time |
| Allowance | None for gains. The savings scale starts at 19 % from the first euro |
| Crypto-to-crypto | Taxable *permuta*; transmission value = the **higher** of the two market values |
| Income | Staking → capital-mobiliario yield (ahorro, casilla 0027); airdrops and hobby mining → general-base gain (casilla 0304); lending interest → capital-mobiliario yield (casilla unconfirmed) |
| Losses | Net against gains in the year; a net loss offsets ≤ 25 % of positive capital-mobiliario yields, then carries forward four years. Gift losses are never computable |
| Form | Modelo 100 (Renta), *Ganancias y pérdidas patrimoniales / Monedas virtuales*, entry at casilla 1804 |
| Report language | Spanish (`es-ES` string table, `1.234,56`, `15/03/2025`) |

## Sources

All retrieved 2026-11-07.

- **[ES-S1]** Ley 35/2006 del IRPF, consolidated text (last update published
  07/10/2026), BOE-A-2006-20764:
  <https://www.boe.es/buscar/act.php?id=BOE-A-2006-20764>. Articles used:
  14 (timing), 25.2 (capital-mobiliario yields from lending), 33
  (definition, non-computable losses), 34–36 (amount, onerous and gratuitous
  transmissions), 37.1.h and 37.2 (permuta; FIFO for homogeneous securities),
  46 (savings income), 48–49 (integration and compensation), 66/76
  (savings scale as amended by Ley 7/2024, BOE-A-2024-26694, from
  1 January 2025).
- **[ES-S2]** AEAT, *Manual práctico de Renta 2025*, chapter 11, "Compra y
  venta de monedas virtuales: tributación en el IRPF del inversor":
  <https://sede.agenciatributaria.gob.es/Sede/ayuda/manuales-videos-folletos/manuales-practicos/irpf-2025/c11-ganancias-perdidas-patrimoniales/monedas-virtuales/compra-venta-monedas-virtuales-tributacion-inversor.html>,
  and the PDF Part 1 dated 20/05/2026, pages 941 and 947:
  <https://sede.agenciatributaria.gob.es/static_files/Sede/Biblioteca/Manual/Practicos/IRPF/IRPF-2025/ManualRenta2025Parte1_es_es.pdf>.
- **[ES-S3]** AEAT, *Informador de Renta 2025*, 9.2.2 "Venta y otras
  operaciones con monedas virtuales":
  <https://www2.agenciatributaria.gob.es/wlpl/AVAC-CALC/InformadorRenta2025?id=9.2.2>.
  It names casillas 1804, 0304 and 0027, has a worked FIFO example, and
  cites DGT binding rulings V1149-18 and V1604-18 (FIFO), V0525-25
  (homogeneous units), V0999-18 (permuta), V1579-22 (platform failure),
  V0808-18 (timing), V1766-22 (staking), V0213-23 and V0376-24. The rulings
  were not read individually. The page says it stopped being updated on
  30 June 2026.
- **[ES-S4]** AEAT, Renta WEB help, "Ganancias y pérdidas patrimoniales
  derivadas de la transmisión o permuta de monedas virtuales":
  <https://sede.agenciatributaria.gob.es/Sede/Ayuda/25Presentacion/100/7_6_6_2/ganancias_perdidas_monedas_virtuales.html>.
- **[ES-S5]** Secondary sources on art. 33.5 for crypto, used only to frame
  open question ES-Q2: PwC *Periscopio Fiscal*
  (<https://periscopiofiscalylegal.pwc.es/la-agencia-tributaria-y-la-fiscalidad-de-las-criptomonedas/>)
  and others report that no DGT ruling settles whether art. 33.5.e applies.

## Rules

Each rule states what the law says and what Kassiber does about it.
"Fail closed" means the row is quarantined with the named reason and
nothing is guessed.

**ES-R1 Taxable events.** Every change in the composition of a person's
wealth is a gain or loss unless the law says otherwise (art. 33.1
[ES-S1]). For crypto this covers:

- selling for euros or another legal currency [ES-S2 a];
- exchanging for another crypto (*permuta*, art. 37.1.h) [ES-S2 b];
- paying for goods or services, which is a permuta of the coins for the
  goods (art. 37.1.h);
- gifting (an *inter vivos* gratuitous transmission, art. 36; see ES-R8).

A move between the taxpayer's own wallets is not a transmission. Buying
and holding are not declared [ES-S3]. Kassiber keeps its existing transfer
and custody proof; this spec changes no pairing rule.

**ES-R2 Gain.** The gain is the transmission value minus the acquisition
value (art. 34.1.a). The acquisition value is the price paid plus the
expenses and taxes inherent in the acquisition paid by the buyer (art.
35.1). The transmission value is the price received minus the expenses
paid by the seller (art. 35.2). If the price is below normal market value,
the market value applies (art. 35.2, last paragraph) [ES-S2 a]. Only
expenses directly related to the operation and paid by the taxpayer count
[ES-S2 a].

**ES-R3 FIFO across all wallets.** When homogeneous units bought at
different times and prices are partly sold, the first acquired are the
first transmitted [ES-S2 a, ES-S3]. AEAT applies this "even if the
purchases and sales took place on different exchanges" [ES-S4], so the
pool is one per coin type per taxpayer. Kassiber: method `fifo` only, pool
scope `global` only. LBTC and BTC are separate coin types unless a reviewed
carrying-value pair links them (open question ES-Q6).

**ES-R4 Permuta value.** In a permuta the transmission value is the higher
of the market value of the coins given and the market value of what is
received (art. 37.1.h). The received coins take that same value as their
acquisition value [ES-S2 b]. Market value is the price independent parties
would agree at the moment of the exchange [ES-S2 b]. Kassiber prices both
legs of a cross-asset taxable pair from its rates, as today. For ES the
engine sets the disposal's transmission value **and** the received lot's
basis to `max(out_value, in_value)`. A payment for goods uses the goods'
price when the row carries one (`consideration_value`, new reviewed
metadata). Otherwise it uses the coins' market value, with a note that the
higher value could not be checked.

**ES-R5 Savings base and form.** Gains and losses from transmitting
elements of wealth are savings income (art. 46.b) [ES-S2]. They are
declared under *Ganancias y pérdidas patrimoniales / Monedas virtuales*
via casilla 1804, per coin, with the type of consideration received
[ES-S3, ES-S4].

**ES-R6 Integration and compensation (art. 49).** Gains and losses of the
year are netted among themselves. A negative balance offsets the positive
balance of capital-mobiliario yields of the same year up to 25 % of that
balance. What remains carries forward to the four following years in the
same order: first against net gains, then against up to 25 % of yields. A
negative yield balance offsets up to 25 % of net gains the same way. Each
compensation must use the maximum amount the year allows (art. 49.2).
Kassiber computes this **crypto only**: the yields it knows are crypto
staking (casilla 0027) and crypto lending interest. The section is labelled
indicative, because dividends, bank interest and other gains of the
taxpayer change it. Unexpired crypto loss balances are listed by year of
origin, with the last year they can be used.

**ES-R7 Income from holding or creating coins.**

| Kassiber kind | Treatment | Base | Casilla | Source |
| --- | --- | --- | --- | --- |
| `staking` | capital-mobiliario yield at market value when received | ahorro | 0027 | [ES-S3], DGT V1766-22 |
| `lending_interest`, `interest` | capital-mobiliario yield (art. 25.2, a loan of own capital) | ahorro | unconfirmed (ES-Q4) | [ES-S1] art. 25.2 |
| `airdrop` | gain not derived from a transmission, at market value of the coins received | general | 0304 | [ES-S3] |
| `mining` (no organised means) | gain not derived from a transmission | general | 0304 | [ES-S3] |
| `mining` with organised material/human means, any business activity | economic activity: out of scope; needs an explicit review mark and then fails closed `es_business_activity_unsupported` | general | — | [ES-S3] |
| `hardfork`, `routing_income`, unlabelled `income`, any other inbound kind | no settled treatment: fail closed `es_<kind>_requires_review` (e.g. `es_hardfork_requires_review`) | — | — | — |
| `wages` | employment income in kind: outside Kassiber; the coins enter at the reviewed EUR value as basis (as today) | — | — | — |

Received coins always take the market value counted as income as their
acquisition value (art. 37.1.l for incorporations; art. 25 valuation in
kind).

**ES-R8 Gifts.** A gift is a gratuitous transmission. The transmission
value is the Inheritance and Gift Tax value, which may not exceed market
value (art. 36). A gain is taxed. A loss from an *inter vivos* gratuitous
transmission is **not computable** (art. 33.5.c). A gain on a donation to
an entity of art. 68.3 (Ley 49/2002 charities) is exempt (art. 33.4.a).
Kassiber: an outbound row reviewed as `gift` is a disposal at market value;
a negative result is shown with `computable = false` and excluded from
casilla 1804. A donation to a qualifying entity needs reviewed metadata
naming the entity type and fails closed (`es_donation_requires_review`)
until then. Today the engine quarantines every gift/donation kind
(`non_sale_disposal_kind`), so ES needs a country hook that replaces that
quarantine for reviewed gifts.

**ES-R9 Non-computable losses.** Not computed: losses that are not
justified (33.5.a), losses due to consumption (33.5.b), gift losses
(33.5.c) and losses on a transmission where the transmitter reacquires
the same element within a year (33.5.e). The two-month and one-year rules
of 33.5.f/g apply to *valores* and are generally read as not applying to
crypto, which is not a security [ES-S5]. Whether 33.5.e applies to
fungible coins is unsettled (ES-Q2). Kassiber computes the loss by default
and flags every loss sale followed by a same-asset acquisition within
366 days (`es_33_5_e_reacquisition_review`). It never defers a loss on its
own.

**ES-R10 Lost coins and failed platforms.** A platform failure or fraud
loss is a *general-base* loss, imputable only when the claim becomes
judicially uncollectible (art. 14.2.k) [ES-S2 c, ES-S3]. Filing a
complaint is not enough. Lost or stolen keys have no published treatment.
Kassiber fails closed (`es_lost_requires_review`) and books neither a loss
nor a disposal.

**ES-R11 Timing.** A gain is imputed when the coins are delivered,
whatever the payment date (art. 14; DGT V0808-18) [ES-S2, ES-S3]. The
tax year is the calendar year. Kassiber takes the civil date in
Europe/Madrid. A transaction at 2025-12-31T23:30Z is 1 January 2026 in
Madrid and belongs to 2026 (vector es-14).

**ES-R12 Fees paid in crypto.** A fee paid in the coin itself is a
transmission of those units for a service (a permuta, ES-R4). On a sale,
the fee units are delivered with the sold units, and the fee's value is a
deductible expense of that sale. The two effects cancel, so Kassiber
reports **one** row: transmission value = gross price, acquisition value =
FIFO cost of the sold **and** fee units (vector es-11). A network fee on a
move between own wallets is a stand-alone disposal of the fee units at
market value, with no expense to deduct (vector es-10). Whether such
micro-disposals must be declared row by row is open question ES-Q3.

**ES-R13 Currency.** Amounts are in EUR. A price paid in another currency
is converted at the exchange rate of the transaction date. The Kassiber
book currency must be EUR, and every row needs an EUR value from the rates
cache or reviewed pricing, as today. A non-EUR fiat leg carries a note.

**ES-R14 Rounding.** The engine computes exactly (32 significant digits).
Each disposal row's transmission value and acquisition value are rounded
half-up to the cent. The row's gain is the difference of the rounded values,
and year totals are sums of rounded rows, so the form total equals its
rows. This is a Kassiber convention (ES-Q5); no AEAT rule was found.

**ES-R15 Year-end holdings.** For information only: holdings at
31 December at market value, per coin and custody location. Wealth tax
(Modelo 714), the Solidarity Tax on Large Fortunes and the foreign-crypto
information return (Modelo 721) use this valuation date. Kassiber reports
the figures but prepares none of these forms.

## Implementation

What Kassiber does today, and where (2026-11-07).

| Piece | Where | Notes |
| --- | --- | --- |
| Registry entry | `crates/tax-countries/src/lib.rs` (`SPAIN`) | `es`, preview, EUR required, `FIFO` only, `global` pool only, no holding period, Europe/Madrid, calendar year, names Spain / Spanien / España, the four ES-S sources, generators `open_positions` and `es.irpf_monedas_virtuales` |
| Engine rules | registry `engine`, sent as the `rules` country (`docs/reference/tax-engine.md#rules-countries`) | `fee_units: disposal` (ES-R12), `swap_values: higher` (ES-R4), gifts given at market value with losses not deductible (ES-R8), `below_market: market_value` (ES-R2), and the refusals below |
| Refusals | engine `refused_receipts` (the shared refusal path, [rules countries](tax-engine.md#rules-countries)) | inbound `hardfork`/`hard_fork`, `routing_income`, `income` are held as `acquisition_valuation_unsupported`; outbound kinds containing `lost`, `stolen`, `theft` or `donat` (`direction: outbound`) as `non_sale_disposal_kind`; each before pricing, with Spain's code in the quarantine detail |
| Seller's expenses | the sale's value | a Kassiber transaction has one fiat value: an exchange's euro commission is netted into it, as for every country, which gives the same transmission value (es-01) |
| Journal facts | `journal_entries.calculation_json` (`loss_deductible`, `valuation` beside the fragments; a fragment's `expenses_exact` as the tip stores them) | present only on entries that have them, so every other book keeps its stored bytes |
| Country layer | `crates/reports/src/es_tax.rs` (figures), `crates/reports/src/report_design/es.rs` (sections) | casilla 1804 annex and totals per coin and contraprestación, income casillas, the indicative integration (ES-R6) with the carry-forward table, and the review flags |
| Strings | `crates/reports/src/report_design/strings_es.rs` | the shared report strings and Spain's layer in Spanish; another country's layer keys fall back to English and never render for a Spanish book |
| Calendar | registry Europe/Madrid; report readers through `kassiber_reports::country::zone`, `calendar::local` and `export_range::book_zone` | export periods, the generic tax summary, the desktop review years and the dashboard's accounting years use Madrid dates; see the open items for the remaining readers |
| Vectors | `crates/reports/tests/country_vectors/es_jp.rs` (`es_compute`) | all 17 pass through the `ledger` operation under the registry's rules and the layer above |
| End to end | none in Rust yet | the Python black-box test (`tests/test_rust_es_tax_country.py`: a CLI-built book through journals to the Spanish PDF, XLSX and CSV, repeat PDFs byte-identical, the review quarantines) went with Python ([plan 22](../plan/22-python-removal.md)) |

### Using it

- CLI: `kassiber profiles create --workspace <ws> --tax-country es <name>`
  sets EUR, FIFO and the global pool; any other method or scope is
  refused with a hint. `journals process` books the rules above;
  `reports export-pdf|export-xlsx|export-csv` write the Spanish full report
  with the sections *España: ganancias y pérdidas patrimoniales* (sheet
  and CSV `es_transmisiones`, `es_casilla_1804`, `es_casilla_1804_totals`),
  *España: rendimientos* (`es_rendimientos`, `es_casillas_rendimientos`),
  *Integración del ahorro* (`es_integracion_ahorro`, `es_perdidas_pendientes`)
  and *Puntos para revisar* (`es_revision`).
- Desktop: Spain appears in onboarding and the Books picker with the
  **Preview** badge from the registry (`ui-tauri/src/lib/taxCountries.generated.ts`).

### Not implemented (fail closed or documented)

- A sale's market value apart from its price: the engine applies ES-R2
  when a row carries `market_value`, but no importer or screen records one
  yet, so a below-market sale is taxed at its recorded price (vector es-12
  exercises the rule through the engine).
- `consideration_value` for payments (ES-R4): a payment's recorded value is
  its consideration; the check against the coins' market value needs the
  same market value as above.
- Business activity (`es_business_activity_unsupported`): Kassiber has no
  review mark for organised mining, so mining is always the hobby
  treatment (0304); a reviewer must exclude business mining by hand.
- Donations to Ley 49/2002 entities: every donation is quarantined
  (`es_donation_requires_review`).
- Year-end holdings (ES-R15) come from the shared Holdings section at the
  report's period end, not a Spain-specific table.
- The year-handoff workbook, the capital-gains CSV, the cross-check and the
  exit-tax estimate still cut years in UTC for Spanish books (only Austria
  has its own calendar there).

### Refusals

| Case | Code |
| --- | --- |
| lost, stolen or theft outflow | quarantine `es_lost_requires_review` |
| hard fork receipt | quarantine `es_hardfork_requires_review` |
| routing income | quarantine `es_routing_income_requires_review` |
| unlabelled income | quarantine `es_income_requires_review` |
| donation | quarantine `es_donation_requires_review` |
| non-EUR currency, other method or scope | profile validation refuses it |
| a quarantined row | the readiness barrier blocks the export, as for every country |

## Kassiber mapping

| Kassiber row | ES event |
| --- | --- |
| inbound purchase (`buy`, unlabelled deposit with value) | FIFO lot at EUR value + EUR fees |
| outbound sale to fiat | disposal, `contraprestacion = moneda_curso_legal` |
| cross-asset taxable pair (BTC↔other) | disposal of the out leg at `max(out, in)`, lot of the in leg at the same value, `otra_moneda_virtual` |
| outbound payment (`spend`) | disposal, `otro_bien_o_servicio` |
| reviewed `gift` | disposal at market value, `sin_contraprestacion`, losses not computable |
| self-transfer with fee | disposal of the fee units (`network_fee`) |
| `lost`, `stolen`, `donation`, `hardfork`, unlabelled `income` | quarantine (reasons above) |
| BTC↔LBTC carrying-value pair | carry basis, no disposal (as generic books today; ES-Q6) |

## Report

The report follows the [shared report design](report-design.md). Spanish
country layer:

- **Cover/method:** method "FIFO (art. 37.2 LIRPF por analogía; DGT
  V1149-18)"; currency EUR; timezone Europe/Madrid; the preview notice;
  "Kassiber no presenta la declaración".
- **Disposals** (shared columns, plus at the end): `contraprestacion`
  (moneda de curso legal / otra moneda virtual / otro bien o servicio / sin
  contraprestación), `valor_transmision`, `valor_adquisicion`, `computable`.
  The shared `proceeds`/`cost_basis`/`gain` columns hold the same values.
- **Income:** shared columns plus `casilla` (0027, 0304, or `—` with the
  reason) and `base` (ahorro/general).
- **Casilla 1804 section** (after Income), one row per coin and type of
  consideration: coin name, type of consideration, sum of transmission
  values, sum of acquisition values, net gain or loss. Then totals:
  computable gains, computable losses, net balance. Non-computable gift
  losses are listed separately. Whether AEAT expects per-operation rows is
  ES-Q1; the per-disposal list stays in Disposals either way.
- **Other casillas:** 0027 total (staking), lending yields total (casilla
  pending ES-Q4), 0304 total (airdrops, hobby mining).
- **Integración del ahorro (indicative, crypto only):** the ES-R6
  computation: G/L balance, 25 % offset, prior losses applied, base,
  carry-forward table (year of origin, amount, last usable year).
- **Year-end holdings:** ES-R15 figures.
- **Issues:** quarantines and the review flags (`es_33_5_e_reacquisition_review`,
  `es_price_below_market`, `es_gift_loss_not_computable`,
  `es_lending_casilla_unconfirmed`).
- **No indicative tax.** The savings scale is progressive (2025 onwards:
  19 % to 6,000; 21 % to 50,000; 23 % to 200,000; 27 % to 300,000; 30 %
  above, art. 66 + 76 as amended by Ley 7/2024). It applies to the whole
  savings base, which Kassiber does not know. A crypto-only estimate would
  mislead, so the report prints the scale for reference only.

Formats: Spanish numbers `1.234,56 €`, dates `15/03/2025` in PDF/XLSX; the
CSV stays machine format. Spanish needs a new string table (`es`) and a
locale with the same digit grouping as German. Until a reviewed table
exists, labels are English, and the official form names (casilla numbers
and their Spanish captions) appear verbatim.

## Registry defaults

| Field | Value |
| --- | --- |
| `tax_country` | `es` |
| `preview` | `true` |
| `fiat_currency` | `EUR` (required; other currencies refused) |
| `accounting_methods` | `fifo` |
| `default_accounting_method` | `fifo` |
| `cost_basis_pool_scopes` | `global` |
| `long_term_days` | none (no holding-period distinction; use `sys.maxsize` as RP2 does for IE so every row reads short) |
| `timezone` | `Europe/Madrid` (new field; Austria uses Europe/Vienna and generic books UTC today) |
| `tax_year` | calendar |
| `generation_language` | `es` (interim `en` with Spanish form labels) |
| `report_generators` | shared full/summary reports + `es_irpf_monedas_virtuales` |
| income categories | per ES-R7 table |
| `cross_asset_carrying_value` | BTC↔LBTC only, as generic |
| sources | ES-S1 … ES-S4 with retrieval date 2026-11-07 |

RP2's ES plugin agrees on currency, FIFO-only and language. It sets
`long_term_capital_gain_period = 365`, which has no meaning in Spanish law.
Kassiber must not print a short/long term column for ES.

## Test vectors

`book` gives currency, method and timezone. Event types: `buy`
(`fiat` = price, `fiat_fee` = EUR commission), `sell` (`fiat` = gross
price, `fiat_fee` = EUR commission, `crypto_fee`/`fee_value` = a fee paid
in the coin, `market_value` = market value when it differs from the
price), `swap` (`out_*`/`in_*` with each leg's market value), `spend`
(`fiat` = value of the goods, `market_value` = the coins' value), `income`
(`income_kind`, `fiat` = market value), `transfer` (`qty` arriving, `fee`
units and `fee_value`), `gift_out` (`fiat` = market value), `lost`.
Quantities are decimal asset units, exact in msat. Results: `disposals`
(with FIFO `lots`), `income`, `quarantines`, `flags`, `years`. The same
format is used by the IE and JP vectors.

Step by step (amounts in EUR):

- **es-01** (AEAT informador example; ES-R2/R3/R14). Lots: t1 0.09 BTC at
  3,000 + 60 = 3,060; t2 0.22 BTC at 6,000 + 120 = 6,120. Sale t3 0.22 BTC
  for 14,046.32 less 280 commission → transmission value 13,766.32. FIFO
  takes t1 entirely (3,060.00) and 0.13 of t2: 6,120 × 0.13 / 0.22 =
  3,616.3636… → 3,616.36. Acquisition value 3,060 + 3,616.3636… =
  6,676.3636… → 6,676.36. Gain 13,766.32 − 6,676.36 = **7,089.96**. AEAT
  prints 6,673.36 and 7,092.96, a 3-euro addition slip in the source.
- **es-02** (ES-R3). Lots t1 1 BTC 40,000 (Exchange), t2 1 BTC 60,000
  (Hardware). Selling 1 BTC from Hardware still takes t1 under global FIFO:
  70,000 − 40,000 = **30,000** (per-wallet FIFO would give 10,000).
- **es-03** (ES-R4). t1 1 BTC at 20,000. Permuta t2: 0.5 BTC (worth 30,000)
  for 10 ETH (worth 30,600). Transmission value max(30,000, 30,600) =
  30,600; cost 0.5 × 20,000 = 10,000; gain **20,600**. ETH basis 30,600.
  t3 sells 10 ETH for 31,000: gain **400**. 2025: 21,000.
- **es-04** (ES-R4). t1 1 BTC at 50,000. Permuta t2: 1 BTC (worth 50,500)
  for 2,000 SOL (worth 48,000): transmission value 50,500, gain **500**,
  SOL basis **50,500** (not 48,000). t3 sells the SOL for 49,000: loss
  **−1,500**. Balance −1,000, no yields to offset, carried forward from 2025.
- **es-05** (ES-R1/R4). t1 0.1 BTC at 2,000. t2 pays goods priced 2,500
  with 0.05 BTC worth 2,450: transmission value max = 2,500; cost 1,000;
  gain **1,500**.
- **es-06** (ES-R8). BTC lot 30,000; ETH lot 3,000. Gift of 0.5 BTC worth
  40,000: 40,000 − 15,000 = **25,000** (computable). Gift of 1 ETH worth
  2,000: 2,000 − 3,000 = −1,000, **not computable**. Casilla 1804: gains
  25,000, losses 0; non-computable 1,000.
- **es-07** (ES-R6/R7). Staking 0.5 ETH at 1,500 → casilla 0027 1,500, ETH
  basis 1,500. Sale for 1,200: −300. Integration: G/L −300; the 25 % limit
  is 375, so all 300 offset; base 1,500 − 300 = **1,200**; nothing carried.
- **es-08** (ES-R7). Airdrop 100 XYZ at 300 and mining 0.001 BTC at 60 →
  casilla 0304 **360** (base general); bases 300 and 60. XYZ sold for 500:
  gain **200** (casilla 1804).
- **es-09** (ES-R7). Lending interest 0.002 BTC at 120 → capital-mobiliario
  yield 120 (ahorro), casilla unconfirmed, flagged. Base 120.
- **es-10** (ES-R12). t1 1 BTC at 30,000. A self-transfer pays a
  0.0001 BTC fee worth 6.00. Fee disposal: 6.00 − 0.0001 × 30,000 = 6.00
  − 3.00 = **3.00**. The 0.5 BTC moved is not a disposal.
- **es-11** (ES-R12). Lots 1 BTC 20,000 (2023) and 1 BTC 40,000 (2024).
  Sale of 1 BTC for 60,000 plus a 0.001 BTC exchange fee worth 60: one row
  of 1.001 BTC, transmission value 60,000, acquisition value 20,000 +
  0.001 × 40,000 = 20,040, gain **39,960**. The same as a sale of 1 BTC
  (59,940 net of the fee's value, cost 20,000) plus a fee permuta (60, cost
  40).
- **es-12** (ES-R2). 0.1 BTC at 2,000 sold for 3,000 when worth 6,000:
  transmission value 6,000, gain **4,000**, flag `es_price_below_market`.
- **es-13** (ES-R9). 1 BTC at 60,000 sold for 40,000 (loss −20,000), and
  1 BTC bought back a month later. The loss is computed by default and
  flagged for art. 33.5.e. It is carried forward from 2025.
- **es-14** (ES-R11). 0.2 BTC at 10,000 sold at 2025-12-31T23:30Z for
  18,000. Madrid date 2026-01-01, so the gain **8,000** is 2026's; 2025
  has no disposal.
- **es-15** (ES-R6). 2024: BTC lot 60,000; staking 1,000 (casilla 0027);
  0.5 BTC sold for 25,000 → −5,000. The 25 % limit is 250, so 250 offsets
  the yields (base 750) and 4,750 carries forward (usable to 2028). 2025:
  0.5 BTC sold for 45,000 → +15,000; prior loss 4,750 applied; base
  **10,250**.
- **es-16** (ES-R10). Lost 0.1 BTC → quarantine `es_lost_requires_review`,
  no disposal.
- **es-17** (ES-R7). BCH from the 2017 fork → quarantine
  `es_hardfork_requires_review`.

## RP2 comparison

RP2's ES plugin (pinned fork `bitcoinaustria/rp2@3d2e03ef`) offers FIFO, EUR,
Spanish and a 365-day long-term period. It has no Spanish report generator:
only `open_positions` and `rp2_full_report`. Its numbers on the vectors (in
`rp2.computed`, explained in `rp2.differences`) differ from this spec in
these ways:

| Rule | RP2 | Spec | Vectors |
| --- | --- | --- | --- |
| Sale commission in EUR (art. 35.2) | ignored: an `OutTransaction.fiat_fee` never reaches proceeds | deducted | es-01 (+280) |
| Permuta at the higher value (art. 37.1.h) | each leg at its own value | higher value for the disposal and the new lot | es-03, es-04, es-05 |
| Sale below market (art. 35.2) | agreed price | market value | es-12 |
| Gift loss (art. 33.5.c) | counted | not computable | es-06 |
| Lost coins | full capital loss | quarantine | es-16 |
| Hard fork | income at FMV | quarantine | es-17 |
| Tax year | year in the timestamp's own offset | Europe/Madrid civil year | es-14 |
| Income bases and casillas, 25 % offset, four-year carry-forward | absent (income is a taxable event in the gain list) | ES-R6/R7 | es-07…09, es-15 |

Disposal rows agree everywhere else (es-02, es-10, es-13, and es-11's total).
RP2 is incomplete for Spain rather than wrong about FIFO.

## Fail closed

- `es_lost_requires_review` — lost, stolen or platform-failure outflows (ES-R10).
- `es_<kind>_requires_review` (hard forks, routing income, unlabelled
  income) and `es_business_activity_unsupported` — receipts without a
  settled classification (ES-R7).
- `es_donation_requires_review` — donations to entities that might be
  exempt (ES-R8).
- A book whose currency is not EUR, or a row without an EUR value, refuses
  as today's pricing quarantines do.
- A year whose casilla 1804 depends on a quarantined row is incomplete; the
  readiness barriers block the export as for every country.

## Open questions for a Spanish tax adviser

- **ES-Q1** Casilla 1804: does AEAT expect one entry per disposal or one per
  coin and type of consideration? Kassiber plans aggregated entries plus the
  full annex.
- **ES-Q2** Art. 33.5.e (reacquisition within a year): does it apply to
  fungible crypto? If it does, a deferred loss joins the gain on the later
  transmission of the reacquired coins, which needs explicit lot tracking.
  Kassiber only flags.
- **ES-Q3** Network fees on own transfers: must each be declared as a
  permuta, and is a de-minimis practice accepted? Kassiber books them (ES-R12).
- **ES-Q4** Casilla for crypto lending interest (art. 25.2): 0027 or another
  box?
- **ES-Q5** Rounding: is half-up to the cent per row acceptable, and should
  totals sum the rounded rows?
- **ES-Q6** BTC↔LBTC (Liquid peg) and Lightning: one coin type for FIFO, or
  a permuta at each peg? Generic books carry basis by default.
- **ES-Q7** Canary Islands taxpayers: should their year boundary be
  Atlantic/Canary? The one-hour difference only matters at midnight on
  31 December.
- **ES-Q8** Hard forks and airdrops not linked to a service: confirm the
  0304 treatment for airdrops and give a treatment for forks.
- **ES-Q9** Permuta valuation when the market value of the coins received
  cannot be established (illiquid tokens): fall back to the coins given?
- **ES-Q10** Self-custody exchange fees charged in EUR by a broker on
  crypto-to-crypto swaps: added to acquisition value of the received coins
  or deducted from the permuta's transmission value?
- **ES-Q11** Which source should the art. 35.2 check use for a sale's
  normal market value (Kassiber's rate cache at the sale's time, a reviewed
  value)? Until one is chosen, the check runs only on rows that record a
  market value apart from the price.
- **ES-Q12** Interest from crypto lending (`interest`, `lending_interest`)
  is booked as art. 25.2 yield; is an `interest` receipt from other
  sources ever a different income?

## Fonts and language

DM Sans, JetBrains Mono and Liberation Sans cover every Spanish character
checked (á é í ó ú ñ ü ¿ ¡ º ª €; checked against the fonts' `cmap` tables
on 2026-11-07). No new font is needed. The missing piece is a reviewed
Spanish string table.
