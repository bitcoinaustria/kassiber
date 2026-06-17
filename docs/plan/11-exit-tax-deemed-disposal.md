# Exit Tax / Deemed-Disposal Report

**Status:** Implemented (v1). Core + CLI + daemon + desktop GUI shipped; see
`kassiber/core/exit_tax.py`, `tests/test_exit_tax.py`, and the surfaces below.
**Current source of truth:** `kassiber/core/exit_tax.py`, `tests/test_exit_tax.py`,
this file plus `06-austrian-tax-engine.md`, `07-austrian-tax-open-questions.md`,
`kassiber/core/austrian.py`, and the RP2 AT plugin behavior.
**Legal gate:** Kassiber is not tax advice. An exit-tax estimate is a draft a
Steuerberater reviews and stamps; the final exit-tax liability across all of a
person's assets is the adviser's determination, not Kassiber's.

## What This Is

An **Austria-first deemed-disposal report**. When a person gives up Austrian tax
residence, §27 Abs 6 EStG treats their unrealized crypto gains as if sold at fair
market value on the departure date (Wegzugsbesteuerung). The report estimates
that liability for the Bitcoin (and other in-scope crypto) held in a book, so the
user can hand a defensible draft to a tax adviser before emigrating.

This is a bonus / advisory feature. A `generic` profile gets a degraded view (no
Altbestand grandfathering, no special rate — total unrealized gain only).

