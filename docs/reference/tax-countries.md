# Tax countries

**Status:** the framework is in place in the Rust backend (2026-11-07).
`generic` and `at` run on it unchanged; new countries are Rust-only and
labelled **preview** until a tax professional for the country has reviewed
them. Per-country specifications live beside this page
(`tax-country-<code>.md`).

## The registry

`crates/tax-countries` (`kassiber-tax-countries`) holds one entry per stored
`tax_country` value. It is the only place that knows a country code; every
other crate asks it a question:

| Field | Meaning | `generic` | `at` |
| --- | --- | --- | --- |
| `code`, `iso`, names | stored value, ISO 3166-1 code, English and German name | `generic`, none | `at`, `AT` |
| `preview` | not yet reviewed by a tax professional for the country | no | no |
| `rules` | the legal rules family the engine and the country layer apply | `Generic` | `Austria` |
| `fiat` | the profile's currency, or the one the country requires | profile | `EUR` |
| `long_term` | the holding-period rule | `ProfileDays` (365) | `Never` |
| `methods`, `default_method` | accounting methods a profile may choose | FIFO, HIFO, LIFO, LOFO, MOVING_AVERAGE; FIFO | those and MOVING_AVERAGE_AT; MOVING_AVERAGE_AT |
| `pool_scopes`, `default_pool_scope` | cost-basis pool scopes (`global`, `wallet`) | global | global |
| `locale` | the reports' string table and formats | `en` | `de-AT` |
| `timezone`, `tax_year_start` | the IANA zone whose local dates cut tax years and count holding periods, and the tax year's first day | UTC, 01-01 | Europe/Vienna, 01-01 |
| `cross_asset_carry` | reviewed carrying-value pairs of unlike assets carry basis | no (only the BTC/LBTC rail) | yes |
| `rail_decision_required` | a BTC/LBTC rail change needs its own policy: the book's `bitcoin_rail_carrying_value` default does not decide it (`transfers pair` refuses without `--policy`; native and reviewed rail pairs without one are held `rail_policy_required`) | no | no |
| `report_generators` | the reports the country offers | `open_positions`, `rp2_full_report` | `open_positions`, `at.e1kv`, `at.exit_tax` |
| `income_categories` | inbound kinds booked as income at their value | RP2's earn types | RP2's earn types |
| `sources` | legal sources with retrieval dates | none | BMF |
| `tax_tables` | the dated rate, threshold and allowance tables the country's layer applies (`tables.rs`, read through `years.rs`) | none | none |
| `engine` | shared tax-engine rules the country is computed with instead of its family's engine country ([rules countries](tax-engine.md#rules-countries)): fee units, swap values, gifts, refused receipts (inbound or outbound, each with the country's code), zero-cost kinds, and the below-market valuation | none | none |
| `name_local` | the name in the country's own report language when that is neither English nor German (`España`, `日本`); only reports read it | none | none |

The `generic` and `at` values are Python's (`kassiber/tax_policy.py`); a
test in the crate pins them. The United States (`us`, preview) has `rules =
UnitedStates`, USD, `MoreThanOneYear` (held more than one year by UTC
calendar date), FIFO, HIFO, LIFO and LOFO with FIFO the default, the pool
scopes `global` and `wallet` with `wallet` the default, `en`, UTC, the
calendar tax year, no cross-asset carry, the generators `open_positions`
and `us.form_8949`, and the IRS sources of its
[specification](tax-country-us.md).

