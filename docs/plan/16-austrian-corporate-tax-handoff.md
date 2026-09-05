# Austrian Corporate-Tax Handoff (K1 / K2)

> Historical research draft. The user's accepted full-accounting scope and
> privacy-preserving AI requirements are consolidated in
> [plan 17](17-general-accounting-and-private-ai-spec.md). Its requirements
> supersede this draft's general-ledger prohibition and staged delivery scope.
> Legal/form claims below remain research inputs requiring current verification.

**Status:** Proposed follow-up after the shipped Austrian E 1kv slice.
**Current source of truth:** this document, `docs/plan/06-austrian-tax-engine.md`,
`docs/austrian-handoff.md`, `kassiber/core/reports.py`, `kassiber/core/report_context.py`,
and TODO.md.
**As of:** 2026-09-03.
**Legal gate:** Kassiber is not tax advice. Austrian corporate filings must be
reviewed by a Steuerberater before filing.

## Short Answer

If Kassiber should support Austrian corporate returns beyond today's E 1kv
handoff, the correct product is **not** "more Austrian tax math" and **not**
"K2 for Vereine, then reuse the same thing for normal corporations".

The right shape is:

- one shared Austrian corporate-tax handoff core
- one adapter for **K2** entities that do **not** fall under `§ 7 Abs. 3 KStG`
- one separate adapter for **K1** entities that **do** fall under
  `§ 7 Abs. 3 KStG`
- one later adapter for **K2 private foundations**

Kassiber must stay the **Bitcoin subledger, evidence, reconciliation, and
reviewed handoff layer**. It must not become a general ledger, annual-accounts
system, payroll engine, or full Körperschaftsteuer engine.

## Official Filing Baseline Kassiber Must Bind To

As of 2026-09-03, the official Austrian annual-declaration package published by
the BMF for software vendors is **Jahreserklärung 2025**. That package includes
`K1`, `K1-B`, `K1-V`, `K1g`, `K10`, `K11`, `K12`, `K12a`, `K12a-G`, `K2`,
`K2a`, `K2b`, and `K2kv`, plus XML structure, XSD schema, and validation
documents.

The official USP / BMF filing guidance currently says:

- `K1` is for unbeschränkt steuerpflichtige, rechnungslegungspflichtige
  Körperschaften such as GmbH and AG.
- `K2` is for unbeschränkt steuerpflichtige Körperschaften that do **not** fall
  under `§ 7 Abs. 3 KStG`, for example certain associations.
- corporate returns are generally filed electronically via FinanzOnline
- the BMF also publishes a data-stream/XML route for external software

Two important scope facts fall out of the current official material:

1. `K1` and `K2` are distinct filing regimes, not two skins over the same
   crypto report.
2. `K2` is not just a "Verein form". The current `K2` form itself contains
   dedicated private-foundation sections, including a cryptocurrency row in the
   private-foundation area.

## Product Boundary

Kassiber owns:

- wallet sync/import
- local storage and provenance
- BTC/LBTC reconciliation and reviewed economic interpretation
- pricing provenance and report gates
- annual Bitcoin subledger output
- reviewed Austrian handoff rows where the legal classification is known
- immutable evidence, hashes, and audit receipts for what Kassiber claimed

Kassiber does **not** own:

- the company or association general ledger
- payroll / wage tax
- VAT / RKSV
- full statutory annual accounts
- full Körperschaftsteuer computation across every income type
- FinanzOnline account operation on the user's behalf
- silent legal classification of uncertain tax facts

Do not market this as "full K1/K2 automation from wallet data". That claim
would be false.

For this track, **complete** has a deliberately narrow meaning: every field,
annex decision, external accounting total, Bitcoin reconciliation, validation
rule, and evidence reference inside a published supported-scenario envelope is
present and reviewed. Anything outside that envelope, or any missing required
fact inside it, blocks completion. A syntactically valid XML document with
blank or guessed non-Bitcoin sections is not a complete return.

## Filing Profiles Kassiber Must Distinguish

### 1. `k2_non_section_7_3_general`

Target examples:

