# India (`in`): tax country specification

**Status:** in the registry as a **preview** country with generic rules on
its legal tax year (2026-11-16). Not reviewed by an Indian tax
professional; every report and screen carries the preview notice. Nothing
here is tax advice, and no Schedule VDA figures are produced.

Scope: an individual holding bitcoin, a virtual digital asset (VDA).

## Sources

All retrieved 2026-11-16 from the Income Tax Department.

| Ref | Source |
| --- | --- |
| s. 115BBH | [Section 115BBH](https://www.incometaxindia.gov.in/w/section-115bbh): income from the transfer of a VDA is taxed at 30 %; no deduction other than the cost of acquisition; a loss is not set off against other income nor carried forward (inserted by the Finance Act 2022, w.e.f. 1 April 2023) |
| ITA 2025 s. 3 | [Section 3 of the Income-tax Act, 2025](https://www.incometaxindia.gov.in/w/section-3-1): "tax year" is the twelve-month financial year commencing on 1 April |
| Transition FAQ | [FAQs on the transition to the Income-tax Act, 2025](https://www.incometaxindia.gov.in/documents/d/guest/income-tax-document-faqs-on-interplay-and-transition): the 2025 Act applies from 1 April 2026 (Tax Year 2026-27); FY 2025-26 is assessed under the 1961 Act, whose previous year is the same financial year |

## Registry entry

| Field | Value | Why |
| --- | --- | --- |
| `fiat` | INR | |
| `tax_year_start` | 04-01, locked | the financial year from 1 April under both Acts; labelled `2024-25` as the department does |
| `timezone` | Asia/Kolkata | +05:30 without summer time |
| `long_term` | `Never` | the VDA rate does not depend on a holding period |
| `methods` | FIFO | no method is prescribed; open (IN-Q2) |
| `pool_scopes` | global | |
| `engine` | generic rules | fees stay part of cost and proceeds as in a generic book, which s. 115BBH may not allow (IN-Q3) |
| `report_generators` | `open_positions`, `rp2_full_report` | |

## Open questions for an Indian tax reviewer

1. IN-Q1 — The 2025 Act's section that replaces s. 115BBH for Tax Year
   2026-27 onward, and whether anything changed.
2. IN-Q2 — Which identification method for VDA cost of acquisition?
3. IN-Q3 — Transfer fees and exchange fees: no deduction other than the cost
   of acquisition; does Kassiber have to remove fees from cost and proceeds?
4. IN-Q4 — Losses: s. 115BBH forbids set-off and carry-forward; should
   reports stop netting losses against gains?
5. IN-Q5 — TDS under s. 194S on transfers: should Kassiber track it?
