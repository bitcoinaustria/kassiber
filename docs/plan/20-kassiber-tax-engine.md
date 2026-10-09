# Kassiber tax engine

**Status:** Direction accepted 2026-10-01. Phases 0-3 are implemented: the
engine is the default and RP2 is a test-only oracle. Phases 4-6 remain; tasks
move to [TODO.md](../../TODO.md) as each phase starts.
**Current source of truth:** [tax engine](../reference/tax-engine.md),
[tax](../reference/tax.md), [Austrian handoff](../austrian-handoff.md),
[plan 06](06-austrian-tax-engine.md),
[plan 16](16-cost-basis-pools-and-employment-compensation.md), current code, and
current tests.

## Decision

- Replace RP2 with a Kassiber-owned tax engine, written in Rust. It is the
  first step of moving the core to Rust.
- The engine is a pure library crate. Until the rest of the core moves, Python
  calls it through one narrow PyO3 function that takes and returns a versioned
  JSON document.
- This plan covers the engine only. Moving custody, storage, the daemon, or the
  CLI to Rust needs its own plan, and so does any change to the desktop UI.
  Mobile stays out of scope per [the overview](00-overview.md).

## Why

Most of the tax path's complexity comes from adapting Kassiber to RP2's model:

- Inventory is tracked in four places: RP2's lots and pools, the adapter's
  per-wallet balance mirror that keeps RP2 from aborting on a negative balance
  (`engines/rp2.py`, `_prepare_rp2_asset_input`), the Austrian Alt
  availability tracker (`austrian.py`, `infer_outbound_regimes`), and the
  generic BTC↔LBTC carry that reuses RP2's source-leg basis
  (`_apply_generic_bitcoin_rail_carry_values`).
- Typed facts are flattened into `notes` tokens (`at_regime`, `at_pool`,
  `at_swap_link`), and wallets travel as RP2's exchange label.
- One unpriced or unclassified lot quarantines every later disposal of that
  asset, because the adapter cannot know which lot RP2 consumes.
- Plan 16's wallet pools are blocked: `at_pool` cannot name both a source and
  a destination pool, and RP2's `per_wallet` mode cannot replay transfers on
  one chronological cursor.
- RP2 writes an import-time disk log that Kassiber has to suppress, and sets
  the importing thread's decimal context to 32 significant digits.
- Every tax change needs pull requests in two repositories plus a pin bump.

Writing the engine in Rust gives compile-time types to the most
correctness-sensitive module and avoids writing it twice once the core moves.
Dropping RP2 also drops its `pycountry` dependency (LGPL-2.1-only).

## Requirements

### No lost functionality

The engine reaches parity with everything reachable through the current seam
before it becomes the default. The parity target is the current path's final
output, RP2 plus the adapter, not RP2's raw calculation:

- `generic` and `at` policies; `fifo`, `lifo`, `hifo`, `lofo`, and
  `moving_average`; the Austrian moving-average default with the generic
  methods still selectable; the generic `tax_long_term_days` holding period.
- Austrian Alt/Neu regimes: the 2021-03-01 Europe/Vienna cutoff, the Alt
  fallback when only Alt inventory remains in the disposing wallet, regime
  availability moved by internal transfers, and `wahlrecht` provenance.
- The `global` pool scope; `wallet` keeps failing closed.
- Receipt mapping: earn-like kinds, `wages` as an ordinary acquisition, and the
  Austrian airdrop/hardfork quarantine.
- Same-asset moves with fees, ownership-derived moves, grouped fan-out moves,
  loan and channel leg roles, BTC↔LBTC carrying value on every profile, and
  reviewed Austrian cross-asset carries.
- Same-timestamp ordering: acquisitions first, then moves in wallet-dependency
  order, then disposals, with remaining ties in projection order
  (`_ordered_rp2_items`). Fixtures include a same-timestamp buy and sale
  without chain positions and a same-time A→B→C move chain.
- Chained carries: a carried acquisition takes the basis its linked outgoing
  disposal selected, including chains where one hop's carried basis feeds the
  next. The generic path reaches this today by recomputing until the carries
  settle (`_apply_generic_bitcoin_rail_carry_values`); the Austrian path
  relies on RP2's native runner. Each leg books at its own timestamp. Fixtures
  include same-time carry chains, direct swap payouts, and legs at different
  times with a sale in between.
- Reviewed overrides: `taxability_override` and `at_category_override` keep
  their precedence over automatic Austrian classification, including the
  non-reportable categories, the `none` category, and a reviewed category's
  Kennzahl (`_append_rp2_journal_entries`).
