# R4-11 render performance checkpoint

This local follow-up starts at `5ade4b81` on `t3code/r4-11-render-2`.
**R4-11 acceptance remains open.** The 20k render budgets are met on the copied
synthetic books. The lossless transitional boundary and complete branch mutation
coverage are not finished. Python still owns production journal construction;
the Rust library and probe remain unregistered.

## Requirement and scope

Preserve every byte of the original build-state oracle, including exact msat,
Decimal representations, candidate order, compatibility metadata, refusals and
the declared UUID bijection. Change rendering and its joins without changing
custody or tax decisions. There are no dependency, persistence, network, CLI or
AI contract changes.

## What changed

The 300-sample gdb profile of multi-wallet 20k attributed 48.7% of all samples
to ownership review, 28.0% to onchain scope lookup, and 26.3% to ledger
rendering (inclusive percentages overlap). The probe also decodes the response
outside the render timer, so these are process-wide sampling percentages.

The 2k book has no quarantine-driven ownership setup; the 20k book has 17
quarantines. Those reasons cannot produce ownership cards, but the old join
still decoded scopes and built physical and legacy groups for every row.
This is an extra full-book pass activated at 20k, rather than evidence of a
quadratic join on these particular books. A book with many pairable blocked
sources can still scan many inbound candidates; that worst case remains.

- Select pairable reasons before building ownership indexes. Preserve inbound
  amount conversion. Compute scopes and legacy/physical groups only when a
  source actually needs them. Cache scopes by immutable row address, including
  separate rows with duplicate ids. A drop guard clears shared graph memos on
  success, early return and refusal.
- Borrow row and wallet-label keys. Cache source provenance in row slots rather
  than a second string-keyed map. Preserve last-row/last-label selection and
  carried-price overlays.
- Validate ordinary UTC calendar dates with the existing timestamp parser once
  per date, normalize the clock directly, and retain the shared parser for
  other spellings and refusals. Unhashable timestamp operands reproduce
  Python's `TypeError`.
- Join diagnostic balances using borrowed keys. Skip the observation index
  when there are no named reasons. Accumulate ordinary postings in checked
  `i128`, promoting to `BigInt` before overflow. Wallet and asset identity,
  output sorting and the single-distinct-reason rule are unchanged.
- Convert ordinary msat amounts with exact machine arithmetic, preserving
  half-up rounding and sign. Wide coefficients, results and exponents retain
  the existing arbitrary-precision/context path. Empty payout-record lists
  no longer build a compatibility lookup table.

`render.inputs` and `render.finish` add static timing spans under the existing,
opt-in timing feature. They do not log in production.

## Timing evidence

The measured boundary excludes custody, tax computation, response decoding,
canonical output serialization and persistence. Every benchmark book first
compares its complete canonical build state. Python and Rust run back to back;
completed states are released outside the timer. Baseline one-minute loads
exceeded 8; both matrices use five alternating trials and take each
implementation's best result. These are shared-host measurements, not
end-to-end journal times.

The baseline and the final matrix each alternate Python/Rust, five trials per
book. Loads are recorded for each individual trial in
`target/r4-render/benchmark-before/timings.json` and the final timing artifact
listed below. Do not compare these render-only times with the end-to-end
pipeline budget.

| Book | Python before (s) | Rust before (s) | Before load 1m |
| --- | ---: | ---: | ---: |
| Generic 2k | 0.083671 | 0.023318 | 9.024 |
| Generic 20k | 0.950398 | 0.291239 | 8.099–13.292 |
| Multi-wallet 2k | 0.088412 | 0.027090 | 17.113 |
| Multi-wallet 20k | 1.280884 | 0.678038 | 21.380–30.915 |

The final source's matrix is retained at
`target/r4-render/benchmark-review/timings.json`. Both 20k budgets hold, with
little margin at generic 20k. Baseline and final host loads differ; the
Python/Rust ratios below use paired final trials on the same host.

| Book | Python final (s) | Rust final (s) | Python / Rust | Budget (s) | Final load 1m |
| --- | ---: | ---: | ---: | ---: | ---: |
| Generic 2k | 0.082105 | 0.012846 | 6.39× | 0.02 | 4.203 |
| Generic 20k | 0.853091 | 0.145763 | 5.85× | 0.15 | 3.784–3.852 |
| Multi-wallet 2k | 0.078251 | 0.016541 | 4.73× | 0.02 | 3.721 |
| Multi-wallet 20k | 1.078828 | 0.170602 | 6.32× | 0.20 | 3.222–3.242 |

