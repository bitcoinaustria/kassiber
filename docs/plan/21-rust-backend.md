# Kassiber backend in Rust

**Status:** Direction accepted 2026-10-26: the owner chose to move the whole
backend to Rust and retire Python, including custody, which
[plan 20](20-kassiber-tax-engine.md) left out. Stage R0 is in progress. The
work lands as one stack of draft pull requests on top of plan 20's phases 4
and 5; nothing reaches `main` until the stack is complete.
**Current source of truth:** current code and tests, the
[tax engine](../reference/tax-engine.md),
[machine output](../reference/machine-output.md), [daemon](../reference/daemon.md),
[database compatibility](../reference/database-compatibility.md),
[privacy and security](../reference/privacy-and-security.md), and the stage
sections below as each stage starts.

## Decision

- The backend becomes one Rust binary, `kassiber`. Like today's Python
  sidecar it is the CLI, the desktop daemon (`kassiber daemon`), the MCP server
  (`kassiber mcp serve`), and the operator broker (`--operator-broker-server`).
  The CLI chat, MCP, the broker, the update check, the state-root migration,
  the Vite dev bridge, and the Linux `/usr/bin/kassiber` all depend on that
  shape, so the binary keeps it.
- The contracts stay: CLI argv and the machine envelope, the daemon's JSONL
  protocol and its kinds, the data-root and project layout, the database
  files and schema (SQLite or SQLCipher, opened and migrated in place), the
  operator broker protocol, the MCP protocol, and the provider broker's JSONL
  protocol. The desktop changes only in how it finds the sidecar.
- Python leaves the product at retirement (stage 7): no interpreter, no
  PyInstaller, no PyO3 binding.
- The TypeScript provider broker (`ui-tauri/provider-broker`) stays; it is not
  Python.
- One toolchain: a `rust-toolchain.toml` at the repository root pins a current
  stable Rust for the tax engine, the new crates, and `ui-tauri/src-tauri`, and
  CI and release builds both read it.

## Why

- One backend language. The desktop shell and the tax engine are Rust
  already, and `bdkpython` and `lwk` are bindings over Rust libraries.
- Packaging. The sidecar stops being a PyInstaller bundle (one-file
  extraction on Linux and Windows, a one-dir build for macOS signing, a Python
  runtime per platform).
- Performance. Plan 20 made the tax stage about 3.4 times faster, but
  journal processing on a 12,000-transaction book improved only about 1.4
  times end to end, because custody now dominates: about 9.7 s of a 15 s
  profiled run. The custody path reads the transactions table seven to nine
  times per run, and component materialization repeats work.
- Types. Custody passes untyped row dictionaries and component dictionaries
  across about 15,000 lines of decision logic.

## Requirements

### No lost functionality

The Rust backend reaches parity with everything reachable through today's
contracts before Python is removed:

- All 313 CLI leaf commands, the global flags, exit codes, the usage-error
  envelope, every error code with its `retryable` flag, and the envelope's
  observable rules: `kind` derivation including its path-only groups and
  explicit overrides, the `page` block, `envelope_meta` keys, and JSON with
  escaped non-ASCII and a trailing newline.
- All daemon kinds (303 today), the `daemon.ready` handshake, streaming only
  for streaming kinds, `event: true` records, terminal-record rules, the
  secret-store control channel, `expected_scope` and `stale_context`,
  `auth_required`, early-cancel holding, tool consent, the freshness engine's
  queue and single execution slot, and the background workers.
- The data-root and project model: state roots per platform and their legacy
  locations and migrations, `projects.json`, `settings.json`, `backends.env`,
  attachments, the environment overrides, the unlock order
  (`--db-passphrase-fd`, remembered unlock, TTY prompt,
  `interaction_required`), SQLCipher's parameters, and the keyring service and
  account names.
- The database: every database Python opens, Rust opens to the same state.
  Schema creation, the compatibility pass, the one-shot semantic migrations
  and their audit rows, and the historical fixtures keep their behavior.
- Persisted bytes: custody evidence hashes, graph and quantity hashes, gap
  candidate fingerprints, issue and decision ids, uuid5 ids, and stored JSON
  columns stay byte-identical, because stored commitments are compared
  against recomputed ones.
- Privacy and egress: no DNS, socket, HTTP, RPC, or provider probe before the
  user's action or narrowly scoped consent; the egress ledger; offline mode;
  `KASSIBER_NO_EGRESS`; RAM-only, redacted logs; redaction of error envelopes,
  AI tool results, and diagnostics.
- AI: the renderer, model, MCP, and broker allowlists, tool-argument
  validation against the catalog, consent before egress or mutation, and book
  and input-version binding.
- Reports: XLSX, CSV, and JSON exports match cell for cell and value for
  value. PDFs match in extracted text, tables, and page structure, not bytes.

### Contracts and invariants

- **Parity first.** Each stage reproduces Python's observable results. A
  difference is a bug or a listed, reviewed difference, as in plan 20. Plan 20
  phase 5's non-legal fixes are the only semantic changes in this programme.
- **Exact amounts.** Quantities are integer msat. Where Python computes with
  `Decimal` (32 digits, round-half-even) Rust uses the engine's
  CPython-compatible decimal type. Floats appear only where Python stores or
  prints floats, and print like Python's `repr`.
- **Fail closed.** Unsupported shapes are refused or quarantined; a panic
  becomes an error, never a partial result or a partial write.
- **Watch-only.** No spending keys, signing, or broadcasting.

## Architecture

### Workspace

A Cargo workspace at the repository root holds the existing tax engine and
the new crates. Names are working names; each stage settles its own.

