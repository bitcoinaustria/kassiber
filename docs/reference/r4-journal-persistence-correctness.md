# R4-12 persistence correctness continuation

This is a local correctness checkpoint from `a63556cc7` on
`t3code/r4-12-persist-accept`. **Acceptance remains open.** The performance
worker owns CPU measurements. The exhaustive branch audit and complete
census/shadow and custody mutation campaigns must also close; this record does
not describe a state with only CPU acceptance pending.

The comparison, preparation and write implementations remain unchanged here.
The separate `9899124af` fix corrects the unregistered probe's response handling:
once COMMIT succeeds, failed response flushing or connection maintenance cannot
append a second, uncommitted failure response. Production CLI and daemon journal
processing remain unwired to this service.

The accompanying [evidence](r4-journal-persistence-correctness.json) contains
source/catalog hashes and aggregate receipts. Private recording payloads are
not included. Disposable verification and mutation build trees are removed
after their results are summarized. Commands run serially within this worktree,
with eight Cargo build jobs and the owner's pinned, offline toolchain.

## Physical caller witnesses

All of these witnesses run on actual SQLite WAL/NORMAL and SQLCipher
DELETE/FULL files. They establish the service and explicit caller contracts;
they do not claim that the future production R4-13 routes are registered.

| Contract | Named witness in `crates/journals/tests/persistence.rs`, unless noted |
| --- | --- |
| Write reservation before accounting snapshots and pre-build repairs | `physical_callers_begin_before_repairs_and_publish_only_committed_dumps` obtains `BEGIN IMMEDIATE`, proves a competing writer is blocked, then reads the snapshot and writes a repair. |
| Whole pre-COMMIT rollback | `physical_driver_refusals_roll_back_partial_writes_and_preserve_caller_repairs` distinguishes the nested writer rollback from the caller's complete rollback; `physical_commit_failure_remains_uncommitted_and_rolls_back_the_whole_caller` injects a deferred foreign-key COMMIT refusal. |
| Settings restoration | `physical_setting_driver_errors_propagate_and_retry_cache_restoration`, signed cache scopes and the physical success/drop tests preserve cache, spill, checkpoint threshold, journal mode and synchronous settings. |
| Committed visibility | `physical_callers_begin_before_repairs_and_publish_only_committed_dumps` compares complete typed dumps from another connection before and after COMMIT and after reopen. |
| Success flushed before checkpoint | `physical_success_is_visible_and_flushed_before_failing_post_commit_maintenance` in the probe tests observes committed rows while writing the response and requires its flush before checkpoint maintenance. |
| Dropped handles and unwinds | `physical_caller_drop_and_unwind_restore_repairs_journals_and_settings` rolls back repairs and journal writes when the transaction handle drops normally or unwinds. |
| Post-COMMIT errors | The three probe tests pin both native/floor commit paths, failed success flush, no second response, refused setting restoration and refused checkpoint maintenance. |

Returned errors and panics are injected after every successful DML statement in
the frozen corpus. Existing crash tests also kill processes at DML and
pre/post-COMMIT boundaries, reopen both physical drivers, and compare complete
typed dumps, integrity and foreign keys.

`physical_automatic_outer_rollback_keeps_original_refusal_and_stale_dump`
adds SQLite's `RAISE(ROLLBACK)` case. The database has already removed all
savepoints when the writer's cleanup runs; that best-effort cleanup must
preserve the original driver error and stale database.

Restoration errors require a precise distinction. Explicit cache restoration
returns its error and its destructor retries. A driver that also refuses the
destructor's PRAGMA can leave that connection's temporary setting in place.
`physical_drop_restoration_errors_remain_best_effort_after_commit` pins this
case without promising impossible restoration: the destructor does not panic,
durable rows remain visible, and the connection can be discarded. No setting
failure turns an already committed operation into an uncommitted response.

The existing locked rusqlite authorizer is enabled through the store's
development-only `test-hooks` feature for deterministic PRAGMA refusals. No
package or lockfile version changes are involved. The tests use its safe API;
the journal crate retains `unsafe_code = "forbid"`.

## Branch/refusal audit boundary

The catalog names each mutation and its killing test. The following additions
extend the earlier binding, delta, source dictionary, comparison cursor,
savepoint and physical-driver witnesses:

