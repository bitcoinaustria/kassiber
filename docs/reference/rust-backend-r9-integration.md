# Rust backend R9 integration checkpoint

**Status: stopped before integration acceptance.** The local branch
`t3code/rust-backend-r9` starts at `3d88f24faf8d60fe56f6c19014c0afc146375035`
(R8). Its only retained change is this checkpoint. No source cherry-picks
remain on R9. No push, PR, remote comment, stash operation or worker
delegation occurred.

## Replay and counts

Read the root and Python contributor instructions, plan 21, the R4 journal
pipeline and the [R8 integration audit](rust-backend-r8-integration.md).
Inventory used `git log --reverse 3d88f24fa..<tip>`, `git cherry HEAD <tip>`
and subject matching for every requested tip, including `19792ea1a` after
`a7da5b474`.

The inventory contains **191 candidate occurrences**, including repeated
ancestry: **64 provisional new subjects** and **127 subject matches requiring
deduplication review**. These are inventory counts, not completed pick/skip
counts. Direct record comparison confirms all four baseline rows from
`1f064db49` already occur on R8; that record must not be picked again.

Six loader commits were prepared in their original order and with their
original messages, before stopping on the seventh:

| Source | Prepared local commit | Subject |
| --- | --- | --- |
| `e8387a966` | `a70c2303a` | Share custody columns to meet the native loader CPU target |
| `05b0d32d4` | `1fa28bd17` | Correct native custody loader filtering and scalar refusals |
| `c60e0cb9f` | `c01d2ff54` | Preserve native custody loader refusal boundaries |
| `39de991a3` | `84be4fdb1` | Preserve custody loader values through projection checks |
| `efcbb7af7` | `e2147c91e` | Preserve downstream custody scalar refusals |
| `c5966c1ff` | `e39630611` | Match mixed revision refusals across Python hash seeds |

The entire batch was subsequently aborted to the requested R8 base. The six
prepared commits were **not compiled or tested** and are not a verified
intermediate implementation. They remain recoverable in the local bundle
`target/r9-integration/prepared-loader.bundle`. The paused working and staged
diffs, inventory, per-line logs/cherry output, prepared commit map and pause
metadata also remain under `target/r9-integration`. These are local scratch
receipts, not committed integration results.

## Conflicts and stop reason

The attempted loader batch encountered seven conflicted source candidates,
with 26 conflicted path occurrences across 11 distinct paths. Existing rerere
resolutions were inspected; automatic reuse was not treated as verification.

| Source | Conflicted paths |
| --- | --- |
| `e8387a966` | `TODO.md`; custody commitments, finalized runtime, gaps and snapshot; reports components; journal pipeline; loader checkpoint |
| `05b0d32d4` | journal pipeline; Rust-port counts; disposition reviews |
| `c60e0cb9f` | journal pipeline; Rust-port counts |
| `39de991a3` | loader tests; journal pipeline; Rust-port counts |
| `efcbb7af7` | `TODO.md`; custody snapshot; loader tests; journal pipeline; Rust-port counts |
| `c5966c1ff` | Rust-port counts |
| `e15900677` | `TODO.md`; journal pipeline; Rust-port counts; disposition reviews |

Earlier resolutions combined the shared-column representation with R8's
typed admission, and nonfinite-value reference normalization with R8's
independent exact scalar comparison. Generated counts were retained pending
final classifier regeneration. Compilation compatibility across these
boundaries remains unverified; the later journals-process line supplies
additional native-loaded snapshot composition that has not been integrated.

At `e15900677`, incoming status prose calls loader-9 accepted except its
BLOB corruption class and 16 seed-dependent messages. R8 carries additional
typed-row, kernel, renderer and persistence checkpoints absent from that
source branch. The proposed documentation reconciliation was rejected by
automatic approval review, which classified reconciling competing acceptance
claims as a design decision subject to the user's instruction to stop.
No rejected staging operation was retried or bypassed. The risk identified
by the review is overstating acceptance or replacing another stage's status.

The concrete proposed continuation is to retain R8's independent stage
status and oracle entries, carry loader-9's qualified acceptance as its
source-branch checkpoint, and explicitly distinguish those inherited receipts
from fresh R9 verification. Preserve this incoming qualification:

> Accepted except the BLOB corruption class (pending owner decision) and
> 16 seed-dependent messages (pending Python fix).

This proposal does not establish combined R9 acceptance. The requested BLOB
line follows loader-9 and must be reconciled with its mutation map before
reporting the final exception state. Approval is needed to continue the
documentation reconciliation rejected by automatic review.

## Verification performed here

`mise exec rust@1.99.1 uv@latest -- cargo fmt --all --check` passes on the
restored R8 source boundary. Toolchain reports Rust 1.99.1. Scratch is under
this worktree's `target`, `HOME` is unchanged, and no dependencies were added.
`git diff --check` passes for this checkpoint. The R8 audit's prior test and
performance results remain inherited evidence; they were not rerun here.

No test job, daemon or other background process remains from this attempt.
No native build or mutation tree was created. The existing stash is untouched.

## Exactly what remains

1. Approve the blocked documentation reconciliation, then restart the ordered
   loader replay from R8 (or inspect the preserved, unverified prepared commits).
   Finish loader-9 and loader-BLOB, preserving both branch lines and their map.
2. Pick automatic pairing, pre-build, rendering, persistence and journals
   process in the requested order. Review later stage re-picks for their
   resolutions instead of skipping solely by subject.
3. Pick custody performance and its record, conversions, sync-14 through
   sync-22, importer-5 through importer-9 and importer-B, all requested report
   and source-of-funds lines, then transfer-10 through transfer-13. Finish
   patch/subject/content deduplication and the original-to-integrated replay map.
4. Run offline workspace fmt, clippy `-D warnings` and tests with at most eight
   build jobs. Set up locked Python 3.11 with `MATURIN_NO_INSTALL_RUST=1`.
5. Run 24,000 custody books, 1,030 permutations, 159 retained replays and their
   permutations, plus focused census and shadow. The retained recording path
   has not been exercised here. Full census/shadow shards remain the owner's
   follow-up as requested.
6. Run every slice's parity suite through the hybrid candidate and establish
   zero fallbacks for registered shapes. No parity or platform/sandbox
   availability conclusion has been established by this attempt.
7. Run the complete mutation catalog in resumable serial shards with incremental
   logs; write each summary before deleting its build tree. Keep task build
   storage below about 12 GB.
8. Measure Python versus native test-only `journals process` and custody CPU on
   gen2k, mw2k, gen20k and mw20k, retaining load averages. No R9 performance
   measurement is available.
9. Run `classify_tests update` and preflight, finish a manual review and update
   this reference with actual integrated counts, conflicts, verification and
   performance. Leave a clean local commit and no background jobs.

Git metadata is outside the writable worktree, so branch/index operations
require sandbox elevation. No verification command beyond formatting was
attempted; the outstanding checks are unrun, not reported as sandbox failures.