| Crate | Role |
| --- | --- |
| `tax-engine/core` | the tax engine (plan 20) |
| `tax-engine/python` | the PyO3 binding; grows a `custody` operation in R2 and leaves the product at retirement |
| `crates/pyjson` | Python-compatible `json.dumps` output (both separator styles, `ensure_ascii`, `sort_keys`, `default=str`), float `repr`, and `Decimal` text, checked against CPython vectors |
| `crates/pydecimal` | the tax engine's CPython-exact decimal type, moved out of `tax-engine/core` so every crate shares it |
| `crates/pytext` | CPython's `str.strip()`, `repr(str)`, `int()`/`float()` parsing, and `json.loads` errors, which argv and the daemon echo |
| `crates/envelope` | the machine envelope, `AppError`, kind derivation, and the redaction rules |
| `crates/runtime` | capability tables, build identity, the `status` payload, and the store interface the shell calls |
| `crates/custody` | pure custody computation: snapshot in, decisions and finalized projection out |
| `crates/store` | SQLite and SQLCipher through `rusqlite`, schema, compatibility pass, data roots and projects, settings, unlock |
| `crates/core` | services: journals, pricing, metadata, transfers and components, reviews, source of funds, loans, attachments, backups, maintenance |
| `crates/imports` | wallet, exchange, and document imports |
| `crates/reports` | report computation and XLSX, CSV, JSON, and PDF rendering |
| `crates/chain` | descriptors and ownership index, sync backends, Liquid, Lightning, BTCPay, rates, chain analysis |
| `crates/net` | the one HTTP and socket layer: consent gate, egress ledger, proxy and Tor, TLS trust |
| `crates/secrets` | keyring, remembered unlock, backup encryption |
| `crates/ai`, `crates/mcp`, `crates/operator` | providers and tool catalog, MCP server, operator broker |
| `crates/daemon`, `crates/cli` | the JSONL daemon and the CLI, both over `crates/core` |
| `kassiber` | the binary |

Replacements for today's dependencies, decided per stage:

| Python | Rust |
| --- | --- |
| `sqlcipher3`, `sqlite3` | `rusqlite` with bundled SQLCipher 4, `backup` and `functions` |
| `bdkpython`, `lwk` | `bdk_wallet` with `bdk_electrum`/`bdk_esplora`, `lwk_wollet`, with Kassiber's own persisted state formats |
| `embit` | `bitcoin`, `miniscript`, `elements`, `elements-miniscript`, `secp256k1`; the hand-written PSBT v2 reader stays hand-written |
| `XlsxWriter`, `openpyxl` | `rust_xlsxwriter`, `calamine` |
| `reportlab` | a rewrite, most likely Typst as a library with the bundled fonts; the hand-written PDF 1.4 report ports directly |
| `pyrage`, `age`/`rage` binary | the `age` crate |
| `keyring` | the same keyring stores the desktop shell uses, with the item formats Python wrote |
| `urllib`, the SOCKS5 client, the Electrum client | one transport layer (`reqwest` or `ureq` with no environment proxy and no redirects, SOCKS with remote DNS) |
| `ssh-keygen -Y verify` | the `ssh-key` crate |
| `zoneinfo`, `tzdata` | `jiff` with its bundled database |
| `Decimal` | `crates/pydecimal`, never `rust_decimal` |

Subprocesses that stay subprocesses: the TypeScript provider broker and the
provider CLIs it drives, `lightning-cli`, and Poppler for document import.

`ui-tauri/src-tauri` keeps its own workspace and the shared toolchain. Linking
the daemon crate into the desktop process instead of running a sidecar is a
later, optional change: the sidecar's crash isolation, process-restart
resets, and the macOS Touch ID helper's signing-identity check all assume a
separate process today.

### Transition

The port proceeds in three shapes, each verifiable against the Python
reference while it still exists:

1. **Compute behind PyO3.** Custody computation moves into Rust behind the
   existing binding, as the tax engine did. Python keeps SQL, descriptor
   derivation, and persistence, so the whole Python suite keeps running and a
   shadow mode compares both implementations on every build.
2. **The binary grows.** The Rust binary opens the same data roots and
   implements services group by group, each through both the CLI and the
   daemon callers at once. Python stays the reference. Comparison runs the
   Python reference and the Rust candidate separately, on separate disposable
   copies of the same starting state. A development-only fallback that hands
   an unported command to the Python CLI may exist for exploratory runs only:
   it chooses before any bootstrap, database access, secret consumption, or
   mutation, delegates the whole invocation, never retries a Rust failure,
   records its use out of band, and any use fails the acceptance gate of the
   group being ported. It never ships.
3. **Cutover.** The desktop and every release artifact switch to the Rust
   binary, and Python leaves the product. A test-only Python reference stays
   for an agreed validation period after that.

## Verification

1. **Crate tests:** unit, property, and golden-vector tests per crate. Vectors
   for Python-compatible text (JSON, float `repr`, `Decimal`) and for each
   operation's rounding (32-digit half-even by default, half-up for BTC to
   msat in `msat.py`, float conversions where matching thresholds use floats,
   parser leniency such as surrounding whitespace) come from CPython, as the
   tax engine's decimal vectors do.
2. **Shadow runs:** a test-only switch runs Python and Rust custody on every
   journal build in the suite and fails on any difference, stage by stage
   (component materialization, interpretation, quantities, finalized
   projection) before the stages are fused. Owners can run the same comparison
   on a disposable copy of a real book; results never leave the machine.
3. **Test disposition.** R0 inventories every test module and assigns each
   test one disposition: a portable boundary fixture, a real subprocess
   scenario, a Rust service or property test, a retained oracle test, or
   Python-only machinery with a named replacement for its intent. The
   [R0 inventory](../reference/rust-port.md) records the assignments and counts.
   Each group's tests migrate with its implementation, not at cutover. Progress is
   measured as tests and accounting or security scenarios that pass on
   native Rust execution, not as commands or kinds touched.
