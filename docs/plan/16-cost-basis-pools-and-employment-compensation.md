# Country-configurable cost-basis pools and compensation acquisitions

**Status:** The compensation-acquisition and global-only foundation is
implemented locally across Kassiber and RP2, including the global
salary/transit/savings fixture. A valued `wages` receipt is an ordinary
acquisition for RP2 while its raw kind remains factual provenance in Kassiber.
Production narrower-pool activation remains
disabled: RP2's existing `per_wallet` two-pass engine cannot replay transfers
and later taxable source events through one chronological inventory cursor.
Fee-bearing `per_wallet` transfers therefore fail closed. Release still
requires RP2 publication, a tested Kassiber dependency pin, and the full G5
verification cutoffs below.
**Executable backlog:** the single matching item in `TODO.md`.
**Current-truth documents until cutover:** `docs/austrian-handoff.md`,
`docs/plan/06-austrian-tax-engine.md`, current code, and current tests.

## Objective

Make two currently-wrong Austrian paths correct without building a second tax
engine in Kassiber:

1. BTC received as employment compensation must create acquisition basis but
   must not be reported as ongoing crypto income.
2. A country may apply cost-basis averaging to a scope narrower than the whole
   book, and a non-taxable transfer between scopes must carry the exact effective
   basis into the destination scope.

The infrastructure must be country-neutral. Kassiber records factual custody
and source semantics; RP2 core owns reusable pool arithmetic; an RP2 country
plugin selects the legal pool policy. Kassiber does not attempt to report the
user's general employment income.

## Terminal states

The plan has one mandatory completion state and one conditional activation
state.

Current cut line: all current country policies advertise only `global`;
`wallet` fails closed. RP2 fixes generic moving-average transfer basis, but its
current `at_pool=<id>` wire
format cannot express both source and destination pools, and its existing
`per_wallet` replay is not a production-safe substitute. No second marker
contract or chronological engine was invented.

### Mandatory foundation complete

This state must ship regardless of the Austrian wallet-scope legal decision:

- `wages` remains factual Kassiber transaction provenance and enters RP2 as an
  ordinary basis-bearing `BUY` acquisition.
- No separate employment-income journal or E 1kv row is created. General
  wage-tax treatment stays outside Kassiber's Bitcoin-only reporting lens.
- A profile has one `cost_basis_pool_scope`. Existing profiles migrate
  to `global`, preserving their current results until a user deliberately
  changes the setting.
- RP2's country-neutral effective-basis transfer bug is fixed and regression
  tested.
- Normalized events carry an opaque `cost_basis_pool_id`. Normalized internal
  transfers carry distinct source and destination pool ids.
- RP2 materializes the effective basis on the destination side of fee-free
  transfers; fee-bearing `per_wallet` transfers fail closed until transfers and
  taxable source events share one chronological inventory cursor.
- Existing global-scope books remain unchanged, and compensation acquisitions
  do not enter E 1kv crypto-income totals because acquisitions are not taxable
  result rows.
- Changing the profile scope invalidates journals and uses the existing stale /
  saved-report safeguards. Raw transactions are never rewritten.
- The generic/global path and existing Austrian global-pool books remain
  regression-green.

### Conditional Austrian wallet activation

This state is reached only if the legal/product ship gate below is satisfied:

- The Austrian country policy advertises `wallet` in addition to `global`.
- Kassiber does not assume that a wallet is the legal pool in any other country;
  each country controls its allowed scopes.
- The salary -> transit/savings-wallet scenario processes from imported
  transactions through journals and the Austrian report with exact, test-pinned
  results.
- Reports disclose the reviewed wallet scope and its legal-review assumption.

If the gate is not satisfied, the mandatory foundation is still a completed,
shippable bugfix. Austrian `wallet` remains unavailable and the plan stops
without approximating it.

## Hard cut-off lines

These are not follow-up tasks inside this plan:

- No payroll calculation, employer payment execution, Lohnzettel generation, or
  employment-law decision engine.
