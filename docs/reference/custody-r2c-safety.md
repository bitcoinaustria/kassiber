# R2c raw holds, privacy and dependency checkpoint

Branch `t3code/r2c-interpretation-9` starts at `29fbf82f`. Python remains the
production reference. Snapshot v1 stays raw-only, and the product continues to
list `interpretation` under `skipped_stages`.

## Composition

`partial.interpretation.safety` composes the existing consolidation, recorded
fan-out and ownership producers with privacy resolution, raw observation holds,
whole-group pair filtering, dependency closure, claim compilation and rowless
native audits. Samourai preparation precedes recorded matching; the recorded
matcher keeps the chain-event/payment-hash gate. Derivers execute consolidation
then fan-out then ownership; compiler candidates follow recorded, Samourai,
consolidation, ownership and fan-out order. Generic recorded pairs touching
producer blocks or derived anchors leave the population before holds.

Raw holds keep Python's conflicting-spend, explicit pending-confirmation,
unresolved multi-leg and unscoped-import reasons in priority order. Confirmation
uses the newest current closed observation, preserving microseconds, UTC offsets,
naive UTC and equal-time disagreement. Missing confirmation timestamps do not
establish pending state. Stored nested transaction wrappers and graphless or
synthetic sibling legs retain physical confirmation safety. Liquid conflict-only
scope can quarantine a transaction without granting its missing asset identity
any matching or basis authority.

Privacy evidence normalizes all five supported boundaries with the CPython text
helpers and preserves Python boolish/source JSON semantics, including nonfinite
and surrogate text inside stored JSON. Exact native observations on both
anchors can resolve generic privacy holds after fan-out or consolidation.
Structured Samourai candidates keep their specific quarantine. An authoritative,
canonically exact fee-only contributor can resolve its boundary; its principal
is computed from canonical serialized fields, never read as a nonexistent
`principal_msat` property. Authored exclusions and supplied native reservations
remain distinct from raw evidence.

Holds close across direct partners, whole groups and multi-step dependency chains.
They preserve sorted immediate causes, exclude reviewed/missing/already blocked
raw anchors, and propagate transient status only when all nonempty causes are
transient. Held candidates leave both ordinary claim compilation and derived
rowless audits. Privacy alone does not rewrite the pair population: a blocked
rowless candidate may still appear as an audit candidate, exactly as Python
returns it; its blocked anchor remains a barrier to custody and tax authority.
The final quarantine order and subtraction of independent blockers match Python.

No dependency, product Python behavior, database read, external I/O, input field,
or raw `codec::wide()` call is added. The original independent producer probes
remain available alongside the composed safety probe.

## Witnesses and producer counts

The frozen partial fixture contains **254 raw snapshot projections**, including
**39 new safety scenarios**. Direct helper vectors contain **87 detector**,
**37 raw-hold/closure**, **10 pair-filter**, **76 privacy-evidence**,
**14 privacy-resolution**, **13 conflict-scope** and **12 outpoint** cases:
**249 total**. Helper-only operand values remain outside snapshot `input`.

The safety generator compares **2,000 raw books**, **78 permutations**, and
**78 full-Python interpreter controls**. The assertion matrix pins row counts,
all quarantine reason counts, surviving candidate pairs, claims, rowless audits
and transient rows separately for Python and Rust. A mixed-rail import case
attaches explicit pending chain evidence to a payment-hash candidate and proves
that dropping its source also holds the partner; a privacy-blocked partner
retains its own permanent reason. A three-source native event independently
normalizes a real fee-only contributor before privacy resolution and fan-out
shape detection.

Each cell below is **Python / Rust books**; claims/audits count books with at
least one result. The code additionally pins exact rows/pairs/claims per book.