| Area | Decisions pinned |
| --- | --- |
| Decimal and SQLite scalars | NULL/TEXT/INTEGER/REAL/boolean binding; overflow and structured-value refusals; all amount scaling/rounding/sign/fallback cases; short/long fixed decimal parsing; signed zero; coefficient widths and padding limits. |
| Retained entries | Every SQL scalar type at all 29 columns; optional REAL/exact-text pairs; exact decimal bytes and all five REAL zero signs; unchanged rows prepare no bindings; repeated preparation returns before a second `OnceCell` assignment. |
| Compact calculations | Exact key sets, nullable text, integer/exact-text pairing, acquisition option pairing, invalid fragment containers, source/pricing fallback, raw integer negative-zero normalization, dictionary identity and byte-exact escaping. |
| Quantity helpers | Python key conversion, nullable references, canonical hash, all Liquid network aliases, both Bitcoin exposure assets, other exposure names, wide slice subtraction and exact endpoint refusals, all four pool-scope fields and first matching reason. |
| Quantity storage | Both accepted internal decision states, rejected states, missing target, basis eligibility, cancellation of zero balances, all five explicit posting/balance/observation refusals, missing relation source and reconciliation quantity/target/claim/component/expected-amount guards. |
| Filed summaries | SQL NULL/INTEGER/REAL/TEXT, invalid UTF-8/BLOB refusal, invalid years, malformed/nonobject JSON, missing snapshot and independent classification/gain changes. |
| Quarantines | Python falsey/list/dict/string coercion, noniterable scalar refusals, pop/parse/sort behavior, all six downstream reasons, internal-only Decimal and non-string-key guards. |
| Driver/maintenance errors | Writer clear/insert prepare/execute/observer propagation; savepoint creation/release; cache read/write/restore; checkpoint mode/read/write/PASSIVE; automatic outer rollback and best-effort destructor failures. |

Reachability and equivalence findings retained for the final per-decision map:

- JSON binding cannot create a BLOB, and borrowed TEXT comes from a valid Rust
  string. BLOB dispatch and failed UTF-8 conversion there are unreachable.
- The amount fast path multiplies at most `u64::MAX` by `10^19`, below
  `u128::MAX`. Its signed conversion occurs before negation; `i64::MIN` and
  wider coefficients use the exact fallback.
- Finite Decimal formatting produces valid Rust float syntax, including
  underflow and overflow. The float parse expectations follow successful
  formatting. `fixed_equal` accepts only matching ASCII coefficient/sign/scale
  bytes before its own UTF-8/float expectations.
- Changed entry bindings are read after preparation. The initial `OnceCell`
  check prevents its second assignment. Typed calculation serialization and
  formatting into `String` cannot encounter an unsupported serializer type or
  an I/O error.
- Eight valid fragments fit the direct comparator's sixteen source slots.
  A small input rejected by direct validation also fails equivalent compact
  validation; the small compact dictionary path has direct helper witnesses.
- Source enumeration and lookup traverse the same immutable calculation, so
  the second pass cannot lose an enumerated source. Ordinary JSON normalizes
  raw integer negative zero; helper tests explicitly construct that spelling.
- Eligible-set state/target filters are redundant with projection validation:
  membership compares the entire immutable decision. Removing those filters
  cannot admit a projection decision rejected by its own guard.
- The economic-relation observation index is unused when both relation lists
  are empty. Dictionary strategy thresholds and the provenance pointer-equality
  false path can choose equivalent algorithms. These need equivalence notes,
  not a claim that every performance-only threshold has a functional killer.
- JSON quarantine decoding produces string dictionary keys and no Decimal
  values. Direct helper witnesses construct those internal-only cases.
- Fixed-column `get_ref` index failures are unreachable after the selected
  fixed-width SQL statement has prepared successfully; driver stepping and
  scalar conversion failures remain distinct reachable failures.

This is still **not an exhaustive per-conditional mapping**. The remaining map
must resolve delta scan/order/fallback decisions and each driver propagation
site, the main writer's defaults/readiness/serialization decisions, and filed
service delegation. Catalog size and broad complete-dump witnesses alone do
not close that work.

The exact kill union is **410/410** recipes: 285 from the earlier verified
runs, 123 from the new helper/maintenance shard and two from its final retry.
All original 201 recipes remain unchanged. One new profile-scope mutation
initially failed to compile; that attempt is excluded from kills, its invalid
replacement was corrected, and its retry compiles and is killed. There are no
unresolved survivors or build errors. The final package run passes 91 journal
tests, and offline all-target journal/daemon clippy passes with `-D warnings`.

