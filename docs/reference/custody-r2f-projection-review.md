# R2f finalized projection independent-review corrections

Local branch `t3code/r2f-projection-fixes`, based on `1b62f405`. No Python
production accounting changes, dependency changes, fusion work, push or PR.

## Findings and witnesses

| Finding | Named witness or correction |
| --- | --- |
| Inbound component-blocked fee becomes taxable | `projection_inbound_fee_not_taxable` |
| Target-side blocking omitted | `projection_target_blocked` |
| Same-asset carrying-value payout admitted | `projection_same_asset_not_carry` |
| Nonzero-start payout slice priced using its end | `projection_nonzero_slice_pricing` |
| Selected move ids unsorted | `projection_moves_sorted` |
| Sweep privacy marker omitted | `projection_sweep_privacy` |
| Ignored interpreter issue narrows before filtering | `projection_adapter_input_families` now includes an ignored surrogate detail; Python independently confirms its irrelevance |
| Other uncatalogued selection and fail-closed operators | `projection_review_operator_sweep`, existing adapter witnesses; 29 additional recipes |
| Missing raw direct-payout population | `test_real_writer_payout_books`: 13 books made through the real writer, each independently permuted |
| Incidental target/other receipts inflate activity | Activity excludes those unconsumed receipt rows; `test_projection_companion_receipts_cannot_supply_activity` demonstrates rejection |
| Inert variants inflate distinct counts | Remove the marker; use 20 distinct amount factors, assert uniqueness without scenario labels |
| Empty results only printed | Assert exactly 48 evidence, 193 channel and 64 gap empty successes; zero in other suites |
| Refusals compared only by code | Rust returns `error.stage`; census, shadow, binding and replay compare it; wrong-stage same-code injections fail |
| Mapping insertion order differs | Document named-field contract and test actual Rust/Python rows; sequence order remains compared |
| Incorrect reference-bug claims | Correct the lossless-seam and complete-stage checkpoints to identify R1/R2/R3 |

The six review witnesses are copied from the trusted review artifacts and
checked against the live Python compiler. The nonzero-start pricing witness
assigns precisely its 40..100 slice to the payout (`out_amount=60`) and supplies
no residual external slice. It isolates slice arithmetic without freezing R1.

## Generator accounting

| Population | Primary books/inputs | Additional permutations |
| --- | ---: | ---: |
| Inherited twelve raw suites | 24,000 | 1,030 |
| New real-writer payout books | 13 | 13 |
| Private selected composition | 1,260 | 1,260 |
| Private quarantine composition | 1,040 | 1,040 |
| Retained read-only recording replay | 159 | 159 |

The new writer books cover taxable and carrying-value payouts, Austrian ETH,
missing value, tiny/huge values, reviewed conversions, partial allocations with
nonzero-start slices, and legacy NaN/infinity/junk/Unicode digits patched into
raw authored economic terms. Each book asserts its own reviewed payout; partial
books assert nonzero-start decisions and the conversion book asserts reviewed
conversion activity. The missing-value book asserts `at_swap_price_required`.
Expected outputs are computed live, including refusal stage and code.

R1 (partial payouts price every external source slice), R2 (partial carry
settlements duplicate ids) and R3 (writer float storage) are known Python
reference bugs, expected to change. Their generated outputs are not frozen as
correct accounting. The partial-carry refusal is included in parity and the
same-code/different-stage binding witness, pending the separate Python fix.

## Operator audit

The sweep inspected composition selection, source/target lookup, eligibility,
connected closure, pool barriers, privacy, fees, payout policy/asset checks,
receipt intervals, fallback fees and final ordering. The 29 added recipes
include blocked receipts, fee-only blocks/non-events/duplicate suppression,
decision and row sorting, basis/payout quarantines, consumed source fees,
dependency row presence, receipt fallthrough/barriers, issue scope and
interpreter filtering, carry policy and connected internal-state selection.
The inherited catalog already covers principal value/rate precedence, payout
amount/value/time admission, fee bundle consumption, conversion amount and
cardinality admission, diagnostic precedence/sets/ordering and exact adapter
round trips. Their recipes are rerun alongside the additions. An initial new-recipe pass
killed 33 of 35: its two survivors (non-event fallback and ignored-issue
admission) prompted stronger witnesses. Both now have building kills.
Interrupted source/target trials are excluded.