**Not yet pluggable — honest scope.** The Austrian rules are inline, not behind a
jurisdiction seam: the Alt/Neu `2021-03-01` cutoff, the flat 27.5% rate, the
regime labels, the EU/EEA-vs-third-country deferral, and the assumption notes are
all `if is_at` / hardcoded. Adding a second jurisdiction is **not** a rate swap —
a fundamentally different regime (e.g. Germany: per-lot 1-year holding period,
taxed at the person's *progressive* rate, no Alt/Neu split) needs (a) the AT
specifics extracted behind an `ExitRule` object, AND (b) data-model work the
current code lacks: per-lot holding duration and a taxpayer-rate input. Do not
build that abstraction speculatively from one example — let the real second
jurisdiction shape the seam (tracked as future work, not v1).

## Core Idea (as built)

The deemed disposal is "what your capital-gains report would show if you sold
everything at FMV on the departure date" — so it must agree with the engine's
own tax math. v1 achieves that **without re-running the engine or synthesizing
rows**: it reads the journal state RP2 already computed
(`hooks.build_ledger_state`) and rebuilds the remaining inventory + cost basis
**per regime, directly from those entries**:

1. Walk the journal entries with `occurred_at <= departure_date`.
2. Acquisitions/income **add** quantity + cost (acquisition-date regime via
   `infer_regime_from_timestamp`); disposals **subtract** quantity + their
   engine-computed consumed cost basis (regime from the entry's `at_category`).
3. Internal transfers (`transfer_in` / `transfer_out`) create no
   acquisition/disposal entry, so they never touch the global pool — the split
   is transfer-invariant and needs no pair reconstruction.
4. Value the remaining Neubestand at FMV; tax the gain at 27.5%; exclude
   Altbestand.

This reuses RP2's own moving-average basis numbers (consistent by construction)
and reads only the in-memory ledger state — nothing is persisted. "Simulate" is
this computation; "generate handoff" renders the same payload to PDF/XLSX.

A synthetic-full-liquidation-through-the-engine variant (forcing per-regime
`at_regime_override` legs) was evaluated and deferred: it is the precision
upgrade path if rp2's per-regime basis ever needs to diverge from the entries
walk (e.g. Altbestand inside the moving-average pool). The entries walk is
preferred for v1 — lower blast radius on the hot tax path, no per-account
balance-gate risk, and a number that always matches the capital-gains report.

## Reuse Map (as built)

No new tax math — the report reads the engine's own numbers.

| Need | Source used |
|---|---|
| Remaining inventory + cost basis per regime | journal entries from `hooks.build_ledger_state` (acquisition `fiat_value`, disposal `cost_basis`), walked in `compute_deemed_disposal` |
| Alt/Neu attribution | disposal `at_category` prefix; acquisition `occurred_at` vs `AT_NEU_CUTOFF` via `infer_regime_from_timestamp()` |
| FMV at the departure date | `rates_cache` lookup at/before the date + latest-rate / transaction-price fallback (`_fmv_at`) |
| 27.5% rate, Kennzahl mapping | `AT_SONDERSTEUERSATZ` (estimate constant) + `kennzahl_for_disposal_category()` |
| PDF / XLSX adviser handoff | `hooks.write_text_pdf` + the generic XLSX writer in `kassiber/core/reports.py` |

Kassiber does not grow a second Austrian tax engine: the regime classification
and per-disposal cost basis are RP2's, read back off the journal entries.

## Engine Boundary

`compute_deemed_disposal(conn, profile, state, *, departure_date, destination)`
in `kassiber/core/exit_tax.py` is the keystone. `report_exit_tax()` resolves
scope, requires processed journals, fetches the ledger `state`, and calls it.
The output is the frozen `ui.reports.exit_tax_preview` payload (camelCase),
reused verbatim by the CLI JSON output.

- **Austria:** exclude Altbestand (tax-free), apply 27.5% to net Neubestand
  gains, set the collection-timing flag from `destination`.
- **Generic fallback:** all holdings pool into one bucket, total unrealized gain
  reported, no rate applied (`estimatedTaxRate`/`estimatedTax` null). Every
  non-AT book still gets a meaningful "what you hold and its unrealized gain"
  view.

## Austrian Rules The Report Must Respect

- **Baseline:** §27b EStG crypto income, flat **27.5%** Sondersteuersatz
  (§27a Abs 1 Z 2). Regime in force since 1 March 2022.
- **Altbestand cutoff is 1 March 2021** (one year before the regime start; do
  not confuse the two). Acquired before `2021-03-01 Europe/Vienna` =
  Altbestand. Already encoded as `AT_NEU_CUTOFF` in `kassiber/core/austrian.py`.
- **Altbestand is tax-free on realization → excluded from the exit-tax base.**
  Only Neubestand unrealized gains are deemed-disposed.
- **Trigger:** §27 Abs 6 Z 1 — deemed disposal at gemeiner Wert when Austria
  loses/restricts its taxing right (residence given up, center of vital
  interests abroad, etc.).
- **Collection timing (the modeled branch):**
  - **EU/EEA destination →** Nichtfestsetzung (§27 Abs 6 Z 1 lit a): tax is
    **assessed but not collected until actual sale** (open-ended deferral). Note
    this is *not* the 7-year Ratenzahlung — that applies to business assets
    under §6 Z 6, not private §27 capital assets.
  - **Third (non-EU/EEA) country →** tax **due immediately** at departure.
- **Step-up on return** (§27 Abs 6 Z 1 lit e): re-entry resets basis to FMV at
  entry. Out of scope for v1 output but note it in the handoff text.
- **Formula:** `exit_tax ≈ 27.5% × Σ max(0, FMV_departure − cost_basis)` over
  **Neubestand lots only**.

## Surfaces

- **CLI (built):** `reports exit-tax --departure-date <YYYY-MM-DD>
  --destination eu_eea|third_country` (plain/JSON/CSV), plus
  `reports export-exit-tax-pdf` / `reports export-exit-tax-xlsx` (each
  `--file <path>`) for the stamped handoff.
- **Daemon (built):** `ui.reports.exit_tax_preview` (query) and
  `ui.reports.export_exit_tax_pdf` / `ui.reports.export_exit_tax_xlsx`
  (mutations). `exit_tax_preview` is in `_DIRECT_AUTO_JOURNAL_REFRESH_KINDS`
  (and the AI auto-refresh set) so the daemon processes journals before serving
  it. Registered in `SUPPORTED_KINDS`, the Vite bridge allowlist, and the Tauri
  Rust allowlist (kept in lockstep — a drift test enforces this).
- **Desktop (built):** standalone `/exit-tax` route + a side-nav "exit-tax" item
  (`ui-tauri/src/routes/ExitTax.tsx`). Controls: departure-date picker +
  EU/EEA-vs-third-country select. Output: headline-liability hero,
  collection-timing banner ("assessed but deferred until sale" vs "due now"),
  per-(asset, regime) deemed-disposal table (Neubestand taxable / Altbestand
  excluded, with Kennzahl), wallet-holdings context table, FMV-source
  disclosure, assumptions + review-gate panel, and PDF/XLSX export with the
  shared save flow. An incomplete-status strip links to the quarantine queue.
  Departure date and destination are report parameters; a persisted
  profile-level departure setting is a later option, not v1.

## Open Questions

These mirror the `AT-00x` style in `07-austrian-tax-open-questions.md`. They are
planning inputs and review notes, not tax advice. The report must name which
defaults it invoked.

| ID | Question | Current default | Gate |
|---|---|---|---|
| EXIT-001 | EU/EEA classification of destination | user-declared destination → eu_eea / third_country flag against a maintained EU/EEA list | user declaration; report states the assumption |
| EXIT-002 | §27 Abs 6 deferral mechanism | EU/EEA = Nichtfestsetzung until sale; third country = immediate; no installment for private §27 | review required |
| EXIT-003 | Planned 1 Jul 2026 tightening (annual proof when deferred gain > €100k) | announced, not enacted — do **not** hard-code; surface as a forward-looking note | re-verify against enacted statute before relying |
| EXIT-004 | Derived tokens (staking/mining/airdrop) | Neubestand regardless of underlying coin age, €0 acquisition cost → enter exit base | review; moderate confidence (rests on BMF/VO, not clean §27b cite) |
| EXIT-005 | FMV at departure date | best available transaction/date rate; do not imply intraday coverage unless cache supports it (see AT-007) | rate-coverage check via `rates range` |
| EXIT-006 | Altbestand exclusion proof | exclude only lots with provable pre-cutoff acquisition; unknown-basis/quarantined lots cannot be confidently excluded | quarantine / incomplete-status surfaced |
| EXIT-007 | Deemed-disposal losses on Neu lots | net within the exit computation per Austrian capital-income loss rules; multi-year carryforward stays out of scope (see 06) | review required |
| EXIT-008 | Departure date, DTA tie-breaker, dual residence | taken as user input; Kassiber does not determine residency facts | adviser determines; out of scope |
| EXIT-009 | Neubestand basis when Alt+Neu coexist | v1 sums Neu-only inflow cost minus Neu-only disposal basis from journal entries; if rp2's `moving_average_at` blends Alt acquisitions into the Neu pool, the entries walk may diverge slightly | the synthetic-disposal precision path (forced `at_regime_override` legs) closes this; review |
| EXIT-010 | Regime of a non-reportable disposal | a disposal the user marked non-reportable (`taxability_override=0`) carries no `at_category`, so the entries walk infers its regime from the disposal date — may misattribute Altbestand sold after the cutoff | surfaced as an assumption note when such rows exist; the synthetic-disposal path would also resolve it; review |

## Gotchas

- **v1 reads, it does not synthesize.** `compute_deemed_disposal` walks the
  in-memory ledger state only — no synthetic rows, no engine re-run, nothing
  persisted. The per-account balance-gate hazard only applies to the deferred
  synthetic-disposal precision path; if that path is built later, the
  full-balance legs must sell exactly the available per-account inventory so
  RP2's `BalanceSet` does not go negative (see `project_rp2_per_account_balance_gate`).
- **Incomplete estimates:** if quarantines or missing prices exist, the estimate
  is incomplete. The payload surfaces `status.needsJournals` / `status.quarantines`
  and the GUI shows a strip linking to the quarantine queue; never present a
  partial estimate as final.
- **Re-run journals first.** The estimate is only trustworthy after journals are
  processed; the daemon auto-processes for `exit_tax_preview`, and the CLI/report
  path requires processed journals.

## Out Of Scope

- exit tax on non-crypto assets (the adviser computes those; Kassiber covers the
  crypto slice only)
- FinanzOnline / authority submission
- residency determination, DTA tie-breaker resolution, exact departure-day legal
  facts
- the §27 Abs 6 lit e step-up computation for returnees (note only)
- non-Austrian exit-tax math beyond the generic unrealized-gain fallback
- NFTs / asset tokens (outside §27b)

## Sources To Recheck When Touching This File

- §27 Abs 6, §27a, §27b EStG (RIS)
- BMF, [Steuerliche Behandlung von Kryptowährungen](https://www.bmf.gv.at/themen/steuern/sparen-veranlagen/steuerliche-behandlung-von-kryptowaehrungen.html)
- current EU/EEA member list
- enacted text of any 1 Jul 2026 Wegzugsbesteuerung change
- Steuerberater/practitioner guidance used by the project owner
