# Australia (`au`): tax country specification

**Status:** in the registry as a **preview** country with generic rules on
its legal income year (2026-11-16). Not reviewed by an Australian tax
professional; every Australian report and screen carries the preview
notice. Nothing here is tax advice, and no Australian form or schedule is
produced.

Scope: an individual Australian resident holding bitcoin as an investment
(a CGT asset). Traders, businesses and personal-use assets are out of scope.

## Sources

All retrieved 2026-11-16 from the Australian Taxation Office.

| Ref | Source |
| --- | --- |
| ATO return | [Preparing your tax return](https://www.ato.gov.au/individuals-and-families/your-tax-return/before-you-prepare-your-tax-return/preparing-your-tax-return): "Your tax return covers the income year from 1 July to 30 June" |
| ATO discount | [CGT discount](https://www.ato.gov.au/individuals-and-families/investments-and-assets/capital-gains-tax/cgt-discount): the 50 % discount for a resident individual who owned the asset for at least 12 months; "exclude the day of acquisition and the day of the CGT event"; an asset acquired on 2 February 2025 qualifies for a CGT event on or after 3 February 2026 |
| ATO crypto | [How to work out and report CGT on crypto](https://www.ato.gov.au/individuals-and-families/investments-and-assets/crypto-asset-investments/how-to-work-out-and-report-cgt-on-crypto): disposals, swaps and exchanges are CGT events; the discount for crypto held at least 12 months |

## Registry entry

| Field | Value | Why |
| --- | --- | --- |
| `fiat` | AUD | the tax return's currency |
| `tax_year_start` | 07-01, locked | the income year runs from 1 July to 30 June (ATO return) |
| `timezone` | Australia/Sydney; a book may choose Adelaide, Brisbane, Darwin, Hobart, Melbourne or Perth | the taxpayer's local date; the calendars are checked against tzdata 2026f (`crates/tax-countries/tests/data/zoneinfo_added.txt`); Perth's calendar starts on 2009-04-01, after Western Australia's summer-time trial |
| `long_term` | `MoreThanOneYear` | "at least 12 months" excluding the day of acquisition and the day of the CGT event is a disposal after the anniversary (ATO discount's example); the engine's long-term flag marks discount-eligible gains, it does not apply the discount |
| `methods` | FIFO | the minimal choice; identification of specific units is open (AU-Q2) |
| `pool_scopes` | global | |
| `engine` | generic rules | every engine feature at its generic value; only the zone, holding period, scope and tax year differ |
| `report_generators` | `open_positions`, `rp2_full_report` | no Australian layer yet |

## Open questions for an Australian tax reviewer

1. AU-Q1 — The 50 % discount: should Kassiber compute the discounted net
   capital gain per income year? The ATO's capital gains tax pages describe
   a change to the discount from 1 July 2027; a reviewer should confirm it
   before the holding-period flag is used for anything but information.
2. AU-Q2 — Which identification methods may an individual use for bitcoin
   (specific identification with records, FIFO otherwise)?
3. AU-Q3 — Are transfer fees part of the cost base, and are swaps valued at
   the received or the given asset?
4. AU-Q4 — BTC↔L-BTC pegs: a CGT event or not?
5. AU-Q5 — Personal-use assets, staking and airdrops: which kinds are
   ordinary income at receipt?
