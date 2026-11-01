# R4 quantity kernel performance

This checkpoint keeps the quantity stage's serde input and Python-model output
unchanged. Typed slices, domains, postings, borrowed native events, shared alias
indexes and direct Python-model construction are private to the stage.

## Measurement

The baseline is dd433e4e plus the requested R4-1 and R4-6 records (d2d3a66eb).
Both release binaries use Rust 1.99.1, the locked Python 3.11 environment and
unchanged Cargo.lock. The four R4-1 cached synthetic snapshots are read locally.
For each book, five pairs alternate the saved baseline and candidate binary.
The primary figure is process CPU time (user + system across all threads).
Temporary timing-feature instrumentation reads Linux CLOCK_PROCESS_CPUTIME_ID
at the same span boundaries as Instant. Whole-process CPU is independently
recorded with getrusage(RUSAGE_CHILDREN). Instrumentation is restored before
commit; it is absent from product code and the binding. The raw records retain
all runs, wall spans, CPU spans, process totals, load and parity digests.

Select each engine's best quantity CPU run and report its wall spans alongside
it, rather than mixing independent subspan minima. The total includes the
entire existing Python-model output boundary. Subspans do not sum to the total:
setup, intermediate destruction and residual operations sit between subspans.
The new quantity.issues span includes projection output construction;
quantity.py includes canonical/output-part construction. Neither is hidden
outside quantity.

The forty retained measurements contain five alternating baseline/candidate
pairs for each of the four books. The 1-minute load ranged from
18.97 to 35.19. Every paired custody-response, ledger-request
and ledger-response digest agrees. The raw
[timing records](r4-quantity-kernel-timings.jsonl) include all stage spans,
binary digests, process totals and load observations. The
[audit](r4-quantity-kernel-audit.json) binds the results to the four source
files' SHA-256 digests and identifies the selected pair for each engine.

The quarter-time **CPU** target is met for all four books. Wall time improves
by 2.5–2.9×; it does not meet the earlier quarter-time wall budget. The
whole-process figures below include the parity oracle and destruction, and
are context from the selected quantity runs, not independently selected
end-to-end minima.

## Results

| Book | Before CPU | After CPU | CPU gain | Before wall | After wall | Wall gain |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| generic-2000 | 0.280342 | 0.058944 | 4.76× | 0.171829 | 0.059454 | 2.89× |
| multi-wallet-2000 | 0.310435 | 0.071481 | 4.34× | 0.182324 | 0.071976 | 2.53× |
| generic-20000 | 3.412621 | 0.733333 | 4.65× | 2.004652 | 0.739432 | 2.71× |
| multi-wallet-20000 | 3.441047 | 0.778705 | 4.42× | 2.053107 | 0.786632 | 2.61× |

### generic-2000

| Span, seconds | Before CPU | Before wall | After CPU | After wall |
| --- | ---: | ---: | ---: | ---: |
| quantity.components | 0.007631 | 0.007682 | 0.001106 | 0.001126 |
| quantity.holds | 0.000004 | 0.000004 | 0.000003 | 0.000003 |
| quantity.native | 0.024537 | 0.024754 | 0.001746 | 0.001754 |
| quantity.project | 0.058723 | 0.059372 | 0.012875 | 0.012968 |
| quantity.issues | 0.018977 | 0.019105 | 0.018560 | 0.018697 |
| quantity.py | 0.138416 | 0.028513 | 0.017861 | 0.018072 |
| quantity | 0.280342 | 0.171829 | 0.058944 | 0.059454 |
| Whole process, including oracle | 1.775564 | 0.965807 | 1.573933 | 0.860853 |

Selected pairs: baseline 2, candidate 2. Full-pipeline compute wall: 0.828852 → 0.731632 s.

### multi-wallet-2000

| Span, seconds | Before CPU | Before wall | After CPU | After wall |
| --- | ---: | ---: | ---: | ---: |
| quantity.components | 0.023824 | 0.023989 | 0.019614 | 0.019747 |
| quantity.holds | 0.000017 | 0.000018 | 0.000013 | 0.000013 |
| quantity.native | 0.015798 | 0.015901 | 0.001701 | 0.001718 |
| quantity.project | 0.057774 | 0.058357 | 0.012596 | 0.012686 |
| quantity.issues | 0.018689 | 0.018839 | 0.013068 | 0.013157 |
| quantity.py | 0.156498 | 0.027088 | 0.010893 | 0.010964 |
| quantity | 0.310435 | 0.182324 | 0.071481 | 0.071976 |
| Whole process, including oracle | 2.472370 | 1.416148 | 2.194144 | 1.291422 |