4. **Recorded sessions.** Where a test drives the product through its
   boundary, the recorder captures whole process sessions at the wire:
   argv, working directory, the environment subset, stdin and inherited file
   descriptors, external fixture files, and for the daemon the startup,
   control-channel exchanges, ordered requests, streaming records and events,
   cancellation, shutdown, and restart; plus permitted subprocess responses
   and network tapes. The starting data root is kept unchanged and executable.
   Wall clock, monotonic clock, and entropy come from injected test sources in
   both implementations instead of being masked afterwards. Newly generated
   identities are compared through declared mappings that preserve uniqueness
   and every reference; no blanket masking of uuids, timestamps, hashes, or
   array order. A recording is accepted as an oracle only after it replays
   cleanly on a fresh Python run, and the harness proves it fails on injected
   discrepancies, including through subprocesses and caught-error paths,
   as the tax shadow harness does. Mocks of Python internals are converted
   into explicit replayable fixtures or the recording is excluded with an
   audited replacement test. Recordings carry the reference commit, schema,
   engine version, generator, and comparison policy; accepted baselines are
   immutable, and regenerated differences are reviewed as contract changes.
   Committed recordings use synthetic data only.
5. **Black-box suites:** a `KASSIBER_TEST_CLI` switch points the subprocess
   tests (CLI smoke, the daemon smoke's subprocess half, CLI chat, MCP, entry
   point smoke), the packaged-binary smoke, and the regtest integration
   harness at the Rust binary. The Tauri supervisor tests and the Vitest suite
   also run against a real Rust sidecar, not only their stubs.
6. **Egress proof:** today's test guard is a Python socket patch that cannot
   reach a Rust process. R0 builds the Rust guard before any Rust binary runs
   a scenario: an inventory of every connection constructor, DNS path, SDK
   client, and network-capable child; consent and routing checks before
   connect; a test build that observes and refuses attempted connections,
   including loopback in strict mode; and CI runs in a network namespace
   without a route out as a second layer. A native client that cannot honor
   the proxy, redirect, or trust policy is refused, as today's observer
   routing does. Product offline mode, `KASSIBER_NO_EGRESS`, and the test
   guards keep their distinct scopes. Every egress surface keeps a regression
   proving no traffic before consent.
   The R0 [connection inventory and gate](../reference/rust-port-egress.md)
   define consent-bound `kassiber-net` routes and a feature-gated attempt observer;
   TLS/HTTP adapters remain refused before I/O until their routing and trust policy is implemented.
7. **Storage durability:** beyond equal content, the storage stage proves
   schema objects and trigger behavior, migration audits and idempotency,
   foreign-key and integrity checks, busy and error semantics, the journal
   mode safe for every coexisting runtime (see R1 transition rule), rollback and failure
   injection before and after each durable step, Python-written databases
   through Rust and Rust-written databases through Python (plaintext and
   SQLCipher, rekey, backup and restore), concurrent daemon and CLI writers,
   and interrupted operations, including the graph-index schema installed at
   open time.
8. **Exports:** XLSX, CSV, and JSON exports match semantically, XLSX
   including formulas, cached results, and verification sheets. New PDFs and
   SVG diagrams match the diagram model and extracted content under a
   reviewed renderer difference. Each stored export digest is checked against
   its actual file; digests saved in filed snapshots before the switch stay
   untouched.
9. **Packaging:** every release artifact builds, signs, and passes the
   packaged smoke on macOS arm64, Linux x86_64 on the AppImage glibc floor,
   and Windows x86_64.

## Stages

Each stage gets its own section here when it starts, moves its tasks into
[TODO.md](../../TODO.md), and lands as one or more draft pull requests on the
stack.

