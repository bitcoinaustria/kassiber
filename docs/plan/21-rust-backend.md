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
   scenario, a Rust service or property test, or a retained oracle test. Each
   group's tests migrate with its implementation, not at cutover. Progress is
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
7. **Storage durability:** beyond equal content, the storage stage proves
   schema objects and trigger behavior, migration audits and idempotency,
   foreign-key and integrity checks, busy and error semantics, the journal
   mode chosen from the embedded SQLite version, rollback and failure
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
     test's disposition.
   - Build the session recorder and replay harness, prove Python-to-fresh-
     Python replay and discrepancy injection, and add the `KASSIBER_TEST_CLI`
     switch.
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
   then join custody and the tax engine in one `journal` operation.
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
  evidence snapshots, migration quarantines, dismissed gap fingerprints, and
  pair dismissals;
- for books above the gap search's row limit, the SQL-selected worklist.

The response carries each stage's result: the materialized components and
their blockers, the interpreter compilation, the quantity state, and the
finalized tax projection, plus warnings and the gap-ignored id sets. Once
every stage compares clean, the joined `journal` operation also carries the
tax ledger, the wallet balance comparison, and the ownership-review
projection.

## Risks

- **Scale.** About 218,000 lines of Python, 313 CLI leaves, 303 daemon kinds,
  91 tables. Per-group acceptance on native Rust execution keeps this
  measurable.
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
- **Toolchain bump** from Rust 1.77 touches the desktop build too.

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