Selected pairs: baseline 1, candidate 0. Full-pipeline compute wall: 1.260688 → 1.118842 s.

### generic-20000

| Span, seconds | Before CPU | Before wall | After CPU | After wall |
| --- | ---: | ---: | ---: | ---: |
| quantity.components | 0.173785 | 0.175159 | 0.026868 | 0.027068 |
| quantity.holds | 0.000016 | 0.000018 | 0.000003 | 0.000004 |
| quantity.native | 0.328983 | 0.331634 | 0.021427 | 0.021587 |
| quantity.project | 0.657763 | 0.662361 | 0.201641 | 0.203201 |
| quantity.issues | 0.274672 | 0.276766 | 0.219715 | 0.221531 |
| quantity.py | 1.649227 | 0.227336 | 0.173553 | 0.174985 |
| quantity | 3.412621 | 2.004652 | 0.733333 | 0.739432 |
| Whole process, including oracle | 18.144838 | 8.995027 | 14.737077 | 7.710253 |

Selected pairs: baseline 1, candidate 0. Full-pipeline compute wall: 7.659135 → 6.562159 s.

### multi-wallet-20000

| Span, seconds | Before CPU | Before wall | After CPU | After wall |
| --- | ---: | ---: | ---: | ---: |
| quantity.components | 0.254238 | 0.256819 | 0.159247 | 0.160467 |
| quantity.holds | 0.000075 | 0.000077 | 0.000028 | 0.000028 |
| quantity.native | 0.253074 | 0.255406 | 0.018138 | 0.018253 |
| quantity.project | 0.686016 | 0.693356 | 0.199631 | 0.201051 |
| quantity.issues | 0.233744 | 0.236704 | 0.178817 | 0.182242 |
| quantity.py | 1.644136 | 0.234188 | 0.114888 | 0.115760 |
| quantity | 3.441047 | 2.053107 | 0.778705 | 0.786632 |
| Whole process, including oracle | 22.975431 | 12.520708 | 20.925743 | 12.148213 |

Selected pairs: baseline 3, candidate 3. Full-pipeline compute wall: 11.290092 → 10.548939 s.

## What made the difference

- Directly construct the existing Python-model output. Typed slices, postings
  and domains write their fields directly; the builder never creates a
  complete intermediate serde state. Owned strings and keys move into the
  output. Borrowed values use exact integer fast paths, and conversion
  vectors reserve known lengths. This removes the generic conversion and
  serialize/reparse work, while keeping all output allocation inside quantity.
- Borrow unchanged native events and copy only augmented events. With no
  audits, validated strictly increasing unique events bypass the event-map
  rebuild. Reversed and duplicate input keeps the sorted, last-key-wins path.
- Share alias lookup across reviewed and native compilation. Validate unused
  aliases locally when possible, with a bounded eight-leg probe and global
  fallback. Cross-event aliases and missing aliases retain their behavior.
  Component commitments intersect the relevant hashes with the snapshot set
  instead of scanning every snapshot per component.
- Keep exact BigInt postings and domains typed through conservation. Compare
  semantic fields directly; cache allocating sort keys and compare borrowed
  decision and posting keys. Unmatched decisions remain typed until output.
- Avoid unused work: defer row and issue-observation indexes, omit lazy
  unmatched decisions from issue-only views, and construct issue-free tax
  eligibility directly. Issueful eligibility still sees every decision and
  retains the complete scoped barrier kernel. Move owned reviewed, hold and
  native claims into arbitration instead of cloning them.

Input and output representations, refusal codes, duplicate precedence and
all ordering contracts remain unchanged. ASCII hash and bounded integer
shortcuts retain the original Unicode/Python and arbitrary-precision fallback.
No neighboring stage interface, dependency or Cargo.lock entry changes.

### Rejected final index experiment

A final attempt replaced arbitration's ordered observation map with a hash
map plus one output sort. Five alternating pairs per probe book did not
reduce CPU on both books, so it was restored before verification and commit.
These are isolated best quantity CPU values, not the retained benchmark table:

| Book | Retained ordered map | Rejected hash map |
| --- | ---: | ---: |
| multi-wallet-2000 | 0.073201 | 0.072267 |
| generic-20000 | 0.742941 | 0.766094 |

