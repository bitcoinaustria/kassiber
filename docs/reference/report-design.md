# Report design

**Status:** proposal for the owner's review (2026-11-04). Nothing here is
rolled out yet; the summary report is prototyped in this design for one
Austrian and one generic book. Plan 21 ports report rendering to Rust; this
document decides what the rendered reports look like once they are Rust's.

## Goal

One minimalist report format for every country and every file type. Today
Austria gets hand-made accountant reports while generic books get a
fixed-pitch text PDF and CSV/XLSX dumps that look like nothing. From now on
every report is built from one **report model**; PDF, XLSX and CSV are three
renderings of the same sections, with the same columns, labels, order and
number formats. A **country layer** adds legal fields (Austrian Kennzahlen,
Alt/Neu regimes, exit tax; the generic method's lot details) inside the shared
structure instead of forking the layout.

Python stays the reference for the **data**: every number, row, label meaning,
threshold, unknown or unsupported state and refusal. Layout, structure and
styling are new and shared; ReportLab's page layouts are not reproduced.

## What exists today

| Export | Command / kind | Audience | Shape | Country-specific |
| --- | --- | --- | --- | --- |
| Full report PDF | `reports export-pdf`, `ui.reports.export_pdf` | book owner, auditor | fixed-pitch Courier text, landscape, Latin-1 only; 14 sections from book metadata to all transactions | none; generic only |
| Summary PDF | `reports export-summary-pdf`, `ui.reports.export_summary_pdf` | owner, treasury | ReportLab: KPI strip, data integrity, balance line chart, wallet donut, PnL and flow bars, largest disposals/activity, wallet appendix | none |
| Austrian E 1kv PDF | `reports export-austrian-e1kv-pdf`, alias `reports export-austrian`, `ui.reports.export_austrian_e1kv_pdf` | Austrian taxpayer, Steuerberater | ReportLab, German: cover, taxable summary 1.1–4, details 1.1–4.5, holdings, Besonderheiten, explanations, landscape transaction appendix, FinanzOnline Kennzahlen, FAQ | wholly Austrian |
| Exit-tax PDF | `reports export-exit-tax-pdf`, `ui.reports.export_exit_tax_pdf` | Austrian taxpayer leaving | fixed-pitch text lines | Austrian |
| Source-of-funds PDF | `reports export-source-funds-pdf`, `ui.source_funds.export_pdf` | bank/exchange compliance | ReportLab: cover, flow diagram, overview, narrative, data sources, gates, source mix rings, landscape detail tables, disclosure preview | report title/subtitle per jurisdiction |
| Source-of-funds bundle | `reports export-source-funds-bundle`, `ui.source_funds.export_bundle` | compliance recipient | ZIP: the PDF, disclosed evidence files, `manifest.json` with SHA-256 | as above |
| Audit package | `ui.reports.export_audit_package` | auditor | directory: `manifest.json` (evidence summary) and copied evidence; no rendered report | none |
| Full report CSV | `reports export-csv`, `ui.reports.export_csv` | spreadsheet users | one file, 12 sections stacked with title rows; floats in Python `repr` (exponents possible); `_sat` float and `_msat` int columns | none |
| Full report XLSX | `reports export-xlsx`, `ui.reports.export_xlsx` | accountant | the same 12 sections as sheets plus Verify, Acquisitions, Disposals, Control, Quarantined with live formulas | Austrian readme text |
| Capital gains CSV | `ui.reports.export_capital_gains_csv` | spreadsheet users | 13 columns incl. `at_category`, `at_kennzahl` | Austrian columns always present |
| Austrian XLSX / CSV bundle | `export-austrian-e1kv-xlsx`, `export-austrian-e1kv-csv` | Steuerberater | Übersicht, 13 numbered section sheets/files, Erläuterungen; German headers | wholly Austrian |
| Exit-tax XLSX | `export-exit-tax-xlsx` | Austrian taxpayer | overview and lots | Austrian |
| Year handoff XLSX | `export-year-handoff` | EA / Jahresabschluss accountant | Overview, Wallets, Transactions, Journal, Balances, Roll-forward, Reconciliation, Excluded | none |
| Cross-check XLSX | `reports cross-check --xlsx`, `ui.reports.export_cross_check` | users of CoinTracking/Blockpit | Summary plus difference sheets | none |
| Transactions CSV/XLSX | `transactions export`, `ui.transactions.export_*` | owner | one ledger table | none |
| Platform CSV | CoinTracking / Blockpit formats | another tool's importer | the other tool's format | — |
| Lightning, commercial subledger CSV | `export-lightning-profitability-csv`, `export-commercial-subledger-csv` | owner | one table each | none |

The generic, non-Austrian user gets the fixed-pitch full report, the summary
PDF and the stacked CSV/XLSX. Since plan 20 the generic numbers come from
Kassiber's tax engine; RP2 is only its test oracle, and nothing RP2 itself
renders reaches a user.

Platform CSVs are another program's import format and keep that format.
Cross-check workbooks compare against such a file and stay a diagnostic tool;
they adopt the visual style only.

## One report model

Every report is a document of **sections**. A section is a titled list of
blocks: facts (label and value pairs), tables, charts and notes. A table has
an id (its sheet and CSV file name), typed columns and rows of typed cells.
Report kinds choose sections; renderers never choose content.

Shared sections, in this order wherever a report has them:

| # | Section (id) | Contents |
| --- | --- | --- |
| 1 | Cover and scope (`scope`) | book (workspace / profile), period, wallets in scope, fiat, country, method, generated at, journal input version, readiness |
| 2 | Summary (`summary`) | start and end value, net flow, realized gain, unrealized gain, fees, holdings quantity, benchmark |
| 3 | Holdings at period end (`holdings`) | per wallet and asset: quantity, cost basis, market value, unrealized |
| 4 | Disposals (`disposals`) | date, wallet, asset, quantity, proceeds, cost basis, gain, category |
| 5 | Income (`income`) | date, wallet, kind, quantity, value |
| 6 | Fees (`fees`) | network and venue fees in BTC and fiat |
| 7 | Transfers and custody (`transfers`) | internal transfers, swaps and pegs, reviewed custody notes |
| 8 | Open issues (`issues`) | quarantines, stale journals, missing prices, review gates, report blockers |
| 9 | Method and provenance (`method`) | method, engine, pricing sources, assumptions, review gate text |

Columns are typed (text, code, date, fiat, BTC, msat, count, percent). The
type decides alignment, face and number format in every renderer. One
exception keeps the PDF readable: the integer `_msat` companion of a BTC
column appears in XLSX and CSV but not on paper.

A report kind is a selection: the summary is 1, 2, 3 (period end), activity
charts, the largest disposals and movements, 8, 9; the tax report is 1, 2, 4,
5, 6, 8, 9 plus the country layer; the full report adds every transaction.
Source-of-funds is a disclosure, not an accounting report: it keeps its own
sections (flow, sources, gates, disclosure preview) in the same style.

### Country layer

A country adds sections at fixed slots and extra columns at the end of shared
tables. It never changes a shared column, label or order.

- **Austria** (`at`): after Income, an *E 1kv* section with one row per
  Kennzahl (code, the form's German wording, amount) for each tax year the
  period covers; disposals gain `regime` (Alt/Neu) and `kennzahl` columns;
  Method lists the moving-average method, the 2021-03-01 Neuvermögen
  cutoff, the self-custody Kennzahl mapping (172/174/176, 801 outside
  E 1kv) and the domestic-provider placeholders (171/173/175) as unsupported.
  Exit tax is its own report kind built from the same sections.
- **Generic**: Method names the lot method (FIFO, LIFO, HIFO, LOFO), the
  long-term threshold in days and Kassiber's tax engine; disposals gain a
  `term` (short/long) column.

Labels are English. The country layer's legal labels are verbatim German
(the Kennzahl wording the filing needs) next to their English description.

## Visual style

- **Type:** DM Sans for text and JetBrains Mono for figures, dates and
  identifiers, the desktop's two families. Mono gives tabular figures; the
  available DM Sans files have proportional digits and no `tnum` feature.
  Liberation Sans/Mono stay bundled only as glyph fallback (₿, →).
- **Colour:** ink `#1f2328`, two greys (`#5b6169` secondary text, `#d0d4d9`
  hairlines) and one accent, Kassiber red `#c8102e`, used only for report
  blockers and the chart's primary series. Everything reads in black and
  white: series differ by line weight and dash, not only by colour.
- **No boxes:** no cell borders, fills or zebra stripes. A table is an
  optional caption, its header (secondary grey, medium weight), a 0.5 pt rule
  under it and a totals row set in medium weight above a hairline.
- **Numbers:** right-aligned; fiat with two decimals and thousands
  separators, the currency named once in the column header; BTC with eight
  decimals, `sat` where the report option asks for it; true minus sign
  (−) for negatives; percentages with one decimal. Unknown values print
  `—` with the reason in a note, never `0`.
- **Dates:** ISO 8601 (`2025-03-15`), times as `2025-03-15 14:05 UTC`.
- **Page:** A4 portrait, 18 mm side margins. Tables wider than the page
  switch the section to landscape pages. Running header from page 2:
  *Kassiber · report title* left, *book · period* right. Footer on every page:
  *Generated … · journal input v…* left, *Page x of y* right.
- **Charts:** few and quiet: a balance line (market value solid, cost basis
  dashed, quantity thin on a second axis) and monthly bars (gain/loss,
  inflow/outflow). Axis labels in mono, no gridlines beyond four hairlines.

## XLSX

The same sections as sheets, in the same order: a `Summary` sheet first with
the cover facts and summary figures, then one sheet per table with the table
id as its name. Headers are frozen and autofiltered; amounts are numbers with
formats (`#,##0.00` fiat, `0.00000000` BTC, `0` msat), dates are dates, never
text. Column widths come from the column type. The Verify/Control sheets of
today's full report stay as appendix sheets with their formulas.

## CSV

Machine-first, one file per table, written into a directory (or ZIP):

- file name = table id (`holdings.csv`, `disposals.csv`, …);
- header row only, stable `snake_case` column names, no titles or blank rows;
- UTF-8 without BOM, `\n` line endings, RFC 4180 quoting;
- dates and times ISO 8601 in UTC (`2025-03-15T14:05:00Z`);
- amounts as plain decimal strings without exponent or grouping: BTC from the
  exact msat value (`0.12345678`; more places only when msat is sub-satoshi),
  fiat at the stored precision;
- every BTC amount has an integer `_msat` column beside it;
- unknown values are empty, with the reason in a `*_status` column where the
  model has one;
- `manifest.json` lists `schema` (`kassiber.report/1`), report kind, book
  (workspace and profile labels), period, fiat, country, generated at, journal
  input version and every file with columns, row count and SHA-256.

### CSV changes against today

CSV consumers break on renames, so every change is listed here and the
manifest's `schema` lets a consumer detect the new layout.

| Today | New | Why |
| --- | --- | --- |
| full report: one file of 12 stacked sections with title rows | one file per table plus `manifest.json` | stacked sections cannot be loaded by any CSV reader without hand-editing |
| `quantity`, `amount`, `fee` as Python float `repr` (`1e-08` possible) | `quantity_btc`, `amount_btc`, `fee_btc` as exact decimal strings | floats lose msat precision and exponents break spreadsheets; the suffix names the unit |
| `*_sat` as float msat/1000 | removed | duplicates `*_msat` with less precision |
| `*_msat` integer | unchanged | exact |
| `cost_basis`, `proceeds`, `gain_loss`, `market_value` floats | same names, decimal strings, `fiat_currency` column | names are already clear; values become exact text |
| capital gains: `at_category`, `at_kennzahl` on every book | only in Austrian books (country columns) | generic users get empty Austrian columns today |
| Austrian bundle: German headers (`Börse`, `Anlage`, …) and merged total rows | snake_case headers, totals in `manifest.json` and the E 1kv table | German headers stay in PDF/XLSX where people read them |

Unchanged: platform CSVs (another program's format), the transaction export's
column names except the unit suffixes above, and every `_msat` column.

## What must not change

- Tax numbers: every amount is the same as Python's model for the same book,
  and the renderers receive finished models; no tax, journal or custody
  decision is made while rendering.
- The Austrian filing's legal labels: Kennzahl codes and their wording.
- Report-readiness barriers: stale journals, custody blockers, freshness
  blockers and review gates refuse an export exactly as today.
- Privacy: no descriptors, xpubs, addresses lists or wallet configuration in
  any export; wallet labels only.
- Reproducibility: byte-identical output across runs and processes for a
  fixed `generated_at`; PDF metadata dates come from the report, fonts and
  templates are embedded, and the Typst world is sealed (no system fonts,
  files, packages or network).

## Prototype

`crates/reports/src/report_design.rs` holds the document types and the three
renderers; `report_design_summary.rs` builds the summary report's document
from the data Python's reports produce: `build_summary_pdf_report_data`, the
profile's tax settings and, for Austria, the E 1kv Kennzahl totals of each tax
year in the period. The PDF template is `crates/pdf/templates/report.typ`.

Evidence:

- `crates/reports/tests/report_design.rs`: the summary model equals Python's
  for 23 fixture cases (refusals included); PDF, XLSX and CSV bytes repeat;
  every CSV file has snake_case headers, plain decimals, integer msat and a
  manifest entry with its SHA-256; display rounding and CSV text come from
  one exact value.
- `crates/reports/scripts/report_design_books.py` builds two books with the
  CLI from seeded CSVs and `journals process` (an Austrian moving-average
  book from 2020 and a generic FIFO book from 2022; hot, cold and Liquid
  wallets with buys, sales, interest, transfers and spends) and records
  Python's data for them.
- `crates/reports/scripts/report_design_prototype.py` renders both books twice
  in separate processes, requires identical bytes and the renderers' model to
  equal Python's exactly, and writes the samples with PNG pages
  (`pdftoppm`): `target/report-design/{austria,generic}/` holds
  `summary.pdf`, `page-*.png`, `summary.xlsx`, `summary-csv/` and
  `model.json`.
- Ten mutation entries (`crates/reports/scripts/report_design_mutations.json`)
  cover the model and the formatting rules.

The prototype is not registered as a command; `reports export-summary-pdf`
still runs in Python until the owner approves the design.

## Open questions for the owner

1. **Language and number format.** English reports with verbatim German
   Kennzahl wording, or German reports (and `1.234,56`, `15.03.2025`) for
   Austrian books?
2. **Fonts.** The prototype bundles the `@fontsource` 5.x Latin subsets of
   DM Sans and JetBrains Mono, converted losslessly from WOFF. They lack
   `tnum`, ₿ and →, so figures are set in mono and Liberation fills the gaps.
   Full upstream TTFs (Google Fonts DM Sans with `tnum`, JetBrains Mono 2.304)
   would need a one-time download.
3. **Accent colour.** Kassiber red only for blockers and the primary series,
   or no colour at all?
4. **CSV breaking changes.** Accept the renames above, or keep the old stacked
   full-report CSV as a legacy option for a release?
5. **Austrian accountant layout.** Keep today's numbered Steuerbericht
   sections (1.1–4.5) as the Austrian country layer's table order, or fold
   them into the shared Disposals/Income tables with a `section` column?
