# Rust crates and CLI

These rules supplement [the root guidance](../AGENTS.md) for the Rust
workspace: the crates under `crates/` and the tax engine under
`tax-engine/core`. Contributor commands and the full gate live in
[CONTRIBUTING.md](../CONTRIBUTING.md).

The `kassiber` binary (`crates/kassiber`) is the CLI, the desktop daemon, the
MCP server, and the operator broker. `kassiber-cli` parses and dispatches,
`kassiber-daemon` routes JSONL requests, and the domain crates (journals,
custody, transfers, wallets, reports, setup, transactions, workspace and the
others) own the behavior both share.

## Composition and accounting

- Keep domain crates independent of the CLI and daemon layers. Handlers adapt
  requests and persist results; they must not recreate domain interpreters.
- `kassiber-custody` is pure computation over versioned raw facts: no storage,
  network, clock, or logging. `kassiber-journals` composes production journals,
  including decisions, issues, lineage, and finalized tax inputs, and owns
  their persistence. The tax engine (`kassiber-tax-core`) consumes the
  finalized projection; it does not accept raw observations as an alternate
  accounting path.
- Use `kassiber_custody::components::review::boundary_amounts` for principal,
  fee, wallet movement, and wallet delta, and
  `kassiber_custody::quantity::allocation::allocate_msat_fifo` for ordered N:M
  reviewed allocations. Specialized transactionless route flattening may
  preserve its intermediate provenance.
- Keep advisory missing-wallet candidates separate from authored components.
  Holds cannot establish a destination edge or carry basis. Incomplete or
  conflicting authored-active components must block raw anchors from booking.
- The [tax engine](../docs/reference/tax-engine.md) owns tax primitives, lot
  selection, carry math, and the Austrian Alt/Neu and moving-average rules.
  Journals prepare reviewed inputs and reports map the results; do not rebuild
  those computations elsewhere. Follow [the tax boundary](../docs/reference/tax.md#implementation-boundary)
  and [Austrian marker contract](../docs/austrian-handoff.md).
- Keep provenance capture, commercial matching, and tax normalization separate.
  Prior tax reports are evidence, not imported ledger totals or lot authority.

## Storage and projections

- Use additive schema changes and lightweight compatibility migrations where
  possible. Preserve existing books, including those written by earlier
  Python-based releases, and validate upgrades against
  [database compatibility](../docs/reference/database-compatibility.md).
- Raw transactions are observations; derived journals and regime projections
  do not rewrite them. Reports need current journal inputs and must expose
  stale or blocked state rather than treating missing prices as zero basis.
- Metadata history, custody revisions, and report amendments are append-only.
  Reverts append forward edits. Export completion may establish a saved artifact;
  only explicit user action establishes a filed report.
- URL attachments remain literal references without fetching or indexing.
  Evidence reuse copies managed files to a new attachment identity, never a
  shared `stored_relpath` that deletion could invalidate.
- Open write transactions with `kassiber_store::begin_write_savepoint`,
  `write_savepoint`, `write_transaction`, or `BEGIN IMMEDIATE`. A deferred one
  that reads first fails at once against another writer instead of waiting;
  `crates/store/tests/write_transactions.rs` enforces this.
- `kassiber-pyjson`, `kassiber-pydecimal`, and `kassiber-pytext` reproduce the
  CPython formats that stored bytes and exports depend on, including Unicode
  tables pinned to CPython 3.11 (Unicode 14). Do not replace them with
  Rust-native formatting; a byte change there changes stored data.

## CLI, daemon, and AI

- Keep CLI/daemon output English and machine-deterministic. Use
  `kassiber_envelope::AppError` and the shared envelope builders for typed
  errors and kinds. Machine and non-interactive commands return
  `interaction_required` instead of prompting.
- When adding commands, extend the grammar in `crates/cli/src/spec_data.rs` and
  the capability tables in `crates/runtime/src/capabilities.rs`; the operator
  broker and the daemon authorize by those tables. Use `commands describe` for
  the catalog rather than maintaining exhaustive command lists in contributor
  docs.
- Read [daemon routing and desktop allowlists](../docs/reference/daemon.md#desktop-invoke-contract)
  when adding kinds. Desktop invocation and AI tool exposure are separate
  authority decisions; do not add a kind solely to silence a drift test.
- AI mutations and egress are independent consent axes. Keep read-only redacted
  execution for egressing reads and exclude them from automatic context reads.
  Use the shared review/source-funds APIs and existing once-only consent path;
  do not add an AI-specific accounting interpreter.
- Background events use `kassiber_envelope::build_event_envelope`, not ordinary
  responses without a request ID. Workers that open a database own their
  connection and obey book-switch, cancellation, and source-revision checks.
- Network connections go only through `kassiber-net`'s consent-bound routes
  (`kassiber-ipc` for same-user local IPC). `crates/net/tests/chokepoint.rs`
  fails when another crate opens a socket or depends on a network client.

For adapters, read the root task index before editing: observation transport,
Lightning sanitization, and graph visibility have different trust
boundaries that must survive reuse across CLI, desktop, and AI.

## Tests and goldens

- Test-only hooks stay behind the test features (`test-harness`, `test-hooks`,
  and `kassiber-net`'s `test-observer`). A product binary carries no network tape or test SQLite seed;
  `scripts/check-no-test-tape.sh` checks the markers.
- Many goldens were produced by the removed Python reference and can no
  longer be regenerated by it. A deliberate change rewrites them through the
  tests' own write mode (`KASSIBER_GOLDEN_WRITE=1`, or `KASSIBER_BLESS=1` for
  the black-box transcripts; both are refused under `CI`), a reviewed diff, and
  a test-data lock bump in the same change. Tax and journal numbers, stored
  data, and exports need owner sign-off. See
  [plan 22](../docs/plan/22-python-removal.md#generators-become-frozen-goldens).
- Mutation catalogs name the tests that must fail when a rule breaks. Run the
  affected catalogs with `kassiber-mutation-check` after moving or rewriting
  tests; a change must not lose kills.
- Comments that cite `kassiber/*.py` paths and line numbers refer to the last
  Python commit, tagged `archive/python-backend`.