| Scenario | Compared | Raw holds | Privacy blocks | Claims | Rowless audits |
| --- | ---: | ---: | ---: | ---: | ---: |
| `pending_pair` | 52 / 52 | 52 / 52 | 0 / 0 | 0 / 0 | 0 / 0 |
| `pending_fanout` | 52 / 52 | 52 / 52 | 0 / 0 | 0 / 0 | 0 / 0 |
| `pending_consolidation` | 52 / 52 | 52 / 52 | 0 / 0 | 0 / 0 | 0 / 0 |
| `pending_ownership` | 52 / 52 | 52 / 52 | 0 / 0 | 0 / 0 | 0 / 0 |
| `replaced_spend` | 52 / 52 | 52 / 52 | 0 / 0 | 52 / 52 | 0 / 0 |
| `conflict_none_confirmed` | 52 / 52 | 52 / 52 | 0 / 0 | 0 / 0 | 0 / 0 |
| `conflict_both_confirmed` | 52 / 52 | 52 / 52 | 0 / 0 | 0 / 0 | 0 / 0 |
| `unscoped_provider` | 52 / 52 | 52 / 52 | 0 / 0 | 0 / 0 | 0 / 0 |
| `blank_identity` | 52 / 52 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `unresolved_fanout` | 52 / 52 | 52 / 52 | 0 / 0 | 0 / 0 | 0 / 0 |
| `unresolved_consolidation` | 52 / 52 | 52 / 52 | 0 / 0 | 0 / 0 | 0 / 0 |
| `unresolved_many_many` | 51 / 51 | 51 / 51 | 0 / 0 | 0 / 0 | 0 / 0 |
| `privacy_coinjoin` | 51 / 51 | 0 / 0 | 51 / 51 | 0 / 0 | 0 / 0 |
| `privacy_collaborative` | 51 / 51 | 0 / 0 | 51 / 51 | 0 / 0 | 0 / 0 |
| `privacy_payjoin` | 51 / 51 | 0 / 0 | 51 / 51 | 0 / 0 | 0 / 0 |
| `privacy_payment_in_coinjoin` | 51 / 51 | 0 / 0 | 51 / 51 | 0 / 0 | 0 / 0 |
| `privacy_sweep` | 51 / 51 | 0 / 0 | 51 / 51 | 0 / 0 | 0 / 0 |
| `privacy_exact_pair` | 51 / 51 | 0 / 0 | 0 / 0 | 51 / 51 | 0 / 0 |
| `privacy_fanout` | 51 / 51 | 0 / 0 | 0 / 0 | 51 / 51 | 0 / 0 |
| `privacy_consolidation` | 51 / 51 | 0 / 0 | 0 / 0 | 51 / 51 | 0 / 0 |
| `privacy_untrusted_pair` | 51 / 51 | 0 / 0 | 51 / 51 | 0 / 0 | 0 / 0 |
| `privacy_rowless` | 51 / 51 | 0 / 0 | 51 / 51 | 0 / 0 | 51 / 51 |
| `confirmed_pair` | 51 / 51 | 0 / 0 | 0 / 0 | 51 / 51 | 0 / 0 |
| `status_string` | 51 / 51 | 0 / 0 | 0 / 0 | 51 / 51 | 0 / 0 |
| `legacy_missing_status` | 51 / 51 | 0 / 0 | 0 / 0 | 51 / 51 | 0 / 0 |
| `privacy_unknown` | 51 / 51 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `closed_newer_pending` | 51 / 51 | 51 / 51 | 0 / 0 | 0 / 0 | 0 / 0 |
| `closed_newer_confirmed` | 51 / 51 | 0 / 0 | 0 / 0 | 51 / 51 | 0 / 0 |
| `closed_equal_disagreement` | 51 / 51 | 51 / 51 | 0 / 0 | 0 / 0 | 0 / 0 |
| `closed_invalid_timestamp` | 51 / 51 | 0 / 0 | 0 / 0 | 51 / 51 | 0 / 0 |
| `liquid_pending` | 51 / 51 | 51 / 51 | 0 / 0 | 0 / 0 | 0 / 0 |
| `liquid_privacy_pair` | 51 / 51 | 0 / 0 | 0 / 0 | 51 / 51 | 0 / 0 |
| `samourai_unverified` | 51 / 51 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| `samourai_verified` | 51 / 51 | 0 / 0 | 0 / 0 | 51 / 51 | 0 / 0 |
| `normalized_fee_only` | 51 / 51 | 0 / 0 | 0 / 0 | 51 / 51 | 0 / 0 |
| `privacy_reviewed` | 51 / 51 | 0 / 0 | 51 / 51 | 0 / 0 | 0 / 0 |
| `dependency_pending` | 51 / 51 | 51 / 51 | 0 / 0 | 0 / 0 | 0 / 0 |
| `dependency_privacy` | 51 / 51 | 51 / 51 | 51 / 51 | 0 / 0 | 0 / 0 |
| `lightning_chain_no_hash` | 51 / 51 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| Total | **2000 / 2000** | **826 / 826** | **459 / 459** | **613 / 613** | **51 / 51** |

The existing composed producer controls remain **1,000 consolidations**, **668
fan-outs** and **834 ownership results per engine** over their own 2,000 books.
The eight generated suites together cover 16,000 seeded raw books. The earlier
composition handoff owns the retained per-scenario transfer-producer matrix.

## Branch and mutation accounting

| Mutation group | Entries | Named Rust witnesses |
| --- | ---: | --- |
| Retained catalog | 681 | Existing decoder, graph, producer, component, evidence and composition tests |
| Raw detectors, scope, holds and dependency closure | 79 | `raw_observation_detector_vectors`, `raw_observation_scope_vectors`, `raw_observation_hold_vectors`, `held_pair_filter_vectors` |
| Privacy evidence and resolution | 24 | `privacy_evidence_vectors`, `privacy_resolution_vectors`, `interpretation_safety_projection` |
| Safety composition and error family | 11 | `interpretation_safety_projection` |
| Total | **795** | **All killed** |

All 681 retained and 114 new entries have named-test kills. Verification
combines 142 completed kills from the initial run with four disjoint remaining
runs of 163, 163, 163 and 162 entries, plus the two independently killed
chain-event/payment-hash admission arms. The initial run's 309 named baseline
filters pass unmutated; every disjoint run also passes its own baselines. The
combined kill ids equal the catalog exactly, with no duplicate or missing entry.

Every ported Python decision has a named mutation witness or the following
explicit equivalence/unreachable disposition. No survivor or nonbuilding
mutation counts as a kill.