- Fee reporting: `fee` and `transfer_fee` entries consume quantity and basis
  with proceeds equal to basis, zero gain, and no Austrian category; the tax
  summary excludes fee and move rows (`_build_tax_summary_rows`).
- Every current quarantine reason, including transfer and group gates, swap
  pair gates, insufficient lots, and lot contamination.
- Every `TaxEngineLedgerResult` field (`engines/base.py`), the gain fragments
  behind [report explanations](../reference/report-explanations.md),
  open-position basis, and the entry types the exit-tax report recognizes
  (`exit_tax.py`, `RECOGNIZED_ENTRY_TYPES`).

Branches that no finalized projection row can reach are not parity
requirements. Delete them first under the existing quarantine follow-up in
TODO.md, so they are not ported.

### Contracts and invariants

- **One seam.** `custody_journal.py` keeps calling one engine with
  `TaxEngineLedgerInputs`. `FinalizedTaxProjection` stays the only input, and
  the result shape stays stable for the CLI, daemon, desktop, reports, and AI
  projections.
- **Exact amounts.** Quantities are integer msat. Fiat values are exact
  decimals, rounded only at points the contract names. During parity the
  engine reproduces RP2's observable results to the cent and msat.
- **Pure.** No I/O, network, logging, clock, or global state. The same input
  gives byte-identical output.
- **Fail closed.** Unsupported shapes return typed quarantines. A panic is a
  bug; the binding turns it into an error that blocks the report, never a
  partial result.
- **Provenance.** Each journal run records the engine version and country
  rules version. Filed reports keep the versions that produced them.

### Clean implementation

Behavior is specified from tax law and official guidance, Kassiber's docs, and
RP2's observable outputs. Do not translate RP2 source; that would keep RP2's
Apache-2.0 obligations on the new engine. Four fork files are copyright
bitcoinaustria (`plugin/country/at.py`, `plugin/country/at_native_tax_engine.py`,
`plugin/accounting_method/moving_average.py`, and
`plugin/accounting_method/moving_average_at.py`). The fork's history shows
every commit to them is by the Kassiber owner. Authorship of commits does not
by itself prove the content is free of adapted upstream code. The engine's
Austrian and moving-average modules port them on the basis that they are the
owner's original work, which the owner must confirm. If legal certainty matters,
add this question to the legal opinion that
[the stack ADR](01-stack-decision.md) already requires.

## Architecture

### Layout

- `kassiber-tax`: a pure Rust library crate with no I/O dependencies.
- `kassiber-tax-py`: a PyO3 binding exposing one function from request JSON
  to response JSON, built with maturin as its own Python package so the main
  package keeps its build backend.
- The repository location, the workspace relationship with
  `ui-tauri/src-tauri`, and the decimal crate are decided in phase 0.

### Boundary

In phases 0-3 the boundary sits where RP2 sat, beneath the adapter.
`kassiber.core.engines.native` mirrors the part of RP2's interface the adapter
calls, and one engine call answers each RP2 call; the
[tax engine reference](../reference/tax-engine.md) owns that format. The same
adapter code then drives either backend, which is what makes the comparison
exact.

Phase 4 lifts the boundary to the projection: the request carries a
`schema_version`, the profile's tax policy, and the finalized projection as
typed events (msat as integers, fiat as decimal strings, ids that anchor back
to imported evidence), and the response maps one-to-one onto
`TaxEngineLedgerResult`. The Python adapter then shrinks to building the
request and reading the response. Drift tests on both sides pin the schema.

### Engine

- One chronological pass over all assets, so cross-asset carries need no
  separate validation pass. Events are ordered by time, then by the
  same-timestamp rules in the parity list, then by projection order; the order
  is total, so ties never depend on iteration order.
- Each leg of a linked carry pair keeps its own timestamp, and the
  same-timestamp order above still holds: the incoming lot is listed when it
  occurs, even before its carry resolves. A lot's basis is read only when a
  disposal draws on it, so the outgoing leg's selected basis replaces the
  incoming lot's basis when that leg is processed, and any event that could
  draw on the incoming lot before then waits for the carry. That is how the
  Austrian runner works today (`tax-engine/core/src/swaps.rs`); phase 4 uses
  the same rule for the generic BTC↔LBTC carry, replacing its repeated
  recomputation. Same-time carry chains resolve in dependency order, and the
  runner fails with an error when no order can resolve them.
- Pools are keyed by a country-chosen pool key. Each pool holds lots or an
  average-cost pool according to the method.
- A basis is `Known(fiat)` or `Unknown(reason)`. Uncertainty flows through the
  arithmetic instead of being guessed. Parity first reproduces today's broad
  contamination rule; narrowing it is a phase 5 change.