0. **R0 Foundations.**
   - Pin the toolchain and form the root workspace; cache Rust builds in CI.
     Move the decimal type into `crates/pydecimal`; build `crates/pyjson`
     against CPython vectors.
   - Make the Python reference deterministic: inject the clock into the
     custody path (swap-dismissal expiry reads the wall clock today), pin the
     custody hash and id serializations with golden vectors, and add test-only
     clock and entropy injection for whole CLI and daemon runs.
   - Inventory the contracts (environment variables and their precedence,
     including legacy `SATBOOKS_*`; machine-wide preference files; operator
     child bindings; `commands describe` metadata; error codes) and every
     test's disposition. The [contract inventory](../reference/rust-port-contracts.md)
     records source evidence and deterministic catalog, kind, error and
     environment snapshots; [test dispositions](../reference/rust-port.md)
     record migration choices and manual samples.
   - Build the session recorder and replay harness, prove Python-to-fresh-
     Python replay and discrepancy injection, and add the `KASSIBER_TEST_CLI`
     switch. Built (`tests/session_harness/`,
     [testing](../reference/testing.md#product-subprocesses-and-recorded-sessions));
     strict replays wait for the clock and entropy injection above.
   - Build the Rust no-egress guard and the connection inventory.
   - Feasibility spikes: SQLCipher interoperability with `sqlcipher3`, keyring
     item formats Python wrote, macOS signing and the Touch ID helper's
     identity check against a Rust sidecar, and ReportLab replacement with
     Typst on one report.
1. **R1 Runtime slice.** A packaged Rust binary on all three platforms that
   opens and unlocks a book (passphrase descriptor, remembered unlock, the
   secret-store control channel), takes and releases project ownership locks
   compatible with the Python broker and daemon, reports `daemon.ready`,
   `status`, locked and unlocked state, and shuts down cleanly; broker build
   identity; one signed Touch ID flow. Python still serves everything else.
2. **R2 Custody.** Port the pure custody computation, including component
   materialization, from raw facts (below), behind the binding; freeze the
   Python modules as an oracle; compare stage by stage in the shadow lane;
   then join custody and the tax engine in one `journal` operation. Every newly
   ported stage must add checked-in mutation entries and kill each with named
   tests; corpus/shadow parity alone is insufficient acceptance evidence.
3. **R3 Storage.** Create, open, and migrate every database Python can, with
   the durability gates above; the data-root and project model; unlock and
   keyring.
4. **R4 Journals end to end** through the CLI and the daemon: snapshot loading
   in Rust, the ownership index, the pre-build repairs, persistence, freshness
   and the journal worker.
5. **R5 onward: services as vertical slices** through the CLI, the daemon, and
   the AI projections that expose them, each with its transport requirements
   in place before it is enabled: imports and documents; reports and exports;
   chain, sync, rates, BTCPay, Lightning, and chain analysis; transactions,
   metadata, transfers and components, gap reviews and the review workflow,
   source of funds, loans, attachments, secrets, backup, maintenance,
   diagnostics, update and release verification. Scope and consent
   interfaces for AI are established with the first slice that needs them.
6. **AI, MCP, and operator orchestration complete:** chat, the provider broker
   client, tool consent, the MCP server, and the broker's queues and agent
   sessions.
7. **Retirement.** Synchronize with `main` one last time on an immutable
   baseline, pass the full cumulative gate and the integration suites against
   the packaged Rust binary, rehearse binary and data rollback, switch the
   desktop sidecar and every release artifact, delete Python and the PyO3
   binding from the product, and update every doc and AI reference that names
   Python.

### R0 feasibility findings

The spikes ran on Linux x86_64 against commit `8709d430`; their code and
evidence are archived on branch `t3code/r0-spikes` (commit `55d93e61`,
`spikes/README.md`). Each is feasible with conditions:

- **SQLCipher.** `rusqlite` 0.40 with bundled SQLCipher 4.14 and Kassiber's
  `sqlcipher3` 4.12 open, write, rekey, export, and back up each other's
  databases, including the full current schema. The two embed different
  SQLite versions (3.51.3 and 3.51.1). Querying Rust's own version is insufficient
  while Python still opens books: Rust keeps SQLCipher in DELETE/FULL, never
  promotes an existing plaintext book to WAL, and retains plaintext WAL only
  when Rust's own SQLite is safe. New books follow the shipped Python policy,
  conservatively DELETE/FULL when unknown. Vendored OpenSSL suits Linux and
  Windows; macOS can use its native
  crypto. macOS and Windows round trips, packaged crypto linkage, and crash
  safety of migration and backup remain gates.
- **Keyring.** A Python-written Secret Service item was read and updated from
  Rust in place on Linux. Windows needs two formats kept apart: remembered
  unlock as Python's `keyring` wrote it (bare service and `account@service`,
  UTF-16) and the desktop's AI items (`account.service`, local persistence,
  UTF-8). macOS keeps generic-password service and account attributes and may
  need the new executable's ACL authorization. Native macOS and Windows runs
  remain gates.
- **macOS identity.** A Rust Mach-O passes the Touch ID helper's checks
  without weakening them if it keeps the sidecar's architecture-specific file
  name, signing identifier, location in the bundle, and team; the broker's own
  PID checks must be ported. A signed hardware run on a Mac gates R1.
- **PDF.** Typst 0.15 as a library, with an embedded template, bundled Vera
  fonts, and no runtime file or network access, rendered the summary report
  with every checked string and chart label present and byte-identical output
  across runs. Pagination and chart styling differ (two pages instead of
  three). It adds about 40 MB to a standalone binary and about five minutes
  to a cold release build. Today's ReportLab path silently falls back to
  Helvetica because the wheel lacks one Vera face; the font choice needs a
  decision. The source-of-funds diagram keeps one geometry model, rendered as
  SVG for the desktop and embedded in the PDF.

### R0 decisions

Recorded 2026-10-27 as the toolchain, workspace, and text crates landed:

- **Toolchain.** `rust-toolchain.toml` pins Rust 1.99.1 with rustfmt and
  clippy; every crate declares `rust-version = "1.99"`. A local action,
  `.github/actions/setup-rust`, installs that channel and makes it the
  default in every CI and release job that compiles Rust, including each uv
  job that builds the binding. The quality gate refuses another `rustc` and
  runs clippy with warnings as errors. The desktop shell never built on 1.77:
  its lockfile already needed edition 2024.
- **Layout.** The root `Cargo.toml` is the workspace: `crates/pydecimal`,
  `crates/pyjson`, `tax-engine/core`, and `tax-engine/python`, with one
  `Cargo.lock` and one `target/`. The tax engine stays under `tax-engine/`.
  Its crates keep their own `0.2.0` release, shared with the binding's
  Python package; new crates start at `0.1.0`. `ui-tauri/src-tauri` stays a
  separate workspace on the same toolchain.
- **CI cache.** `Swatinem/rust-cache` keeps the registry and dependency
  builds under shared keys: one for the uv jobs, saved by `Preflight`, one
  for the Rust job. Only pushes to `main` save; release builds take no
  cache. Cargo rebuilds the workspace's own crates after every checkout, so
  the cache saves the dependency part of each binding build.
- **`kassiber-pydecimal`** is the engine's decimal type unchanged; the
  engine's `decimal` module re-exports it.
- **`kassiber-pyjson`** models the Python object rather than JSON: ordered
  dicts with `str`, int, float, bool, or `None` keys, unbounded ints, and
  strings as code points, surrogates included. Floats print like
  `float.__repr__`, whose exact ties keep the even digit where Rust's own
  formatting rounds up. It refuses, rather than guesses, a NaN key under
  `sort_keys`, nesting past 1,000, and a surrogate in `ensure_ascii=False`
  output. Its README holds the `json.dumps` survey: 253 calls in 14 option
  sets.