- The Python `_closed_confirmation_observation` dataclass cannot be supplied in
  raw snapshot v1. Rust recomputes it from current hashes, physical scope,
  explicit status and the corrected timestamp parser. The raw current-hash
  predicate was already ported and retains its own mutation witnesses.
- The detector's explicit synthetic-prefix check precedes `onchain_scope`,
  which independently rejects that entire prefix set. Removing only the first
  check is equivalent. Conflict/pending recovery intentionally resets those ids
  before asking for physical scope, with a separate killed witness.
- Conflict-only fallback and outpoint helpers accept a wider operand domain
  than the stored graph parser emits. Direct Python/Rust helper vectors pin the
  invalid/missing txid, asset/network, canonical case, negative-index and
  arbitrary-precision branches without adding derived fields to raw snapshots.
- A held pair's value membership is equivalent to Python's identity membership:
  pairs with identical values have the same anchors and group and therefore the
  same filtering result. Duplicate pair witnesses pin this behavior.
- Missing raw ids in dependency closure are callable and tested. The initial
  detector set is drawn from the raw row population; its missing-id guard is
  defensive for typed snapshot ids. No synthetic or absent raw row is invented.
- Privacy's initial presence gate is equivalent to applying its normalizer to
  every row: null/empty boundaries never produce evidence. Invalid nonempty
  values still enter normalization and remain unsupported, as Python does.
- The current seam supplies no native-transition ids; that producer is a later
  slice. Direct `privacy_resolution_vectors` pin its reserved-id branch. Channel,
  loan and native-transition-specific quarantine branches are not claimed here.
- Removing already held ids from the transient growth worklist is a termination
  guard, not another accounting decision. Nonempty/all-cause checks and repeated
  growth are separately mutation-killed, including empty causes and mixed
  permanent/transient causes.
- `holds.transient_ids` is disjoint from preexisting interpreter blocks because
  both root insertion and dependency insertion exclude them. The final
  subtraction of those same blocks is retained literally but is equivalent to
  its omission in this partial composition. Post-compilation pair blockers keep
  their independent subtraction.
- No sorting callback is supplied by the journal. Existing shared admission,
  graph decoding, CPython integer/case/time behavior and producer branches retain
  their previous catalog entries; this slice does not introduce a new input
  validator or expand snapshot v1.

## Verification

Tooling: `mise exec rust@1.99.1 uv@latest -- ...`,
`MATURIN_NO_INSTALL_RUST=1`, locked Python 3.11 sync, and offline Cargo, including
child mutation builds via `CARGO_NET_OFFLINE=true`. Data roots, recordings,
virtual environment, caches and mutation scratch trees are disposable `/tmp`
paths. The cached Rust toolchain and Python installation are read-only inputs.

- `cargo fmt --all --check` and workspace/all-target Clippy with warnings denied
  pass. Locked, offline workspace tests excluding `kassiber-net` pass **726
  tests**. The JSON-feature check reports **145 byte-identical ledger outputs**.
- Inventory validation, inventory/contracts and mutation-checker contracts pass
  **13 tests and 14 subtests**. Whole-catalog applicability reports **795 entries
  apply**.
- All **795 mutations** are killed with no survivor or nonbuilding mutation
  counted as a kill.
- The generated/partial comparison modules pass **15 tests**. Together with the
  component corpus in the broader Python run, all eight generated suites compare
  **16,000 seeded books**. The safety corpus includes 78 permutations and 78
  full-Python controls; retained main permutations cover evidence 58,
  components 2,000, Samourai 68, recorded pairs 92, fan-out 94, graphs 16 and
  composition 24. Both engines retain the per-scenario producer counts above.
  The post-review safety/frozen-reference rerun also passes **three tests**;
  it verifies that preparation reuses the already gated recorded-pair population.
- The custody Python modules plus ownership/transfers/reviews, historical
  compatibility and onchain modules report **1,138 passed and 2,710 subtests
  passed, with nine failures**. Five daemon tests need the unchanged stable
  owner-lock runtime `~/.kassiber/run`, which is read-only in this sandbox:
  the two `CustodyComponentDaemonSurfaceTests`, the two real-JSONL
  `CustodyGapDaemonProtocolAcceptanceTests`, and
  `test_cold_daemon_opens_plaintext_book_before_handoff_scope_check`. Four
  `ReviewRegressionTest` BTCPay sync cases fail creating their local HTTP test
  sockets with `EPERM`. No runtime guard is weakened and no real book is used.
- The fresh recording set passes integrity and Rust replay: **350 snapshots,
  350 recorded requests and 350 permutations**, zero pre-request refusals and
  zero differences.

Full census, custody-shadow shards and the full quality gate belong to the
orchestrator. This checkpoint does not claim those lanes. No push or PR is made.

## Remaining slices

1. Channel lifecycle and loan roles, native HTLC transition production and
   reservations, native component reconciliation and final interpreter
   composition. Keep `interpretation` skipped until the entire stage compares.
2. Gap search, quantity arbitration and finalized projection before joining
   custody with `ledger`.
