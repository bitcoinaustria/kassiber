# Report design

**Status:** approved by the owner on 2026-11-04 and rolling out report by
report (see [Rollout](#rollout)). Plan 21 ports report rendering to Rust; this
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

### Austrian sections

Today's Austrian report numbers its sections (1.1 Veräußerungen Altvermögen …
4.5). The shared report keeps one Disposals and one Income table; each row
carries a `section` column with today's Austrian section number (`1.2`), so an
accountant can still filter a section. The Kennzahl page keeps an index of the
sections: number, title, the Kennzahlen it feeds, whether Kassiber models it,
rows and amount. Python's English assumption texts are translated by their
code; an unknown code keeps Python's text. The transaction appendix is wide
and gets landscape pages.


## Language

The book's tax country picks the language: Austrian books read German, every
other book English. Every label, note and heading is a key in one string
table per language (`crates/reports/src/report_design/strings.rs`); builders
and templates never translate ad hoc, and a test requires both tables to hold
the same keys. German uses the Austrian tax terms (Veräußerungen,
Anschaffungskosten, Einkünfte, Altvermögen/Neuvermögen, Spekulationsfrist,
Wegzugsbesteuerung) and Austrian month names (Jänner); the Kennzahl wording is
verbatim from today's Austrian PDF.

| Format | English | German |
| --- | --- | --- |
| PDF numbers | `1,234.56`, `−0.5` | `1.234,56`, `−0,5` |
| PDF dates | `2025-03-15`, `2025-03-15 14:05 UTC` | `15.03.2025`, `15.03.2025 14:05 UTC` |
| PDF months | `2025-03` (axis `2025-03`) | `März 2025` (axis `Mär 25`) |
| PDF percent | `59.2%` | `59,2 %` |
| XLSX | real numbers and dates; headers in the language; date format `yyyy-mm-dd` | the same numbers; date format `dd.mm.yyyy` |
| CSV | machine format: plain decimals, ISO dates, English `snake_case` headers | identical to English |

Sheet and CSV file names are the table ids in every language, so a script
reads either book the same way.

## Visual style

- **Type:** DM Sans (upstream 4.004, Regular and Medium) for text and
  amounts, amounts with its tabular figures (`tnum`) so columns line up;
  JetBrains Mono (2.304 Regular) only for identifiers such as codes, ids and
  hashes. One typeface per key/value block. Liberation Sans is the glyph
  fallback (Greek, Cyrillic, ₿); characters no bundled font draws become
  U+FFFD before layout, so nothing renders as a missing glyph. Sources and
  hashes: [`crates/pdf/fonts/README.md`](../../crates/pdf/fonts/README.md).
- **Colour:** ink `#1f2328`, two greys (`#5b6169` secondary text, `#d0d4d9`
  hairlines) and Kassiber red `#c8102e` only for report blockers (stale
  journals, quarantined transactions) and the main chart line. Everything
  reads in black and white: series differ by line weight and dash, not only
  by colour.
- **No boxes:** no cell borders, fills or zebra stripes. A table is an
  optional caption, its header (secondary grey, medium weight), a 0.5 pt rule
  under it and a totals row set in medium weight above a hairline.
- **Numbers:** right-aligned; fiat with two decimals and thousands
  separators, the currency named once in the column header; BTC with eight
  decimals, `sat` where the report option asks for it; true minus sign
  (−) for negatives; percentages with one decimal. Unknown values print
  `—` with the reason in a note, never `0`. In sparse columns (fees, gains
  and disposal counts by month) a zero prints `–` so the few values stand
  out; XLSX and CSV keep the zero.
- **Dates:** per [language](#language).
- **Page:** A4 portrait, 18 mm side margins. Tables wider than the page
  switch the section to landscape pages. Running header from page 2:
  *Kassiber · report title* left, *book · period* right. Footer on every page:
  *Generated … · journal input v…* left, *Page x of y* right.
- **Charts:** few and quiet: a balance line (market value solid, cost basis
  dashed, quantity thin on a second axis) and monthly bars (gain/loss,
  inflow/outflow). Axes step on round values (1, 2, 2.5 or 5 times a power of
  ten: `0`, `20k`, `40k`; German `20 Tsd.`, `1,5 Mio.`), one hairline per
  tick. Charts never split across pages; the Activity section follows the
  holdings on page 1 when it fits.

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
- `summary.csv` holds the facts and headline figures as `section`, `key`,
  `value`, `unit`; labels stay in PDF and XLSX;
- `manifest.json` lists `schema` (`kassiber.report/1`), report kind, book
  (workspace and profile labels), period, fiat, country, generated at, journal
  input version and every file with columns, row count and SHA-256.

The new layout replaces today's CSVs outright; there is no legacy option.

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

## Implementation

`crates/reports/src/report_design/` holds the document types and the three
renderers (`mod.rs`), the language rules (`locale.rs`), the string tables
(`strings.rs`) and one builder per report kind (`summary.rs`, `full.rs`,
`austrian.rs`, `exit.rs`). Builders
take the data Python's reports produce and decide nothing a tax, journal or
custody decision depends on. The PDF template is
`crates/pdf/templates/report.typ`; the engine is sealed (`crates/pdf/src/engine.rs`).

Evidence for every rolled-out report:

- a data-parity test: the model handed to the renderers equals Python's for
  fixture cases, refusals included (`crates/reports/tests/report_design.rs`
  for the summary's 23 cases, `crates/reports/tests/full_pdf.rs` for the full
  report's 89: Python's `_build_full_report_context` values, exact;
  `crates/reports/tests/austrian_pdf.rs` for the Austrian report's 47: the E
  1kv report, portfolio and transaction rows Python's writer receives; the exit-tax report in its vectors);
- whole-process tests (`tests/test_rust_pdf_exports.py`) run each CLI leaf and
  daemon kind against Python and require identical envelopes, errors, files,
  saved and filed markers and database effects, except where this document
  lists a necessary difference;
- PDF, XLSX and CSV bytes repeat across runs and processes;
- mutation entries (`crates/reports/scripts/report_design_mutations.json`,
  `crates/pdf/scripts/mutations.json`) for the model and formatting rules;
- PNG samples for two books built with the CLI
  (`crates/reports/scripts/report_design_books.py`: an Austrian
  moving-average book from 2020 and a generic FIFO book from 2022) rendered by
  `crates/reports/scripts/report_design_prototype.py` into
  `target/report-design/{austria,generic}/`.

## Rollout

One report per commit, each with data parity against Python's model.

| # | Report | Commands | State |
| --- | --- | --- | --- |
| 1 | Summary | `reports export-summary-pdf`, `ui.reports.export_summary_pdf` | native, shared design |
| 2 | Full report | `reports export-pdf`, `ui.reports.export_pdf` | native, shared design |
| 3 | Austrian E 1kv | `reports export-austrian-e1kv-pdf`, its alias `reports export-austrian`, `ui.reports.export_austrian_e1kv_pdf` | native, shared design |
| 3 | Exit tax | `reports export-exit-tax-pdf`, `ui.reports.export_exit_tax_pdf` | native, shared design |
| 4 | Source of funds | `reports export-source-funds-pdf`, `ui.source_funds.export_pdf` | pending |
| 5 | XLSX | full, Austrian, exit tax, cross-check, verification, year handoff | pending |
| 6 | CSV | full, capital gains, Austrian bundle | pending |
| 7 | Bundles | `reports export-austrian`, `ui.reports.export_audit_package`, source-of-funds bundle | pending |
| 8 | Registration | every CLI leaf and daemon kind native, no fallback | pending |

### Envelope differences

Envelopes, file names, overwrite and managed-destination rules and errors
keep byte parity with Python except where the new format changes them:

| Report | Field | Difference |
| --- | --- | --- |
| Summary | `pages`, `bytes`, `report_snapshot.content_sha256` | the new PDF's; `title` stays Python's (`Kassiber Summary Report`) while the PDF's own title follows the book's language |
| Full report | `pages`, `bytes`, `report_snapshot.content_sha256` | the new PDF's; `title` stays Python's (`Kassiber PDF Report - <scope>`) |
| Exit tax | `pages`, `bytes` | the new PDF's; Python registers no saved marker for it, and neither does Rust |
| Austrian E 1kv | `pages`, `bytes`, `report_snapshot.content_sha256`, `renderer` | the new PDF's; `renderer` is `typst` instead of `reportlab`; `title`, `sections` (Python's section ids, each still present in the new layout), `assumptions` and `transactions` stay Python's |

## Owner decisions (2026-11-04)

1. **Language:** German with Austrian formats for Austrian books, English
   otherwise; one string table per language. See [Language](#language).
2. **Fonts:** full upstream DM Sans and JetBrains Mono, pinned by SHA-256;
   amounts move from mono to DM Sans tabular figures; Liberation Sans as
   fallback with a test for Greek and Cyrillic wallet labels.
3. **CSV:** the new format, no legacy option.
4. **Austrian sections:** shared Disposals and Income tables with a `section`
   column; a section index on the Kennzahl page.
5. **Colour:** red only for blockers and the main chart line.