- No FinanzOnline submission or general wage-tax return.
- No domestic-provider KESt automation. The current self-custody /
  `ausländisch` report limitation remains explicit.
- No automatic address-, UTXO-, xpub-, provider-, or user-defined group pooling.
  The opaque identifier is future-compatible with those scopes, but this plan
  permits only `global` and, conditionally, `wallet`.
- No per-transaction pool-override table and no new pool-membership table.
- No Austrian RP2 `per_wallet` application mode. A legal cost-basis pool is not
  assumed to equal RP2's `(exchange, holder)` account partition.
- No second chronological inventory engine for RP2 `per_wallet` transfers.
  Fee-bearing shapes fail closed; fee-free transfer materialization remains a
  gated seam rather than a production narrow-pool engine.
- No new report, new desktop route, new review queue, or salary wizard.
- No second Austrian calculation path in Kassiber.
- No broad RP2 plugin framework rewrite. Extend the existing country interface
  only as far as the two candidate scopes require.
- No coupling to RP2 wheel publication, unrelated Kennzahl/domestic-provider
  work, staking classification, disposal-order election, or address discovery.

If Austrian counsel requires address-level treatment for descriptor/xpub
self-custody, stop after the generic RP2 basis-carry and acquisition-mapping
work. Do not approximate address pools with wallet ids. Address attribution then
needs its own separately-approved plan.

## Legal/product ship gate

Before `wallet` becomes selectable for an Austrian profile, record a reviewed
decision stating that wallet-level pooling is acceptable for the intended
self-custody setup. This can be a Steuerberater memo or an explicit project-owner
acceptance of the remaining legal uncertainty.

This is a release-time gate, not per-book runtime data. Until it is accepted,
the Austrian country policy hardcodes `global` as its only allowed scope. The
accepted implementation change adds `wallet` to that country allowlist; no
second profile flag or persisted legal-memo state is introduced.

This gate does not block:

- treating valued compensation as an ordinary acquisition;
- fixing RP2's country-neutral moving-average transfer bug;
- adding the generic pool fields with legacy `global` behavior;
- proving the engine with non-production/fake-country tests.

It does block:

- presenting Austrian wallet scope as report-ready;
- changing the default of an existing Austrian profile;
- claiming descriptor/xpub wallet grouping as settled law.

## Starting gaps and current disposition

1. Kassiber's Austrian-only `resolve_pool_id()` seam has been replaced locally
   by the country-neutral profile policy and opaque pool resolver.
2. `NormalizedTaxTransfer` now carries distinct source and destination ids.
   Austrian cross-pool serialization remains intentionally blocked because the
   RP2 plugin has no reviewed two-ended marker contract.
3. RP2's initial `TransferAnalyzer` correction failed adversarial conservation,
   straddling-fee, and rollback tests. Those are mandatory blockers until the
   revised core implementation and full suite pass.
4. Kassiber maps valued `wages` receipts to RP2 `BUY` acquisitions while
   retaining the raw source kind. Runtime cutover waits for RP2 publication and
   pinning.
5. RP2 now rejects `per_wallet` when it would bypass an overridden country-wide
   computation hook.
6. Current-truth Kassiber docs use the generic pool contract; publication notes
   must still record the final RP2 revision.

## Target module seams

### Kassiber: factual normalization

Keep the existing `NormalizedTaxEvent` / `NormalizedTaxTransfer` interface and
replace the Austrian-only pool field with country-neutral facts:

```text
NormalizedTaxEvent.cost_basis_pool_id: str | None

NormalizedTaxTransfer.from_cost_basis_pool_id: str | None
NormalizedTaxTransfer.to_cost_basis_pool_id: str | None
```

The identifiers are opaque outside Kassiber's resolver. For the two shipped
scopes:

- `global` resolves to one stable id per asset/profile computation;
- `wallet` resolves to the existing stable `wallet_id`.

Do not expose raw addresses or descriptor material through this interface.
Country-specific adapter code may serialize the generic ids into existing RP2
plugin markers such as `at_pool`; the generic normalized model must not acquire
more `at_*` pool fields.