## Campaign evidence and limits

The twelve generated suite shards pass **24,000 books and 1,030 permutations**.
There are 24,852 successful custody comparisons and 178 matching upstream
refusals. Rendering compares 24,705 pairs exactly; 147 custody successes refuse
before rendering. No refusal is counted as a successful render comparison.
These are custody/render campaigns, not 25,030 whole-pipeline persistence dumps.

All **159 retained requests and 159 permutations** pass custody and rendering
replay, in four shards of 73, 6, 40 and 40 requests. Fresh census recordings add
68 requests and 68 permutations; fresh shadow recordings add 66 and 66. The
fresh capture runs pass 29 census tests and 27 shadow tests. Both variants of
the read-budget test are excluded from fresh shadow capture; the recovered-route
failure is reproduced separately.

The known Python-only shadow read-budget reproduction is retained: 64 reviewed
routes make **5,254 observation reads against a 3,840 budget**, including without
the persistence capture plugin. Its loader still matches exact v1 bytes. It is
not relabeled as a persistence mismatch or silently treated as a passing test.

The custody mutation campaign completes two shards: all 21 Python recipes and
the first 200 Rust recipes are killed, with all 171 Rust baseline test filters
passing first. This is **221/2,327**, leaving **2,106 recipes**. The next shard
was interrupted before completing its baseline; it contributes no kills or
passing shard receipt. Its process group exited, and the mutation build trees
were removed. Completed receipts bind the named kill sets and logs. Resume
only after checking the aggregate code/fixture input digest as well as the
individual receipts; changed inputs require fresh evidence.

The full census attempt has passing preflight, CLI smoke, wallets/sync,
daemon/CLI, security/replication and serial-network shard receipts. The core
attempt found long AF_UNIX scratch paths plus daemon-ready order and stale XLSX
refusal expectations. A shorter `target/t` root resolves the socket-path cases.
The reports shard found fourteen harness-isolation failures: two deliberately
mocked refusal tests compared against a baseline captured before their patch
when census was already installed. The test-only correction passes all 80
harness tests under census and all 80 under shadow, preserving the separate
patched-function skip guards. The full reports shard still needs its rerun.

The separate `cb0cd4d1f` correction restores Python's daemon-ready capability
order for transaction XLSX, Austrian exports and transfers, preserving the
same supported members. The transaction-listing refusal test now leaves XLSX
to its existing implementation witnesses. Its offline native build, 26 daemon
tests and all-target clippy pass. Exact setup sessions, transaction listing and
CLI/daemon XLSX route checks pass 17 tests and 57 subtests; 129 service-probe
cases are explicitly skipped because their separate probe is not enabled.
One sandbox-only rerun exposed the Python reference's fixed per-user owner-lock
directory chmod refusal. The guarded disposable-book run passes with runtime
access. The native verification build tree is removed after its receipt.

The long serial-daemon attempt showed five failures and was interrupted. It
has no completed passing receipt and contributes no passing shard count.
The remaining census shards and the full shadow overlay remain uncompleted.

Before this record's commit, the required classifier update adds the storage
oracle's SQL flag and all 57 preflight checks pass. A manual second review
confirms the unchanged implementation bodies, original recipe preservation,
exact completed kill sets and native verification source hashes. Verification
processes have exited; mutation/native build trees and disposable pytest roots
are removed. Local probe binaries and aggregate receipts remain for resumption.

## Remaining acceptance work

1. Finish the per-decision branch/refusal map and kill any additional recipes.
2. Complete the full census and shadow campaigns, rerunning affected shards
   after the harness/daemon fixes and retaining the known read-budget exception.
   Census needs the core/accounting and reports/contracts reruns plus complete
   serial daemon, regression and integration lanes. The full shadow overlay
   remains unrun.
3. Kill the remaining 2,106 custody recipes. This checkout contains 2,327
   recipes (2,306 Rust and 21 Python), a superset of the requested historical
   2,285; only the completed 221-recipe prefix is claimed here.
4. Integrate the performance worker, rerun mutations touching merged code, and
   close its CPU matrix with exact dumps before declaring R4-12 accepted.
