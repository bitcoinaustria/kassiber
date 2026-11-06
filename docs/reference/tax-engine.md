# Tax engine

Kassiber's tax engine is a Rust crate (`tax-engine/core`) called from Python
through a PyO3 binding (`tax-engine/python`, imported as `kassiber_tax`). Native
callers can use `ledger_typed(&LedgerRequest) -> Result<LedgerResponse,
StagedError>` without serializing a request or response. It accepts schema 2's
`ledger` operation and shares computation with the text entry point. It
replaced RP2 under [plan 20](../plan/20-kassiber-tax-engine.md). This page owns
the engine contract: the schema-2 `ledger` format journal processing uses,
the schema-1 operations kept as the lot engine's fixture API, the Python
layer around them, and the decimal rules both sides follow.

The binding is not published. Kassiber depends on `kassiber-tax==0.2.0+local`,
and PyPI refuses local version labels, so PyPI can never supply an impostor.
Private indexes and `--find-links` sources can still host local versions.
`uv sync --locked` builds the binding from the workspace; a pip install needs
`pip install ./tax-engine/python .` from the checkout, with a Rust toolchain.
The package version and the engine crates' Cargo version (`engine_version()`)
share the `0.2.0` release; bump them together.

## Ledger performance checkpoint (R4, 2026-10-30)

The R4 speed pass preserves schema-2 response bytes, decimal exponents,
list order and the first failing operation. The complete text-in/text-out
`ledger` operation is 2.03–3.96× faster on the four synthetic bench books.
This measures the tax engine; Python request building, response rendering,
custody and database persistence remain outside these timings.

The baseline is `de18d9af` (the pre-optimization engine with timing spans),
copied from the first run's `target/ledger-perf/base-src`. For the timing
build its only local change is the benchmark's main-thread CPU counter. The
optimized engine is `de836bd2`. Both examples were rebuilt offline with
Rust 1.99.1 in release mode with `--features timing`, then run back to back
against the same captured Python request and expected response. The machine
is Linux x86_64, with 48 logical CPUs. Each invocation runs one request and
requires byte equality with the captured response. Five rounds alternate
`before, after` and `after, before`; each table uses the fastest complete run of its version,
with all phase values taken from that run. The 1-minute load was 9.02–9.59,
so the owner's above-8 rule (alternating, best of five) applies.

| Complete `ledger`, seconds | Before | After | Speedup |
| --- | ---: | ---: | ---: |
| Generic 2,000 | 0.067 | 0.033 | 2.03× |
| Multi-wallet 2,000 | 0.170 | 0.054 | 3.15× |
| Generic 20,000 | 0.721 | 0.299 | 2.41× |
| Multi-wallet 20,000 | 1.737 | 0.439 | 3.96× |

Phases below are **before → after**, seconds. `0.000` means less than
0.5 ms. A reused settled carry pass needs no separate final prepare,
validation or computation; `reused` marks those spans. Rail carry includes
its own prepare and compute passes. Uninstrumented request-head scanning,
allocation and destruction explain the difference from the total.

| Phase | Generic 2k | Multi-wallet 2k | Generic 20k | Multi-wallet 20k |
| --- | ---: | ---: | ---: | ---: |
| Typed request parse | 0.005 → 0.005 | 0.005 → 0.005 | 0.046 → 0.044 | 0.046 → 0.044 |
| Book references | 0.001 → 0.001 | 0.001 → 0.001 | 0.007 → 0.007 | 0.007 → 0.005 |
| Rail carry | 0.000 → 0.000 | 0.099 → 0.032 | 0.000 → 0.000 | 0.984 → 0.267 |
| Carried request and references | 0.000 → 0.000 | 0.001 → 0.002 | 0.006 → 0.000 | 0.010 → 0.011 |
| Final prepare pass | 0.017 → 0.011 | 0.017 → 0.000 | 0.175 → 0.099 | 0.175 → 0.000 |
| Lot inputs | 0.002 → 0.000 | 0.001 → reused | 0.030 → 0.000 | 0.022 → reused |
| Validation | 0.006 → 0.000 | 0.006 → reused | 0.059 → 0.001 | 0.058 → reused |
| Final computation | 0.020 → 0.004 | 0.024 → reused | 0.222 → 0.045 | 0.239 → reused |
| Composition | 0.007 → 0.007 | 0.007 → 0.009 | 0.081 → 0.066 | 0.090 → 0.068 |
| Response serialization | 0.005 → 0.003 | 0.006 → 0.003 | 0.057 → 0.017 | 0.063 → 0.019 |

The recorded run time / 1-minute load at invocation start, in round order:

| Book / version | Round 1 | Round 2 | Round 3 | Round 4 | Round 5 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Generic 2k before | .073 / 9.59 | .076 / 9.59 | .073 / 9.59 | .068 / 9.59 | .067 / 9.59 |
| Generic 2k after | .035 / 9.59 | .034 / 9.59 | .033 / 9.59 | .034 / 9.59 | .033 / 9.59 |
| Multi-wallet 2k before | .170 / 9.59 | .174 / 9.59 | .181 / 9.59 | .177 / 9.59 | .179 / 9.59 |
| Multi-wallet 2k after | .054 / 9.59 | .062 / 9.59 | .054 / 9.59 | .056 / 9.59 | .056 / 9.59 |
| Generic 20k before | .721 / 9.59 | .741 / 9.59 | .722 / 9.59 | .739 / 9.30 | .738 / 9.30 |
| Generic 20k after | .307 / 9.59 | .310 / 9.59 | .299 / 9.30 | .303 / 9.30 | .302 / 9.30 |
| Multi-wallet 20k before | 1.758 / 9.30 | 1.737 / 9.20 | 1.755 / 9.20 | 1.779 / 9.02 | 1.764 / 9.02 |
| Multi-wallet 20k after | .451 / 9.30 | .441 / 9.20 | .449 / 9.20 | .439 / 9.20 | .466 / 9.02 |