Kassiber remains responsible for:

- determining the source/destination wallets from reviewed custody evidence;
- requiring a fiat value for a `wages` acquisition;
- preserving pricing provenance and attachments;
- quarantining incomplete transfer or valuation evidence;
- invalidating derived journals when the profile scope changes.

### RP2: pool arithmetic and country interpretation

Keep the deep external seam at the existing country-wide computation interface:

```python
country.compute_tax_for_assets(configuration, accounting_engine, asset_to_input_data)
```

Callers and end-to-end tests should not learn a second calculation interface.
Inside RP2:

- core owns a reusable operation that moves quantity and effective basis from
  one opaque pool id to another;
- accounting methods provide the effective basis through the existing
  `AcquiredLotAndAmount` result;
- the Austrian country/plugin layer interprets its markers, applies Alt/Neu and
  ordering rules, and invokes the core operation;
- source labels such as `wages` remain a caller-side provenance concern and do
  not require a new country-specific RP2 income category.

The same internal basis-carry implementation must serve `TransferAnalyzer` and
the country-wide pool-transfer path; do not duplicate fiat/basis arithmetic.
The pool-transfer implementation must be observable through
`compute_tax_for_assets`; do not publish its internal helper as a second public
interface merely to make tests convenient.

### Profile policy

Add exactly one profile column:

```text
cost_basis_pool_scope TEXT NOT NULL DEFAULT 'global'
```

Validation, supported values, and defaults belong in `kassiber/tax_policy.py`.
The country policy exposes its allowed set; Kassiber resolves factual ids. Add
the field to the existing profile snapshot/update paths and to the replication
allowlist as a high-stakes profile field.

Do not add a table for pool definitions. If `global` and `wallet` cannot cover
the reviewed use case, stop and rescope.

## Goal states and implementation order

### G0 — Contract and red tests

**Work**

- Add a Kassiber fixture proving that a valued `wages` receipt becomes an
  acquisition with basis and creates no Kz 172 row.
- Add an RP2 fixture with acquisitions at EUR 100 and EUR 300, a pool transfer,
  and a destination disposal. The correct destination unit basis is EUR 200.
- Add a fee-bearing version that pins quantity and total-basis conservation.
- Pin the current global-pool output for an existing Austrian fixture.
- Record the Austrian wallet-scope legal/product decision or mark it explicitly
  unresolved.

**Goal state**

The intended behavior is executable and fails for the known reasons before
implementation begins; existing global output is pinned.

**Cut-off line**

No schema, UI, report, or production behavior change in this goal. If the
fixtures reveal a different root cause, rewrite the later steps before coding.
Do not merge a red-test-only commit; land each failing fixture with its fix.

### G1 — Country-neutral RP2 transfer-basis correctness

**RP2 work**

- Fix `TransferAnalyzer` so a synthetic destination lot carries the accounting
  method's effective unit basis rather than the selected historical lot's
  nominal basis.
- Centralize synthetic destination-lot basis materialization in one internal
  RP2 implementation reused by G2; do not create a new public interface.
- Define fee-field behavior explicitly: principal basis and fee consumption must
  conserve the source basis without inventing income or duplicating fees.
- Add the first tests combining `per_wallet` with plain `moving_average`.
  Prove fee-free destination basis and fail closed on fee-bearing transfers
  until one cursor can interleave movements and taxable events.
- Add a fail-loud guard if a country-wide computation hook would otherwise be
  silently skipped by an enabled application method.

**Goal state**

The fee-free EUR 100 / EUR 300 reproduction carries EUR 200 into the
destination and no country-specific code is involved. Unsafe fee-bearing
`per_wallet` shapes are rejected before analysis.

**Cut-off line**

Do not enable Austria `per_wallet`; do not change Kassiber; do not redesign RP2
application methods. The guard may reject an unsafe combination until a future
country implements it correctly.

### G2 — RP2 Austrian semantics on the existing country seam