- certain associations
- other unbeschränkt steuerpflichtige Körperschaften outside `§ 7 Abs. 3 KStG`

Key traits:

- multiple income categories can coexist on `K2`
- crypto can land in different tax lanes depending on facts
- charitable / exempt status can sharply change what is taxable

### 2. `k1_section_7_3_accounting_corporation`

Target examples:

- GmbH
- AG
- other accounting corporations under `§ 7 Abs. 3 KStG`

Key traits:

- income is treated through the `§ 7 Abs. 3` / `§ 5 Abs. 1 EStG` business path
- crypto must be bridged into the accounting / business-income layer
- this is **not** a reuse of the current E 1kv model

### 3. `k2_private_foundation`

Target examples:

- Privatstiftungen using `K2`

Key traits:

- current `K2` form has private-foundation-only rows
- the 2025 form includes a dedicated crypto row in that section
- this is materially different from both the ordinary Verein path and the `K1`
  corporation path

This should be tracked as a later adapter, not smuggled into a "Verein MVP".

## Accounting Regimes Kassiber Must Distinguish

The filing form alone is not enough. Kassiber also needs a separate accounting
regime axis:

### `verein_einnahmen_ausgaben`

For smaller associations under `VerG § 21`, the baseline is
Einnahmen-Ausgaben-Rechnung plus Vermögensübersicht, not a full UGB-style
annual account.

### `verein_jahresabschluss`

For larger associations under `VerG § 22`, the baseline moves to
Jahresabschluss with Bilanz and GuV; larger thresholds add Anhang and
Abschlussprüfung obligations.

### `ugb_jahresabschluss`

For ordinary accounting corporations, the tax handoff must reconcile against
their annual accounts / ledger totals.

This accounting-regime axis must stay independent from the filing-profile axis.
`K2` is not enough to infer whether the source accounting is
cash-style-Verein, larger-Verein annual accounts, or something else.

## Austrian Tax Facts That Must Be Explicit, Not Guessed

### Association / nonprofit classification

For associations, Kassiber must not guess whether a Bitcoin flow belongs to:

- ideeller Bereich
- Vermögensverwaltung
- unentbehrlicher Hilfsbetrieb
- entbehrlicher Hilfsbetrieb
- begünstigungsschädlicher wirtschaftlicher Geschäftsbetrieb / Gewinnbetrieb

The BMF guidance for associations makes this distinction load-bearing. A
charitable association can be generally privileged while still having taxable
business operations. The same wallet history can therefore produce very
different `K2` consequences depending on the sphere assignment.

### Capital lane versus business lane

Kassiber must not assume that every Austrian crypto amount for a corporate body
belongs in a capital-income annex. For `K2` entities this may be partly true in
some cases and false in others. For `K1` entities it is the wrong default.

### Domestic versus foreign / withheld tax metadata

Today's Austrian slice explicitly assumes the current ausländisch /
self-custody lane and lacks structured domestic-provider withheld-KESt data.
That limitation becomes more dangerous, not less, in `K1` / `K2`.

### Filing-year and schema version

The BMF publishes year-specific XML/XSD/validation packages. Kassiber must
bind every corporate export to an explicit form package version. No drifting
"latest Austrian corporate report" mode is acceptable.

## Hard Blocker Found During Adversarial Review

Before any `K1` / `K2` expansion, re-check the current Austrian
moving-average-pool assumption against primary Austrian guidance.

Current repo docs/code today say:

- Austrian Neuvermögen is pooled globally per asset across wallets
- `resolve_pool_id()` intentionally returns one constant pool id
- report verification text also describes per-asset cross-wallet pooling

But current BMF crypto guidance says the moving average applies to units of the
same cryptocurrency that are acquired in sequence and held on the **same**
crypto address / wallet.

This is directly relevant to any `K2kv` path and to reuse of today's `§ 27b`
facts. It must not be generalized blindly to `K1`, where the accounting and
tax-basis bridge needs its own reviewed rule set. Do not broaden Austrian
product claims until the affected corridors are resolved separately with
primary-source and Steuerberater review and, if needed, a coordinated
Kassiber/RP2 change.