[Probe records](r4-quantity-kernel-index-probe.jsonl) retain the rejected binary
digest. Its observation-order mutation was retained and re-anchored to reverse
the equivalent ordered-map output; no catalog entry was removed.

## Byte parity and activity

The baseline and retained candidate agree byte for byte over **25,348 requests**:
24,000 primary books from all twelve generators, 1,030 unordered-read
permutations, and 318 retained replay requests. The [corpus driver](r4-quantity-kernel-parity.rs) hashes the
serialized quantity output independently of full-pipeline success, as well as
the decision-only custody response, full custody response, ledger request and
ledger response. All 178 generated full-pipeline refusals agree exactly;
quantity output is still compared for those requests. Every request in this
corpus produces a quantity result. Frozen vectors and separate refusal tests
cover rejected quantity inputs and the first refusing stage.

Activity is measured from each primary input's own quantity decisions and
issues. Permutation output cannot supply activity. Distinct inputs are counted
within each suite, with every primary seed retaining its digest, decision count,
issue count, activity, distinct-input flag and active-distinct-input flag in the
[compressed activity record](r4-quantity-kernel-activity.jsonl.gz). The audit
retains scenario totals and the declared inactive controls.

| Suite | Primary books | Distinct inputs | Active distinct inputs |
| --- | ---: | ---: | ---: |
| evidence | 2,000 | 2,000 | 1,333 |
| components | 2,000 | 2,000 | 2,000 |
| samourai | 2,000 | 2,000 | 2,000 |
| recorded | 2,000 | 2,000 | 2,000 |
| fanout | 2,000 | 2,000 | 1,958 |
| graph | 2,000 | 2,000 | 2,000 |
| pipeline | 2,000 | 1,841 | 1,841 |
| safety | 2,000 | 2,000 | 1,949 |
| channels | 2,000 | 2,000 | 1,875 |
| native | 2,000 | 2,000 | 2,000 |
| reconciliation | 2,000 | 100 | 100 |
| gaps | 2,000 | 2,000 | 1,938 |

The required scenarios each have positive activity on distinct inputs;
declared zero-activity controls stay zero. A deliberately vacuous scenario exits with status 1 from the existing
activity checker, and `test_vacuous_quantity_scenario_fails_activity_check` passes.
The whole campaign and focused frozen-vector/refusal checks are repeated after
the final commit, with unchanged source digests. No raw retained recording is
committed.

## Verification

All checks run serially with Rust 1.99.1, locked Python 3.11, offline cargo,
`MATURIN_NO_INSTALL_RUST=1` and disposable data roots. The retained binding is
rebuilt with `uv sync --locked --offline --python 3.11`. Both census and custody
shadow use `KASSIBER_TEST_TAX_ENGINE=census`.

| Check | Result |
| --- | --- |
| `quantity-python-contracts` | 106 passed in 345.85s (0:05:45) |
| `census-core-accounting` | 2033 passed, 75 skipped, 460 subtests passed in 668.50s (0:11:08) |
| `tax-shadow-scratch-path` | 4 passed in 5.64s |
| `cargo-private-build-dir` | Pass |
| `quantity-port-private-build-dir` | 16 passed in 21.46s |
| `census-reports-contracts` | 2475 passed, 10 skipped, 2824 subtests passed in 749.05s (0:12:29) |
| `shadow-core-accounting` | 681 passed, 40 skipped, 231 subtests passed in 109.04s (0:01:49) |
| `shadow-reports-contracts` | 1875 passed, 10 skipped, 146 subtests passed in 689.63s (0:11:29) |
| `mutation-applicability` | Pass |
| `fmt` | Pass |
| `clippy` | Pass |
| `workspace-tests` | 1495 passed, 0 failed, 0 ignored |
| `classify-tests` | Pass |
| `preflight` | 57 passed, 60 subtests passed in 55.88s |

The gate includes all quantity Python/private compiler tests, twelve raw
quantity generator suites, the shadow-harness regression, frozen full-stage
wrappers and refusals, and frozen projection/binding refusal regressions.
Rust workspace tests include the complete frozen quantity vectors and the new
native-event-order and unused-alias-validation regressions. Formatting and
workspace/all-target clippy use `-D warnings`. Inventory regeneration and the
preflight shard run before the final commit.