**RP2 work**

- Keep `TransactionType.WAGES` available as a generic RP2 input type for other
  callers, but do not invent an Austrian employment-income result category for
  Kassiber's acquisition flow.
- Keep acquisition-lot and fiat-basis behavior unchanged.
- Correct the stale Kz 175 comment; report-field numbers remain a Kassiber
  presentation responsibility.
- If the Austrian wallet gate is accepted, teach the Austrian country-wide
  computation path to process same-asset transfers carrying explicit source and
  destination pool ids, using the G1 core basis-carry operation. Otherwise keep
  this production integration disabled and prove the generic operation through
  a non-production country test.
- Preserve Alt/Neu ordering, swap carry, miner-fee treatment, and cross-asset
  `at_swap_link` behavior.

**Goal state**

RP2 preserves ordinary acquisition behavior without adding a Kassiber-specific
wage-tax result. If the gate is accepted, it also processes a tax-neutral move
between two opaque Austrian Neu pools; otherwise that Austrian capability stays
disabled.

**Cut-off line**

No E 1kv rendering, provider withholding, address discovery, new application
mode, or Kassiber-specific wallet logic in RP2.

### G3 — Kassiber generic pool facts and legacy-safe profile policy

**Kassiber work**

- Add `profiles.cost_basis_pool_scope` with `global` as the migration/default
  value.
- Thread it through `TaxPolicy`, profile create/copy/get/update, daemon schemas,
  audit-safe snapshots, replication invitations/allowlist/merge, and journal
  input versioning.
- Replace normalized `at_pool` with the generic event/source/destination fields.
- Resolve `global` and `wallet` ids from existing profile/wallet/custody facts.
- Serialize those ids into the RP2 Austrian marker format only inside the RP2
  adapter.
- Map `wages -> BUY` at the RP2 adapter boundary; quarantine a wages receipt
  only when its fiat valuation or required pricing provenance is missing. Keep
  the raw `wages` kind and attachments unchanged as provenance.
- Update the RP2 pin only after G1/G2 are green in RP2.

**Goal state**

Existing profiles reproduce their prior global results. If the gate is
accepted, a reviewed wallet-scope profile emits distinct source/destination
pool ids for an internal transfer and reaches RP2 without an alternate tax path.

**Cut-off line**

One new profile column, zero new tables. No automatic address/provider grouping,
manual grouping, transaction rewrite, or compatibility alias beyond the RP2
adapter's wire markers.

### G4 — Persisted journal/report meaning and minimal product surface

**Kassiber work**

- Persist the same ordinary acquisition meaning used for every other `BUY`;
  there is no employment-income category or exceptional E 1kv filter.
- Retain basis and pricing provenance. Existing transaction kind, notes, tags,
  history, and attachments explain the source without adding payroll fields.
- Keep the current self-custody mapping for real crypto income/gains/losses and
  disclose that Kz 171/173/175 domestic-provider handling is unsupported.
- Add the active pool scope to Austrian structured/PDF/XLSX/CSV report
  assumptions and audit-package metadata.
- Reuse the existing profile editor in CLI/daemon/Books UI for the scope choice.
  Show only scopes advertised by the country policy. For Austria that remains
  `global` until the release-time ship gate adds `wallet` to the allowlist.
- Reuse transaction attachments for payroll evidence. Do not add a salary
  workflow.
- Scope changes invalidate journals and use existing saved/filed-report impact
  handling; reverting to `global` plus journal reprocessing is the rollback.

**Goal state**

The user can select a supported reviewed scope in the existing book settings
and reprocess journals. Valued compensation appears through the existing
Acquisition surface; the Austrian report contains no invented wage-income row.

**Cut-off line**

No new route, modal, report kind, evidence schema, or payroll UI. If the existing
profile editor cannot host one select field cleanly, fix that editor rather than
creating a new surface.

### G5 — Conditional wallet proof, documentation cutover, and stop