## What Kassiber Must Receive From Outside Its Native Bitcoin Scope

The missing work is not just new rendering. A full handoff needs explicit
non-Bitcoin inputs.

| Input group | Needed for | Source of truth | Can Kassiber derive it? |
| --- | --- | --- | --- |
| legal entity master data | K1/K2 header, filing metadata | user / accountant | no |
| filing profile (`K1` vs `K2`) | declaration routing | user / accountant | no |
| accounting regime (`VerG 21`, `VerG 22`, UGB) | reconciliation and attachments | user / accountant | no |
| annual-accounts totals | K1/K2 body, reconciliation | external books / accountant | no |
| tax-sphere assignment for BTC activity | correct row mapping | user / accountant review | no |
| carryforwards / off-book tax adjustments | final tax result | accountant | no |
| non-BTC income / expense totals | full return | external books | no |
| K10 / K11 / K12 / K12a / K1g / K2a / K2b facts when applicable | annex completeness | accountant / external systems | no |
| domestic KESt / foreign-tax-credit metadata | several current and future rows | banks / brokers / accountant | not today |
| Bitcoin transaction evidence and reviewed journal rows | crypto subledger | Kassiber | yes |
| immutable report snapshot / evidence receipt | audit trail | Kassiber | yes |

This is the main reason the product must be framed as a **corporate-tax
handoff** rather than a standalone filing engine.

## Proposed Architecture

Build one deep module before any form-specific UI or XML work:

```python
CorporateTaxHandoffBuilder.build(
    inputs: CorporateTaxHandoffInputs,
) -> CorporateTaxHandoff
```

### Input types

`CorporateTaxHandoffInputs` should contain:

- `filing_profile`
- `accounting_regime`
- `tax_year`
- `form_package_year`
- `entity_snapshot`
- `bitcoin_bridge_snapshot`
- `annual_accounts_bridge`
- `annex_inputs`
- `manual_classifications`
- `report_context`

### Output contract

`CorporateTaxHandoff` should contain:

- normalized declaration rows by semantic bucket, not by PDF cell name
- explicit annex payloads by annex id
- required attachments list
- blocking issues
- assumptions / unresolved facts
- reconciliation results against annual accounts
- immutable claim receipt: source rows, hashes, review state, form package

### Adapters over the shared core

Put the form-specific logic in thin adapters:

- `K2NonSection7_3Adapter`
- `K1Section7_3Adapter`
- `K2PrivateFoundationAdapter`

Their job is:

- validate that the shared handoff facts are sufficient for that filing path
- map semantic buckets to current form rows / annex rows
- emit explicit blockers instead of zeros when data is missing
- version-bind every output to the chosen package year

### Why this seam fits Kassiber

This follows the current product boundary:

- Kassiber already owns reviewed Bitcoin evidence and report gating
- report exports already flow through core report builders and `ReportContext`
- Austrian tax math already belongs on the RP2 side, not in new Kassiber math

The new module should therefore be a **reviewed bridge from Bitcoin evidence to
corporate filing facts**, not another tax engine.

### Suggested module boundary

Keep form mechanics and business facts separate under one deep module:

```text
kassiber/core/austrian_corporate_tax/
  models.py                 # exact domain types and immutable result shapes
  applicability.py          # K1/K2/defer plus annex routing, fail closed
  accounting_bridge.py      # imported annual-accounts facts and reconciliation
  bitcoin_bridge.py         # evidence-backed BTC facts, never raw form codes
  compiler.py               # reviewed semantic facts -> form fields
  validation.py             # cross-form and year-pack semantic rules
  xml_export.py             # deterministic BMF envelope generation
  snapshots.py              # saved/filed/amended receipts
  form_packs/at/2025/       # reviewed mappings and source manifest
```

`bitcoin_bridge.py` must expose quantities, tax-basis facts, valuations,
provenance, and unresolved issues. It must not decide Verein spheres, annual-
account book values, or K1/K2 field placement. Those decisions belong to
reviewed inputs and the corridor-specific compiler.

Use thin adapters over the shared compiler:

