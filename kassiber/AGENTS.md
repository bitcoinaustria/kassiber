# Python core and CLI

These rules supplement [the root guidance](../AGENTS.md). Contributor commands
and the full gate live in [CONTRIBUTING.md](../CONTRIBUTING.md).

## Composition and accounting

- Keep core modules independent of the CLI layer. Handlers adapt requests and
  persist results; they must not recreate domain interpreters.
- `core/custody_journal.py` owns production journal composition, including
  decisions/issues/lineage and finalized tax inputs. Its
  `CustodyJournalBuilder.build_custody_decisions()` is the decision-generation
  and performance boundary. `GenericRP2TaxEngine` consumes finalized projections;
  it does not accept raw observations as an alternate accounting path.
- Use `normalize_boundary_amounts` in `core/custody_evidence.py` for principal,
  fee, wallet movement, and wallet delta. Use `allocate_msat_fifo` in
  `core/custody_allocations.py` for ordered N:M reviewed allocations. Specialized
  transactionless route flattening may preserve its intermediate provenance.
- Keep advisory missing-wallet candidates separate from authored components.
  Holds cannot establish a destination edge or carry basis. Incomplete or
  conflicting authored-active components must block raw anchors from booking.
- RP2 owns tax primitives, lot selection, and carry math. Kassiber owns reviewed
  input preparation and report mappings. Do not rebuild Austrian Alt/Neu or
  moving-average computation here; follow [the tax boundary](../docs/reference/tax.md#implementation-boundary)
  and [Austrian marker contract](../docs/austrian-handoff.md).
- Keep provenance capture, commercial matching, and tax normalization separate.
  Prior tax reports are evidence, not imported ledger totals or lot authority.

## Storage and projections

- Use additive schema changes and lightweight compatibility migrations where
  possible. Preserve existing books and validate upgrades against
  [database compatibility](../docs/reference/database-compatibility.md).
- Raw transactions are observations; derived journals and regime projections
  do not rewrite them. Reports need current journal inputs and must expose
  stale or blocked state rather than treating missing prices as zero basis.
- Metadata history, custody revisions, and report amendments are append-only.
  Reverts append forward edits. Export completion may establish a saved artifact;
  only explicit user action establishes a filed report.
- Replicate only the authored-table allowlist. Derived data, execution grants,
  backend/provider secrets, and private wallet material stay outside replication. Read
  [device sync](../docs/reference/device-sync.md) before changing replay or storage.
- URL attachments remain literal references without fetching or indexing.
  Evidence reuse copies managed files to a new attachment identity, never a
  shared `stored_relpath` that deletion could invalidate.

## CLI, daemon, and AI

- Keep CLI/daemon output English and machine-deterministic. Use `AppError` and
  the shared envelope builders for typed errors and kinds. Machine and
  non-interactive commands return `interaction_required` instead of prompting.
- Extend `cli/command_registry.py` effect/bootstrap annotations when adding
  commands. Use `commands describe` for the catalog rather than maintaining
  exhaustive command lists in contributor docs.
- Read [daemon routing and desktop allowlists](../docs/reference/daemon.md#desktop-invoke-contract)
  when adding kinds. Desktop invocation and AI tool exposure are separate
  authority decisions; do not add a kind solely to silence a drift test.
- AI mutations and egress are independent consent axes. Keep read-only redacted
  execution for egressing reads and exclude them from automatic context reads.
  Use the shared review/source-funds APIs and existing once-only consent path;
  do not add an AI-specific accounting interpreter.
- Background events use `build_event_envelope`, not ordinary responses without
  a request ID. Workers that open a database own their connection and obey
  book-switch, cancellation, and source-revision checks.

For adapters, read the root task index before editing: observation transport,
Lightning sanitization, graph visibility, and replication have different trust
boundaries that must survive reuse across CLI, desktop, and AI.