| Book | Trial | Python (s) | Python load 1m | Rust (s) | Rust load 1m |
| --- | ---: | ---: | ---: | ---: | ---: |
| Generic 2k | 1 | 0.120446 | 4.203 | 0.017771 | 4.203 |
| Generic 2k | 2 | 0.082727 | 4.203 | 0.016052 | 4.203 |
| Generic 2k | 3 | 0.086732 | 4.203 | 0.014450 | 4.203 |
| Generic 2k | 4 | 0.082105 | 4.203 | 0.013810 | 4.203 |
| Generic 2k | 5 | 0.083065 | 4.203 | 0.012846 | 4.203 |
| Generic 20k | 1 | 0.883255 | 3.839 | 0.168880 | 3.839 |
| Generic 20k | 2 | 1.328407 | 3.852 | 0.151739 | 3.852 |
| Generic 20k | 3 | 0.857500 | 3.852 | 0.146426 | 3.852 |
| Generic 20k | 4 | 1.316990 | 3.852 | 0.151210 | 3.852 |
| Generic 20k | 5 | 0.853091 | 3.784 | 0.145763 | 3.784 |
| Multi-wallet 2k | 1 | 0.080455 | 3.721 | 0.022835 | 3.721 |
| Multi-wallet 2k | 2 | 0.079368 | 3.721 | 0.017328 | 3.721 |
| Multi-wallet 2k | 3 | 0.130279 | 3.721 | 0.017178 | 3.721 |
| Multi-wallet 2k | 4 | 0.079946 | 3.721 | 0.016541 | 3.721 |
| Multi-wallet 2k | 5 | 0.078251 | 3.721 | 0.017897 | 3.721 |
| Multi-wallet 20k | 1 | 1.083081 | 3.242 | 0.208220 | 3.242 |
| Multi-wallet 20k | 2 | 1.654662 | 3.242 | 0.178706 | 3.242 |
| Multi-wallet 20k | 3 | 1.113029 | 3.222 | 0.179310 | 3.222 |
| Multi-wallet 20k | 4 | 1.555252 | 3.222 | 0.171230 | 3.222 |
| Multi-wallet 20k | 5 | 1.078828 | 3.222 | 0.170602 | 3.222 |

## Parity and mutation evidence

The full generated campaign checks all seven custody/fusion stages and then
captures Python's actual rendering operands and UUID tape. It compares the
entire canonical build state, or the exact refusal. A request-builder refusal
is counted separately from a render comparison. Per-family and per-scenario
activity, distinct active seed inputs and non-vacuity checks are retained in
`target/r4-render/full-generated-final/summary.json`.

| Corpus | Matched | Separate refusals |
| --- | ---: | --- |
| Twelve generators: 24,000 seeds plus 1,030 permutations | 24,705 / 24,705 render comparisons | 178 upstream refusals matched; 147 before rendering |
| One scenario cycle per generator | 508 / 508 | Before-render refusals recorded separately |
| Tax corpus | 145 / 145 | Includes 25 render refusals; 21 additional cases refused before rendering |
| Retained custody recordings plus permutations | 318 / 318 | 159 original requests; no pre-request refusal |
| Retained render replay plus permutations | 318 / 318 | No render refusal |
| Fresh raw-recording render replay plus permutations | 2,496 / 2,496 | 8 upstream refusals matched; 4 before rendering; 2,508 total inputs |
| Focused census after capture fixes | 65 / 65 | No render refusal |
| Focused custody-shadow after capture fixes | 65 / 65 | No render refusal |
| Frozen rendering corpus | 265 / 265 | Includes exact refusal witnesses |
| Exact msat conversion vectors | 256 / 256 | 80 original plus 176 machine/fallback boundary operands |

The final generated run exercised 4,907 entries, 125,117 quarantines and 328
ownership candidates. All seven custody/fusion stages matched on 24,852
non-refused inputs. The 45,441 diagnostic quantity differences are rendered
output, not oracle mismatches. The summary retains per-scenario counts as well.