### R1 shell decisions

Recorded 2026-10-27 as the binary shell landed
([Rust shell](../reference/rust-shell.md) holds the contract):

- **Grammar.** The argparse tree is generated from `build_parser()` into Rust
  statics rather than restated by hand or rebuilt with clap, which cannot
  reproduce argparse's prefix matching, error order, messages, or usage
  layout. The engine ports CPython 3.11's parsing and formatting; every
  parser's usage and help, every leaf's kind and catalog entry, and about
  5,000 command lines replay Python's answers. A `--check` mode keeps the
  tree in step with `main`.
- **Unported work fails typed.** A leaf or daemon kind the port lacks parses
  and classifies as in Python, then answers `not_implemented` before it
  touches a project; there is no Python fallback. `commands describe` lists
  only implemented leaves, so an agent's catalog matches what runs.
- **Store seam.** The shell calls a narrow `BookStore` trait in
  `crates/runtime` (bootstrap, unlock mode, ownership, the CLI and daemon
  unlock paths, passphrase verification, status facts). The binary uses a
  stub that fails closed until the storage crates implement it; an in-memory
  fake behind a test-only feature drives the shell's tests.
- **Redaction now.** The daemon echoes request text into error envelopes, so
  `redaction.py`'s rules are ported with the envelope rather than deferred.
- **Test harness.** One `test-harness` feature adds the egress observer, the
  fake store, and protocol test kinds; a client applying the desktop
  supervisor's routing rules drives the real binary, and
  `tests/test_rust_shell_parity.py` compares it with Python when
  `KASSIBER_RUST_BIN` is set. The binary carries the product version.
- **Differences proposed for review** are listed in the reference: a pure
  `commands describe`, and answers where Python's daemon hangs or crashes.
### R1 decisions

- **Existing-book runtime.** `kassiber-paths`, `kassiber-ownership`,
  `kassiber-store`, and `kassiber-secrets` expose root resolution, compatible
  role leases, SQLCipher/plaintext opens, transactions/backup, and zeroized
  unlock sessions to the CLI and daemon. Python product behavior is unchanged;
  the [Rust store reference](../reference/rust-store.md) owns the public API.
- **No initialization or migration.** Resolution writes no directories,
  settings, or catalog metadata. Missing state returns `not_initialized`;
  required legacy/schema/semantic work returns `requires_python_migration`.
  R1 compares normalized table/index/FK structure and trigger/view SQL plus
  migration sentinels and read-only semantic checks. Python-migrated historical
  layouts are accepted; unported custody matching returns an explicit
  unsupported-layout refusal. Native migration still waits for R3 acceptance.
- **Interop, not a new format.** Bundled SQLCipher 4 uses the Python keying
  order. During Python coexistence SQLCipher uses DELETE/FULL; Rust never
  promotes existing plaintext books to WAL and retains existing WAL only when
  its own SQLite has the WAL-reset fix. New books use the shipped Python
  runtime's journal policy, conservatively DELETE/FULL when unknown. Native keyring
  adapters use the desktop's 1.0.0 stores with Python-compatible lookup keys;
  remembered unlock requires unattended mode and both file/database bindings.
  Ownership retains Python's shared compatibility, exclusive per-role and
  admission locks. All tests use disposable books and owner namespaces.
- **Platform acceptance.** Linux cross-language opens/writes, refusal gates,
  path matrices, descriptor input, ownership and backup are exercised by
  `test_rust_store_interop.py`. Secret Service runs only with an available
  unlocked session and a synthetic item. Native macOS/Windows, packaged crypto,
  signed identity and Touch ID hardware remain gates; this storage slice alone
  does not complete R1's packaged binary requirement.

### R3 decisions

- Keep R1 review corrections in separate commits, using structural admission,
  exact semantic predicates where ported and explicit unsupported-layout
  refusals for unported custody matching. Add explicit creation/migration APIs
  separately; display/runtime reads do not create or migrate storage implicitly.
- Leave Python DDL in place. Embed a generated empty-book write transcript
  with a Python drift test; never replay it on existing schemas. Preserve
  complete `sqlite_master` SQL, including whitespace, and inject identity and
  audit time before parity tests rather than masking persisted values.
- R3 remains in progress until historical migration and the full storage
  durability matrix pass. Empty-book parity alone is not historical parity.
- Authored legacy pair/payout consolidation, immutable revisions/issues, and
  exact matcher-based componentless dismissal recovery move after R2's custody
  matcher port. Until then Rust returns `unsupported_schema_layout` for books
  reaching those steps; Python must migrate them. The ordered private R3 pass
  proves exact prefixes for both historical fixtures, including SQLCipher
  copies, but does not approximate these custody operations.