- `K2NonSection7_3Adapter`
- `K1Section7_3Adapter`
- later `K2PrivateFoundationAdapter`

Sharing the canonical fact and validation infrastructure is desirable;
sharing the legal mapping is not.

### Persistence model

Add typed, append-only or versioned records for:

- entity/tax profile and fiscal periods
- applicability answers and the rule-pack version that evaluated them
- imported accounting fact sets with source hashes and review state
- reviewed Verein sphere and tax-lane classifications
- typed manual tax adjustments with reason, author, evidence, and supersession
- compiled declaration snapshots with source-fact and form-pack digests
- artifact/filed markers and amendment links

Reuse the existing managed attachment store and filed-report snapshot pattern;
do not create a second blob store or overwrite filed facts. A changed input
creates a new declaration snapshot and marks the old draft stale. A filed
snapshot remains immutable and any correction becomes a linked amendment.

### Exact amounts and provenance

New official-filing amounts must use decimal strings or integer euro-cents,
not binary floats. Define rounding once at the year/form boundary. Each emitted
field must carry a trace to its imported accounting fact, reviewed adjustment,
or Kassiber Bitcoin evidence set, including source hash and review status.

Entity tax numbers, representative identity/address data, FinanzOnline
identifiers, and source documents remain inside the SQLCipher boundary and are
excluded from AI context, diagnostics, default audit packages, and replication
unless a separate explicit policy allows them. No FinanzOnline credential is
needed for the first product and none should enter the AI path.

## Official 2025 Machine Contract

The researched BMF 2025 package is more than an XSD:

- the XML envelope is `ERKLAERUNGS_UEBERMITTLUNG`
- declarations are distinguished by `ERKLAERUNG art="K1"` and `art="K2"`
- the published example declares `iso-8859-1`
- the package has XML structure documentation, an XSD, value sets, validation
  rules, and a document-version record
- K1 includes annual-account and tax-reconciliation structures
- K2 includes its income-category and annex structures

The local form pack must pin source URLs, retrieval date, document versions,
and hashes for every one of those inputs. XSD validity alone is insufficient:
the BMF validation document was updated after the example XML within the same
filing-year package, so unreviewed source drift must fail closed.

Minimum semantic checks for the 2025 pack include:

- K2 totals such as `KZ 610`, `636`, and `650` reconcile to their applicable
  K2a/K2b/K2kv and related inputs
- K1 annual-account profit and book-to-tax corrections reconcile through the
  relevant `KZ 704` path
- calculated K1 totals such as `KZ 777` satisfy the published formula
- every required annex implied by applicability answers is present
- absent data is distinguished from a reviewed numeric zero

The export sequence is therefore: compile canonical facts, run internal
semantic validation, generate deterministic XML, validate against the pinned
XSD, and render the human review bundle from the same facts.

FinanzOnline test uploads do not constitute filing. A return can be marked
`filed` only from a positive processing confirmation for the production
transmission; generated, XSD-valid, test-accepted, submitted, and filed are
separate states.

## Initial Supported-Scenario Envelopes

### First K2 envelope

- filing year 2025
- one Austrian, unbeschränkt steuerpflichtige entity per Kassiber profile
- ordinary Verein/non-`§ 7 Abs. 3` corridor, not a private foundation
- explicit VerG accounting regime and reviewed Verein sphere assignments
- imported final non-Bitcoin accounting facts
- only annexes explicitly implemented by the form pack
- no group, reorganization, foreign permanent-establishment, or other special
  regime unless separately added and tested

### First K1 envelope

- filing year 2025
- standalone domestic `§ 7 Abs. 3` accounting corporation
- imported and approved annual accounts plus a reviewed tax-correction bridge
- reconciled Bitcoin subledger contribution
- no tax group, bank/insurance special regime, reorganization, or unsupported
  cross-border case

The applicability engine must name the first unsupported answer and stop; it
must never silently coerce an out-of-envelope entity into the closest adapter.

## What Is Missing Versus Today

Compared with the shipped Austrian E 1kv path, Kassiber is currently missing:

