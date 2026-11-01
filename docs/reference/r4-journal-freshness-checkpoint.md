# R4-15 journal freshness integration

The native daemon composes its existing on-chain freshness handler with the
registered native journal service on the worker-owned connection. The three
desktop sync/freshness routes admit their default journal shapes. Enabled
unported sources still refuse the complete invocation before consent.
Python retains production authority until R4-16; this integration does not
change the packaged sidecar or authorize that switch.

The plan holds the single foreground execution slot through recovery, queueing,
prefetch, wallet publication and local journal refresh. Competing foreground
requests return retryable `project_operation_in_progress`. Cancellation,
revocation and lifecycle retirement use the existing request worker and join
its connection owner before switching books, locking or shutdown. Journal
processing is local and adds no egress. All wallet/graph transport stays in
`kassiber-net`; saved queue entries grant no network authority.

`ui.wallets.sync` retains its separate priority-80 post-wallet journal job with
automatic pairing and `skip_rebuild_when_current`. It appends the job rows,
refreshes the final snapshot and returns the nonfatal journal summary. Active
freshness captures explicit pairing authority; workspace freshness retains its
existing ordering and no-pairing contract. Coalescing retains pairing intent.
Current, disabled, deferred, failed and cancelled summary projections share
the Python contract. Stale-first builds commit before pairing; applied pairs
and their second rebuild remain atomic.

Python cancellation rounds 7/7b are integrated: prefetch stops after cancel,
transport checks run on the SQLite-owning thread, BDK/LWK SDK steps are fenced,
and control results omit private checkpoints. Both runtimes use interruptible
HTTP backoff with at most 100 ms between checks. No privacy deviation remains.

## Development replay and refusal census

Production Rust builds contain no Python journal bridge and return native
refusals. The development hybrid launcher enables the test-build adapter and
its fixed reference helper. A replay concerns only the individual journal job;
it never repeats wallet network acquisition. The native adapter captures the
database, WAL and rollback-journal bytes after job progress is committed. It
replays only after rollback, an idle connection and byte-identical durable
files, with fresh cancellation, revocation and saved-job checks. A completed
first rebuild makes a later refusal ineligible. The child receives its bounded
session key through a RAM pipe, retains no ownership role, and is killed and
joined when cancellation or retirement interrupts the replay.

The opt-in hybrid log records every observed journal refusal as
`{command: freshness.journal_refresh, event: refusal, refusal_shape:
<last phase>:<exception family>, replay_safe: <boolean>}`. A selected replay has
an additional `served_by: python` record. These records expose no operand,
descriptor, transaction identifier, error message or secret. The existing CLI
`fallback_shape` records remain part of the same lossless handoff.

| Retained shape | Refusal category | Replay |
| --- | --- | --- |
| Unsupported stored tax country | `preparing:ValueError` | Eligible before writes |
| Invalid stored gains algorithm | `building:AppError` | Eligible after exact rollback |
| Invalid stored transaction timestamp | `pricing:AppError` | Eligible after exact rollback |
| Invalid stored timestamp with an existing price | `building:EngineValueError` | Eligible after exact rollback; engine refusal projection belongs in the lossless audit |
| Injected storage refusal before the first commit | `storing:DatabaseError` | Eligible after exact rollback; the reference replay completes the journal job |
| Injected storage refusal in a second rebuild | `storing:DatabaseError` | Forbidden after the first committed build |

This is an observed census, not a claim that malformed-scalar support is
lossless. The R4-13 census also retains injected pre-commit storage failures.
The follow-up must close NaN stage values, actual surrogate scalars/keys,
non-string stage keys, setup-resolver BLOB profile values and malformed stored
scalar/refusal precedence before R4-16. Existing supported BLOB consumers must
remain native. Valid-book product comparisons enable development replay and
require zero refusal records and zero fallbacks.

## Verification

The affected native suite passes **468 tests** across wallets, journals,
daemon, binary, chain, CLI and runtime, including encrypted publication
rollback, journal crash recovery, complete refresh coordination and the
102-round wallet-publication corpus. Formatting and production/test-harness
clippy pass with warnings denied.

The inherited regression run passes **522 tests** for queueing, admission,
execution, Python cancellation/control projections, hybrid selection, CLI
journal registration and the no-refresh product corpus. Its **327 complete
native product round trips** retain zero served-shape fallbacks. All **114
new journal freshness cases** pass across the focused runs, including three
SQLCipher cases for native execution, successful development replay and
cancellation during replay. The latter preserves every domain table and
joins the child before lock completion. The final encrypted/HTTP selection
passes eleven tests, including the Python interruptible retry regressions.

The default-refresh product comparisons retain **117 valid-book native round
trips**: 35 wallet sync, 45 active freshness and 37 workspace freshness. Each
compares envelopes, ordered events and every logical table. Development replay
is enabled and every valid case requires zero journal refusal records and
zero fallbacks. The twelve explicit census cases separately prove all six
retained shapes with replay disabled and enabled. The inherited invalid graph
fixture exposed a timestamp refusal; valid graph cases now use an ISO
timestamp, while both invalid timestamp paths remain in the census.

All **ten touched freshness mutations** are killed after ten unmutated named
selections pass: the seven new journal/slot/replay entries, two adjusted
admission entries and default-journal product routing. The runner rejects
compile errors, skips and empty selections as kills and removes its scratch
and build trees. All affected freshness, journal and chain catalogs pass
applicability checks; this does not recertify their inherited kill campaigns.
The native HTTP-backoff mutation is also killed after its unmodified Rust
baseline passes, bringing the touched campaign to **eleven killed mutations**.

The required classification update records **5,979 test definitions** and the
complete preflight shard passes **57 checks**. Its initial contract-snapshot
failure was reviewed as Python source-line metadata drift only: command bytes,
daemon kinds, error codes/retry flags and environment names are unchanged.
The three snapshots are refreshed and the complete shard passes on rerun.
No verification remains blocked by the sandbox.

The manual second review covers admission before consent, the foreground slot,
caller pairing authority, idle progress commits, atomic pair/rebuild writes,
durable rollback replay eligibility, cancellation and lifecycle joins,
SQLCipher key ownership, bounded child output and redacted refusal records.
No production Python bridge, dependency or egress path is added.

The full repository quality gate, inherited complete mutation/performance
campaigns, tax/custody shadow overlays, Docker regtest and packaged/platform
gates are not claimed here. The local 20,000-book timing gate was not rerun:
the shared machine's observed one-minute load was 14.56–21.13, above its 1.5
quiet-load threshold. This checkpoint changes daemon integration and makes no
new stage-speed acceptance claim. Those budgets and the lossless census
closure remain prerequisites for R4-16.

Task-owned logs and served-by/refusal summaries are under
`target/r4-15-artifacts/`. Commands use locked Python 3.11.17 and Rust 1.99.1,
offline Cargo, eight build jobs and disposable data roots with the account
HOME inherited. Mutation and verification build trees are removed after
summarizing.
