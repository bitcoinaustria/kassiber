# New Zealand (`nz`): tax country specification

**Status:** in the registry as a **preview** country with generic rules on
its legal tax year (2026-11-16). Not reviewed by a New Zealand tax
professional; every report and screen carries the preview notice. Nothing
here is tax advice, and no IR3 figures are produced.

Scope: an individual New Zealand resident who disposes of bitcoin acquired
for the purpose of disposal (taxable as income on realisation).

## Sources

All retrieved 2026-11-16 from Inland Revenue.

| Ref | Source |
| --- | --- |
| IRD year | [What happens at the end of the tax year](https://www.ird.govt.nz/income-tax/income-tax-for-individuals/what-happens-at-the-end-of-the-tax-year): "The tax year is from 1 April to 31 March"; a tax year is named by the year it ends in (the 2025 tax year is 1 April 2024 to 31 March 2025) |
| IRD crypto | [Calculating cryptoasset income](https://www.ird.govt.nz/cryptoassets/taxing/income-expenses/calculating-cryptoasset-income): income is sale price less purchase cost and fees; first-in first-out (FIFO) or weighted average cost; reconcile holdings at 31 March |

## Registry entry

| Field | Value | Why |
| --- | --- | --- |
| `fiat` | NZD | |
| `tax_year_start` | 04-01, locked | IRD year. Kassiber names a year by its first day (`2024` labelled `2024-25`); Inland Revenue calls the same year "2025" |
| `timezone` | Pacific/Auckland | checked against tzdata 2026f from 2007-06-01 (`NZST-12NZDT,M9.5.0,M4.1.0/3`) |
| `long_term` | `Never` | a disposal's taxability turns on the purpose of acquisition, not a holding period |
| `methods` | FIFO | weighted average cost is allowed (IRD crypto) but whether it is Kassiber's moving average is open (NZ-Q2) |
| `pool_scopes` | global | |
| `engine` | generic rules | |
| `report_generators` | `open_positions`, `rp2_full_report` | |

## Open questions for a New Zealand tax reviewer

1. NZ-Q1 — Which disposals are income (purpose test, the business test),
   and should Kassiber record the purpose of acquisition?
2. NZ-Q2 — Is IRD's weighted average cost a running (moving) average or a
   periodic one?
3. NZ-Q3 — How are fees on transfers between own wallets treated?
4. NZ-Q4 — Mining, staking and airdrops: income at receipt?
