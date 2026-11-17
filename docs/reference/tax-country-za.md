# South Africa (`za`): tax country specification

**Status:** in the registry as a **preview** country with generic rules on
its legal year of assessment (2026-11-16). Not reviewed by a South African
tax professional; every report and screen carries the preview notice.
Nothing here is tax advice, and no ITR12 figures are produced.

Scope: an individual South African resident holding bitcoin as a crypto
asset.

## Sources

All retrieved 2026-11-16 from the South African Revenue Service.

| Ref | Source |
| --- | --- |
| ITR12 guide | [Comprehensive Guide to the ITR12 Income Tax Return for Individuals](https://www.sars.gov.za/wp-content/uploads/Ops/Guides/IT-AE-36-G05-Comprehensive-Guide-to-the-ITR12-Income-Tax-Return-for-Individuals-External-Guide.pdf): the year of assessment for individuals runs from 1 March to the last day of February; crypto assets on the return (profit and loss source codes 2572/2573 when revenue) |
| CGT guide | [ABC of Capital Gains Tax for Individuals](https://www.sars.gov.za/wp-content/uploads/Ops/Guides/Legal-Pub-Guide-CGT02-ABC-Guide-on-CGT-for-Individuals.pdf): the annual exclusion and the 40 % inclusion rate for individuals |

## Registry entry

| Field | Value | Why |
| --- | --- | --- |
| `fiat` | ZAR | |
| `tax_year_start` | 03-01, locked | ITR12 guide; the year ends on 28 or 29 February |
| `timezone` | Africa/Johannesburg | +02:00 without summer time |
| `long_term` | `Never` | capital or revenue is a question of intention, not of a holding period |
| `methods` | FIFO | the Eighth Schedule also allows specific identification and weighted average (ZA-Q2) |
| `pool_scopes` | global | |
| `engine` | generic rules | |
| `report_generators` | `open_positions`, `rp2_full_report` | |

## Open questions for a South African tax reviewer

1. ZA-Q1 — Capital or revenue: should a book record the intention, and
   report revenue disposals separately from capital gains?
2. ZA-Q2 — The Eighth Schedule's identification methods for identical
   assets: is the weighted average Kassiber's moving average?
3. ZA-Q3 — The annual exclusion and the inclusion rate: compute the taxable
   capital gain per year of assessment?
4. ZA-Q4 — Crypto-to-crypto swaps and BTC↔L-BTC pegs: disposals at market
   value?