`transfer_fee_implausible` is covered at the projection adapter seam by the
existing `issue_fee` and `barrier_fee_reason` vectors, including source-only
scope and the fee-only barrier reason. It is not claimed as raw writer-book
activity. Current raw producers do not supply an independently authored issue
of that reason: it is emitted only by the generic pair guard in
`custody_interpreters.py`. Same-event native pairs bypass that guard; Lightning
pairs require equal principal; recorded fanouts conserve total principal;
channel pairs clone amounts; native transition kinds request the full available
source and consume their difference as an inferred fee. The channel producer
explicitly documents that the generic guard cannot fire on a cloned-amount
pair. Manufacturing a raw verdict to force that guard would break snapshot v1's
raw-fact boundary. This is a coverage limitation of the raw producers, not a
reason to weaken projection handling or change Python in this branch.

## Verification

Artifacts and per-recipe logs live under `target/r2f-review/`. Mutation trials
use isolated disposable source trees and bounded independent Cargo targets.
Each kill requires a passing unmutated test, successful mutant compilation,
nonzero selected named tests and failure of every named witness. Nonbuilds,
zero-test filters, interrupted runs and survivors do not count as kills.

| Check | Result |
| --- | --- |
| Raw generators, new writer books, wrapper parity and private quarantine lane | 23 passed; zero differences; exact 305 empty successes |
| Private composition/quarantine lane | 8 passed before the final raw rerun |
| Retained replay | 159 requests + 159 permutations; zero differences/pre-request refusals |
| Focused census | 72 passed; finalized 8 compared / 2 skipped; one explicitly injected build |
| Focused shadow | 72 passed; finalized 8 compared / 2 skipped; one explicitly injected build |
| Refusal/injection harness, including wrong-stage same-code probes | 58 passed |
| Rust workspace | 835 passed with disposable local socket binds permitted |
| All-target workspace clippy | Passed with `-D warnings` |
| Required preflight after final inventory update | 57 passed |
| Requested gap, interpretation, nonfinite-pricing and CLI-runner modules | 106 passed, 6 subtests passed |
| daemon-cli shard | 657 passed, 169 subtests passed |
| reports-contracts census | 2,384 passed, 2,820 subtests passed, 1 skipped; four temporary-path assertion failures described below |
| reports-contracts custody shadow | 1,815 passed, 146 subtests passed, 1 skipped; same four temporary-path assertion failures |
| Tax-shadow harness rerun with external temporary roots | 21 passed, 16 subtests passed in each census/shadow environment |
| Mutation catalog | 2,273 / 2,273 building kills; inherited 2,236 plus 37 additions; 402 named filters |
| Final affected mutation recheck | 270 / 270 building kills; additional refusal-identity recipe separately killed |

All commands use Rust 1.99.1, locked Python 3.11, offline Cargo and
`MATURIN_NO_INSTALL_RUST=1`. `HOME` is unchanged. The retained replay corpus is
read only. The first workspace attempt encountered four sandbox-denied local
socket binds; its failure is retained and the permitted complete rerun passed.

## Earlier-stage shard regressions

The fanout books in both earlier-stage suites have 178 finalization refusals
(including permutations) that Python and Rust share: nonfinite `fiat_value`
produces code `validation` at `finalized_projection`. The harness verifies both
identities, counts these refusals, then requests decisions without finalization
to compare interpretation/gaps. A Rust-only refusal or mismatched code/stage
still fails. A planted identity-bypass mutation is killed by the named test
`test_earlier_stages_account_matching_finalized_refusal`.

Seven migration wrapper vectors exposed a separate Rust-only finalization
refusal: an unknown surrogate blocker id was narrowed through serde even though
it matched no observed transaction. Rust now retains that diagnostic losslessly
and omits only irrelevant unrepresentable string ids from the composition
adapter. Matching ids still fail closed. All seven wrappers now compare their
full finalized output; the interpretation wrapper test still checks its own
stage. The catalog also kills removal of this surrogate admission fix.

The nonfinite-pricing CLI test now uses `tests.cli_runner.run`, preserving its
atomicity and error assertions and respecting `KASSIBER_TEST_CLI`.

Shared `/tmp` quota exhaustion required permitted local runs with disposable
`TMPDIR` directories under `target/r2f-review/`. No quota failure is counted as
a passing test or mutation kill. Consolidated recipe-identical evidence is in
`mutation-results-consolidated.json`; original and final recheck logs remain.

The two complete reports-contracts runs each failed four assertions in
`tests/test_tax_shadow_harness.py::SessionEndTest`: diagnostic text was expected
to contain an absolute external path, but the quota workaround placed child
tests inside the checkout and the diagnostic deliberately rendered relative
paths. All four failures have this cause; neither run reports a custody test
failure. Rerunning the entire tax-shadow harness module with disposable
`/var/tmp/kassiber-r2f-review-{census,shadow}` roots passed in both environments
(21 tests and 16 subtests each). The full shards were not repeated after that
environment correction; their original nonzero exits are retained, not reported
as clean shard passes. The requested four modules passed together (106 tests,
6 subtests), including both fanout generators and interpretation wrappers.