The shared sandbox blocks local socket tests and writes to implicit default
paths. Verification therefore uses a private bwrap mount namespace with
workspace-backed disposable `/tmp`, XDG runtime, Kassiber runtime and default
data directories. Path-sensitive tax-harness self-tests use a private XDG
runtime scratch alias outside the checkout, still backed by workspace storage.
Their four initial path-assertion failures pass after that environment correction;
the complete affected shard is rerun. Cargo target and intermediate directories
are pinned inside this worktree because the machine's global intermediate-cache
setting points outside the writable test namespace. The mutation build uses its
own directory for both. HOME is unchanged; ordinary test network guards remain in
force. No product permission or network behavior changes.

The split-payout regression intentionally changes Python decisions after the
raw snapshot boundary. Commit `c017987c9` declares that injected refusal with
the existing stage-and-exception-identity helper; it does not suppress ordinary
census/shadow comparisons. Saved baseline binaries proved raw-input parity
before that correction.

## Mutation catalog

The global catalog grows from **2,292 to 2,299**, with no removed IDs. All
**363 recipes** in the four touched Rust files build and are killed by their
named tests; the kill set equals the selected catalog exactly. Global catalog
applicability also passes. This is the requested touched-file campaign;
recipes in unchanged files have no fresh kill claim. The audit records every
kill, baseline filter count, source digest and old/new recipe. Only one mutation
build tree exists at a time; each is deleted after writing its summary.

The initial campaign found that `quantity-arbitration-atomic-hold` still wrote
the old decision value field in its replacement. The equivalent reason-only
write through `value_mut()` builds and is killed in a serial follow-up. All
successful recipes remain identical; production source stays unchanged across
both runs. The audit preserves the initial compile error, corrected recipe,
follow-up result and aggregate kill set.

All 32 inherited recipes that moved retain equivalent mutants:

| IDs | Equivalent mutation |
| --- | --- |
| `quantity-admission-asset-exposure` | Exclude LBTC from Bitcoin exposure in the typed domain constructor. |
| `quantity-arbitration-semantic-state`, `quantity-arbitration-semantic-destination_kind`, `quantity-arbitration-semantic-external_economic_subtype`, `quantity-arbitration-semantic-transfer_kind`, `quantity-arbitration-semantic-transfer_policy`, `quantity-arbitration-semantic-component_id` | Omit the same field from authoritative semantic equality. |
| `quantity-arbitration-semantic-target` | Ignore translated target-slice equality. |
| `quantity-arbitration-authority-agreement` | Disable the incompatible-authority check. |
| `quantity-arbitration-atomic-hold` | Replace the incomplete-bundle conflict with the same reason-only write through the new decision accessor. |
| `quantity-arbitration-decision-order` | Reverse source-slice decision ordering. |
| `quantity-arbitration-posting-order` | Reverse posting-id ordering. |
| `QN-001` | Disable truth-based native text conversion after the imports change. |
| `QN-133`, `QN-134`, `QN-135`, `QN-136`, `QN-137` | Rename the same native output keys: canonical_input, events, rejected_events, claims and issues. |
| `QX-019`, `QX-020`, `QX-021` | Reverse the same evidence-ordered native groups after cached-key sorting. |
| `QRT-071` | Omit gap-hold claims from arbitration. |
| `QRT-072` | Omit native claims from arbitration. |
| `QRT-074` | Reverse whole-issue ordering. |
| `QRT-102`, `QRT-103`, `QRT-104`, `QRT-105`, `QRT-106`, `QRT-107`, `QRT-108`, `QRT-109` | Null the same state output fields: canonical_input, projection, issues, tax_eligibility, gap_candidate_transaction_ids, gap_holds, reviewed_conversion_pairs and reviewed_direct_payouts. |

Seven added recipes pin the new branches and direct-output contracts:
`quantity-perf-issue-barrier`, `quantity-perf-ascii-hash`,
`quantity-perf-native-event-order`, `quantity-perf-default-decision-state`,
`quantity-perf-default-decision-barrier`,
`quantity-perf-unused-alias-validation`, and
`quantity-perf-observation-order`.

## Remaining hot spots

The owned output representation still requires allocation and copies for
canonical events, observations, decisions, eligibility and postings. Direct
output construction removes the intermediate serde tree and generic conversion;
it does not make materialization free. Canonical/model construction and
arbitration dominate generic quantity. Reviewed compilation is an additional
multi-wallet cost. Issueful books still use the complete scoped-barrier path.

The CPU target is met at this checkpoint; the quarter-time wall target and
whole-custody performance at 10× Python remain open. Neighboring stages and
whole-pipeline representation changes belong to their separate R4 work.