1. **Corporate filing profiles.**
   There is no `K1`/`K2` corporate routing model today.
2. **Accounting-regime modeling.**
   There is no explicit `VerG § 21` versus `VerG § 22` versus UGB axis.
3. **Association tax-sphere classification.**
   No reviewed model for ideeller Bereich / Vermögensverwaltung / taxable
   business lanes exists today.
4. **Annual-accounts bridge.**
   There is no imported or reviewed reconciliation layer from external books to
   Kassiber's Bitcoin subledger totals.
5. **Corporate annex support.**
   No support exists for K10/K11/K12/K12a/K1g/K2a/K2b-level completeness.
6. **Corporate withheld-tax metadata.**
   The current Austrian path cannot populate domestic withheld-KESt-sensitive
   rows.
7. **Corporate year/version binding.**
   No K1/K2-specific schema/version registry exists today.
8. **FinanzOnline XML export.**
   Current Austrian support is report handoff only.
9. **Corporate launch gates and falsification fixtures.**
   No test matrix exists yet proving K1/K2 fail closed when the accounting
   bridge or sphere assignment is incomplete.

## Must-Not-Build Boundaries

Do not build any of the following:

- automatic "best guess" sphere allocation for nonprofit associations
- silent promotion of all crypto to `K2kv`
- reimplementation of `§ 7 Abs. 3` business-income tax logic inside Kassiber
- a fake "full K1/K2" mode that leaves non-Bitcoin sections blank without
  saying so
- yearless XML generation against whichever BMF schema happens to be current
- a filing mode that ignores unresolved report blockers or missing annual
  accounts reconciliation
- direct FinanzOnline submission before the XML payload has its own validation,
  test-submission, and audit receipt layer

## Phase Plan

### Phase 0 — Claims freeze and drift repair

Completion:

- docs state that Austrian support is currently partial, not absent
- product/docs state clearly that E 1kv is shipped, but full K1/K2 is not
- the new K1/K2 scope is described as handoff, not full accounting software

### Phase 1 — Shared corporate handoff core

Completion:

- add filing-profile and accounting-regime types
- add entity master snapshot and annual-accounts bridge input models
- add the year-bound form-pack registry and source-drift gate
- add fail-closed blockers for missing sphere, missing reconciliation, missing
  annex facts, unsupported year package
- persist immutable handoff snapshots and receipts

### Phase 2 — K2 non-`§ 7 Abs. 3` MVP

Completion:

- reviewed `K2` handoff path exists for the narrow, declared scope
- association sphere assignment is explicit and auditable
- `K2kv` population is supported only where the classification is explicit
- output clearly lists what still comes from external books / accountant
- manual-entry / adviser handoff is supported before machine export; it must not
  be described as submitted or filed

### Phase 3 — K1 `§ 7 Abs. 3` MVP

Completion:

- reviewed `K1` handoff exists for accounting corporations
- Bitcoin rows reconcile against annual-account totals
- no reuse of the E 1kv/K2kv model where the business-income path is required
- applicable annex blockers fail closed

### Phase 4 — K2 private-foundation adapter

Completion:

- dedicated private-foundation routing exists
- the foundation-specific crypto row is treated as its own semantic target
- nothing is backfit into the Verein adapter

### Phase 5 — XML / submission-grade export

Completion:

- year-bound XML/XSD generation for supported packages
- formal XSD plus published semantic and cross-form validation
- imported validation errors mapped back to user-visible blockers
- deterministic output for identical facts and controlled envelope metadata
- manual FinanzOnline test and production-upload workflow with immutable
  confirmation receipts
- direct credentialed submission remains a separate later security decision,
  not an MVP requirement

## Fail-Closed Rules

The corporate path must refuse "complete" output when any of the following is
true:

- unresolved journal quarantines affect the tax year
- BTC activity is missing a reviewed sphere / tax-lane classification
- annual-accounts bridge totals do not reconcile
- a required annex has no reviewed input
- the user selected an unsupported form package year
- domestic / foreign withholding facts are needed but absent
- output would require a private-foundation path but the entity is running
  through the ordinary `K2` adapter