All 40 responses match the captured Python responses byte for byte. The
2× multi-wallet 20k acceptance target is met, so no further profiling pass
is required by that criterion. Main-thread CPU values from `ledger-bench`
exclude worker threads and must not be interpreted as total CPU savings.

### What changed

- `41ec8223`, `31ac8516`, `0537479d`, `de836bd2`: coefficients that fit
  `u128` use fixed-width arithmetic, with 256-bit intermediates and the
  existing `BigUint` fallback. Rounding, exponent limits, signed zero and
  exceptional results retain their exact rules. Differential tests compare
  the fast and fallback paths. `Decimal` remains 40 bytes.
- `06edc4cb`: reuse a generic rail carry's settled prepare and computation
  when its pricing and excluded rows equal the final pass's inputs.
- `2ac6c521`, `c93a6b19`, `3a39f611`, `2d92bb27`, `7ef06f2e`, `03c343a0`:
  keep constructed entries, borrow them across the lot engine, avoid
  redundant books and derived copies, render text into one buffer, and
  parse a valid request once. Malformed requests retain their earlier
  diagnostic path.
- `6dee1a9f`, `16c79421`, `0c664267`, `0d1deebb`, `5bfbcaa2`: prepare and
  compute independent assets, construct entries, derive fragment values,
  compose independent rows and serialize contiguous response chunks
  concurrently. Results, errors and deferred panics are consumed in the
  original order. Each map uses at most eight scoped workers and joins
  them before returning; Austrian linked-asset computation retains its
  ordered runner.

The remaining largest spans at multi-wallet 20k are rail carry (267 ms),
composition (68 ms), parsing (44 ms) and serialization (19 ms). Within the
carry, preparation totals 138 ms and lot computation 57 ms; these are nested
spans, not additional time. Fixed-point carry still may run several passes.
On generic 20k, final preparation (99 ms), composition (66 ms), computation
(45 ms) and parsing (44 ms) dominate. The separately planned typed R4 fusion
can remove request parsing; settling carry in one pass remains a distinct
accounting change.

### Mutation equivalence record

The speed pass retains every original tax recipe and adds
`walk-last-constructor-failure-wins`, which reverses the consumed constructor
outcomes to prove that the earliest constructor failure wins across chunks.
Nine existing recipes move to equivalent mutations:

| Recipe | Equivalent mutation after the refactor |
| --- | --- |
| `walk-entries-in-insertion-order` | Chain moves before outs instead of outs before moves. |
| `walk-infinite-values-checked-first` | Raise for an infinite argument before the deferred constructor validates its other arguments. |
| `compose-unit-cost-multiplies` | Multiply basis by quantity instead of dividing in the extracted acquisition helper. |
| `run-method-gate-without-computation` | Remove the empty parsed-assets guard, formerly the empty input-list guard. |
| `run-compute-failure-names-no-asset` | Drop the failing parsed asset's name, formerly the input asset's name. |
| `carry-stops-after-a-pass` | Stop when pricing or blocks changed, at the new continue branch. |
| `run-carry-pricing-unread` | Ignore carried pricing and disable settled-output reuse so it cannot mask the ignored pricing. |
| `run-carry-blocks-unread` | Give the final pass an empty exclusion set at its new borrowed-book/settled-pass call. |
| `run-carried-rows-unreported` | Omit carried rows from the response at its borrowed-book call. |

The complete catalog kills all **187/187** mutations (150 Rust, 37 Python),
after all 121 distinct unmutated test selections pass. No recipe is removed,
no mutant survives and no mutated build fails. The nine re-anchored recipes
above and the new constructor-order recipe are all in that kill set.

### Python boundary verification

The binding was rebuilt with `MATURIN_NO_INSTALL_RUST=1`, offline Cargo and
`mise exec rust@1.99.1 uv@latest -- uv sync --locked --python 3.11`.
`tests/test_tax_*` passes: **1,028 passed, 160 subtests passed, one expected
skip** (the release binding has no panic-injection hook). This includes
golden regeneration/certification, the live frozen-oracle comparison and
the seeded prepare-stage comparisons. The rebuilt `kassiber_tax.ledger`
also matches all four captured response texts.

The full nine-lane shadow overlay observes **968 production calls compared
with the frozen RP2 oracle**, with no unlisted differences and zero LD-5
refusals. The test results include three custody comparison failures in
`reports-contracts`; the complete overlay is therefore not green.

| Shadow lane | Tests passed | Subtests passed | Skipped | Failed | Calls compared |
| --- | ---: | ---: | ---: | ---: | ---: |
| core-accounting | 545 | 231 | 0 | 0 | 343 |
| wallets-sync | 134 | 25 | 0 | 0 | 21 |
| daemon-cli | 307 | 66 | 0 | 0 | 184 |
| security-replication | 16 | 0 | 0 | 0 | 0 |
| reports-contracts | 1,801 | 146 | 1 | 3 | 332 |
| serial-network | 157 | 41 | 0 | 0 | 1 |
| serial-daemon | 205 | 42 | 0 | 0 | 19 |
| serial-regressions | 192 | 16 | 0 | 0 | 55 |
| serial-integration | 27 | 4 | 10 | 0 | 13 |

The requested `KASSIBER_TEST_TAX_ENGINE=census` runs also find no missing
shadow-overlay modules or calls outside tests. Census records production
routes; the shadow runs above perform the oracle comparisons.

| Census lane | Tests passed | Subtests passed | Skipped | Failed | Calls / modules |
| --- | ---: | ---: | ---: | ---: | ---: |
| core-accounting | 1,875 | 426 | 24 | 0 | 343 / 14 |
| reports-contracts | 2,368 | 2,820 | 1 | 3 | 332 / 32 |