Germany (`de`, preview) has `rules = Germany`, EUR, `MoreThanOneYear` in
Europe/Berlin (§ 23 EStG: taxable while not long-term), FIFO and
`MOVING_AVERAGE_FIFO_PERIOD` with FIFO the default, the `wallet` scope only,
`de-DE`, `rail_decision_required`, the generators `open_positions` and
`de.anlage_so`, the § 23 and § 22 Nr. 3 Freigrenzen, and engine rules
(`fee_units: disposal_before_sale`, `swap_values: received_given`, received
gifts refused, airdrops and hard forks refused with their codes)
([specification](tax-country-de.md#implementation)). Switzerland (`ch`,
preview) has `rules = Switzerland`, CHF, `Never` in Europe/Zurich (private
gains are tax-free), FIFO and MOVING_AVERAGE, the `global` scope only,
`de-CH`, the generators `open_positions` and `ch.wealth_income`, and engine
rules that refuse hard forks
([specification](tax-country-ch.md#implementation)).
Ireland (`ie`, preview) has `rules = Ireland`, EUR, no holding period,
FIFO only, the `global` pool scope only, Europe/Dublin, the calendar year,
the generators `open_positions` and `ie.cgt`, the `ie.cgt_rate` and
`ie.annual_exemption` tables, and engine rules: fee units as disposals, a
swap valued at the received coins, gifts at market value, and airdrops,
hard forks, routing income and unlabelled income refused with Ireland's
codes. The United Kingdom (`gb`, preview) has `rules = UnitedKingdom`, GBP,
no holding period, the one method `UK_SHARE_MATCHING`, the `global` pool
scope only, Europe/London, tax years from 6 April, `rail_decision_required`
(as Germany), the generators `open_positions` and `gb.sa108`, the AEA, proceeds-threshold and main-rate
tables, and engine rules: fee units taken before the sale, swaps at the
received value with the given value as the new cost, and airdrops, hard
forks and unlabelled income refused with the UK's codes (gifts stay
quarantined: their recipient class is not recorded). Their specifications
([Ireland](tax-country-ie.md), [United Kingdom](tax-country-gb.md)) record
what is implemented and what is open.
Spain (`es`, preview) has `rules = Spain`, EUR, `Never`, FIFO only, the
`global` scope only, `es-ES`, Europe/Madrid, the calendar tax year, the
generators `open_positions` and `es.irpf_monedas_virtuales`, and engine
rules: fee units as disposals, the higher swap value, gifts at market value
with non-deductible losses, a sale below market value at market value, and
hard forks, routing income and unlabelled income refused on receipt and
lost, stolen and donated coins on their way out, each with Spain's code
([specification](tax-country-es.md#implementation)). Japan (`jp`, preview)
has `rules = Japan`, JPY, `Never`, `JP_TOTAL_AVERAGE` (default) and
`JP_MOVING_AVERAGE`, the `global` scope only, `ja-JP`, Asia/Tokyo, the
calendar tax year, the generators `open_positions` and
`jp.crypto_keisansho`, the `jp.tax_regime` table, and engine rules: fee
units as disposals, the received swap value, gifts and donations at market
value, hard forks at zero cost, the 70 % rule, and airdrops, routing income
and unlabelled income refused on receipt and lost and stolen coins on their
way out, each with Japan's code
([specification](tax-country-jp.md#implementation)).

Callers keep the reference's own reading of the stored value (where Python
strips, where it lower-cases, where it compares literally) and look the
normalized code up. An unknown code has no entry, so it is refused where
Python refused it, with the same error. Where Python's `"at"` branch meant
one of several things, the caller now asks the matching question: the
Vienna calendar (`timezone`), German reports (`locale`), cross-asset carry
(`cross_asset_carry`), the holding period (`long_term`), or Austria's own
rules (`rules == Austria`: Alt/Neu, Kennzahlen, the E 1kv, the exit tax's
regimes). For `generic` and `at` every answer equals the old branch, so
numbers, stored rows and exports are unchanged; the parity suites check it.

### What reads it

- Profile create and update (`crates/setup/src/profiles.rs`): allowed and
  default methods and pool scopes, the required currency, the stored
  `tax_long_term_days`, refusals.
- The desktop book snapshot (`ui.profiles.snapshot`): the policy label,
  `allowedCostBasisPoolScopes`, the workspace jurisdiction, and
  `taxCountryPreview: true` for a preview country (absent otherwise, so
  reviewed books keep the reference's projection). The AI tool
  `ui.profiles.snapshot` reads the same projection.
- The tax engine request (`crates/custody/src/ledger_request/config.rs`): the
  engine country, the method gate and the currency check. Its `book_currency`
  (the country's `fiat`, else the profile's) is what a stored price must be
  in: a priced row in another currency is held as `price_currency_mismatch`.
  A switch to a country with another currency (a EUR book to `us`, `gb`,
  `ch` or `jp`) therefore holds every priced row until it is repriced in the
  new currency; the profile update reports the count in
  `price_currency_holds` ([quarantines](tax.md#quarantines)). Kassiber does
  not convert prices.
- Custody and transfer authoring: cross-asset carrying-value pairs.
- Reports: calendar, language, kicker and country name, the Austrian
  layer, and the preview notice.
- The CLI's `--tax-country` help (`Help::TaxCountries`), which lists the
  registry's countries, preview ones marked; `commands describe` shows it.
- The desktop (`ui-tauri/src/lib/taxCountries.ts`) through a generated copy,
  `ui-tauri/src/lib/taxCountries.generated.ts`. A test in the crate fails
  when the copy is stale; `KASSIBER_UPDATE_TAX_COUNTRIES_TS=1 cargo test -p
  kassiber-tax-countries` rewrites it.

## Preview countries

A preview country's book shows a **Preview** badge wherever the country is
named or chosen (onboarding, the Books screen, the book list), and the
notice "Not yet reviewed by a tax professional for this country — check
results before filing." (German: "Noch nicht von einer Steuerberaterin oder
einem Steuerberater für dieses Land geprüft — Ergebnisse vor der Einreichung
kontrollieren.") next to the picker, in onboarding's review and on the
Reports screen. Every report rendered in the shared design carries the same
notice (`Document::preview_notice`): the PDF under the title and above the
footer of every page, the XLSX summary sheet's third row, the CSV bundle's
`summary.csv` (section `preview`, key `notice`) and its manifest
(`preview_notice`). A report of a reviewed country is byte for byte what it
was. The desktop bundles' strings equal the registry's (a vitest checks).

A country leaves preview when a tax professional for it has reviewed its
specification, its vectors and a sample book's reports; the review is
recorded in the country's specification and the flag is removed in the same
commit.

## Python

Python keeps `generic` and `at` only (`ACTIVE_TAX_COUNTRIES`). Should the
hybrid hand it a book of a Rust-only country, every tax read and write stops
before touching the book: profile reads and updates, journal processing and
report reads and exports fail with `internal_error`, "Unsupported tax country
'<code>'", and the database is unchanged.
`tests/test_python_rust_only_countries.py` checks that for every registry
country Python lacks.

## Adding a country

1. Specify it in `docs/reference/tax-country-<code>.md` with sources and
   retrieval dates, and its vectors in `tax-country-<code>-vectors.json`
   (schema `kassiber.tax-country-vectors/1`, written by
   `docs/reference/tax-country-reference/build.py` from a reference model).
2. Add the registry entry with `preview: true`, regenerate the desktop copy,
   and add its names to the desktop where a screen needs its own wording.
3. Implement whatever the engine lacks (a method, a holding-period rule, a
   pool scope) behind the registry's answers; an unsupported combination
   must fail closed.
4. Add its country layer to the shared report design
   ([report design](report-design.md#country-layer)).
5. Run the shared vector runner over its vectors.

## Designed for the countries to come

The registry's shape was checked against the research specifications for
Germany, Switzerland and the United Kingdom (branch
`t3code/tax-country-research-de-ch-uk`) and Spain, Ireland and Japan
(`t3code/tax-country-research-es-ie-jp`). What each needs and where it goes:

| Need | Where | Status |
| --- | --- | --- |
| A time zone per country (Europe/Berlin, Europe/Zurich, Europe/London, Europe/Madrid, Europe/Dublin, Asia/Tokyo) for dates, holding periods and year boundaries | registry `timezone`; its calendar in `calendar.rs`; the report readers' `kassiber_reports::country::zone`, `calendar::local`, `calendar::year_start` and `export_range::book_zone` | every zone has a calendar from 1996 (checked against zoneinfo), which the engine uses; the report readers cut periods in the book's zone (Vienna its pinned table, Berlin, Zurich, Dublin, London, Madrid and Tokyo the registry rule, checked against tzdata 2026e); a zone outside `SUPPORTED_ZONES` (UTC, Vienna, Berlin, Zurich, Dublin, London, Madrid, Tokyo) still needs that check, and a test refuses it until then; Madrid follows the EU rule and Tokyo its fixed offset (`book_zone`), both checked against tzdata from 1996 |
| Holding periods counted in calendar dates in that zone (DE, US; 29 February's anniversary 28 February) | registry `long_term` (`MoreThanOneYear` or `Never`) with `engine`; engine `Country::is_long_term` | in place: the US rule in UTC, any other country's in its zone |
| Tax years that do not start on 1 January (UK 6 April) | registry `tax_year_start`; `years::tax_year` and `tax_year_label`; engine `tax_year_start` | in place: the engine's yearly totals and summary of a rules country are by its tax year in its zone; `kassiber_reports::tax_years::tax_year` gives a layer an instant's year and label |
| Per-wallet pools (US from 2025, DE) next to global ones | registry `pool_scopes`; engine pool scope `wallet`, and the rules' `pool_scope` when a country allows one | in place: one country-neutral wallet pass for the lot methods and DE's `moving_average_fifo_period`; a rules country with one allowed scope refuses the other in the engine too (DE-POOL-WALLET, GB-POOL-GLOBAL); the US keeps its own 2025 refusal of the global scope |
| Country methods: UK share matching, DE `moving_average_fifo_period`, JP `jp_total_average` (default) and `jp_moving_average` | registry `methods`; engine method gate and the `Matching` hook ([matching methods](tax-engine.md#matching-methods)) | in place: the hook, DE's `moving_average_fifo_period`, the UK's `uk_share_matching` and Japan's two averages; a matching method may replace a fragment's whole cost (`cost_override`, Japan) beside its per-unit cost, and the open lots' basis (`open_basis`, the UK's pool, Japan's closing value) |
| Fee coins as disposals (DE, UK; ES, IE for transfers) | registry `engine.fee_units`; engine `fee_units` | in place: `disposal` realizes fee and transfer-fee gains at their value; `disposal_before_sale` also books a sale's fee units first, the sale deducting their value as expenses |
| Crypto-to-crypto values: received coin's value as proceeds and given coin's value as cost (DE, UK); the received value for both (IE, JP); the higher of the two (ES) | registry `engine.swap_values`; engine `swap_values` | in place, with the other leg's value when one has none (DE-SWAP-FALLBACK, GB-SWAP-VALUES) |
| Gifts and donations at market value (ES, IE, JP, DE, UK); today every gift kind is quarantined (`non_sale_disposal_kind`) | registry `engine.gifts`; engine `gifts_given`, `donations_given`, `gifts_received`, `gift_losses` | in place: given or donated at value, received gifts refused (DE), gift losses marked not deductible (ES); the quarantine stays the default, and US books keep booking received gifts (open question 7) |
| Rates and thresholds by local date with mid-year changes (IE 33 % → 31 % from 7 October 2026 with the exemption applied to the 33 % gains first; UK 2024-25 split at 30 October; DE Freigrenze €600/€1,000 and €256) | `years::Schedule` tables in `tables.rs` (DE § 23 and § 22 Nr. 3 Freigrenzen, UK annual exempt amount, proceeds threshold and main rates, IE rate and exemption, JP regime), listed in an entry's `tax_tables`; `kassiber_reports::tax_years` (`exemption_limit`, `allowance_highest_rate_first`, `table_value`) | in place; the country layers apply them |
| Years without tables or with an unsettled regime fail closed (JP from 2027 until the separate-taxation start is confirmed; US after 2026) | a table's `Lookup::Unknown` after its `known_until` and before its first step, and `Lookup::Refused(code)` from a refusing step | in place: IE before 6 December 2012 and JP from 2027 refuse with their codes; every other gap is unknown, never the neighbouring year's value; the US layer keeps its own form years |
| Locale string tables (DE `Januar`, CH without ß and `1'234.56`, apart from Austrian German) | registry `locale`; `report_design::Lang` and the string tables, a locale's own table (`DE_DE`, `DE_CH`) read before the shared German one | `en`, `de-AT`, `de-DE`, `de-CH`, `es-ES` (`strings_es.rs`) and `ja-JP` (`strings_ja.rs`, with Noto Sans JP in the PDF engine); a test keeps every Swiss string free of ß; the Spanish and Japanese tables carry every shared key and their country's layer, another country's layer keys falling back to English |
| Kinds a country does not book yet (DE unclassified airdrops and hard forks, CH hard forks; ES and JP lost coins on their way out) | registry `engine.refused_receipts` (`direction`); engine `refused_receipts` | in place: a receipt held as `acquisition_valuation_unsupported`, an outflow (`direction: outbound`, a non-sale disposal kind matched as a token) as `non_sale_disposal_kind`, each with the country's code, before pricing; inbound rules keep their bytes |
| Zero-cost receipts (JP hard forks) and sales below market value (ES market value, JP 70 %) | registry `engine.zero_cost_kinds`, `engine.below_market`; engine `zero_cost_kinds`, `below_market` and a row's `market_value` | in place; no importer records a market value apart from the price yet (ES-Q11, JP-Q13), so the vector runner exercises the rule |
| Facts a country layer reads back from the journal (a fragment's expenses, a gift loss that does not count, a sale valued at market) | `calculation_json`: each fragment's `expenses_exact`; the entry's `loss_deductible` and `valuation` beside the fragments | in place: present only where they apply, and an entry with any of them keeps the legacy JSON form, so every other book keeps its stored bytes |

Spain and Japan use the mechanisms Germany, Switzerland, Ireland and the
United Kingdom brought, each extended once and generically where they
needed more, so every country takes the same path; for a book that uses
none of the additions, requests, results and stored rows are unchanged:

- **Refusals:** a `refused_receipts` rule may name `direction: outbound`
  (absent: inbound). Its kinds are non-sale disposal tokens matched in the
  row's kind, and the row is held as `non_sale_disposal_kind` with the
  country's code, as an inbound one is held as
  `acquisition_valuation_unsupported`.
- **Matching hook:** a fragment may carry its whole cost
  (`cost_override`) beside the per-unit override; the hook's `open_basis`
  serves the UK's pool and Japan's closing value alike.
- **Engine rules:** `zero_cost_kinds` and `below_market`, with a row's
  optional `market_value`.
- **Journal facts:** beside each fragment's `expenses_exact`, an entry's
  `loss_deductible` and `valuation`; an entry with any of them keeps the
  legacy JSON form, as one with expenses does.
- **Calendars:** without tzdata, a registry zone without summer time
  (Tokyo) reads its fixed offset in `export_range::book_zone`.
- **Methods:** profiles and `--gains-algorithm` accept any method a
  registry country offers; the book's country decides
  ([Rust shell](rust-shell.md)).
- **Reports:** a locale whose table carries only the shared keys and its
  own layer (`es-ES`, `ja-JP`) reads another country's layer keys in
  English; the PDF engine bundles Noto Sans JP as its last fallback.

Every country's vectors use the schema `kassiber.tax-country-vectors/1`; one
runner (`crates/reports/tests/tax_country_vectors.rs`) reads them all,
checks every file's shape, and computes the vectors of the countries the
registry lists (the United States: all 22; Germany: 25 of 26 through the
engine, de-24's rail decision by the custody and transfers tests the
runner names; Switzerland: all 20; Ireland: all 18, with RP2's
rows checked lot by lot; the United Kingdom: all 23, three of them refused
with their codes; Spain: all 17 and Japan: all 19, through
`country_vectors/es_jp.rs` and `kassiber_reports::es_tax` and `jp_tax`,
jp-14's yen commission restated as netted into the price, as Germany's
and Ireland's are).

Before a country's entry exists, the runner already exercises the shared
features its entry will use: each probed country's vectors run through the
`ledger` operation under the rules its specification's registry defaults
give (`PROBES`), and the figures the engine and the shared tables decide
are compared. What only a country layer decides (form lines, income
categories, flags, classifications) or a method not implemented yet needs
is listed with its reason and not computed:

| Country | Compared | Passing |
| --- | --- | --- |
| `de` | from its registry entry since it landed: fragments, § 23 against the dated Freigrenze, holdings, and the refusals | 25 of 26 (de-24's rail decision is custody's) |
| `ie` | per disposal the gain, Dublin date and dated rate, the lots; the exemption to the highest rate first in years without losses | 18 of 18 |
| `ch` | from its registry entry since it landed: fragments with the six-month check in Zurich dates | 18 of 20 (the hard fork and professional trading are refusals the full computation checks) |
| `gb` | tax-year labels, the annual exempt amount and proceeds threshold by year, 2024-25's 30 October split (share matching is the UK worker's) | 23 of 23 |

When a country's entry lands, its worker builds the probe's rules from the
entry (`engine` and the fields beside it) instead of `PROBES`, and adds its
full computation next to the US one.