- The historical custody-view blocker is resolved: Python and Rust now create
  the projection relation view directly after the suspense-role rebuild.
  Both pair-free historical fixtures pass exact full first-pass dumps and
  unchanged Python reopen for plaintext and SQLCipher, without masking values
  or adding a reference pass. Both original fixtures still prove exact prefixes
  followed by the explicit R2 refusal. Full original-book migration, legacy
  flat-book/project/network partition migrations, and the complete crash/backup
  matrices remain R3 gates; the ordered pass stays private. See the
  [ordered-pass status](../reference/rust-storage-r3-status.md#ordered-pass-attempt).
- Legacy root cutover uses Python's prepared proof and rename-only protocol.
  Preserve file identities and fixed runtime paths; refuse merges and retain
  legacy authority on cross-device failure. Resolution itself remains read-only.

### R2 custody snapshot

One versioned request document, built in the journal transaction after the
source-overlap repair, the legacy component refresh, and auto-pricing. It
carries raw facts, never Python's verdicts:

- profile facts, including the journal input version and an explicit `as_of`
  that replaces the wall clock for dismissal expiry;
- wallets with labels, kinds, accounts, and normalized configuration facts
  (chain, network, the Samourai block), never descriptors or xpubs;
- the active transactions in their database order, with every column the path
  reads and their provenance;
- the owned index restricted to the scripts, outpoints, and txids the rows'
  graphs reference, or none when no row has provenance;
- authored components as stored, not materialized: headers, ordered legs,
  allocations, economic terms, evidence commitments, revision links, and the
  facts of every anchor they reference, including excluded and missing
  anchors; invalid authored-active components stay in the input so their
  anchors remain blocked;
- the receipt-reconciliation sources, Lightning channel records, loan legs,
  evidence snapshots, migration quarantines, raw latest gap-review rows, and
  pair dismissals;
- for books above the gap search's row limit, the SQL-selected worklist.

The response carries each stage's result: the materialized components and
their blockers, the interpreter compilation, the quantity state, and the
finalized tax projection, plus warnings and the gap-ignored id sets. Once
every stage compares clean, the joined `journal` operation also carries the
tax ledger, the wallet balance comparison, and the ownership-review
projection.

#### Determinism of the Python reference (R0)

An audit of everything `build_custody_projection` reaches (about 1,075
functions in 43 modules, traced through the custody-heavy test modules)
found these sources of run-to-run variation:

- **Clock.** Swap-dismissal expiry read the wall clock in the native HTLC
  interpreter. `CustodyJournalBuilder` now takes `as_of`, or reads the clock
  once per build, and passes it down; nothing else in the path reads the
  clock. Every timestamp parse pins naive values to UTC, so the host time
  zone does not matter.
- **Randomness.** The only `uuid4` is the savepoint name inside
  `require_book_accounting`, which rolls back and returns no data.
- **Environment.** None is read. The decimal context (32 digits, half-even,
  set when `kassiber` is imported) is process state the path relies on.
- **Iteration order.** No set iteration reaches an output: every
  `next(iter(s))` is guarded by `len(s) == 1`, the other set loops feed
  order-independent reductions, and keyed sorts either sort lists or include
  a unique id. Rebuilding all 178 builds of those test modules in a fresh
  process under another `PYTHONHASHSEED` gives identical results.
- **Database order.** The reads of wallet refs, loan legs, Lightning channel
  records, and pair dismissals have no `ORDER BY`, and the latest-rates read
  now orders by time and descending id to settle ties (an explicit R2a Python
  reference change). The snapshot keeps
  the order Python saw; whether outputs depend on it is still open.

The stored hashes and ids the path computes or compares are pinned by golden
vectors (see [testing](../reference/testing.md#custody-oracle)).

#### Snapshot v1

The raw request, SQL-read inventory, pure stages and recorder are specified in
[custody engine](../reference/custody-engine.md). The product loader supplies the
same v1 request to Python, Rust shadow runs and the test recorder. The earlier
v0 survey exposed full wallet configuration in copied tax rows, hidden component
and network reads, the raw Samourai substring gate, and order-sensitive script
matches; v1 retains those facts without wallet key material. Fresh-process
verification replays only this request and needs no database image.

#### Components milestone (R2b)

Rust now implements `components` before `evidence` through the same PyO3
operation. It materializes authored-active components and recomputes exact
conservation, inferred/explicit allocations, anchors, author commitments,
native support, route scope/chronology, revision and membership conflicts, and
report blockers. Snapshot v1 already contains every required raw fact; its
inventory and version remain unchanged. Python's component computation is
unchanged and remains authoritative.

The independent component generator covers 156 scenarios over 2,000 requests
and 2,000 permutations, including interrupted child imports, conflicting active
revisions, multi-leg allocations, missing/excluded anchors and hash drift.
Named Rust tests pin exact CPython stages and refusal codes. The mutation
catalog contains 167 entries with named killing tests, including the review
X-branches, raw-fact refusals and Unicode row-asset hashing. Successful controls
include peg-in/out, Lightning, swaps, suspense and multi-component routes.
Each scenario asserts its expected activation count. Each mutation must be killed. CPython Unicode decimal/uppercase and datetime vectors also pin
integer parsing, microseconds and overflow refusals. See the
[custody engine](../reference/custody-engine.md#r2-components-milestone)
for corpus counts and verification results.

#### Interpreter checkpoint (R2c, partial)

The shared raw-fact comparison seam and the first isolated interpreter,
Samourai's Whirlpool lifecycle, are ported. It recomputes grouping, exact MOVE
slices, coordinator fees, residual suspense and unverified-group quarantines
from schema-1 facts. Canonical aliases and authored reservations are retained;
CPython metadata strings, dict order and Unicode 14 repr behavior are pinned.
No snapshot field or version changed and Python remains authoritative.

`interpretation` remains explicitly skipped in product PyO3 responses. The
unpackaged `interpretation_partial` example runs isolated slice comparisons in
census, custody shadow, generated books and recorded replay. Its output is not
accounting authority; other interpreters' holds and interactions await the full
stage. There are 38 named frozen cases, and the added 2,000-book corpus reaches
1,176 active, 332 blocked and 492 inactive/reserved Samourai books. The mutation
catalog grows from 143 to 178 entries (three shared and 32 Samourai/claim/text
entries), each required to fail its named tests when mutated.

All 178 mutations are killed and 432 workspace tests outside `kassiber-net` pass.
The full Python matrix was executed with no custody differences, but filesystem/socket
sandbox refusals leave it non-green; the four net-crate loopback refusals were
not escalated. Generated and recorded/permuted comparisons, formatting, clippy,
inventory and contract checks passed. Exact counts and limitations are recorded
in the custody reference.

The recorded-pair checkpoint ported the generic pair compiler and recorded
Bitcoin/Liquid/Lightning matching. Samourai now shares that compiler, and the
raw-fact seam adds `partial.interpretation.recorded`. Snapshot v1 is unchanged;
`interpretation` stays skipped. Frozen coverage adds 46 raw-snapshot cases,
66 compiler-boundary cases and 36 transfer-predicate cases, including exact
msat, CPython integer/numeric/timestamp semantics and invalid nonfinite graph
identities. Candidate pairs and native proof-validation fixtures remain outside
the raw request; ownership/channel/HTLC proof production is still unported.

The new recorded generator compares 2,000 books: 789 active, 176 blocked and
1,035 inactive, plus 92 permutations. The mutation catalog has 232 entries
(178 previous, 11 recorded matcher, three scope and 40 generic compiler entries).
All 232 mutations are killed. All 545 workspace tests outside `kassiber-net`,
formatting and clippy pass.
Focused census passes 344 tests and 43 subtests; recorded replay compares 128
requests and 128 permutations without a difference. Detailed verification is in the
[custody engine checkpoint](../reference/custody-engine.md#recorded-pair-checkpoint-r2c-slice-1).
Full census/shadow shards remain for the orchestrator outside this sandbox.

The recorded-fan-out checkpoint ports that producer in isolation via
`interpretation/fanout.rs`, retaining exact ids, first-seen scope order,
amount/id leg order, first-leg fees, raw integer conversion failures and
Python decimal split pricing. It adds 47 raw-snapshot witnesses, 33 direct
deriver witnesses and 50 named mutation entries. Snapshot v1 remains raw-only;
the new seam is `partial.interpretation.fanout`; production interpretation stays
skipped. The new 2,000-book corpus reaches 1,244 active, 126 blocked and 630
inactive books, with 94 permutations. Every scenario in each interpreter
generator now has exact activity assertions.

All 282 mutations are killed after 237 baseline selections pass. Formatting,
workspace clippy, 625 workspace tests outside `kassiber-net`, the generated
comparison suites, focused custody Python modules, recorded/permuted replay,
inventory/contracts and ledger feature comparisons pass. Full census and
custody-shadow shards remain assigned to the orchestrator outside this sandbox.

That checkpoint stopped at the recorded-fan-out commit boundary, leaving ownership and
multi-source consolidation unported. First complete their graph prerequisites:
Liquid raw-transaction decoding, exact graph-conflict details, Liquid scope
recovery and index lookup precedence. Then port those producers and their
consolidation -> fan-out -> ownership composition. No incomplete ownership
producer is enabled. Continue with raw holds/privacy; channel lifecycle/loans;
HTLC transition production; native component reconciliation and final
compilation. Detailed branch accounting, per-scenario generator counts and
verification live in the
[custody engine checkpoint](../reference/custody-engine.md#recorded-fan-out-checkpoint-r2c-slice-2-partial).

The subsequent interpretation rebase retains every slice on the fixed components
tip `16b5b57d`. A separate audit removes raw-fact `wide()` reads, fixes conversion
order and Python exception families, and applies CPython casing throughout.
The combined mutation catalog has 318 entries (167 evidence/components, 139
retained interpretation, twelve audit regressions). Generated comparisons pass
10,000 books with per-scenario activation assertions; recorded replay passes
195 requests and 195 permutations. Snapshot v1 stays raw-only and interpretation
stays skipped. See the [rebase audit](../reference/custody-r2c-rebase.md) for
verification limits and the next slice's Liquid decoder compatibility findings;
ownership, consolidation and their composition are still unported.

The next prerequisite checkpoint adds a dependency-free compatible Liquid
decoder (3,744 Python vectors), exact graph-conflict labels, scope recovery and
index precedence (297 graph witnesses). A new 2,000-book generator asserts 250
successful Python ownership/consolidation cases per scenario on Bitcoin and
Liquid while comparing only Rust's graph prerequisites. The mutation catalog
adds 28 decoder, 42 graph and one receipt precision entry to the retained 318.
Snapshot v1 stays raw-only and interpretation stays skipped. The
[decoder/graph checkpoint](../reference/custody-r2c-decoder-graph.md) records the
crate deviations, remaining stored-mapping admission audit and verification;
ownership and consolidation production/composition remain to be ported.

The subsequent [admission/arithmetic audit](../reference/custody-r2c-admission-arithmetic.md)
keeps Python scalar types through stored-mapping parsing and profile merges,
preserves nonfinite index error families and admission order, and corrects
exact numeric equality and asset-string admission. Its graph corpus contains
497 witnesses after the separate graph arithmetic audit: exact unbounded native
deltas and shared-fee calculations, numeric fee equality, shape precision and
error ordering. Five full-evidence fixtures also exercise the arithmetic fixes.
The catalog retains the original 389 rules plus twelve admission and 37 arithmetic
mutations. Ownership production, consolidation production and their composition
remain unported. Snapshot v1 and the skipped interpretation stage are unchanged.

The [ownership production checkpoint](../reference/custody-r2c-ownership.md)
ports that producer through the partial seam, including exact per-asset Liquid
conservation, source checks, destination reuse/decline/synthesis, cloned row
pricing, fee/residual handling and tentative-receipt rollback. The graph
corpus asserts 250 successful results in **each engine** for each of the six
ownership scenarios (Bitcoin, Liquid and four Liquid raw-byte variants).
Consolidation retains its two 250-success Python controls and remains unported.
The catalog retains all 438 prior rules and adds 120 ownership branch witnesses,
all killed by named tests.
Snapshot v1 stays raw-only; Python remains authoritative and `interpretation`
stays skipped. Consolidation production is next, followed by the consolidation
-> recorded fan-out -> ownership composition. Ownership stops at its own verified
commit; this checkpoint does not start consolidation.

The [consolidation production checkpoint](../reference/custody-r2c-consolidation.md)
ports multi-source production through the partial seam. Both Bitcoin and Liquid
consolidation controls now assert 250 successful results per engine, with two
claims per success. Collaborative own-delta qualification, canonical versus
legacy fee treatment, complete destination admission and exact FIFO cells are
recomputed from raw facts. It adds 84 named-test mutation entries to the retained
558. Snapshot v1 remains raw-only and `interpretation` remains skipped. The
consolidation -> recorded fan-out -> ownership row pipeline follows its own
verified consolidation commit.

The subsequent [producer composition checkpoint](../reference/custody-r2c-composition.md)
composes consolidation -> recorded fan-out -> ownership, including row flow,
handled ids, canonical fee normalization, residual annotations and atomic event
quarantines. Its 2,000-book corpus asserts per-scenario successes separately in
Python and Rust: 1,000 consolidations, 668 fan-outs and 834 ownership results
per engine. Twenty-four controls match the full Python interpreter's claims and
audits. It adds 39 killed pipeline mutations to the retained 642. The product
still skips `interpretation`. The subsequent
[safety checkpoint](../reference/custody-r2c-safety.md) ports raw observation
holds, privacy boundaries and fixed-point dependency closure in Python order.
Its 39-scenario, 2,000-book corpus pins producer counts separately for Python
and Rust, with 78 permutations and full-Python controls; 249 direct helper
witnesses cover scope, confirmation precedence, whole-group filtering and
transient propagation. It adds 114 safety mutation entries to the retained 681.
Snapshot v1 stays raw-only and `interpretation` remains skipped. The subsequent
[channel and loan checkpoint](../reference/custody-engine.md#channel-lifecycle-and-loan-roles-checkpoint-r2c-partial)
ports opens, closes, curated force-close sweeps, batched balances, one close fee,
strict adapter atomicity and persisted loan-role reservations/non-events through
the partial seam in Python order. Its 2,000-book generator asserts all 32
scenarios independently for both engines, including mixed transfer producers and
authoring direction controls. Splice/other record tags follow Python's funding
branch; no new splice semantics or ownership inference is added. Snapshot v1
stays raw-only and production still skips `interpretation`. Continue with
native transition production, component reconciliation and final
composition.

After interpretation, port gap search, quantity arbitration and finalized
projection, then join custody with `ledger`.

## Risks

- **Scale.** About 218,000 lines of Python, 313 CLI leaves, 303 daemon kinds,
  and about 100 schema tables. Per-group acceptance on native Rust execution
  keeps this measurable.
- **A long stack against a moving `main`.** Python features keep landing on
  `main` during the port, and a feature inside an existing command keeps the
  same coverage while changing behavior. Every change on `main` that touches
  migrated behavior carries a Rust equivalent or a tracked parity blocker;
  plan 20's semantic changes stay separate from migration parity; the stack
  tip is validated against a recorded `main` commit regularly; retirement
  needs a final synchronized baseline.
- **Mostly white-box tests.** About 160 of 273 test modules call Python
  internals, and the flagship custody tests call services directly. Recording
  cannot recover their coverage; the per-test disposition inventory and
  per-group migration carry it.
- **Recorder privacy.** Full data roots, environments, and stdin can hold
  descriptors, credentials, and decrypted books. Committed recordings use
  synthetic data; owner-book comparisons stay local and ephemeral.
- **Byte-exact text.** Hashes and stored JSON depend on Python's `json.dumps`
  (254 call sites) and float `repr`. `crates/pyjson` and its vectors carry
  this.
- **Parsing leniency.** `datetime.fromisoformat`, `csv.Sniffer`, argparse's
  prefix matching of long options, and RP2-era label rules accept specific
  shapes that Rust parsers do not by default; each needs vectors.
- **TLS.** `rustls` rejects certificates OpenSSL accepts, such as LND's
  self-signed certificate; custom trust replaces system trust for some
  backends and adds to it for LND. Each case needs a test before the switch.
- **SQLCipher compatibility** between `sqlcipher3` and `rusqlite`'s bundled
  SQLCipher (compatibility 4, 256,000 KDF iterations, 4096-byte pages).
- **Descriptor derivation** moves from `embit` to `miniscript`; address and
  script vectors from Python pin it.
- **Keyring and platform identity.** Service and account names, the item
  formats Python's `keyring` wrote on Windows and Secret Service, the macOS
  keychain access rules, and the Touch ID helper's check of the sidecar's
  signing identity must carry over; items Python stored must still unlock.
- **PDF and SVG output.** No Rust library matches ReportLab's layout engine,
  so the four ReportLab reports and the source-of-funds SVG diagrams are
  rewritten. Extracted text, tables, and page
  structure are compared, and the owner reviews the new layouts.
- **Toolchain bump** from Rust 1.77 touches the desktop build too. Done in
  R0; newer dependency lines (PyO3 beyond 0.25) are separate changes.

## Decisions this changes

- [Stack ADR](01-stack-decision.md): the sidecar becomes a Rust binary, and
  storage, custody, and tax logic move to Rust.
- [Plan 20](20-kassiber-tax-engine.md): its out-of-scope line for custody,
  storage, the daemon, and the CLI is superseded; the tax engine becomes one
  crate of the backend.

## Out of scope

- Replacing the webview UI or the TypeScript provider broker.
- New features, and semantic changes beyond plan 20 phase 5's non-legal
  fixes.
- Linking the daemon into the desktop process.
- Mobile.