- the selected filing profile contradicts the declared accounting regime

Missing data must surface as blockers, never as zero rows.

## Falsification Tests

Before launch, Kassiber must prove it rejects the following dangerous cases:

1. a charitable association whose wallet activity is left unassigned between
   Vermögensverwaltung and taxable business
2. a `K1` corporation routed through a `K2` capital-income path
3. a corporate report whose Bitcoin totals do not reconcile to the imported
   annual accounts
4. a year-2025 XML export built with a later package mapping
5. a private foundation with crypto rows forced through the ordinary Verein
   adapter
6. a domestic-KESt case rendered with today's foreign/self-custody assumption
7. a report exported while material journal quarantines still exist
8. a missing value silently serialized as numeric zero
9. binary-float rounding changes a declared cent value
10. XML text injects markup or cannot round-trip through the required encoding
11. the BMF changes a source document without a reviewed form-pack bump
12. a test-upload confirmation is presented as a filed declaration

## Launch Gates

Do not ship a `K1` or `K2` feature flag without all of:

- explicit scope statement in UI/CLI/docs
- year-bound package registry
- immutable handoff snapshot with source hashes
- annual-accounts reconciliation proof
- association sphere-review workflow where relevant
- fail-closed blockers
- fixtures covering Verein, ordinary corporation, and private-foundation
  boundaries
- exact-money, encoding, XML-injection, XSD, and published semantic-rule tests
- byte-identical XML for identical canonical facts and controlled metadata
- a successful FinanzOnline test upload for each supported golden case
- Steuerberater review of the supported scope and naming

## Open Questions That Need External Confirmation

These should be resolved with Austrian tax counsel before product claims are
broadened:

- exact practical attachment expectations for each supported filing path:
  the specific Körperschaftsteuer page still speaks about attaching annual
  accounts, while the general FinanzOnline page says many attachments are only
  produced on request; Kassiber should support exportable attachments without
  hard-coding one universal submission assumption
- exact K2 treatment for crypto held by associations in Vermögensverwaltung,
  especially where self-custody means there is no bank-side KESt workflow
- scope of cases where `K2kv` is the right lane versus the business-income lane
- whether a manual-entry-first K1/K2 handoff is the acceptable first shipping
  step before XML export

## Recommended Shipping Order

If the immediate customer need is "Verein + K2", do **not** start by drawing
PDF rows. Start with the shared handoff core and the accounting/sphere blockers.
Then ship:

1. Phase 1 shared core
2. Phase 2 narrow `K2` MVP
3. Phase 3 `K1` adapter
4. Phase 4 private-foundation adapter
5. Phase 5 XML/export

That order keeps Kassiber inside its moat: reviewed Bitcoin evidence plus
honest, auditable handoff.

## Source Notes

- USP, Körperschaftsteuererklärung, last updated 2026-01-01:
  `https://www.usp.gv.at/themen/steuern-finanzen/koerperschaftsteuer-ueberblick/koerperschaftsteuererklaerung.html`
- BMF, Jahreserklärungen / data-stream package list:
  `https://www.bmf.gv.at/services/finanzonline/informationen-fuer-softwarehersteller/softwarehersteller-jahreserklaerungen.html`
- BMF, Datenstromübermittlung:
  `https://www.bmf.gv.at/services/finanzonline/informationen-fuer-softwarehersteller/datenstromuebermittlung.html`
- USP, FinanzOnline guidance, last updated 2026-01-01:
  `https://www.usp.gv.at/themen/steuern-finanzen/steuerliche-rechte-und-pflichten/weitere-informationen-zu-steuerlichen-rechten-und-pflichten-als-unternehmen/finanzonline.html`
- BMF FAQ for associations / nonprofits:
  `https://www.bmf.gv.at/themen/steuern/spenden-gemeinnuetzigkeit/haeufig-gestellte-fragen-zu-vereinen-gemeinn%C3%BCtzigkeit-und-registrierkassenpflicht.html`
- RIS: `KStG § 7`, `KStG § 5 Z 6`, `VerG §§ 21-22`, `BAO §§ 45-45a`