Run the wallet-specific fixture below only if the Austrian ship gate is
satisfied. If it is not, run the same proof under `global`, verify that `wallet`
is rejected by policy, complete the mandatory foundation, document the bounded
stop, and do not claim the salary/transit/savings isolation outcome.

**Required fixture**

One Austrian self-custody book contains:

1. two valued BTC wage receipts in a salary wallet;
2. a reviewed internal transfer into a savings wallet;
3. a fee-bearing transfer into a transit wallet;
4. a disposal from each destination wallet;
5. payroll evidence attached to at least one receipt.

**Assertions**

- Wage receipts create basis but contribute zero to E 1kv 171/172.
- A EUR 100 / EUR 300 source average carries EUR 200 per transferred unit.
- Principal quantity, fee quantity, source basis, destination basis, realized
  gain, and remaining basis reconcile exactly.
- When legally enabled, `wallet` scope isolates untransferred pools but permits
  disposal after an evidenced pool transfer; otherwise selecting it fails.
- `global` scope reproduces the legacy result.
- Missing wage valuation quarantines in every mode. When wallet scope is
  enabled, an incomplete cross-pool transfer also blocks later basis-dependent
  reporting.
- Changing scope makes journals/reports stale; reprocessing clears the stale
  state and records the new assumption.
- Profile create/get/update and `ui.profiles.snapshot` round-trip the scope.
- A scope change bumps journal input/stale state and blocks report use until
  reprocessing.
- Austrian structured, CSV, PDF, and XLSX exports contain the same scope
  assumption.
- Audit-package metadata contains the scope; backup/migration and replication
  preserve the profile field.
- No scope surface exposes raw addresses, descriptors, or other private wallet
  material.

**Verification**

- RP2 targeted tests plus its full test/quality gate.
- Kassiber targeted engine/report/migration/replication/UI tests.
- `./scripts/quality-gate.sh`.
- One fresh-book run and one migrated legacy-book run.
- Independent correctness review of basis conservation and report exclusion.

**Goal state**

With the gate accepted, the salary/transit/savings scenario is reproducible from
raw inputs to existing report artifacts. Without it, the global-only foundation
is reproducible and wallet selection fails closed. In either case, all
current-truth docs describe the shipped behavior.

**Cut-off line**

Update `docs/austrian-handoff.md`, `docs/plan/06-austrian-tax-engine.md`,
`docs/plan/07-austrian-tax-open-questions.md`, user docs, and TODO status; delete
stale global/wallet claims; then stop. Any address/provider/KESt/payroll feature
becomes a separate proposal.

## Cross-repository delivery order

| Order | Repository | Review unit | May merge when |
|---|---|---|---|
| 1 | RP2 | G1 transfer-basis fix + guard | generic moving-average transfer tests pass |
| 2 | RP2 | G2 Austrian pool policy + ordinary acquisition compatibility | AT and cross-asset regressions pass |
| 3 | Kassiber | G3 schema/normalization/adapter + RP2 pin | legacy global and migration/sync tests pass |
| 4 | Kassiber | G4 report cutover + conditional G5 wallet activation | mandatory global proof passes; wallet fixture also passes if legally enabled |

Do not open parallel Kassiber implementations against an unmerged RP2 contract.
Do not bundle RP2 wheel publication or unrelated RP2 maintenance into these
review units.

## Rollback

- The migration sets existing and new profiles to `global` unless a user makes
  a reviewed change.
- A scope change mutates only profile policy and derived journal/report state.
- Rollback is: set the profile back to `global`, reprocess journals, and rebuild
  affected reports.
- Raw transactions, attachments, custody evidence, and prior saved/filed report
  snapshots remain intact.
- If basis conservation fails at any gate, the feature stays unavailable; do
  not silently fall back from a requested wallet scope to global.

## Explicit terminal stop

After G5, the implementation is done. The existence of the generic opaque pool
id is not authorization to implement address pools, provider imports, custom
groups, UI visualization, tax optimization suggestions, or more country
plugins. Add one only when a real country requirement and an end-to-end fixture
exist.