The three failures are identical in shadow and census:

- `tests/test_custody_gap_port.py::test_every_gap_generator_matches_and_counts_both_engines`, fanout parameter.
- `tests/test_custody_interpretation_stage.py::test_every_generator_matches_and_counts_both_engines`, fanout parameter.
- `tests/test_custody_interpretation_stage.py::test_full_stage_vectors_wrappers_and_refusals`.

The fanout comparisons stop Python at interpretation or gaps but request
Rust's full finalization. Seed 24 (`nan_value`) therefore gets Rust's
`validation` refusal. Python's full projection also refuses that price.
The four non-finite fanout cases return exactly the same Rust responses
before and after the speed pass. The wrapper test additionally reaches an
existing Rust finalization discrepancy: `migration_1_0` through
`migration_1_6`, whose stored transaction-id array includes null, boolean,
numeric, structured and surrogate values, return `ValueError` on both
baseline and tip while Python's full projection succeeds. This discrepancy
is retained and needs separate custody work; it is not a tax parity result.

The local reproduction, `target/ledger-perf/repro-custody-stage.py --full`,
checks those 11 refusals against a rebuilt baseline custody probe. With
`finalized=False`, **2,094 fanout histories/permutations and all 106 wrapper
vectors match** at their declared stage boundaries. No Python reference,
harness or product code is changed. These three failures still prevent a
fully green reports gate.

There is an existing harness path assumption: four `SessionEndTest` tests
expect a diagnostic containing `in /...`, but `tests/conftest.py` deliberately
reports a test module inside the repository as a relative path. Reproduce
without changing Python by setting `TMPDIR` to an existing absolute
directory under this checkout's `target/`, then running:

```sh
mise exec rust@1.99.1 uv@latest -- uv run --locked python -m pytest \
  tests/test_tax_shadow_harness.py::SessionEndTest -q
```

The initial direct run therefore has four path-text assertion failures and
1,024 passes; the ledger comparisons themselves agree. The clean runs above
used an unchanged disposable source copy under `target/ledger-perf/verify-src`
and `TMPDIR` under `target/ledger-perf/tmp`, outside the tested source root.
They imported the rebuilt binding from this checkout's Python 3.11 environment;
all 1,034 relevant copied source, test, script and dependency files were
checked byte for byte against the checkout. The mutation and verification
build trees were removed after their summaries were saved.

Local evidence remains under `target/ledger-perf/`: `measurements.json` and
`measurements.jsonl` contain every timing, phase and load; `gates/` contains
the test logs and summaries; `custody-repro-summary.json` and its input files
record the baseline refusals. `repro-shadow-temp.sh` preserves the separate
temporary-path harness reproduction.

The final classification update leaves `tests/rust_port/dispositions.json`
unchanged, and `scripts/python_test_shards.py run preflight --ci` passes
**57/57** tests. The earlier checkpoint at `de836bd2` already passed fmt,
both clippy runs, 871 workspace tests and the custody corpus/bench hash
comparisons. This completion commit changes the report and backlog docs;
the optimized engine remains at that verified source revision.

## Where the engine sits

Journals run on `KassiberTaxEngine` (`kassiber/core/engines/ledger.py`). It
takes one `FinalizedTaxProjection` per book and nothing else: raw rows are a
`TypeError`, and a projection without rows gives an empty ledger without any
check or engine call. For every other book it:

1. builds a schema-2 request (`ledger_request.py`), which runs the
   configuration checks and refuses shapes a finalized projection cannot
   have;
2. runs the request through the engine's `ledger` operation;
3. renders the response into `TaxEngineLedgerResult` (`ledger_response.py`).

Python keeps the jobs that read evidence or render Kassiber's shapes: the row
facts of the request (normalized kind, exact pricing, privacy-hop evidence,
conflict and pending holds, chain-order evidence, timestamps), the
configuration checks, the reader's rendering (uuids, ids, row times, legacy
floats, quarantine details, the Austrian Kennzahl), and persistence in
`custody_journal.py`. The engine makes every tax decision in between:
normalization, the Austrian regime tracker, the walk order and balance
gates, the swap-link selection, the generic rail carry, the lot engine, and
the composition of journal entries and their descriptions.