| Generator | Seeds | Permutations | Render matches | Distinct active seed inputs | Entries | Ownership candidates |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Evidence | 2,000 | 58 | 1,987 | 1,379 | 1,207 | 0 |
| Components | 2,000 | 312 | 2,312 | 177 | 0 | 0 |
| Samourai | 2,000 | 68 | 2,068 | 1,588 | 60 | 0 |
| Recorded | 2,000 | 92 | 2,092 | 1,655 | 814 | 0 |
| Fanout | 2,000 | 94 | 1,916 | 1,494 | 0 | 220 |
| Graph | 2,000 | 16 | 2,016 | 2,000 | 0 | 0 |
| Pipeline | 2,000 | 24 | 2,024 | 1,675 | 0 | 0 |
| Safety | 2,000 | 78 | 2,078 | 767 | 160 | 108 |
| Channels | 2,000 | 64 | 2,064 | 1,563 | 194 | 0 |
| Native | 2,000 | 110 | 2,034 | 1,601 | 803 | 0 |
| Reconciliation | 2,000 | 50 | 2,050 | 96 | 1,476 | 0 |
| Gaps | 2,000 | 64 | 2,064 | 249 | 193 | 0 |

| Mutation set | Killed / catalog | Status |
| --- | ---: | --- |
| Original journal recipes | 39 / 39 | Fresh kills |
| New arithmetic, timestamps, row slots and balance joins | 17 / 17 | Fresh kills |
| New ownership selection and physical/legacy fallback indexes | 5 / 5 | Fresh kills |
| Inherited custody catalog | Not rerun / 2,285 | All anchors apply; applicability is not a kill |

All 61 journal recipes were killed in one fresh run on the final implementation
(`target/r4-render/mutations-complete-final.log`). All seven named baseline
tests passed; every mutated tree built and its named test failed. The checker
runs offline and removes each disposable build tree after writing the summary.
This is not an exhaustive branch-coverage claim.

Reproduce the campaigns after building the release `journal-render` example:

```sh
uv run --locked python scripts/check-journal-render-campaign.py \
  --output target/r4-render/full-generated-final
uv run --locked python scripts/check-journal-render-campaign.py \
  --recordings /home/tg/.cache/kassiber-verify/custody-recordings \
  --output target/r4-render/recorded-render
uv run --locked python crates/journals/scripts/mutation_check.py
```

Use the pinned Rust/uv toolchain and a disposable `TMPDIR` under `target/`.
No native journal entry point is registered by this checkpoint.

The capture hook also waits for the render oracle to finish importing before
installing child-process custody capture. Four subprocess regressions cover
both import orders in census and shadow, and an explicit capture followed by
an ordinary build. Without the import guard, xdist workers
could exit during setup with a partially initialized oracle module.
An active-capture context guard also prevents an explicit oracle capture from
being wrapped again by the automatic hook. The nested wrapper could miss UUIDs
consumed by the inner capture and emit an invalid outer tape.

The first full-lane capture directories were generated before the nesting fix.
Their verification stopped with `declared UUID tape exhausted` on the oracle
regeneration test's invalid outer capture. They are **not** counted as render
parity evidence. The focused 65-build census/shadow corpus is recaptured after
the fix. Regenerate the full-lane captures when rerunning the outstanding gates.

### Core-accounting fixture failure before rendering

The full census and custody-shadow core-accounting lanes each fail
`tests/test_cost_basis_pool_policy.py::CostBasisPoolPolicyTests::test_supported_scope_change_blocks_reports_until_reprocessed`.
That test patches Python's generic `POLICY_BUILDERS` to permit wallet-scoped
cost-basis pools. The Rust fusion probe uses the real generic policy and
refuses the scope, so the custody comparison sees a Python request versus a
Rust refusal before rendering. An isolated run with render capture disabled
reproduces the same failure. This checkpoint does not modify the policy,
the test or fusion comparison, and does not count either lane as passing.

```sh
KASSIBER_TEST_TAX_ENGINE=census KASSIBER_TEST_CUSTODY_ENGINE=census \
  uv run --locked python -m pytest \
  tests/test_cost_basis_pool_policy.py::CostBasisPoolPolicyTests::test_supported_scope_change_blocks_reports_until_reprocessed -q
```

The local failure logs are `target/r4-render/policy-fixture-census-repro.log`,
`census-core-accounting.log` and `shadow-core-accounting.log` in that directory.

### Tax-shadow harness assumptions about temporary paths

The reports-contracts census run passes 2,411 tests and skips four, but four
`SessionEndTest` assertions in `tests/test_tax_shadow_harness.py` fail:

- `test_a_swallowed_difference_still_fails_the_session`
- `test_a_test_that_stubs_the_engine_runs_uncompared`
- `test_an_ld5_refusal_is_counted_and_fails_the_session`
- `test_census_fails_an_engine_caller_outside_the_overlay`