- Every realized gain lists the lots or pool states it consumed, back to the
  evidence ids.

### Countries

A country is a compiled-in manifest plus optional hooks:

- Currency, timezone, tax year, tier, and rule sets with non-overlapping
  effective date ranges.
- Each rule set declares allowed methods and pool scopes, matching rules,
  holding rule, allowances, swap treatment, and income mapping, built from
  shared engine primitives.
- Hooks cover rules no other country has, such as Austrian Alt/Neu and the
  Wahlrecht ordering.
- Anything a manifest does not declare fails closed.

| Tier | Promise |
|---|---|
| First-class | Reviewed rules, country form exports, report-readiness gates (Austria) |
| Supported | Lot math, holding periods, and income mapping from a reviewed manifest; standard gains and holdings reports; no official forms |
| Generic | User-chosen currency, method, and holding period (today's `generic`) |

The tier is shown in the UI, in reports, and to the in-product AI. Countries
are not loaded at runtime.

### Phase 0 decisions

Recorded 2026-10-01, from the behavioral specs written before implementation:

- **Crates.** `tax-engine/core` is the pure engine; `tax-engine/python` is the
  PyO3 0.25 binding (abi3, CPython 3.10 and newer), built by an exactly pinned
  maturin as a uv workspace member. Both declare Rust 1.77, and CI builds them
  with exactly that toolchain.
- **Decimal.** Fixed-size Rust decimals hold about 28 digits and normalize
  exponents, so the engine has its own decimal type over `num-bigint` that
  matches CPython's `decimal` exactly, checked against generated CPython
  vectors.
- **Precision.** Importing RP2 set 32-digit precision only on the importing
  thread, so the same book was stored with different values depending on
  whether the CLI, the daemon's main thread, or its background worker processed
  it. Kassiber now pins 32 digits with round-half-even for every thread
  (`kassiber/__init__.py`), and the parity target is RP2 and the adapter
  computing at 32 digits.
- **Listed differences.** The native backend raises Kassiber error classes
  instead of RP2's, and does not write RP2's temporary configuration file.

### Findings for follow-up

The specs found current behavior that parity deliberately keeps. Each is a
TODO entry, because fixing it changes results and needs its own reviewed
change:

- An Austrian sale is tagged Neu whenever the wallet holds any Neu, even when
  the sale needs more, and RP2 has no Alt fallback, so the whole report aborts.
- On a Neu swap the fee's share of pool basis is neither realized nor carried.
- Description text is parsed for Austrian markers, so user text can add or
  conflict with `at_regime`, `at_pool`, or `at_swap_link`.
- Transfer-leg quantities in the transfer audit pass through `float`.
- [Plan 06](06-austrian-tax-engine.md) says Neu moving average applies from
  2023; the code applies it to every Neu disposal from 2021-03-01.

## Verification

1. **Invariants** as property tests: msat and basis conservation across moves
   and carries, no negative pool, deterministic output, and pricing an unknown
   lot never adding a quarantine.
2. **Existing tests** on the tax path run unchanged through the seam against
   both engines.
3. **Authoritative fixtures:** BMF worked examples and scenarios reviewed by a
   Steuerberater, starting with plan 16's salary → savings-wallet case.
4. **Black-box comparison with the current path** (RP2 plus the adapter) on
   generated histories and the existing fixtures, with RP2 as a test-only
   dependency. Fee-bearing histories are always included. Owners may run the
   comparison locally on real books; results never leave the machine. Every
   difference is a bug or a listed, reviewed difference.
   The comparison is semantic. It ignores generated journal ids
   (`uuid.uuid4()` today) and the engine-identity fields in calculation
   metadata (`engine`: `"kassiber_tax"` or `"rp2"`). It compares transaction
   anchors, exact amounts, classifications, entry order, journal descriptions,
   and calculation evidence.
5. **Shadow runs:** a test-only switch makes journal processing run both
   engines and fail on any semantic difference. It is not a product setting.
6. **The full quality gate**, plus extension builds for macOS arm64, Linux
   x86_64 on the AppImage glibc floor, and Windows x86_64, and a packaged-build
   smoke test.

## Phases

0. **Spike.** Build the crate and binding into the dev environment and one
   packaged build. Pick a decimal type that reproduces RP2's results. Implement
   FIFO and moving average for one asset with buys, sells, and fees, behind a
   test-only switch, and run the RP2 comparison on generated histories.
   Done when the comparison shows no differences for that subset, the packaged
   build runs, and the dependency and license entries are drafted. If decimal
   parity cannot be reached, stop and re-plan.
1. **Generic parity.** All methods, holding period, every move and receipt
   shape, quarantines, holdings, tax summary, gain fragments, and generic
   BTC↔LBTC carry. Done when every generic tax test passes on both engines and
   the comparison is clean.
2. **Austrian parity.** Alt/Neu, regime availability, moving average, swap
   carries, and Wahlrecht provenance; Kennzahl mapping stays in the report
   layer. Done when the Austrian tests, BMF fixtures, and comparison are clean.
3. **Cutover.** Make the new engine the default and RP2 a test-only oracle.
   A one-shot migration marks processed books stale with the new engine
   provenance; filed reports are untouched. Update every doc and AI reference
   that names RP2, `THIRD_PARTY_LICENSES.md`, and the dependency manifests,
   and remove the runtime RP2 and `pycountry` dependencies. Before removal,
   give Kassiber's remaining Python decimal arithmetic an explicit precision
   policy, and run the comparison in a fresh process that never imports RP2,
   including exact report values near rounding boundaries. Until RP2 is
   removed, a release can switch the default back; after that, rollback means
   reverting the release.
4. **Simplify behind zero difference.** Fold the adapter's balance mirror, the
   Austrian regime tracker, and the generic carry workaround into the engine,
   and drop the `notes` markers used to pass input to RP2. Persisted journal
   descriptions keep their `at_regime=` markers, because the exit-tax report
   reads them (`exit_tax.py`, `_entry_is_alt`), until those consumers move to
   equivalent typed fields in a reviewed change. The comparison must stay
   empty.
5. **Semantic changes, one reviewed change at a time,** each with fixtures and
   a stale migration:
   1. Quarantine only disposals that consume an uncertain basis.
   2. Make the Wahlrecht ordering configurable, under the existing TODO
      entry; changing the default needs legal review and a migration.
   3. Austrian wallet pools, behind plan 16's legal gate.
   4. Zero-cost acquisitions, behind their own legal review.
   5. Per-wallet physical lots, only if a jurisdiction requires them, as the
      existing TODO entry says.
6. **More countries.** Add US (including the 2025 per-wallet switch), ES, and
   IE as Supported with reviewed manifests, then the countries the owner
   chooses. An `add-country` skill turns the manifest, fixtures, tier, and docs
   into a checklist.

## Risks

- **Decimal parity.** RP2 computes at 32 significant digits with half-even
  rounding and compares at 13 decimal places; common fixed-size Rust decimals
  hold about 28. Phase 0's exit criterion covers this.
- **Python precision after RP2.** Importing RP2 sets the importing thread's
  decimal context to 32 digits, so Kassiber's own Decimal arithmetic, such as
  transaction-derived rate division in `custody_journal.py`, can run at 32
  digits in that thread and Python's default 28 elsewhere.
  `report_explanation_carry.py` already pins 32 locally. Removing RP2 changes
  the ambient precision, and a comparison run with RP2 imported would hide it.
  Phase 3 requires an explicit policy and an RP2-free comparison.
- **Packaging.** The extension adds a native module per platform, another
  macOS dylib to sign, the Linux glibc floor, and a build step in every CI
  shard. Phase 0 builds one packaged artifact; the full matrix lands before
  phase 1 ends.
- **Two engines during phases 0-2.** RP2-side changes are limited to bug
  fixes, and each fix lands in the new engine with a fixture.
- **Contract drift** between the Python request builder and the Rust types.
  The versioned schema and drift tests on both sides cover it.
- **Developer setup.** Python tests now need the Rust toolchain, which the
  desktop already requires. `scripts/bootstrap-dev-env.sh` and
  [testing](../reference/testing.md) change with phase 0.
- **Scope creep.** Core, storage, and UI migrations stay out of this plan.

## Decisions this changes

- [Stack ADR](01-stack-decision.md): the tax engine moves to Rust. Custody,
  storage, and the daemon stay in Python until a later plan.
- [Plan 16](16-cost-basis-pools-and-employment-compensation.md): its "no second
  chronological inventory engine" cut line does not bind this replacement.
  The engine replaces RP2 rather than running beside it in production, so
  there is still one Austrian calculation path. Plan 16's legal gate for
  `wallet` pools stands.
- [Plan 06](06-austrian-tax-engine.md): the fork-divergence risk ends at
  cutover.
- TODO's RP2 wheel publication is dropped: since cutover, RP2 no longer
  ships.

## Out of scope

- Moving custody, storage, the daemon, or the CLI to Rust.
- Replacing the webview UI.
- New tax claims before phase 5.
- Runtime-loaded country plugins.