Plan 20's phase 4 moved the books to this path one capability at a time
(`generic`, `generic_carry`, `at`, `at_swap`) and then deleted the phase-3
adapter (`engines/rp2.py`), its normalizer, the Austrian regime tracker, and
`pair_allocation.py` from the product. The adapter lives on only as the
frozen oracle in `tests/tax_oracle/`, which runs on RP2's calculation; tests
compare production with it ([testing](testing.md#tax-engine-backends)).
Product code has no setting or environment variable for a backend. Journal
entries record what computed them in their calculation metadata: `"engine":
"kassiber_tax"` for production, `"rp2"` for the oracle on RP2. They do not
record the engine version yet (TODO "Record the engine and country-rules
versions").

## Decimal rules

- Every engine value is a decimal with a sign, an integer coefficient, and an
  exponent, following Python's `decimal` module (General Decimal Arithmetic).
  Exponents are part of the result: `3.0E+2` and `300` are different outputs.
  The type is the `kassiber-pydecimal` crate (`crates/pydecimal`), which the
  backend crates share; generated CPython and RP2Decimal vectors check it.
- Arithmetic uses 32 significant digits with round-half-even. Exact results
  keep their ideal exponent; inexact ones are rounded to 32 digits.
- Comparisons follow RP2: `a` and `b` compare by rounding `a - b` half-even to
  13 decimal places, so values within `5E-14` are equal. `<` is "not `>=`" and
  `<=` is "not `>`". The ledger layer's own balance mirror compares exactly,
  as the phase-3 adapter did.
- Kassiber pins its own Python decimal context to the same precision
  (`kassiber/__init__.py`), for every thread. Before that, the precision of
  journal processing depended on which thread first imported RP2. Importing
  RP2 also trapped `FloatOperation` on that thread; Kassiber's context does
  not, so mixing floats and decimals does not raise.
- The engine returns exact values only. The reader alone performs the
  legacy float conversions (`float()`, `Decimal(str(float))`), JSON
  serialization, and uuid generation, so no CPython float behavior is
  reimplemented in Rust.

## Boundary format

The binding exposes one JSON-in, JSON-out function, `ledger`, and releases
the GIL while the engine runs; a Rust panic becomes `RuntimeError`, never a
partial result.
Decimals cross the boundary in the canonical form `[-]<digits>E<exponent>`,
for example `-12345E-3`, which Python's `Decimal()` parses back to the
identical value. `engine_version()` returns the crate version (0.2.0).

| Function | `schema_version` | Operations | Callers |
|---|---|---|---|
| `ledger` | 2 | `ledger`, `prepare`, `manifest` | journal processing (`KassiberTaxEngine`) and the tests |
| `check_entry`, `compute` | 1 | `check_entry`; `compute`, `validate`, `compute_multi` | the core's fixture tests only; the binding does not expose them ([schema 1](#schema-1-the-fixture-api)) |

Each function checks `schema_version` exactly and answers a document of
another version with a `RequestError`.

### The `ledger` request

`kassiber/core/engines/ledger_request.py` builds the request from the profile
and one `TaxEngineLedgerInputs`, with the standard library only;
`tax-engine/core/src/ledger/request.rs` reads it:

```json
{
  "schema_version": 2,
  "operation": "ledger",
  "policy": {
    "country": {"kind": "generic", "long_term_days": 365},
    "method": "fifo",
    "method_label": "FIFO",
    "require_coarse_review": false
  },
  "config": {"assets": ["BTC"], "exchanges": ["Cold", "Hot"], "holders": ["Main"]},
  "holder": "Main",
  "wallets": [{"id": "A", "label": "Cold"}, {"id": "B", "label": "Hot"}],
  "rows": ["<row>", "..."],
  "intra_pairs": [{"out": "<row id>", "in": "<row id>", "source": null, "group_id": null}],
  "cross_asset_pairs": ["<pair>", "..."]
}
```

| Field | Meaning |
|---|---|
| `policy` | `country` is `{"kind": "generic", "long_term_days": N}` (capped at `sys.maxsize`) or `{"kind": "at"}`. `method` is the profile's method (`fifo`, `lifo`, `hifo`, `lofo`, `moving_average`, or `moving_average_at`) when its policy allows it, else `null`; `method_label` is the stored method text for the method gate's message. `require_coarse_review` is the profile's opt-in |
| `config` | The names the lot engine's membership checks accept: the assets, wallet labels, and profile label, sorted, stripped, and read as RP2 read its INI file |
| `holder` | The profile label, unstripped, as entries carry it |
| `wallets` | `{id, label}` per wallet reference, in order; account fields stay in Python |
| `rows` | One typed row per projection row, in projection order (below) |
| `intra_pairs` | The same-asset moves: the `out` and `in` row ids, how the pair was proven (`source`), and its atomic bundle (`group_id`; `null` leaves the move ungrouped) |
| `cross_asset_pairs` | `pair_id` (raw), `out_id`, `in_id`, the legs' transaction ids and assets, `policy`, and `bitcoin_rail` (a pair between BTC and LBTC) |

A row:

| Field | Meaning |
|---|---|
| `id`, `anchor` | The synthetic projection row id, and the `journal_transaction_id` that journals and quarantines name |
| `asset`, `wallet_id`, `direction` | As projected; `direction` is `inbound` or `outbound` |
| `ts` | The time as RP2 parses it, as in a [schema-1 entry](#schema-1-the-fixture-api); its `raw` text orders every string comparison |
| `cutoff_instant` | The time as Kassiber's Austrian rules read it for the Neuvermögen cutoff (`datetime.fromisoformat`, with `Z` as UTC and a naive time taken as UTC): `{"us": ...}`, or `{"error": ...}` with the `ValueError` message Python raises |
| `amount_msat`, `fee_msat` | Integer msat |
| `rate`, `value` | `fiat_rate_exact` and `fiat_value_exact`, else `fiat_rate` and `fiat_value`, as canonical decimals; stored non-finite text passes through as CPython writes it; `null` when unpriced |
| `coarse` | Priced from a coarse fallback the owner has not accepted at this exact price (`pricing_review_accepted_price`) |
| `kind` | `normalized_transaction_kind(row)`; the kind tables are the engine's (`manifest.rs`) |
| `text` | The first non-empty of the row's note, description, and kind |
| `regime_override`, `category_override`, `taxability_off` | The reviewed Austrian overrides: `alt` or `neu`, a category, and `taxability_override == 0` |
| `privacy_hop`, `hold` | Privacy-hop evidence, and a conflicting-spend or pending-confirmation hold found over all rows of the asset (a conflict wins) |
| `chain_order` | Chain-order evidence: the move's `(chain, network, txid, asset)` scope and the sorted scopes it spends, or `null` |

The request carries facts, never marker text: the engine derives every
Austrian marker value itself. Every type rejects unknown fields and missing
ones (a field without a value is `null`, not left out; only a failed
timestamp may omit `error_class`).

Before it builds a request, the builder runs the phase-3 adapter's
configuration checks, with the same exception classes and messages:

- every wallet label, every asset, and the profile label must be a non-empty
  token without `, \n \r [ ] = : # ;`;
- the book needs a wallet and an asset;
- the policy must build, and a generic book's currency must be ISO 4217;
- the names must survive RP2's INI parse (`%` interpolation, and no empty or
  duplicate element after stripping).

These rules come from RP2's INI configuration file (TODO "Relax the
configuration label rules"). The builder then raises what the adapter raised
for a move whose legs cross assets (`AssertionError`) and for a wallet-scoped
cost-basis pool (the policy's `AppError`). Last, it refuses with an
`EngineContractError` naming the shape (`ledger_request.INVARIANTS`) every
input a finalized projection cannot have: a wallet reference whose key is not
its id; a row id without a synthetic prefix, a missing anchor, an unknown
wallet or direction, non-integer msat, a negative fee or amount, a
non-positive amount (a zero-amount outbound row with a fee is a fee-only spend),
`amount_includes_fee`, custody component fields, or corrupt decimal text; and
a move with `group_block_rows`, an unreviewed privacy crossing, a leg shared
with another move, or unequal amounts. The phase-3 normalizer handled these
through branches no finalized row reaches.

### The `ledger` operation

`ledger` (`tax-engine/core/src/ledger/run.rs`) runs the phase-3 adapter's
pipeline in its order, with the rows grouped by asset in first-appearance
order. Independent pure work may run concurrently, but its results and
failures are consumed in that order:

1. **Generic rail carry** (generic books, `ledger/carry.rs`). A
   carrying-value pair between BTC and LBTC whose legs are rows prices both
   legs at what the out leg's lots cost. The lot engine knows that only after
   a whole prepare pass and computation, and a carried acquisition can be the
   next hop's source, so the carry repeats such passes until one prices every
   row as before and blocks nothing new, at most one pass more than there are
   such pairs. Running out of passes keeps the last pricing (TODO "Settle the
   generic Bitcoin rail carry in one pass"). A pair whose out leg has no
   positive fragment basis is blocked for good: both legs are quarantined as
   `bitcoin_rail_carry_basis_unresolved` with the out leg's first reason, else
   `source_basis_unavailable`, and its rows (a direct payout's target-asset
   disposal too) leave every later pass. A priced row's value and rate are
   `format(x.normalize(), "f")` of the basis and of the basis per unit of its
   amount, read back as exact pricing. When a pass settles and its pricing
   and excluded rows match the final pass's inputs, that pass's stages and
   computed outputs are reused for the final run.
2. **First prepare pass**, with results consumed in asset order: normalize
   (`ledger/normalize.rs`, with the Austrian regime tracker in
   `ledger/regimes.rs`), then the walk (`ledger/order.rs`, `walk.rs`,
   `describe.rs`). The walk orders the asset's events and moves (acquisitions,
   then moves in wallet-dependency order, then disposals, per timestamp),
   keeps the adapter's per-account balance mirror and gates, and builds the
   lot engine's entries.
3. **Swap-link selection** (Austrian books, `ledger/swaplinks.rs`) reads the
   first pass. Of the carrying-value cross-asset pairs whose legs are rows, a
   pair a leg of which that pass dropped or quarantined is blocked: both legs
   are quarantined as `at_swap_basis_carry_unresolved` with that reason, else
   `swap_leg_unavailable` (a direct payout only on its source, with its
   target-asset disposal `<pair>:out` withheld too). A pair whose out leg was
   Neu is linked, and an Alt one books as a sale and a purchase.
4. **Second prepare pass**, when the selection linked or blocked a pair. It
   leaves the blocked rows out and gives the linked rows' events their link,
   and its stages replace the first pass's. It does not check a link again, so
   a chained swap whose first hop is blocked, or a link whose out leg the
   second pass makes Alt, leaves a marker unpaired and the swap check aborts
   the report, as the adapter's did (TODO "Choose Austrian swap links on the
   final pass").
5. **Lot engine**, over each asset with an inbound entry: the country's
   swap-pair checks, the accounting-method gate, then the multi-asset hook
   (the Austrian swap runner, when the entries' notes pair `at_swap_link=`
   markers) or each asset's own computation. An asset without an inbound
   entry is not computed: its other entries book nothing, but its transfer
   audit rows remain.
6. **Composition** (`ledger/compose.rs`): acquisitions, realized entries with
   their gain/loss fragments, transfer legs, holdings, and the tax summary, as
   the adapter composed them. On an Austrian book each fragment's category
   follows the row's reviewed overrides.

Quarantines come in the adapter's order: the rail carry's, the swap
selection's, then each asset's normalize and gate quarantines.

The walk composes descriptions with the phase-3 grammar (`describe.rs`),
writing each token only when it has a value:

- an event: `at_regime=`, `at_regime_basis=`, `at_pool=` (Austrian books, the
  global pool written `default`), `at_swap_link=`, then the row's text;
- a move: `at_regime=`, `at_pool=`, a pairing phrase, then its text;
- a transfer leg: `at_regime=`, `at_alt_out=` and `at_alt_in=`, `at_pool=`,
  `Transfer <from> -> <to>`, then a provenance suffix.

An event's and a move's description is also the `notes` of the lot engine's
entry, which the Austrian rules read for their markers (see the
[Austrian handoff](../austrian-handoff.md#wire-format)). The row's text comes
last and verbatim, so marker tokens a user typed into a note or description
keep their phase-3 effect: paired `at_swap_link=` tags on a Neu sale and a
receipt of another asset give a swap carry under `moving_average_at`, and a
duplicate or conflicting marker aborts where the rules consult it (TODO "Stop
parsing user description text"). The walk's gates read only the selected
links. Persisted descriptions keep their markers, which the exit-tax report
and the desktop read.

### The `ledger` response

```json
{"schema_version": 2, "ok": true, "engine_version": "0.2.0",
 "entries": [...], "quarantines": [...], "intra_audit": [...],
 "tax_summary": [...], "holdings": [...], "carried_rows": [...]}
```

Amounts are canonical decimals or integer msat, and every list is in the
order the reader keeps (`tax-engine/core/src/ledger/response.rs`):

| Field | Meaning |
|---|---|
| `entries` | Per asset, in first-appearance order: acquisitions in lot order (`row`, `wallet` label, `quantity`, `fiat_value` at the effective basis, `unit_cost`, `description`); realized entries (`income`, `disposal`, `fee`, `transfer_fee`) in the order their taxable event first appears, with the signed `quantity`, `proceeds`, `cost_basis`, `gain_loss`, `description`, `at_category` after the reviewed overrides, `capital_gains_type`, and `fragments`, one per consumed lot or pool state with its event's and lot's values; Austrian outflows split by pre-override consumed regime and replace input `at_regime=` tokens with that regime; then transfer legs (`transfer_out`, `transfer_in`), each naming its `intra_audit` row |
| `quarantines` | `{row, reason, detail_kind, detail}`: the row whose anchor the quarantine names, the reason, which template built the typed detail (`pricing_review`, `privacy_hop`, `hold`, `partner_leg`, or `plain`), and the detail, where a float the adapter wrote is `{"float": "<decimal>"}` |
| `intra_audit` | One row per booked move: its `pair` index, `sent`, `received`, and `fee` as decimals and in msat, `spot_price`, the Austrian `at_regime`, `regime_flows`, and `at_pool`, and the legs' `description` |
| `tax_summary` | Per asset, sorted by year, asset, type, and holding period, without fees and moves: `year` (in the event's own offset), `asset`, `transaction_type`, `long_term`, `quantity`, `proceeds`, `cost_basis`, `gain_loss` |
| `holdings` | Per asset and balance, positive balances only: `asset`, `wallet` label, `quantity`, `cost_basis` |
| `carried_rows` | The rows the rail carry priced: `row`, and its `value` and `rate` as fixed-point text |

The reader (`ledger_response.py`) adds what is not tax logic: uuid4 ids;
workspace and profile ids; wallet and account ids looked up by label; each
entry's transaction id and time from its row; the Kennzahl of its category;
the `calculation` metadata, whose fragments carry their rows' facts and
pricing; the row facts a quarantine's `detail_kind` names (by kind, never by
reason, since one reason can come from two templates) and the detail's JSON;
the legacy floats of the audit, gate details, and tax summary; and the
holdings accumulated per account and wallet. Transfer-leg quantities pass
through `float`, as the adapter's did (TODO "Keep transfer-leg quantities
exact"). The projection's own quarantines come first, and its cross-asset
pairs and direct payouts pass through.

A failure is
`{"schema_version": 2, "ok": false, "error": {"stage", "asset"?, "inner": {"class", "message"}}}`.
`kassiber/core/engines/errors.py` raises what the phase-3 adapter raised at
that stage; a wrapper is raised from the inner error:

| `stage` | Raised |
|---|---|
| `prepare` | The error of a prepare pass (normalization, an entry constructor in walk order, or the rail carry's arithmetic), unwrapped |
| `validate` | `AppError("RP2 cross-asset input validation failed: ...", code="rp2_input_validation")`, from the Austrian swap-pair checks |
| `compute_multi` | `AppError("RP2 multi-asset tax calculation failed: ...")`, from the multi-asset hook. The method gate fails here (`Unsupported RP2 accounting method '<label>'`), before any asset is computed |
| `compute` | `AppError("RP2 tax calculation failed for asset '<asset>': ...")`, from one asset's computation |
| `classify` | The first failing Austrian classification, or another composition failure, unwrapped |
| `request`, `unsupported` | `EngineContractError`: the engine could not read the request, or does not implement it (no operation at present) |

The "RP2 ..." texts are the adapter's, byte for byte (TODO "Rename the RP2
texts of engine failures").

### `prepare` and `manifest`

`prepare` is a diagnostic for tests ([stage comparison](testing.md#stage-comparison)).
It stops after the final prepare pass and returns, per asset:

- the normalized stage (`ledger/normalize.rs`, `regimes.rs`): the events and
  moves the walk books, with their Austrian regimes, elections, and regime
  flows, their order, normalize's quarantines, and the time from which the
  asset's lot state is tainted;
- the prepared stage (`ledger/order.rs`, `walk.rs`, `describe.rs`): the lot
  engine's entries as schema-1 entry documents with their composed notes and
  text, the gates' quarantines, and the transfer audit.

It also returns the swap-link selection (`swap_links`: the rows it links, the
ids its blocked pairs leave out, and their quarantines) and the rail carry
(`rail_carry`: the rows it priced with their value and rate, the ids its
blocked pairs withhold, their quarantines, and its passes). A failed `prepare`
lists the stages the failing pass reached (a failing carry pass's own, and
then neither carry nor selection), the carry when it ended, and the selection
when the first pass after it succeeded.

`manifest` (`{"schema_version": 2, "operation": "manifest"}`) returns the
engine's names and tables (`manifest.rs`): its version, the kind tables,
marker names, the Austrian cutoff, default pool, and categories, the entry
types, and the quarantine reasons and detail kinds it emits.
`tests/test_tax_ledger_contract.py` compares them with the copies Python still
spells (`transaction_kinds`, `journal_markers`, `austrian`,
`exit_tax.RECOGNIZED_ENTRY_TYPES`, `quarantine_catalog`).

## Schema 1: the fixture API

`check_entry` and `compute` expose the lot engine (`engine.rs`, `swaps.rs`,
`entry.rs`), one call for each RP2 call the phase-3 adapter made. Journal
processing does not call them: the `ledger` operation runs the same lot
engine directly. They remain the core's interface for the RP2 fixtures
below, which pin that lot engine (`tests/api.rs`, `engine_fixtures.rs`); the
Python binding does not expose them. `tax-engine/scripts/v1_request.py`
builds their documents with the standard library only: the canonical decimal
encoding, the timestamp fields as RP2 parses them, and the entry text. The
fixture generator uses it, and so does the stage comparison
([testing](testing.md#stage-comparison)), which records the oracle's entries
as these documents.

The request builder parses ISO 8601 with an optional offset only, where RP2's
`dateutil` also took free-form text. The custody projection therefore hands
the engine canonical RFC3339 UTC timestamps. Row times are canonical when
written; a direct payout's `payout_occurred_at` stored before that rule is
canonicalized when projected (accepting typed forms such as
`2025-09-01 12:00:30 +00:00` or a trailing `UTC`), and a value that still does
not parse fails journal processing with a `validation` error naming the payout.

### `check_entry(request) -> response`

```json
{
  "schema_version": 1,
  "operation": "check_entry",
  "config": {"assets": ["BTC"], "exchanges": ["Cold"], "holders": ["Main"]},
  "entry": "<entry>"
}
```

Runs one entry through the checks of RP2's constructor in RP2's order, with
its 13-place comparisons and messages. `config` holds the configuration's
names for the membership checks; without it they are skipped. The response's
`entry` holds the values the constructor stores and derives: `unique_id`,
`notes`, `transaction_type` (the lowercase value), `spot_price`, `crypto_fee`,
`fiat_fee`, the kind's amounts (`crypto_in`, `fiat_in_no_fee`,
`fiat_in_with_fee`; `crypto_out_no_fee`, `crypto_out_with_fee`,
`fiat_out_no_fee`, `fiat_out_with_fee`; `crypto_sent`, `crypto_received`, with
`null` for other kinds), `crypto_balance_change`, `crypto_taxable_amount`,
`fiat_taxable_amount`, `is_taxable`, and `is_earn`. `is_taxable` is `null`
when evaluating it raises; RP2 evaluates it lazily.

### `compute(request) -> response`

```json
{
  "schema_version": 1,
  "operation": "compute",
  "country": {"kind": "generic", "long_term_days": 365},
  "method": "fifo",
  "assets": [{"asset": "BTC", "entries": ["<entry>", "..."]}]
}
```

`operation` is one of:

- `compute`: RP2's `compute_tax` for exactly one asset.
- `validate`: the country's `validate_input_data` over every asset of the
  run. Austria checks the `at_swap_link` pairs in RP2's order with its
  messages; other countries check nothing. Success is
  `{"schema_version": 1, "ok": true}`.
- `compute_multi`: the country's `compute_tax_for_assets` over every asset.
  Without a swap pair (and for every non-Austrian country) it answers
  `{"schema_version": 1, "ok": true, "handled": false}` and the caller runs
  `compute` per asset. Otherwise it runs the Austrian multi-asset runner,
  which carries Neuvermögen swap basis into the incoming lots, and answers
  `"handled": true` with `assets` for every asset in request order.

`validate` and `compute_multi` take the assets in the caller's order, each
named once (a repeated name is a `RequestError`). `country` is
`{"kind": "generic", "long_term_days": N}` or `{"kind": "at"}`, whose holding
period is `sys.maxsize`. `method` is one of `fifo`, `lifo`, `hifo`, `lofo`,
`moving_average`, and `moving_average_at`. Entries are in insertion order;
the engine re-runs their constructor checks, then reproduces `compute_tax`,
including its cross-set duplicate-row check and the
`Internal error: AVL tree has no root node` failure when no lot is visible.
Every operation and method is implemented, for both countries.

An entry is one constructor call with the arguments as given:

| Field | Meaning |
|---|---|
| `kind` | `in`, `out`, or `intra` |
| `row`, `unique_id` | The caller's row number (RP2's `internal_id`, the entry's identity) and its transaction id; `null` stands for `None` |
| `asset`, `transaction_type`, `notes` | As given to the constructor; an `intra` entry has no `transaction_type` (it is always `MOVE`) |
| `exchange`, `holder` or `from_exchange`, `from_holder`, `to_exchange`, `to_holder` | Wallet labels and the profile label |
| Amount fields | `in`: `spot_price`, `crypto_in`, `crypto_fee`, `fiat_in_no_fee`, `fiat_in_with_fee`, `fiat_fee`. `out`: `spot_price`, `crypto_out_no_fee`, `crypto_fee`, `crypto_out_with_fee`, `fiat_out_no_fee`, `fiat_fee`. `intra`: `spot_price`, `crypto_sent`, `crypto_received`. Each canonical or `null` (not passed) |
| `ts` | `raw` (`str()` of the argument), `us` (absolute microseconds since the Unix epoch), `offset_s`, `date` and `year` in the timestamp's own offset, `vienna_date`, `display` (`str(datetime)`), and `iso` (`datetime.isoformat()`). When parsing failed, `raw`, `error` (RP2's message), and `error_class` (`TypeError` for a non-string, otherwise omitted) instead |
| `text` | `str(transaction)`, which RP2 embeds in its negative-balance and duplicate-entry messages; the `ledger` operation renders it itself (`ledger/text.rs`) |

A field that does not belong to the entry's kind is a `RequestError`, as is a
`ts` whose date and year disagree with its instant and offset.

A successful response lists, per asset:

| Field | Meaning |
|---|---|
| `in_transactions` | Visible lot rows, in lot-list order |
| `in_fiat_in_with_fee` | Effective acquisition basis per lot row |
| `gain_losses` | `event` (`kind`, `row`), `lot` (row or `null`), `crypto_amount`, `fiat_cost_basis`, `proceeds`, `fiat_gain`, `unit_cost_basis_override`, `long_term`, and for Austrian books (any method) either `at_category` or `at_category_error` |
| `yearly` | `year`, `transaction_type`, `long_term`, `crypto_amount`, `fiat_amount`, `fiat_cost_basis`, `fiat_gain_loss`, in RP2's order |
| `open_positions` | `row`, `sold_percentage`, `fiat_in_with_fee` |
| `balances` | `exchange`, `holder`, `final_balance`, `acquired_balance`, `sent_balance`, `received_balance`, in RP2's order |

`at_category` is the `AtDisposalCategory` value RP2's `classify_disposal`
returns for the row: `income_general`, `income_capital_yield`, `neu_gain`,
`neu_loss`, `neu_swap`, `alt_spekulation`, or `alt_taxfree`.
`at_category_error` (`class`, `message`) is the error it raises instead, for
example on conflicting `at_regime` markers of a lot that a lot method never
parsed. The caller raises it when it classifies that row, so the run aborts
only where RP2's would.

A failure is `{"ok": false, "error": {"class": ..., "message": ...}}`. The
class is `ValueError`, `TypeError`, or `RuntimeError` for RP2's errors (which
Python raises as `EngineValueError`, `EngineTypeError`, or
`EngineRuntimeError` with that message when a `ledger` failure carries them);
`InvalidOperation`, `DivisionByZero`, or `Overflow` for the `decimal`
exceptions RP2's arithmetic raises (for example a comparison of values of
`1E+19` or more), with CPython's message; `RequestError` for a malformed
request (including a timestamp without the `vienna_date` an Austrian rule
needs); and `Unsupported`, reserved for a method or operation a build does
not implement (none at present). The last two are caller or engine bugs and
block the report, as a Rust panic does.

## Fixtures

`tax-engine/scripts/gen_engine_fixtures.py` runs RP2 as the oracle, building
its inputs as the phase-3 adapter built them, and writes two files under
`tax-engine/core/tests/data/`:

- `generic_fixtures.json`: engine requests with RP2's responses for the lot
  methods: the spec scenarios, seeded random histories, and constructor
  checks.
- `at_fixtures.json`: the moving averages and Austria. Each scenario is one
  adapter run over every asset of a book, recorded as its `validate`,
  `compute_multi`, and per-asset `compute` steps with RP2's results,
  including `classify_disposal` for every Austrian row. It holds the spec
  scenarios, edge probes, and seeded random histories (swaps, pools, regimes
  and routing, Wahlrecht, fees, earn types, cutoff and Spekulationsfrist
  boundaries, and injected errors).

`tax-engine/core/tests/engine_fixtures.rs` reports every differing field by
case and path. The generator is deterministic for a seed; larger stress sets
can be checked through `KASSIBER_ENGINE_FIXTURES` and `KASSIBER_AT_FIXTURES`.
`tax-engine/core/tests/ledger_text.rs` checks the engine's own rendering of
`str(transaction)` (`ledger::text`, which the `ledger` operation uses because
it builds its entries without a request `text`) against the RP2 text these
fixtures store.

`ledger_fixtures.json`, from `tax-engine/scripts/gen_ledger_fixtures.py`, holds
whole-ledger goldens: finalized projections with the frozen oracle's ledger,
the one production must give, the schema-2 request production's builder
makes of each, and the engine's response to each request
([projection goldens](testing.md#projection-goldens)).
`tax-engine/core/tests/ledger_fixtures.rs` reads every stored request into
the typed request and writes it back unchanged, replays every stored
response, and checks that `prepare` answers for every book, agrees with
`ledger` on the rail carry, and lists the stages a failed pass reached.

`tax-engine/core/tests/ledger_properties.rs` checks invariants of the
`ledger` operation on seeded generic and Austrian books (plan 20
Verification 1): the same request gives the same bytes; msat are conserved
per asset and per move; every book succeeds and each wallet's journal
replays to its holding without going below zero; and pricing an unpriced
inbound row (a purchase, income, or a move's in leg) quarantines no new
row, except the shortfall a released earlier sale causes, which the oracle
shares (TODO "Keep pricing from adding quarantines"). An Austrian book may
instead end in the Neu shortfall abort, rarely, and pricing may release
one, as on the oracle (TODO "Stop an Austrian Neu shortfall from aborting
the whole report"); its entries start with their regime, its moves' regime
flows conserve and match their legs' descriptions, and its categories
follow the reviewed overrides. Austrian swap books add cross-asset swaps
between BTC, LBTC, and USDT. Besides the Neu shortfall they may end in the
other failures the oracle shares (a Neu swap under a lot method, an
unpaired link after the second pass, and rarely cyclic swaps or an account
a linked swap-out overdrew beyond the lot engine's tolerance; TODO "Choose
Austrian swap links on the final pass"); otherwise a blocked pair books
neither leg, both booked legs of a linked pair carry its link, a Neu swap's
acquired lot takes what was swapped (its fee aside) at the source pool's
average, each wallet ends at its holding (below zero only where a linked
swap-out skipped the gates), and pricing quarantines no new row, with the
same exclusion. Generic rail books add Bitcoin rail changes between BTC and
LBTC, some straight back at the same instant: they conserve msat per asset
and per wallet as other generic books do; a pair the carry blocked books
neither leg and is quarantined on both; both legs of every other pair are
priced at one carried value, and the in leg is acquired at what the out
leg's fragments cost, unless the carry ran out of passes before its carries
settled, which the oracle shares (TODO "Settle the generic Bitcoin rail
carry in one pass"); and pricing quarantines no new row, with the same
exclusion, where the shortfall can also block a later rail pair. On every
generic book, with holding periods of 0, 1, 30, and 365 days, a realized
entry is long term exactly when each lot it took was held at least
`long_term_days` whole days (24 hours each, floored), and short when it took
no lot.

## Privacy

The engine performs no I/O, network access, or logging and keeps no state
between calls. Production does not write RP2's temporary configuration file;
only the oracle on RP2 does, in tests.


The shared PyO3 module also exposes the independent R2 `custody` operation.
Its raw request and stage response are specified in [custody engine](custody-engine.md).
It does not change the `ledger` request or allow raw observations into the tax engine.