Each nested test session reports the intended accounting/guard result. The
outer assertion requires the temporary probe module's printed path to start
with `/`. With this task's required `TMPDIR` under the worktree's `target/`,
the harness correctly identifies it relative to the repository instead:
`target/tmp/kassiber-tax-shadow-session-…/test_shadow_session_probe.py`.
These assertions are unchanged, and the lane is recorded as failing. The full
trace is `target/r4-render/census-reports-contracts.log`.

| Requested lane (`KASSIBER_TEST_TAX_ENGINE=census`) | Passed | Skipped | Failed |
| --- | ---: | ---: | ---: |
| Core-accounting census | 1,911 | 62 | 1 policy fixture |
| Core-accounting custody-shadow | 561 | 27 | 1 policy fixture |
| Reports-contracts census | 2,411 | 4 | 4 temporary-path assertions |
| Reports-contracts custody-shadow | 1,835 | 4 | 4 temporary-path assertions |

Fresh custody replay compares 1,252 recorded requests and their 1,252
permutations; two additional recordings differ at recorded/replayed Python
fusion because of the patched-policy fixture above. The full trace is
`target/r4-render/fresh-custody-final.log`. The separate fresh render replay
computes the ordinary reference from raw inputs and counts upstream and
pre-render refusals separately.

## Confirmed boundary gaps

The synthetic repros are retained in `target/r4-render/known-gaps/`; regenerate
them with [the boundary repro script](../../scripts/check-journal-render-boundary.py).
The script exits nonzero while any gap remains. These are **known failures**,
not parity successes or ignored tests. They predate this performance change.

| Operand | Python reference | Current Rust boundary |
| --- | --- | --- |
| Profile `workspace_id = '\ud800'`, with a captured normal engine response | Preserves the actual lone-surrogate scalar in canonical state | Probe exits 101 during serde JSON decoding (`unexpected end of hex escape`) |
| Plain quarantine detail `{"large":{"float":"1E999"}}` | `detail_json` is `{"large": Infinity}` | Writes `{"large": null}` |
| Direct payout `payout_amount = 'bad'` | `ValueError: invalid literal for int() with base 10: 'bad'` | Produces a state with a defaulted compatibility amount |

`serde_json::Value` and the engine response's Rust `String` fields cannot
represent an actual lone surrogate. Closing that gap needs a lossless scalar
boundary rather than a sentinel that could collide with book data. Legacy
nonfinite projections/details must preserve their Python scalar type and
serialization, including finite Decimal-to-float overflow. Transitional
integer, row, list, dictionary and response operands still need an exhaustive
Python refusal-order pass. Native production registration must stay disabled.

No Python reference bug was identified, and no Python product source changed.
The final tracked boundary-repro run exited 1 and reproduced all three
differences (`target/r4-render/known-gaps-final.log`).

## Remaining acceptance work

1. Close all lossless and malformed transitional boundary gaps above, with
   complete byte/refusal witnesses. This checkpoint only adds timestamp
   boundary witnesses; it does not certify arbitrary stage JSON.
2. Complete the branch-to-mutation audit and add missing killed recipes. The
   current ownership recipes cover pairable-reason selection, physical and
   legacy indexes, and exact-versus-compatible precedence, but do not establish
   complete coverage of component exclusions, ambiguous script owners, scope
   and transaction-id filtering, amount tolerance, or refusal propagation.
   Direct-payout missing/ambiguous source selection, integer coercion and
   refusal order also remain unaudited. Rerun the
   inherited catalog: this checkout has 2,285 recipes (the design's count of
   2,246 is historical). Applicability is not a kill claim.
3. Recheck the native typed custody/loader integration as the parallel R4
   stages land. This performance checkpoint retains today's adapter.
4. Run the complete repository quality gate before calling the branch
   push-ready. This task is local-only and does not authorize a push or PR.

## Workspace verification

`cargo fmt --all --check`, workspace clippy with `--all-targets -- -D warnings`,
and `cargo test --offline --workspace` pass. The workspace reports 1,301
passing tests across 105 harness results, with no ignored tests. Both focused
Python modes pass 108 tests and 12 subtests; the six render-oracle/capture
regressions also pass. Test data and temporary build roots are under `target/`.
Local test runs needed access to the mandatory account owner-lock namespace
and loopback fakes outside the sandbox; they did not use real books.

Before the checkpoint commit, `python -m tests.rust_port.classify_tests update`
regenerated the inventory and reference counts, and
`python scripts/python_test_shards.py run preflight --ci` passed all 57 tests.
The latter ran inside the sandbox. Logs are
`target/r4-render/classify-final.log` and `preflight-final.log`.
