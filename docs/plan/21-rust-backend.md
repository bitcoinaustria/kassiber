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
   The HTTPS/HTTP foundation now layers rustls and HTTP/1.1 over that guarded
   connector; service slices remain disabled until their own consent and behavior
   contracts are implemented. Rust offline mode also blocks local AI.
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
  touches a project; there is no Python fallback. `commands describe` now returns
  the complete Python metadata catalog; execution support is tracked separately
  and unported commands still fail before touching a project.
- **Store seam.** The shell calls a narrow `BookStore` trait in
  `crates/runtime` (bootstrap, unlock mode, ownership, the CLI and daemon
  unlock paths, passphrase verification, status facts). The binary now implements
  it with the real storage crates for supported books. A feature-gated fake
  remains only for deterministic protocol and resource-failure tests. `paths`
  owns root resolution; `runtime` owns the seam.
- **Redaction now.** The daemon echoes request text into error envelopes, so
  `redaction.py`'s rules are ported with the envelope rather than deferred.
- **Test harness.** One `test-harness` feature adds the egress observer, the
  fake store, and protocol test kinds; a client applying the desktop
  supervisor's routing rules drives the real binary, and
  `tests/test_rust_shell_parity.py` compares it with Python when
  `KASSIBER_RUST_BIN` is set. The binary carries the product version.
- **Differences proposed for review** are listed in the reference: a pure
  `commands describe`, and answers where Python's daemon hangs or crashes.

### R1 storage decisions

- **Existing-book runtime.** `kassiber-paths`, `kassiber-ownership`,
  `kassiber-store`, and `kassiber-secrets` expose root resolution, compatible
  role leases, SQLCipher/plaintext opens, transactions/backup, and zeroized
  unlock sessions to the CLI and daemon. Python product behavior is unchanged;
  the [Rust store reference](../reference/rust-store.md) owns the public API.
- **Explicit bootstrap and admission.** Resolution stays read-only. The binary's
  explicit `init`, `status` and daemon-open paths now create the Python schema,
  run the ordered supported compatibility pass, and migrate default state and
  legacy project layouts. The admission-only library API remains separate.
  Structural/semantic checks and unsupported custody-layout refusals remain;
  this integration does not complete the full R3 acceptance matrix.
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
  separately. Resolution stays read-only; command bootstrap explicitly selects
  supported creation/migration and preserves the remaining refusal barriers.
- Leave Python DDL in place. Embed a generated empty-book write transcript
  with a Python drift test; never replay it on existing schemas. Preserve
  complete `sqlite_master` SQL, including whitespace, and inject identity and
  audit time before parity tests rather than masking persisted values.
- R3 remains in progress until historical migration and the full storage
  durability matrix pass. Empty-book parity alone is not historical parity.
- Authored legacy pair/payout consolidation, immutable revisions/issues, and
  exact matcher-based componentless dismissal recovery move after R2's custody
  matcher port. Until then Rust returns `unsupported_schema_layout` for books
  reaching those steps; Python must migrate them. The ordered R3 pass
  proves exact prefixes for both historical fixtures, including SQLCipher
  copies, but does not approximate these custody operations.
- The historical custody-view blocker is resolved: Python and Rust now create
  the projection relation view directly after the suspense-role rebuild.
  Both pair-free historical fixtures pass exact full first-pass dumps and
  unchanged Python reopen for plaintext and SQLCipher, without masking values
  or adding a reference pass. Both original fixtures still prove exact prefixes
  followed by the explicit R2 refusal. The real binary now exercises the supported
  pass and project migration. Full original-book migration and the complete
  crash/backup/native-platform matrices remain R3 gates. See the
  [ordered-pass status](../reference/rust-storage-r3-status.md#ordered-pass-attempt).
- Legacy root cutover uses Python's prepared proof and rename-only protocol.
  Preserve file identities and fixed runtime paths; refuse merges and retain
  legacy authority on cross-device failure. Resolution itself remains read-only.
- R3b legacy project migration now has 20 layout/workspace/codec Python
  reference cases and 20 Rust parity cases. Database equality uses complete
  normalized logical dumps, including internal tables, storage classes and
  float hex; physical SQLite counters and SQLCipher pages may change. All
  non-database bytes and modes stay exact.
- The standalone Python recovery fix records pending artifact moves atomically
  in the target marker before cleanup, moves the database last, and resumes
  before catalog selection. Reordering alone cannot finish the final-move
  marker window. Both runtimes resume the same protocol. The Rust
  port passes the publication/move and deferred-validation interruption matrix;
  read-only resolution, final runtime admission and Python-safe journals stay
  intact.
  See [R3b acceptance](../reference/rust-storage-r3-status.md#r3b-legacy-project-migration).
- Multiple workspaces remain in one default project. Network partitioning in
  `book_network_migration.py` is an explicit encrypted export, owned by R5
  exports; no automatic network partition migration is introduced.

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
stays raw-only and production still skips `interpretation`. The subsequent
[native HTLC checkpoint](../reference/custody-engine.md#native-htlc-production-and-reservations-checkpoint-r2c-partial)
ports full-population hash/refund matching, independent provider conflicts,
canonical quantity/fee-date guards and dismissal expiry. Reservations compose
before graph derivation and before holds/claim arbitration, in Python order.
Its 55-scenario, 2,000-book generator pins counts for both engines, with 110
permutations and 110 full-Python controls. LND/CLN settlement and timeout refunds
compare; NWC is a negative authority control because Python admits no native
NWC payment-hash row. Failed attempts contribute no native transition.
Snapshot v1 remains raw-only, Python remains authoritative, and `interpretation`
stays skipped at that checkpoint. The following checkpoint completes native
component reconciliation and final composition.

#### Complete interpretation checkpoint (R2c)

Branch `t3code/r2c-interpretation-12`, based on `cc18b710`, completes native
component reconciliation, the private reviewed quantity compiler, exact FIFO,
chronology and full interpreter compilation in Python precedence. It promotes
`interpretation` into the PyO3 implemented-stage inventory and the census/shadow
comparison, retiring the partial runtime seam while retaining frozen vectors.
Python remains authoritative and Rust results never reach stored state; v1
snapshots remain raw-only.

All eleven generators match full output for 22,000 books and 966 permutations,
with independently asserted per-scenario counts for both engines. Recorded
replay matches 88 requests and 88 permutations. All 1,178 mutations (1,035
retained plus 143 new) have named-test kills. Workspace formatting, Clippy and
740 offline tests outside `kassiber-net` pass. Focused census and shadow compare
78 and 76 complete interpretations respectively. The broad Python run retains
four account-home owner-lock sandbox failures, documented honestly with all
counts, branch dispositions and remaining verification in the
[completion checkpoint](../reference/custody-engine.md#complete-interpretation-checkpoint-r2c).
Full census/shadow shards and the four nonsandboxed daemon reruns belong to the
orchestrator.

The following checkpoint completes gap search.

#### Gap search checkpoint (R2d)

Branch `t3code/r2d-gaps`, based on `16234030`, ports bounded gap discovery,
candidate scoring, matcher capacity, source blockers and the two ignored-id
populations. `gaps` is now implemented in PyO3 and compared in census and
custody-shadow. Pure helpers also match seeded keyset pagination, fingerprints,
latest-review dismissal revisions and privacy-safe candidate display. Python
still owns SQL worklist selection, review writes, readiness and activation.
Snapshot v1 already carries the raw facts; no schema or dependency changes.

All twelve generators match exact gap output for 24,000 books and 1,030
permutations, with independent Python/Rust counts for 515 scenarios, including
Samourai/Whirlpool missing-wallet bridges and capacity-only source blockers.
Recorded replay matches 79 requests and 79 permutations. Adversarial 60,301-row
searches retain Python's inspection ceilings and pass timing smoke tests.
The mutation catalog contains 1,315 entries: all 137 new entries and 16 affected
retained entries have been re-killed; the other retained kills are inherited
from R2c, not claimed as rerun here. Full census/shadow shards remain with the
orchestrator. See the [gap checkpoint](../reference/custody-engine.md#gap-search-checkpoint-r2d)
for branch dispositions, counts and verification.

Next port quantity arbitration and finalized projection, then join custody with
`ledger` in the journal operation.

### R5 rates checkpoint (partial)

The `t3code/r5-rates` slice starts from network tip `1409ffef` and adds
`crates/rates`, six implemented CLI leaves, four desktop daemon kinds, and a
shared exact cache/lookup API. Coinbase Exchange, CoinGecko and mempool
requests use `kassiber-net` with book/action/source/pair-bound consent; manual,
CSV, cache and desktop display reads are inert. Enabled refresh requires and
rechecks the per-profile market-rate feature. AI projections remain Python.

Native tests, a killed nine-entry selection/fallback mutation catalog,
hybrid served-by assertions and direct daemon byte comparisons accompany this
checkpoint. A separate Python regression fixes acceptance of negative Kraken
OHLC prices. The [shell status](../reference/rust-shell.md#rates-service-checkpoint-r5-partial)
records API/selection details and open acceptance, and the
[egress inventory](../reference/rust-port-egress.md#rates-service-paths-r5)
records the consent regressions.

Rates remains partial: desktop rebuild needs the unported pricing/journal
services; complete native intent coverage, provider error/date-retry parity,
archive edge/resource acceptance, label-context parity and installed bundled
resource-path parity remain gates. Unported operations keep failing closed.

### R5 encryption checkpoint (partial)

`t3code/r5-secrets`, based at `0d4fbc28`, implements only `secrets init`,
`init-resume`, and `verify` through a dedicated CLI family and shared secrets
service. It reuses R3 encryption/export and ownership, adds verified private
publication for empty containers, and exposes a non-migrating integrity reader.
Fourteen empty-publication boundaries receive fifty killed-child recovery cases
through either runtime. Eight secret-handling/publication/rekey mutations have
named killing tests; the rekey tests cover R3's primitive, not a ported rotation
command. Python product code is unchanged.

The 62-case hybrid scenario module records 23 Rust calls (14 `init`, four
`init-resume`, five `verify`) and zero Python fallbacks. Local workspace tests
pass 853 cases with the two socket-dependent crates excluded; the full
workspace gate still requires an environment permitting their local sockets.
The required shell/hybrid/runner/inventory suite passes 63 tests and 178
subtests; full slice acceptance remains open below.

At that checkpoint this was a clean partial boundary, **not acceptance of the requested service
slice**. Five secrets leaves, both age bundle leaves, maintenance, diagnostics
and its explicit export flag, and all desktop kinds remain unported. Bundle
round trips, archive safety mutations, remembered-credential lifecycle and its
cross-runtime publication recovery remain required. See the [shell checkpoint](../reference/rust-shell.md#encryption-service-checkpoint-r5-partial)
for the exact boundary, mutations, verification and remaining work.

### R5 secrets lifecycle continuation

`t3code/r5-secrets-2`, based at `96461ae3`, adds the remaining five secrets CLI
commands and the existing desktop rotation/revocation kinds through shared
secrets services. Descriptor/TTY-only input, authenticated rekey callbacks,
Python's DB-bound native item identities, independent stale guards, legacy
cleanup/quarantine and dotenv credential migration retain their boundaries.
Python product code and AI projections are unchanged. Dotenv publication uses
complete flushed staging and refuses conflicting snapshots or observed source edits;
the [shell continuation](../reference/rust-shell.md#secrets-lifecycle-continuation-r5)
records those conservative differences, per-command routing and the named
mutation table.

The hybrid modules pass 91 cases with zero Python fallbacks for each new
command and the daemon session. Seventy-two killed-child cases include the
initial fifty plus 22 new credential-publication/guard/rekey recovery cases
through either runtime. All 18 mutations are killed. Twenty-nine additional
native interop/interruption cases are gated on a newly launched disposable
Linux keyring; this sandbox cannot bind its D-Bus socket. Native platform/TTY
acceptance, complete slice dispositions and error parity remain open. This
does not complete backup, maintenance, diagnostics or `ui.secrets.init`.
### R5 reports: first read boundary (partial)

`t3code/r5-reports` starts at `0d4fbc28` and adds `crates/reports` over the
real store. Five CLI leaves (`summary`, `capital-gains`, `balance-sheet`,
`portfolio-summary`, `filed-snapshots list`) and three desktop kinds
(`ui.reports.summary`, `ui.reports.balance_sheet`,
`ui.reports.portfolio_summary`) delegate to the same stored-journal service.
Summary includes JSON and its line-oriented table/plain presentation. No
report read starts acquisition, repricing, journal processing or a connector.
Journals still come from Python in the hybrid candidate. AI projections stay
in Python.

The live reads retain network review, component integrity, persisted quantity
blockers and journal timestamp/count/input-version admission, in reference
order. Capital-gains split rows retain occurred/created/id ordering and
version-bound explanation references. Holdings retain exact cached/fallback
rate precedence and the reference Decimal-to-float boundaries. Saved/filed
snapshot lists expose immutable captured summaries even when live journals
are stale; no snapshot or amendment is updated.

The continuation on `t3code/r5-reports-2`, based at `571190d9`, adds four
stored reads: `reports tax-summary`, `austrian-e1kv`, `austrian-tax-summary`
and the complete `journal-entries` listing. `ui.reports.tax_summary` uses
the same service, including Python-compatible year filtering. Austrian forms
map stored categories to current Kennzahlen, disclose stored-code mismatches
and quarantines, retain the Alt/Neu/income sections and round each detail to
integer euro cents with HALF_UP before aggregation. Vienna dates use frozen
timezone transitions and the future EU rule; no host timezone lookup or
repricing runs during reads. Generic summaries retain the stored-summary
adjustments and the reference float boundaries. The new leaves support
JSON/table/plain; their CSV mode remains `not_implemented` with the exports.
There are now nine native report CLI leaves and four native desktop kinds
(80 native CLI leaves overall). AI remains Python; no schema, dependency,
snapshot, amendment or journal producer changes are included.

The continuation's twelve report oracle scenarios match Python at 64 and
3,000 transactions on FIFO, HIFO and Austrian moving-average books, with
107 native report calls and zero fallback for ported reads. Its 174 mutation
entries have named kill evidence across catalog and targeted runs; the
checkpoint records the scratch-copy timing limitation and the unrelated
Python/sandbox failures. Native formatting, both clippy configurations and
all 949 workspace tests pass.

The third continuation (`t3code/r5-reports-3`, based on `89f978ac`) adds
`reports balance-history` and `ui.reports.balance_history`, reconstructing
historical holdings from stored journals, with compensated income/transfer-fee
quantities and profile-wide pooled basis for wallet/account filters. The read
holds the same admission savepoint and never rebuilds journals or acquires
prices. Existing portfolio CLI/desktop contracts have no as-of argument;
the core-only as-of portfolio API gains no invented process surface.

Its twelve report process scenarios pass at 64 and 3,000 transactions on FIFO,
HIFO and Austrian moving-average books. All 154 ported CLI calls (47 history)
and 207 desktop responses (150 history) run natively with zero fallback. The
316 frozen cases include 171 historical/desktop cases; every vector verifies
that reads make no writes. Seventy-five new mutation entries cover historical
admission, quantities, basis, ordering, calendar buckets, quote selection and
desktop projection, bringing the report catalog to 249. Formatting, both
clippy configurations, all 956 workspace tests, the harness tests/build and
the focused inventory/contract checks pass. The required shell/hybrid suite
passes 63 tests and 178 subtests; the broader hybrid report/AI smoke and
historical regressions pass 129 tests and four subtests.
All 249 mutations have named kill evidence: the full audit kills all 174
inherited rules, and the final historical-only run kills all 75 new rules
after fixture/mutant corrections. This is aggregate evidence across runs;
the checkpoint records the corrected cases and discarded cache baseline.

Individual snapshot lookup and custody amendment-review state are explicitly
deferred. This stack is phase 4: the phase-5 upgrade-impact model lives on
separate, unstacked branches, and these core APIs have no CLI or desktop read
contract at this base. Port them when an actual process caller is ported and
the stack is restacked onto phase 5. Snapshot listing remains supported;
saved/filed captures and impact/resolution rows remain append-only.

The third continuation also diagnoses the earlier AI-provider smoke failure:
Python chat cannot create its account-home owner-lock directory in the
read-only sandbox, before it reaches provider acknowledgement. Python-only
and hybrid reruns pass with that filesystem access; all nine provider/chat
calls route to Python. No Python reference bug or routing fix is needed.

The fourth continuation (`t3code/r5-reports-4`, based on `6d0d7c88`) enables
`--format csv` on all ten ported report leaves, including the Austrian detail
and summary projections. The CLI reuses Python-compatible CSV rendering;
explicit report CSV `--output` files use the shared atomic storage writer, without
changing export-directory permissions. Explicit output symlinks keep their
destination semantics. CSV reads retain all readiness barriers and neither
register saved artifacts nor mark reports filed. The 64/3,000-transaction
FIFO/HIFO/Austrian process oracle and 29 named CSV/publication mutations cover
this boundary; the catalog contains 278 entries including the 249 inherited
rules. This is a partial checkpoint within requested step 1, not completion of
the CSV artifact, snapshot-mutation or daemon-export steps. The
[CSV checkpoint](../reference/rust-shell.md#reports-tabular-csv-checkpoint-r5-fourth-continuation-partial)
records verification and the precise remaining work.

The fifth continuation (`t3code/r5-reports-5`, based on `349bf2ca`) ports
`reports filed-snapshots create`: external saved/filed markers and explicit filing
of captured local exports. Bounded summaries and scope serialize through
`crates/pyjson`; filing inherits sealed identity and copies frozen dependencies
in an atomic append, preserving existing impact/resolution history. The 182-case
write oracle and FIFO/HIFO/Austrian process sequences compare raw stored rows,
and the two formerly Python-served creation fixtures now run in Rust.
Automatic export-completion saving, app-computed hashes/summaries, dependency
capture, artifact CSV exporters and daemon export kinds remain open. See the
[snapshot-write checkpoint](../reference/rust-shell.md#reports-explicit-snapshot-write-checkpoint-r5-fifth-continuation-partial)
for verification and the exact remaining boundary.

The final snapshot checkpoint passes all 22 report process scenarios: 309 report
CLI calls are Rust-served with zero fallback. All 355 report mutations (278
inherited and 77 new) are killed in one fresh-target audit. Workspace formatting,
both clippy configurations, 966 workspace tests, 14 harness tests, the required
shell/hybrid suite, and inventory/contracts pass. This evidence accepts the
explicit marker boundary only; it does not complete the requested export slice.

The sixth continuation (`t3code/r5-reports-6`, based on `3b20eecc`) adds an
internal CSV preparation boundary for the twelve-section full report and all
fifteen Austrian bundle files. It reuses the stored-journal report services,
retains readiness admission in one read savepoint, and neither publishes files
nor registers snapshots. Its Python-generated oracle checks 123 preparation
cases and 543 file contents, including quoting, exact prices, wallet scope,
history limits, booked transfer labels and Austrian rendering. These are frozen
synthetic journal fixtures, not completed export sequences on processed books.
The transaction-sheet builder is preparation for the full report only; standalone
transaction draft/range behavior is still unported. No CLI command or daemon
kind is enabled. The
[CSV preparation checkpoint](../reference/rust-shell.md#reports-csv-preparation-checkpoint-r5-sixth-continuation-partial)
records the verification and remaining publication/save boundary. The fresh
mutation audit kills all 435 entries (355 inherited and 80 renderer rules),
after all 37 baseline selections pass. Formatting, both clippy configurations,
969 workspace tests, 14 harness tests, the 22 report process scenarios and the
required shell/hybrid/runner/port suite pass; inventory and contracts pass too.

The seventh continuation adds internal atomic CSV publication, preserving
Python's ordinary successful bytes, modes and final-symlink destinations. The
same 543 frozen files now pass through disk publication. Eleven new mutation
rules are killed by named native tests. Preparation/publication does not write
snapshot or dependency rows, and export commands/kinds remain disabled until
completion binds publication to the admitted book version. Python's direct CSV
writer truncates old artifacts on failure; the
[publication checkpoint](../reference/rust-shell.md#reports-csv-publication-checkpoint-r5-seventh-continuation-partial)
records a disposable repro, the intentional atomic failure difference, the
inode/ownership parity limit, and remaining completion/transport work.

This is **not full reports slice acceptance**. Exit-tax outputs,
capital-gain explanations and desktop capital gains remain unported.
So do privacy, Lightning and cross-check reads, commercial/source-funds reads,
all PDF/XLSX/artifact exports (including full-report and Austrian bundle CSV),
standalone transaction CSV, automatic export-completion saved registration and every
network-dependent operation. Unsupported commands retain `not_implemented`;
the [continuation checkpoint](../reference/rust-shell.md#reports-austrian-and-journal-read-checkpoint-r5-continuation-partial)
records exact parity, mutations, routing counts and verification limits.
The [historical read checkpoint](../reference/rust-shell.md#reports-historical-read-checkpoint-r5-third-continuation-partial)
records this continuation's process-boundary evidence and deferrals.

### R5 read-path performance

Existing-book opens compare exact current schema definitions before falling
back to the normalized structural gate. Compatibility repairs avoid replacing
unchanged indexes, views and triggers, reuse column inventories within ordered
ranges, and probe Liquid assets through existing covering indexes. All audit,
semantic, ownership, unlock and no-egress gates remain in place; no schema or
dependency changes are introduced.

Report admission uses the same network evidence interpreter without building
the review-only digest and presentation payload. Neutral observations reuse
wallet/asset evidence facts within that read, without caching admission across
reads. Holdings stream candidate quotes with the original error precedence;
summary reuses ordered capital-gains rows without constructing unused public
references. Python accounting and report output remain the reference. See
[measurements and verification](../reference/rust-read-performance.md).

Round 2 streams admission observations, summary gains and history projections
through reusable row dictionaries, scales SQLite msat quantities exactly, and
loads wide transaction fields only after selecting page rowids. Gates, error
precedence and ordered arithmetic remain intact. The
[round 2 report](../reference/rust-read-performance-round2.md) records phase
spans, parity and measured index proposals; no proposed DDL is applied.

The eighth reports continuation cherry-picks the fixed Python atomic CSV writer
and matches replacement of existing read-only artifacts while retaining their
modes. Standalone transaction CSV preparation adds draft withholding, ordered
multiple-wallet selection and resolved-range filtering, with frozen byte/error
oracles and named mutations. It remains unregistered: completion saves,
authoritative dependencies, admission retained through publication/completion,
complete DB export sequences and transport/hybrid proof still gate registration.
See the [standalone CSV checkpoint](../reference/rust-shell.md#standalone-transaction-csv-preparation-r5-eighth-continuation-partial).

The ninth reports continuation implements saved CSV completion (34 Python
vectors, actual file/manifest hashes, exact summaries and authoritative
dependencies) and calendar ranges (824 cases over 25 frozen zones). The tenth
adds nine whole-export sequences comparing 36 actions, 51 files and every one
of 112 SQLite tables after each action, including repeated explicit filing and
failed completion. One admitted write transaction spans preparation,
publication and completion; stale input versions fail before publication.
See the [completion checkpoint](../reference/rust-shell.md#saved-csv-completion-and-export-sequence-checkpoint-r5-ninthtenth-continuations)
for verification and the deferred XLSX/PDF rendering, font and parity needs.
The tenth continuation also registers the two CLI report CSV artifacts,
transaction export's CSV form and four desktop CSV kinds, including capital
gains. Thirteen process scenarios compare wire output, literal files and full
normalized DB state; the routing log proves zero Python fallbacks for each
registered CSV command. Non-CSV transaction formats remain refused before
bootstrap. A repro records Python's nondeterministic staging-path error when a
CSV destination is an existing directory; exact error-byte parity remains open
for that case. See the [transport checkpoint](../reference/rust-shell.md#csv-export-transport-checkpoint-r5-tenth-continuation)
for per-command counts, mutations and final verification.

### Service slice contract (R4 onward)

From R4 on, work lands as vertical slices: one command family, or one
coherent group of families, through the CLI, the daemon and their AI
projections. Each slice follows the same contract so slices can be built in
parallel and stacked in any order.

- **Base and branch.** Start from the current stack tip and use one branch per
  slice. Commit locally; the stack is integrated in order.
- **Placement.** Domain logic goes in a crate named for its area
  (`crates/<area>`). Its commands register in `crates/cli` through one module
  per command family, and its daemon kinds in `crates/daemon` the same way, so
  parallel slices touch shared files only to add a registration line. Storage
  goes through `crates/store`, and paths through `crates/paths`. Python stays
  the reference, and this checkout's Python code is not modified except to fix
  a reference bug. Such a fix goes in its own commit with a regression test.
- **Parity.** The slice's entries in
  [test dispositions](../reference/rust-port.md) define its acceptance:
  - `boundary-fixture` and `subprocess-scenario` tests in the slice's group pass
    with `KASSIBER_TEST_CLI` naming the
    [hybrid candidate](../reference/testing.md#hybrid-rust-candidate), and its
    `KASSIBER_HYBRID_LOG` shows the slice's own commands were served by Rust.
    Setup commands another slice has not ported yet fall back to Python on the
    same book;
  - `rust-test` entries are ported as Rust tests in the owning crate;
  - `oracle` entries keep comparing the two implementations;
  - `python-only` entries list their Rust replacement.

  Daemon kinds are compared through the daemon smoke sessions the same way.
  Output must match byte for byte where the harness compares bytes.
- **Computation.** Any slice with accounting, matching or selection logic
  ships checked-in mutation entries, each killed by named tests, as R2 does.
- **Egress.** Every connection goes through the `kassiber-net` gate, with
  consent bound to the exact user action. Each new egress path adds a test
  proving no traffic before consent.
- **Fail closed.** A command the slice does not port keeps answering
  `not_implemented`. A command the slice ports must not silently accept input
  Python refuses.
- **Done means:**
  - fmt, clippy with warnings as errors, and workspace tests pass;
  - the slice's parity tests pass against the Rust binary;
  - the inventory and contract checks pass;
  - `crates/cli/src/describe.rs` lists the newly implemented commands;
  - rust-shell.md and this plan record what changed.

### R5 setup: first partial boundary

`kassiber-setup` now owns workspace/profile/account/context/saved-view services.
All 14 CLI leaves of these five families are registered individually; the
JSONL daemon registers four profile mutation kinds and three saved-view kinds.
Both callers use the same local services through the book seam. Scope lookup
has one implementation, matching the base's ID-first, case-folded-label
ambiguity rules rather than anticipating the phase-5 resolver on another branch.

Policy edits preserve Python's journal invalidation and stored-method rules.
Every operation has an atomic database boundary. No service creates network
clients or probes, and backend configuration/secrets are not newly exposed.
Python product code is unchanged. The setup crate reuses existing internal
storage and Python-compatible text/JSON crates; no external dependency is added.

Evidence: 25 Rust tests, a 71-operation deterministic Python trace with complete
affected-table comparisons, six killed mutations, real-book byte comparisons
through CLI and daemon, and hybrid CLI execution. The
[Rust shell reference](../reference/rust-shell.md#setup-parity-and-remaining-work)
owns the detailed coverage and limits.

The continuation adds the complete 313-command Python catalog and all four
`projects` CLI leaves. Catalog metadata includes unported families and exact
prefix queries, without enabling their execution. Project commands use the
same setup crate through the local, no-database service seam. They preserve
selection, explicit paths, metadata refresh and descriptor-only encrypted
creation. Eleven native project tests, two byte/state process oracles and five
new selection mutations extend the earlier evidence; all eleven setup
mutations are killed. No Python product code or external dependencies change.

The next continuation restores Python compatibility for owned `0644`
settings/catalog metadata. Admission now refuses group/world writes and
foreign ownership while accepting group/world reads. Regression tests cover
`0600`, `0644`, `0664`, `0666`, foreign-owner policy and unchanged read modes;
Rust's private writer continues publishing `0600` files.

The book-network continuation adds all five `networks` CLI leaves and six
`ui.networks.*` kinds through the same setup services. Evidence inventory,
immutable bindings, explicit unknown-wallet declarations, regtest instances,
preview commitments and journal invalidation match Python. Partition exports
preserve authored IDs and closure, exclude credentials and historical reports,
verify attachments, reset processing state and create a fresh database identity.
Encrypted books are keyed before rows reach the target. Private staging and
no-overwrite publication preserve the existing export boundary. This adds
`age`/`tar` for encryption/archives and reuses `tempfile` at runtime; recipient
mode keeps Python's external `age`/`rage` requirement. No transport or network
consent is added.

Evidence includes twelve native tests, 136 frozen responses and 69 evidence
vectors, ten new killed mutations (21 total), exact CLI/daemon process
comparisons and Python restoring a Rust-produced encrypted partition. The
hybrid witness serves all nine network CLI calls in Rust with zero Python
fallbacks. The Rust shell reference records per-command counts and limits.
Python product code is unchanged; no reference bug was found.

The offline continuation adds three CLI leaves and both machine-preference
desktop kinds through a no-database service seam. CLI layout bootstrap stays
compatible; encrypted books remain locked during desktop preference operations.
The bounded reader fails closed on malformed, oversized, symlinked and special
files. Private atomic writes and the separate process-override status match
Python. Seven native service tests, a FIFO-reader regression, a locked-daemon
process comparison and thirteen hybrid CLI calls (all Rust) pin the boundary.
Three new policy mutations bring the killed setup catalog to 24. Storage now
reuses the existing `tempfile` package at runtime for private publication.
This adds no egress path and leaves AI projections in Python.

The backend continuation adds all nine backend CLI leaves and seven desktop
configuration kinds through the shared setup service. Credentials preserve
Python's encrypted-table storage; normal projections retain allowlisted
redaction, while explicit reveal remains gated by book unlock. Bootstrap
promotion, ephemeral overrides, deletion references, derived-inventory
invalidation and default selection have native and process parity evidence.
Nine native tests and eight new mutations bring the killed catalog to 32.
The hybrid byte oracle serves all fifteen backend calls in Rust with zero
Python fallbacks. Broader hybrid backend CLI and desktop scenarios pass.
Existing internal crates and already-locked JSON support supply the port;
there is no new package or egress path. A separately committed Python reference
fix prevents successful backend edits from shutting down the desktop daemon.
Another separately committed reference fix replaces malformed URL/proxy
locations with a fixed redacted placeholder rather than echoing credentials;
Rust mirrors the corrected behavior with native and process regressions.
The next continuation adopts the refined Python reference redaction: unparsed
locations are visible only on a full ASCII bare-host[:port] match (including
bracketed IPv6 text, with one to five numeric port digits). Three mutations pin
visibility, full matching and port length.

Workspace desktop create/rename/delete, profile reset/snapshot/display labels,
owner-only `backends.reveal_token`, and Core credential references now use the
Rust services. Destructive confirmations target the exact current names;
reset preserves other books, connections and shared rates unless explicitly
cleared, and confines managed-file cleanup to the attachment root. Retained
legacy-ledger rows still block destruction. Display labels use the actual
stored accounting method. Reveal and destructive actions require fresh
passphrase verification with backoff or the exact plaintext acknowledgement,
after taking desktop ownership. Renderer and AI authority are unchanged.

Core references are opaque hashes of candidate identity, excluding passwords.
Python re-resolves them from current local configuration on backend creation;
there is no session credential store. Rust follows that behavior, replacing
the endpoint/network and conflicting authentication together. Detection and
all backend probes stay refused. Shared CPython ISO parsing moves to the text
crate for custody and profile display; only internal crate dependencies change.

Frozen desktop response/state vectors, native policy tests, and plaintext and
encrypted hybrid daemon sessions extend the parity evidence: 36 daemon
requests have zero Python fallbacks, and all 57 setup mutations are killed.
Fmt, both clippy configurations, the harness build, vectors, inventory and
contracts pass. The non-network workspace gate passes 527 tests; four existing
loopback tests hit sandbox `EPERM`. The requested combined Python suite has
61 passes and 59 subtests, with the known unported descriptor fallback blocked
by its account-home owner namespace. The Rust shell
reference records per-kind counts, named mutation killers and verification
limits. No additional Python reference bug was found beyond the supplied
bare-host refinement.

This is **not the completed setup slice**. AI projections remain in Python.
Import/sync network-admission integration belongs to those later service slices.
Keep networking probes refused until a consent-bound HTTP/transport slice is
implemented. Project desktop lifecycle and the remaining R3 dispositions still
belong to their runtime/storage boundary. Full mixed daemon sessions and
sandbox-blocked Python shards still require orchestrator verification.

### R5 partial local evidence boundary

The `t3code/r5-transactions` slice starts from `82f6b77b`. Its first verified
boundary ports the four `loans` and six `attachments` CLI leaves plus their
ten desktop/AI daemon kinds into `crates/transactions`, with one registration
module per family in each transport. Storage and path resolution use the R3
APIs. Loan marks preserve observations and invalidate journal inputs; attachment
reuse copies managed files into independent identities and URL references stay
local. Seven checked-in selection/storage mutations are killed by named tests.

The continuation `t3code/r5-transactions-2` adds 18 local metadata CLI leaves
and `ui.activity.stale`, sharing one domain module with atomic, append-only
audit writes and journal invalidation. Notes retain empty/null distinctions;
tag labels use CPython 3.11 casing and canonicalization; exclusion preserves
active custody reviews; kind overrides preserve imported provenance. Nine new
selection/state mutations are killed by named tests (16 with the predecessor).

The next continuation, `t3code/r5-transactions-3` at `185c3707`, adds record
pagination, history list/activity and forward-edit reverts (four CLI leaves),
plus `ui.transactions.history`, `ui.activity.history` and
`ui.transactions.history.revert`. History preserves redacted values, complete
event fields, report anchors, filter-bound CPython cursors and one audit/version
transition for grouped reverts. Frozen CPython 3.11 vectors cover timestamp
filters and legacy price formatting. Fourteen additional mutation entries cover
pagination, selection, forward edits and the custody-exclusion guard.
A separate Python reference fix prevents reverting an inclusion from excluding
a newly reviewed custody leg; its regression fails before the fix.

The next continuation, `t3code/r5-transactions-4` at `f417d50a`, adds the four
BIP329 leaves and existing `ui.metadata.bip329.preview/import/export` kinds.
Label normalization, duplicate merges, unknown JSON fields, conflict previews,
optional tag projection, append-only audit writes, list cursors, and all three
export modes follow Python on the ported local-index boundary. Exports preserve
Python JSONL bytes, stored/synthesized precedence and wallet selection order.
Twenty new mutations pin merge, matching, ordering and fail-closed rules (50
with predecessors). No dependency, schema, egress path or AI projection changes.

The next continuation, `t3code/r5-transactions-5` at `a6359897`, completes grouped
metadata updates and `ui.transactions.metadata.update` through the same typed
book service. It preserves explicit null versus omitted fields, canonical tag
merges, observed kind provenance, custody-safe exclusion, exact price strings
and the application's 32-digit decimal context. Tags, transaction columns,
ordered audit fields and journal invalidation share one savepoint. No-op saves,
including capture-time-only changes, append nothing. Twelve additional mutation
entries cover grouped selection, merges, order, arithmetic and custody guards
(62 with predecessors). The only new crate dependency is the existing workspace
decimal implementation; no third-party dependency, schema or egress path is added.
A separate Python reference-fix commit rejects nonfinite grouped prices with
typed validation; its regression failed before the fix.

The next continuation, `t3code/r5-transactions-6` at `17a69a38`, ports
`transactions list` through a typed book service and its CLI family module.
Observed pages retain scoped filters, four primary sorts and chronological
ties, Python cursor bytes and filter binding, total counts before pagination,
integer msat and exact decimal-to-float display. Custody flow filters use only
current stored projections. Seven Rust tests and 28 new mutation entries pin
the listing boundary. This is a partial CLI boundary: transaction desktop
projections and the complete document family remain unported. No Python
product source, schema, dependency, network path or AI projection changes.
The acceptance record tracks the lexical fractional-time-bound limitation at
that historical boundary and its correction in the next continuation.

The next continuation, `t3code/r5-transactions-7` at `e66eafae`, corrects timestamp
bounds only. Standalone Python reference fix `4f3cb12d` replaces lexical bound
comparisons in transaction, metadata/history, journal, rate and desktop/audit
filter paths with exact integer microseconds from the persisted timestamp
parser. Its regressions fail against the original filters. Rust aligns the
three already ported list/history services, including filtered counts and
pagination, and checks SQL instants against the frozen CPython vectors. Eight
new mutation entries cover lexical selection, microsecond precision, offset and
terminal-Z normalization and unknown/calendar-overflow refusal (98 total).
No new CLI leaf or daemon kind is claimed. This is a verified partial boundary;
the remaining transaction desktop surfaces and documents still need their port.

The `t3code/r5-transactions-8` continuation starts at `d219bed5` and ports three
of the eight requested desktop projections: `ui.transactions.list`,
`ui.transactions.search` and `ui.transactions.extremes`. A typed projection
allowlist routes through the admitted book into `crates/transactions/src/desktop`.
The service reads stored custody decisions, economic relations, journal transfer
rows and all freshness markers. It preserves the desktop's wallet population,
query/filter semantics, signed display sorting, grouped totals before pagination,
queue status under wallet/date scope, cursor bytes and pending-pair suppression.
It shares the corrected instant helper and retains observed kinds and exact msat
until presentation. Eleven Rust regressions, a reproducible Python fixture and
raw JSONL differential scenarios pin this partial boundary; 78 new mutation
entries supplement the 98 predecessors. All 176 final mappings build and are
killed by named tests across the two final mutation runs; the acceptance record
distinguishes those runs. The three ported kinds have zero Python fallbacks.
Standalone Python reference fix
`4fc40b2f` rejects container sort/order arguments with typed validation errors;
its eight list/search cases fail before the fix and pass afterward.

Dashboard still needs `suggest_transfer_candidates` and timezone/calendar bucket
composition. Resolve still needs authored pair-review materialization and its
allocation-bound fingerprint. Graph needs full ownership derivation and graph
semantics/reference-cache composition, including its explicit lookup consent
path. Review context still needs journal, commercial, source-funds, privacy and
audit/evidence section composition. Ledger template still needs the managed
CSV/XLSX template writer. These five whole kinds remain `not_implemented`;
the full editor smoke session and all eight-kind acceptance are unfinished.

The `t3code/r5-transactions-9` continuation starts at `d5296a02` and adds all four
`documents list/create/attach/import-report` commands and their four existing
desktop kinds. `crates/transactions/src/documents.rs` reuses managed attachment
storage and keeps prior reports as evidence without ledger or lot authority.
`commercial.rs` adds local `btcpay provenance list/links/suggest` and their desktop
kinds, plus the pure stored `ui.transactions.commercial_context` read needed by
the deferred review-context projection. Matching preserves the reference's
transaction-ID/payment-hash/stable-key signals, unique-observation requirement,
principal confidence, exact document reference/currency/value comparisons,
existing-link enrichment and rejected-link retention. The context projection
preserves invoice-origin precedence, payout-batch currency barriers, ordering
and stored freshness. Suggestions do not apply prices or transaction kinds.
URL evidence remains literal and is never fetched. No schema or dependency is
added. The acceptance record gives verification, per-command/kind served-by
counts and the named mutation mappings for this continuation. The document
commands and kinds have zero Python fallbacks; 50 new mappings supplement the
176 inherited killed mutations.

This remains a partial finish. Descriptor-backed ownership derivation, complex
input graphs and lone surrogates in standard BIP329 string fields refuse
`not_implemented`; unknown JSON keys/values retain surrogates. Global stored and
synthesized exports do not require that ownership index. Transaction exports,
the five desktop projections listed above, journal/report processing, commercial
review and BTCPay provenance sync remain unported and fail `not_implemented`.
AI projections remain Python. The
five loan tax-composition dispositions remain with journal processing; the
collateral mark storage tests are ported.
The [acceptance record](../reference/rust-transactions-r5-status.md) records
parity scope, Rust-served counts, mutation tests and remaining platform work.
A separate Python reference fix aligns attachment open-path containment with
verify/remove and carries its own regression.

### R5 import-run checkpoint

The complete `imports` command family now runs through the real Rust binary:
`imports list` and `imports rollback`, with `ui.imports.list` and
`ui.imports.rollback` in the daemon. `crates/imports` owns the services;
at this checkpoint `crates/wallets` owned wallet resolution and journal invalidation.
The implementation retains run-created-row scope, schema-discovered collateral,
exact remaining counts, large-run staging and atomic journal invalidation.
Applied daemon rollback retains Python's session shutdown. CLI/daemon wire
comparisons and full SQLite/SQLCipher dump comparisons cover this checkpoint;
the original rollback module runs through the hybrid candidate and its three
scale tests have Rust replacements. A checked-in mutation catalog exercises
run/profile selection, collateral traversal/counting, journal versions and
failure rollback. There are no new external dependencies or egress paths.

This checkpoint does **not** complete the requested wallets/file-import slice.
At the import-run checkpoint all 39 wallet commands remained disabled, including CRUD, descriptors/derivation,
all file importers and sync. Descriptor vectors, importer mutation catalogs and
the remaining wallet/import test dispositions must land with those services.
Python setup and file-ingestion calls in hybrid sessions remain explicit
fallbacks; only the two import-run commands count as native service coverage.
See [the shell reference](../reference/rust-shell.md#import-run-checkpoint-r5)
for the implemented boundary and remaining work.

### R5 descriptor checkpoint

The next partial checkpoint enables only `wallets derive` and the desktop's
`ui.wallets.preview_descriptor`. `crates/wallets` owns public Bitcoin/Liquid
descriptor plans, material normalization and address derivation over mature
Bitcoin/Elements Miniscript crates. The CLI reads existing wallet configuration;
the preview is pure local computation. Neither path grants network consent,
ownership evidence, signing authority or access through an AI projection.

Python vectors landed before the native implementation: 124 plans, 43 material
inputs, 3 normalizations, 165 previews and 32 branch selections, with both
branches sampled at indices 0–4. They cover the supported script families,
SLIP132 public/private versions, export formats, origins, checksums, gap/network
checks and Liquid confidential addresses. Separate reference-fix commits pin
concrete xpub suffix derivation, Elements Taproot domain separation/script trees,
and Blech32m witness-v1 encoding/decoding. Twelve computation mutations are
killed by named native tests. See [the shell reference](../reference/rust-shell.md#wallet-descriptor-checkpoint-r5)
for coverage and the mutation table.

At the descriptor checkpoint, CRUD/list/get and the corresponding desktop
kinds were still `not_implemented`.
The 41 frozen wallet-config cases were preparation for those writes, including
public policy/private-presence storage, durable policy epochs, book/history
guards, atomic observer/freshness invalidation and mutation reauthentication.
A broader malformed-descriptor error/preflight audit and the remaining wallet
test dispositions are still required. This is a verified descriptor checkpoint,
not completion of the requested wallet service slice. File importers, sync,
journals and AI projections remain outside this run.

### R5 wallet lifecycle checkpoint

The real Rust store now serves `wallets create`, `list`, `get`, `update` and
`delete`, plus daemon create/update/delete. The 41 stored-config vectors have
a native validator and safe projection. Policy epochs, retained material and
coverage, canonical policy identity, profile/account selection, book/history
guards, cascade deletion, atomic observer/freshness invalidation and fresh
local reauthentication have native tests. The wallet mutation catalog now has
35 entries (12 descriptor, 23 lifecycle), killed by named tests.

Python remains the reference. Two isolated reference fixes consume advertised
descriptor stdin/FD options and refuse change-only private spending material
before storage. There are no new external dependencies, schema changes or
egress paths; AI projections remain Python, and importers/sync stay disabled.

At this checkpoint, nonempty birthdays and configured Silent Payments
returned `not_implemented`; the desktop wallet-list snapshot and successful-create
clock/entropy wire comparison were still required. The verified boundary,
per-command hybrid counts and mutation table live in
[the shell reference](../reference/rust-shell.md#wallet-lifecycle-checkpoint-r5).

### R5 wallet lifecycle remainder checkpoint

The native lifecycle now normalizes birthdays with CPython-compatible ISO
parsing and validates/stores watch-only Silent Payments scan configuration.
Scan-private material remains local and redacted; spend-private material is
refused, including inactive ordinary key fields. Scan birthdays, network
families and independent full-history/server warnings retain Python's guards.
The daemon adds the safe `ui.wallets.list` snapshot without probing backends.
AI projections remain Python. There are no dependency, schema or egress changes.

The oracle now has 193 config vectors and seven desktop snapshots. Native
tests cover persistence, refusal atomicity, selection, freshness and redaction;
test-only clock/identity injection enables successful-create wire-byte and
complete SQLite-dump comparison. Lifecycle dispositions name replacements and
explicitly preserve mixed sync/observer obligations. The mutation catalog has
51 entries (35 inherited plus 16 birthday/SP/snapshot/preflight entries).
Two separate reference-fix commits type embit miniscript validation errors and
preflight inactive key fields on Silent Payments wallets, with regressions
observed failing before each fix.

That run stopped at a lifecycle boundary. Generic CSV and Phoenix ingestion had
not been ported; `wallets import-csv`, `wallets import-phoenix`, all other
importers and sync stay `not_implemented`. Hybrid smoke imports still use
Python, so the requested native import-step acceptance remains unmet. The
malformed-descriptor audit also remains incomplete, including Python's accepted
nonterminal wildcard suffixes. See [the shell reference](../reference/rust-shell.md#wallet-lifecycle-remainder-checkpoint-r5)
for verification, per-command routing, named mutation killers and remaining work.

### R5 generic CSV ingestion checkpoint

`wallets import-csv` now delegates to the local wallet file-import service.
Python-compatible CSV parsing, decimal/msat and timestamp normalization,
fingerprints, dedupe/enrichment, batch provenance and journal invalidation are
native. Import-run membership includes only inserted observations, preserving
the existing rollback contract. The custody crate supplies the closed observer
commitment predicate; file contents cannot acquire observer authority. The
Python core and AI projections remain the reference and are unchanged.

Native reader/normalization vectors, persistence/atomicity tests and thirteen
killed importer mutations cover this checkpoint. Exact CLI output and complete
book dumps also compare against Python with harness-only clock/entropy inputs.
Phoenix, the broader hybrid acceptance run, and the optional descriptor audit
remain to follow. Other importers and all sync commands stay disabled. See
[the shell reference](../reference/rust-shell.md#wallet-csv-ingestion-checkpoint-r5).

### R5 file-ingestion checkpoint

`wallets import-phoenix` now shares the generic CSV service and its admitted
book transaction. Phoenix's signed integer msat, split fee units, zero-amount
directions, exact pricing, payment hashes, sanitized evidence and idempotent
notes/tags follow Python. Generic enrichment also preserves Python's pricing
rank and distinction between import-authored and user-authored metadata.
Import-run creation, rollback and journal invalidation remain atomic; a file
cannot replace a closed observer commitment.

The importer oracle has 145 frozen Python cases, eight native persistence and
normalization tests, and 32 importer mutations (83 wallet entries total).
CLI wire/state comparisons cover SQLite and SQLCipher through rollback. The
hybrid import oracle compares the complete typed book dump after each native
import. No third-party package, egress, Python production change or AI
projection port is included. Detailed verification and per-command routing
counts live in [the shell reference](../reference/rust-shell.md#wallet-file-ingestion-checkpoint-r5).

The seven hybrid modules passed 409 tests and 54 subtests. All 78 generic CSV
imports and the Phoenix import ran in Rust, with zero Python fallbacks and
identical complete normalized book dumps. All 83 wallet mutations were killed;
workspace tests, clippy, formatting, the harness and frozen vectors passed.

Other file importers, sync and the optional nonterminal-wildcard/malformed
descriptor audit remain open. Unsupported commands still fail closed.
### R5 backup archive checkpoint (partial)

The [backup checkpoint](../reference/rust-shell.md#backup-archive-checkpoint-r5-partial)
starts at secrets tip `a051e1ef`. A dedicated `crates/backup` service and CLI
family add SQLCipher archive export and staging-only import without Python
fallback. Both-direction age/tar interoperability and complete logical book
comparisons are exercised, alongside plaintext-payload import and exact
plaintext-export refusal. Schema-1 attachment counts stay unchanged; hashes
are a possible future product change. The 19-entry format/safety/gating
mutation catalog has passing baselines and killed entries.

At this initial checkpoint, installation, all four desktop backup kinds and
cross-runtime restore recovery were unimplemented and stayed closed. Python
product code was unchanged. The continuation below supersedes installation and
recovery status. Compressed/malformed archive parity and native recipient,
TTY and platform acceptance remain open.

### R5 backup installation and recovery continuation (partial)

`t3code/r5-backup-2` starts at `2c14fdc0`. CLI `backup import --install` now
uses the store's same-filesystem restore transaction, retaining both owner-role
reservations and the original recovery copy. A separate Python reference fix
first closes a confirmed restart bug: a kill after moving the live database
previously let `open_db` initialize an empty book. Both runtimes now read the
same bounded intent before settings, layout or database creation; pending
transactions roll back and committed transactions retain the complete restore.

The [continuation checkpoint](../reference/rust-shell.md#backup-install-and-recovery-continuation-r5)
records exact routing, interruption counts, named mutation killers and local
verification limits. It does not complete the service slice. The four desktop
backup kinds, compressed archives, hardlink-export parity, broader malformed
input/I/O parity and native-platform acceptance remain open. Attachment counts
stay unchanged and AI projections remain in Python.

### R5 desktop backup continuation

`t3code/r5-backup-3`, based at `0d775706`, ports all four `ui.backup.*`
kinds through the backup domain service and a narrow daemon lifecycle adapter.
Preview authenticates/integrity-checks the encrypted inner payload, exposes only
the reference's label/count summary, and retains a bound, expiring snapshot
without either passphrase. Confirmation, target/connection/SQLite generations,
staging commitments, cancellation and idle/lock/shutdown cleanup preserve the
Python contract. Apply retains desktop ownership and broker exclusion through
checkpoint/close and publication, invalidates credential settings without native
store probes, and reports locked outcomes and retained recovery copies.

The byte oracle normalizes no JSONL response fields: test-only randomness and
install-name injections and a reused disposable inode make both runs identical.
Python emits no backup progress events; the synchronous native handlers match.
Export now retains PAX disk metadata and Python's tar record padding so its
reported encrypted size also matches. Plaintext export remains refused and
attachment manifests retain counts without hashes. Python product code and AI
projections are unchanged; no new dependency version or egress path is added.

The [desktop checkpoint](../reference/rust-shell.md#desktop-backup-continuation-r5)
records per-kind routing, the 107-entry mutation table, verification and sandbox
limits. The desktop oracle passes 11 cases with zero fallbacks for each kind,
the install/recovery oracle passes 1,370 cases, and all 107 mutations are killed.
This supersedes the preceding desktop omissions, while compressed tar,
hardlink export, broader malformed/I/O parity, native/platform/TTY acceptance
and the remaining service dispositions keep the full backup slice open.
#### Quantity primitive checkpoint (R2e, partial)

Branch `t3code/r2e-quantity`, based on `23c059dc`, ports the exact arbitration
kernel, ordered N:M FIFO cells and residuals, wallet balance differences and
quantity issue/decision/economic-relation hashes. All amounts use unbounded
integer msat. CPython constructor and behavior vectors, independent generated
counts and permutations exercise conflicting sources, partial allocations,
one-msat boundaries and explicit negative-balance suspense claims.

This is a verified primitive checkpoint, not completion of the `quantity`
stage. The PyO3, census and custody-shadow inventories still skip `quantity`.
The full reviewed component compiler, native-audit augmentation, gap holds,
issue composition and tax eligibility barriers must be ported and compared
before changing that inventory. Python remains authoritative, snapshot v1
remains raw-only and Rust results never reach stored state. The test-only
primitive driver receives separate intermediate fixture inputs; it is not a
new product or snapshot interface.

See the [quantity primitive checkpoint](../reference/custody-engine.md#quantity-primitive-checkpoint-r2e-partial)
for branch dispositions, mutation accounting and verification. After quantity
composition is complete, the next stage is **finalized projection**, followed
by journal fusion with `ledger`.

### Canonical quantity transfer rows (R2e, partial)

The follow-on `t3code/r2e-quantity-2` checkpoint ports canonical internal-transfer
rows as a private comparison primitive, including full-decision tax eligibility,
anchor ids, wallet labels, provenance and exact numeric ordering. Its independent
Python generator asserts active counts for every scenario, including exclusion
controls with a separate exact companion edge. See the
[checkpoint evidence](../reference/custody-engine.md#canonical-transfer-row-checkpoint-r2e-partial).
Quantity remains skipped until reviewed compilation, native augmentation,
gap holds, issues and scoped tax barriers match the Python runtime.

### Reviewed quantity compilation and native augmentation (R2e, partial)

The local `t3code/r2e-quantity-3` checkpoint, based on `180edd62`, ports the
reviewed component/conversion compiler and native audit augmentation. It covers
transactionless routes, partial conversion coordinates and economic records,
virtual channel sources, aggregate native targets and connected contradiction
closure. The exact compiler operands and per-engine scenario activity remain
private comparison inputs. See the [checkpoint evidence](../reference/custody-engine.md#reviewed-compilation-and-native-augmentation-checkpoint-r2e-partial).

Quantity remains skipped. Gap holds, whole-state issue composition, scoped tax
barriers and finalized projection still precede journal fusion. Python owns
production decisions and persistence; snapshot v1 stays raw-only.

### Complete quantity runtime (R2e)

The local `t3code/r2e-quantity-4` checkpoint, based on `a4151d44`, adds gap-hold
composition, component/runtime diagnostics, whole-state issues and scoped tax
barriers. Quantity now participates in PyO3, census and custody-shadow comparison;
only finalized projection remains skipped. Exact msat, CPython diagnostics and
raw-only admission retain their existing boundaries. See the
[checkpoint evidence](../reference/custody-engine.md#complete-quantity-runtime-checkpoint-r2e).

Finalized projection and journal fusion remain separate work. Python continues
to own production decisions and persistence, and snapshot v1 contains no Rust
compiler operands or verdicts.

### Finalized row primitives (R2f, partial)

The local `t3code/r2f-projection` checkpoint, based on `9d2ab49c`, ports the
finalized row contract, exact synthetic slice/fee/payout identities, original
row/anchor fallback, chain-order proof capture and fee-only row construction.
The private comparison example consumes independent primitive operands; it is
not snapshot admission and does not provide a partially implemented product
stage. Frozen Python vectors and generated per-engine activity counts preserve
raw quantities, timestamps, provenance, privacy markers and validation order.
See the [checkpoint evidence](../reference/custody-engine.md#finalized-row-primitive-checkpoint-r2f-partial).

`finalized_projection` remains skipped in PyO3, census and custody-shadow.
Slice pricing, direct payouts, fee allocation, selected MOVE/external/inbound
composition, connected blocking, basis barriers and quarantine composition must
match before enabling it. Journal fusion remains separate. Python stays
authoritative, Rust never supplies persisted state, and snapshot v1 stays raw-only.

### Principal pricing, payouts and fee allocation (R2f, partial)

The local `t3code/r2f-projection-2` checkpoint starts at `a178a48f` and adds
principal slice pricing, reviewed payout admission/time/settlement and price
overrides, and exact fee allocation/deduplication. Decimal operations retain
Kassiber's 32-digit half-even context and both independently rounded divisions;
all msat coordinates and fee sums use BigInt. The private primitive comparison
has independent Python/Rust activity assertions, money-boundary vectors and
named mutation witnesses. See the
[checkpoint](../reference/custody-engine.md#principal-pricing-payouts-and-fee-allocation-checkpoint-r2f-partial).

Finalized projection remains skipped, snapshot v1 remains raw-only and Rust
results never reach stored state. Selected-row composition, blocking/barriers,
quarantines and conversion metadata remain for the next slice; journal fusion
remains separate. A nonfinite-payout reference issue is documented with a repro;
Python is unchanged in this parity checkpoint.

### Selected rows, connected blocking and scoped barriers (R2f, partial)

The local `t3code/r2f-projection-3` checkpoint starts at `e589c9a3` and ports
selected MOVE, external, payout, unconsumed inbound and fee-sibling composition.
Connected internal paths close before selection; scoped basis barriers preserve
only the barrier event's finalized siblings at equal timestamps. Quantities
and slice coordinates remain BigInt. The private comparison uses the Python
compiler and its original nested barrier/closure code. Distinct seed activity
and refusals are counted without companion contributions, with a deliberately
vacuous activity-check regression. See the
[checkpoint](../reference/custody-engine.md#selected-row-composition-checkpoint-r2f-partial)
for tests, mutations and the deferred Python quarantine-origin reproducibility bug.

Finalized projection remains skipped. Issues/quarantines and reviewed conversion
metadata are the next slice; journal fusion follows its own parity gate.
Python remains authoritative, snapshot v1 stays raw-only and Rust results never
reach stored state.

### Reviewed conversion metadata (R2f, partial)

The local `t3code/r2f-projection-4` checkpoint starts at `1526b1dc` and ports
reviewed conversion audit metadata after row selection. It matches finalized
anchors, exact optional BigInt amounts, leg directions, singleton admission,
durable pair-id deduplication and reviewed field defaults. Its independent
oracle executes the unchanged Python audit AST. See the
[checkpoint](../reference/custody-engine.md#reviewed-conversion-metadata-checkpoint-r2f-partial)
for activity, refusal, mutation and comparison evidence.

Finalized projection remains skipped; snapshot v1 stays raw-only, Python remains
authoritative and Rust results never reach stored state. Issue/quarantine
composition is deferred until the Python dependency-origin reference is fixed.
Conversion metadata does not read quarantine origins. Journal fusion remains
separate.

### Projection restack on reviewed quantity (R2f, partial)

The local `t3code/r2f-projection-5` branch restacks the five projection commits
onto `fc63c1e0`, retaining the quantity-review corrections, then includes Python
fixes `117eb35d` (finite pricing) and `ca9a672f` (fixed-point, sorted dependency
origins). Rust base rows, principal rates and payout pricing now follow the
fixed Python refusals. This is a step-1 checkpoint: issue/quarantine composition
and its Rust origin-completeness test remain unported. Finalized projection
stays skipped in PyO3, census, shadow and replay. Complete that stage before
journal fusion with `ledger`. See the
[restack evidence](../reference/custody-engine.md#projection-restack-on-reviewed-quantity-r2f-partial).

### Issue/quarantine composition (R2f, partial)

Local branch `t3code/r2f-projection-6` starts at `72325e38`. The private
compiler now joins selected rows, conversion metadata and explaining diagnostics.
It preserves fixed-point, sorted connected origins, interpreter explanations,
quantity-blocker priority, scoped barrier roots, unbooked receipt explanations,
missing payout prices and eager Python diagnostic admission.

`finalized_projection` remains explicitly skipped. The remaining gate is a
lossless raw-stage adapter followed by complete-stage comparisons, including
refusals, generators, recorded replay and the census/shadow inventories. The
private operand comparison cannot prove that adapter. Snapshot v1 stays raw-only
and Rust results never reach stored state. Journal fusion with `ledger` follows
complete projection parity. See the [checkpoint evidence](../reference/custody-engine.md#issuequarantine-composition-checkpoint-r2f-partial).

### Lossless finalized interpreter diagnostics (R2f, partial)

The local `t3code/r2f-projection-7` checkpoint, based on `746cda8c`, preserves
CPython values and surrogate strings in copied interpreter diagnostics through
the private finalized compiler. This is preparation for the raw-stage adapter;
that adapter and whole-stage parity remain outstanding. Finalized projection
stays skipped everywhere. See the [lossless diagnostic checkpoint](../reference/custody-engine.md#lossless-interpreter-diagnostic-seam-r2f-partial)
for comparison counts, mutation witnesses and remaining gates.

### R5 transfers/review checkpoint (partial)

The transfer slice starts at integration tip `65c74e84` on
`t3code/r5-transfers`. The [29-command classification](../reference/rust-shell.md#transfers-and-review-classification-r5)
separates stored reads from authoring and current-state computation.
`crates/transfers` serves stored pairs/payouts, gap history, matching-rule
configuration and historical review receipts through `Book::transfers`, CLI
and desktop family adapters. No matcher, journal builder, egress path or AI
projection is enabled by these reads. Native Python vectors and real-process
byte oracles cover the five stored-read commands.

The first authoring checkpoint adds rule create/delete/enable/disable and
advisory dismissal, giving **10/29 CLI commands** (9 transfers, 1 review) and **9 desktop
kinds** with no Python execution in a supported path. Rules preserve validated
Python JSON bytes, Unicode 14 policy defaults and fail-closed enable behavior.
Dismissals preserve scope, ambiguity rejection, stable identity/creation time,
reason coalescing and expiry. Savepoints contain failed writes; these authoring
operations do not invalidate journals. The native oracle pins complete rows
after 41 sequential operations (11 writes, 30 refusals), and asserts every
other application table stays unchanged. The mutation table and verification
evidence live in the linked Rust shell reference.

The continuation on `t3code/r5-transfers-2` starts at `c9bd660b` and completes
numbered step 1: pair, unpair and update, including their desktop kinds. The
supported total is **13/29 CLI commands (12 transfers, 1 review) and 12 desktop
kinds**, with zero Python fallbacks on those paths. Aggregate persistence reuses
the existing `crates/custody` component stage through its pure `components::review`
facade for conservation, anchor/route/lineage and raw-active conflict validation,
canonical evidence snapshots and commitment pairs. Pair authoring preserves
append-only revisions, immutable children, deterministic Python UUID/hash/JSON
bytes, residuals, membership updates and atomic journal invalidation.

The 20-scenario fixed-source oracle pins normalized rows after **115 operations
(67 successes, 48 refusals)** and requires every unrelated application table to
stay unchanged. Additional tests retain store immutability triggers, roll back
late invalidation failures and preserve caller-owned transactions. Focused
hybrid witnesses record pair 2, update 2 and unpair 2 calls, all served by Rust.
The [shell reference](../reference/rust-shell.md#transfers-and-review-classification-r5)
records per-command evidence, mutation entries and verification limits.

Two reference atomicity bugs have separate regression/fix commits: activation
refusals committed drafts (`26e64be8`); final invalidation failures committed
pair changes (`2091516c`). Both were demonstrated failing before their fixes.

The next checkpoint adds payout deletion through CLI and desktop with exact
fixed-source rows, idempotence, membership reconciliation and atomic rollback.
Automatic pairing during journal processing belongs to R4 and is unchanged.

The continuation on `t3code/r5-transfers-4` starts at `3bd2370e` and adds
payout creation through CLI and desktop. The supported total is **15/29 CLI
leaves and 14 desktop kinds**. Its 38-scenario fixed-source oracle compares
98 operations (36 successes, 62 refusals), including full database rows and
journal invalidation. Rust rolls back late invalidation failures completely;
the separate Python atomicity fix remains pending. The shell reference records
the native rollback test, repro, mutation table and process routing evidence.

Component plan/apply is ported in the [component checkpoint](../reference/rust-shell.md#component-planapply-checkpoint)
below; gap plan/apply is ported in the gap checkpoint below with version-bound
candidate fingerprints, filed-report impacts, complete normalized row parity
and mutation evidence. All ten
computation-dependent commands remain `not_implemented`, including component
materialization, suggestions/bulk/rules matching and review cases/plan/apply.
AI projections stay Python. This verified numbered-step boundary does not claim
full transfers/review slice acceptance.

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

### R5 chain clients: first partial boundary

The chain client slice starts at integrated stack `0d4fbc28` and adds
`crates/chain`: consent-bound Esplora/mempool HTTP, Core JSON-RPC and Bitcoin/
Liquid Electrum TCP/TLS with batching and explicit per-request deadlines.
It reuses the reviewed network crate and its proxy/trust policy. Core history
uses the required 64 MiB cap. No wallet observation persistence or sync
orchestration is ported here.

Five daemon probes are native: HTTP, Electrum, Core RPC, BTCPay store-catalog
probe and local Core detection. Their shared service checks an exact action/
arguments/book grant before using the clients. The Python reference's nested
Core config URL credential-scope bug is fixed separately with a regression
shown failing before the fix. Python otherwise remains the reference.

The chain candidate module compares these five kinds' JSONL bytes against
Python on identical loopback servers and asserts Rust hybrid routing. A
separate fixture example compares the direct client APIs and typed failures.
Twenty native regressions and twenty named mutation entries cover this
checkpoint. See
[rust-shell](../reference/rust-shell.md#chain-clients-and-backend-probes-r5-partial)
and [egress paths](../reference/rust-port-egress.md#chain-client-paths-r5-partial).

This does not complete the requested chain-client/probe slice:
`ui.backends.lightning.test` still requires the LND/CLN node-snapshot adapters;
full failure/TLS/proxy differential coverage and live regtest/platform gates
remain. No new CLI grammar is invented: this Python reference has no backend
probe/detection leaves outside the daemon. Wallet sync remains the next slice.

### Complete finalized projection runtime (R2f)

Branch `t3code/r2f-projection-8`, based on `84b2181b`, connects the existing
lossless compiler to the raw snapshot and computed stages. The whole stage
matches the unchanged Python entry point across 24,000 inherited books,
1,030 permutations and 159 retained requests plus their permutations before
normal comparison admission is enabled. PyO3, census, custody-shadow and
replay now include finalized projection. Decision-only calls explicitly skip
finalization and its refusals through a control outside the raw snapshot.

Confirmation provenance is captured before rates and channel admission.
Interpreter diagnostics remain CPython values throughout copying, grouping
and sorting. Rust results remain comparison-only, Python owns accounting and
persistence, and snapshot v1 remains raw-only. See the
[adapter and parity evidence](../reference/custody-engine.md#raw-finalized-stage-adapter-checkpoint-r2f-partial).
Journal fusion with `ledger` is the next separate R2 boundary.


### Journal request builder checkpoint (R2g step 1)

The [request builder](../reference/custody-engine.md#rust-journal-request-builder-checkpoint-r2g-step-1)
now compiles ledger operands in Rust over finalized projections, ordered wallet
references and raw profile facts. Independent Python comparisons cover 145
new vectors, 166 existing ledger operands and 515 raw generated books; six
new named mutations are killed. The empty outcome makes no engine call.
This is a local compiler checkpoint: ledger-response comparison, complete
campaign reruns and normal fusion comparison/skip admission remain outstanding.
Python retains production authority and post-ledger joins remain R4.

### Fusion harness integration checkpoint (R2g step 3)

The local `t3code/r2g-fusion-6` checkpoint based on `e2be9b41` compares fusion
through PyO3, census, custody-shadow and recorded replay. It preserves exact
request text and full ledger responses, explicit decision-only and injected
skips, and upstream refusing stages. Both focused lanes have non-zero fusion
comparisons and zero differences; retained and fresh recordings replay cleanly.
See the [integration evidence](../reference/custody-engine.md#fusion-harness-integration-checkpoint-r2g-step-3).
Rust fusion remains comparison-only, snapshot v1 remains raw-only, and the
production journal joins and authority switch remain R4. Complete mutation
campaign evidence is still a separate acceptance boundary.

The [fusion campaign evidence](../reference/custody-engine.md#fusion-generated-campaign-and-bounded-mutation-rerun)
now covers all 24,000 raw books, 1,030 permutations, 159 retained requests and
both fresh 50-request recording sets with zero differences. All ten fusion
mutation recipes and 25 inherited boundary recipes were freshly killed;
**2,211 inherited recipes still needed rerunning at that checkpoint**. The
[R6 integration audit](../reference/rust-backend-r6-integration.md) now records
the full R4-0 catalog rerun. The bounded source-branch result did not complete
that gate; the R4 production authority switch still remains future work.

### R5 diagnostics service slice

`diagnostics collect`, its optional public report save, and the global
`--diagnostics-out PATH|auto` error hook now run natively. The hybrid probe
admits implemented commands carrying the flag. The reference defines no
diagnostics daemon kind. Collection reads the admitted connection before
close, projects aggregate state and applies the reference's ordered redaction
rules without egress or disk logging.

The owner approved honest native runtime and stack values inside the same
ordered schema: the historical Python identity keys describe Rust, SQLite
reports its linked version, report stacks are empty arrays, and requested CLI
`error.debug` strings are empty. Python tracebacks are never invented. CLI
debug remains null without `--debug`, including for unhandled failures.
Differential tests normalize only the named runtime/stack fields and,
separately for unhandled diagnostics export failures, `error.debug`; tests pin
the normalization scope. All other bytes retain Python parity.
See the [slice acceptance](../reference/rust-diagnostics-r5-status.md) and
[implementation contract](../reference/rust-shell.md#diagnostics-service-slice-r5).

### R4 journal pipeline design

The [R4 design](../reference/r4-journal-pipeline.md) (2026-10-30; R4-1 onward
remains design) profiles Python's `journals process` by phase and plans the Rust binary's
in-process pipeline: typed SQLite loading, typed custody stages, typed fusion
into the tax engine's own request and response types, rendering and atomic
persistence, with the JSON snapshot and stage interface kept as the
byte-compared parity oracle. It splits the work into seventeen single-worker
steps (R4-0 to R4-16), each keeping the generators, replay, census and
shadow and the full mutation catalog.

Measured on this branch, Python takes 1.8 s at 2,000 and 12.8 s at 20,000
generic transactions, and 21.7 s at 20,000 multi-wallet transactions. Custody
is 40–75% of that. Behind the binding, Rust custody is only 1.0–2.3× faster,
and the parse and serialization alone cost Rust 0.8–0.9 s and Python 1.9–2.2 s at
20,000. SQLite's own write floor (1.25–1.7 s plaintext, about 3 s SQLCipher at
20,000) caps end-to-end gains under byte parity. The design therefore targets
10× for custody and at least 4× end to end, guarded by deterministic counters
and a ratio gate in CI and a local 20,000-transaction gate. Its open owner
decisions cover an early PyO3 production path, the targets and a tax-engine
performance slice, automatic pairing, persisted-shape changes beyond parity,
the book-open cost and the allocator.

### R4-0 integrated stack

The local `t3code/rust-backend-r6` stack integrates the R2 custody, projection
review, performance and fusion lines with R5 reads, diagnostics and the shipped
Python corrections. The [integration audit](../reference/rust-backend-r6-integration.md)
records the linear picks, skips, conflict resolutions, custody campaigns,
hybrid parity and full mutation rerun. R4-0 is complete; R4-1 onward and the
production journal authority switch remain planned.

### R5 transfers/review stored-read checkpoint

The transfer slice starts at integration tip `65c74e84` on
`t3code/r5-transfers`. The [29-command classification](../reference/rust-shell.md#transfers-and-review-classification-r5)
separates stored reads from authoring and current-state computation.
`crates/transfers` serves stored pairs/payouts, gap history, matching-rule
configuration and historical review receipts through `Book::transfers`, CLI
and desktop family adapters. No matcher, journal builder, egress path or AI
projection is enabled by these reads. Native Python vectors and real-process
byte oracles cover the five stored-read commands. Authoring remains the next
checkpoint; append-only revisions, commitments, native residual validation and
invalidation must be ported together before pairing/components/gap application
can be advertised.


### R5 transfer atomicity reference checkpoint

The transfer continuation incorporates Python's savepoint atomicity fix and
compares payout create/delete late-failure errors and complete rollback state
against the repaired reference. The old bug repros are replaced by shared
failure-path oracle cases. See the [transfer checkpoint](../reference/rust-shell.md#savepoint-reference-fix-parity-checkpoint)
for parity counts and verification. Component/gap plan/apply and their daemon
kinds were outstanding at that boundary; the following checkpoints port them.
Automatic pairing remains R4.

### R5 component plan/apply checkpoint

The continuation on `t3code/r5-transfers-6` starts at `01e113a8` and serves
component plan/apply for all five review actions through CLI and desktop:
**17/29 CLI leaves and 16 desktop kinds**, with zero Python fallbacks on those
paths. Planning and persistence live in `crates/transfers`; conservation,
anchors, routes, evidence and materialization reuse the `crates/custody`
component stage through three new `components::review` facades. A 253-operation
fixed-source oracle (125 successes, 128 refusals) compares ordered outputs and
complete normalized rows, including untracked-wallet creation and late-failure
rollback. 27 named mutations are killed. Two reference behaviors (a case-folding
plan/apply disagreement and `location_ref` echo in create plans) are recorded
with repros and matched, not changed.

Gap plan/apply was still `not_implemented` at this checkpoint because its
create, revise and dismiss actions require the R2 gap-search dependency.
The next checkpoint imports that dependency and ports authoring. See the
[shell reference](../reference/rust-shell.md#component-planapply-checkpoint).

### R5 gap plan/apply checkpoint

The continuation on `t3code/r5-transfers-7` starts at `c3d18194`, imports only
the pure R2 gap search, and serves `transfers gaps plan|apply` and
`ui.custody.review.plan|apply`: **19/29 CLI leaves (18 transfers, one review)
and 18 desktop kinds**. The shared transfer service handles create, dismiss,
reopen, revise and residual classification. Candidate and correction
fingerprints bind exact evidence, versions, history and deterministic component
identities; apply rechecks inside a savepoint and preserves caller transactions.
Review events, relations and saved/filed-report impacts remain append-only.

Fixed-source Python oracles compare complete normalized database rows,
ordered outputs, refusal messages, invalidation and late-failure rollback.
Large-book SQL worklists retain the Python bounds and incompleteness barriers.
CLI and daemon hybrid transcripts require Rust execution without fallback.
The [shell reference](../reference/rust-shell.md#gap-planapply-checkpoint)
records parity counts, named mutation witnesses and verification results.
The ten computation-dependent commands and their kinds remain unported;
AI projections remain Python and automatic pairing belongs to R4.

### R4 automatic pairing library checkpoint

R4-14 on `t3code/r4-14-pairing`, based at `c3d18194`, ports suggestions, rule
selection and bulk exact pairing in an isolated `crates/transfers/src/pairing`
module. The four-pass journal helper reuses parsed observations, preserves
projection guards and leaves all successful writes pending for the journal
caller. [The pairing reference](../reference/rust-pairing.md) records parity,
performance, mutation evidence and the R4-13/R4-15 calling contract. This library
checkpoint activates no product route and does not complete R4 journal routing.

### R5 Bitcoin sync: compatibility discovery boundary only

`crates/wallets::scan` now ports the Bitcoin compatibility descriptor gap
walker for Esplora/mempool and Electrum, reusing the ported descriptor plan and
the gated chain clients. Immutable actions bind wallet/backend/checkpoint and
database/profile/wallet/environment/source scope. Grants are explicit,
revocable and single-use. Offline refusal, cancellation, request deadlines,
bounded HTTP workers, Electrum subscription batches and Python progress/target
semantics have native regressions. No new upstream dependency is added.

Six synthetic fixture families compare preparation output and query multisets
with unmodified Python over both protocols. These are discovery comparisons,
not normalized book dumps. Twenty checked-in discovery mutations are killed
by thirteen named baseline selections. The guarded fixture driver is built by
the required serial-daemon lane and local quality gate.

This is **not service slice acceptance**. No sync command or desktop kind is
enabled, and no observation is persisted. Ordinary Electrum must still port
BDK's state representation and rebuild/reorg semantics; compatibility scanning
does not replace it. Transaction/history fetching, confirmation and absence
authority, provenance and graph cache writes, atomic retractions/inventory/
coverage/freshness, journal invalidation and full normalized book comparisons
remain. `wallets sync` and desktop wallet/freshness sync kinds retain
`not_implemented` in Rust. Liquid, Core RPC, Lightning and Silent Payments sync
are also unported; AI projections stay Python. See the detailed
[discovery checkpoint](../reference/rust-shell.md#bitcoin-compatibility-discovery-r5-preparation-only).

### R5 Bitcoin sync: protocol acquisition boundary (partial)

`crates/wallets::acquisition` now fetches request-local Bitcoin observations
under its own exact, revocable, single-use action grant. Esplora retains
pre/post statistics, paginated confirmed and mempool history, and raw UTXOs.
Electrum acquires subscriptions, histories, raw transactions, distinct parents,
raw block headers/timestamps and UTXOs. Decoding preserves integer satoshis,
witness/size/weight and Python's version-one inline-prevout graph shape;
height/depth observations retain unknown depth when the tip is unavailable.
No dependency is added. This helper always fetches fresh protocol data; it does
not port incremental observer state or record reuse.

The guarded example and 22 loopback scenarios compare protocol preparation
against Python, including the six discovery target families and changed-height,
empty/disappeared, mempool and coinbase observations over both protocols.
These are **not normalized book dumps or canonical reorg/retraction tests**.
Nineteen acquisition mutations and the inherited twenty discovery mutations
are killed by named tests. A separate Python reference correction skips the
coinbase null outpoint in compatibility prefetch and graph normalization; its
two regressions failed before the fix and kill two additional mutations.

The service slice remains incomplete. BDK observer state, canonical-chain
selection, reorg/retraction authority, normalized record composition, provenance,
graph/inventory/coverage application, freshness and journal invalidation remain.
There is no new CLI/daemon/AI registration: `wallets sync`, `ui.wallets.sync`
and freshness sync kinds still return `not_implemented` in Rust, with zero new
native product-sync calls and no zero-fallback acceptance claim. Liquid, Core
RPC, Lightning and Silent Payments sync stay unported; AI projections stay Python.
See the [acquisition checkpoint](../reference/rust-shell.md#bitcoin-protocol-acquisition-r5-preparation-only).

### R5 Bitcoin sync: private state/reuse kernel (partial step 1)

`crates/wallets::observer_state` now normalizes Python's complete version-one
BDK aggregate JSON and ports its structural identity hash, record fingerprint
and confirmed-record reuse guards. Reuse requires every active projection to
retain the exact sole observer's closed graph and integer-msat quantity proof.
Full scans, release changes, changed inputs and unconfirmed records cannot reuse
saved records. Private SQL reads go through `crates/store::observer`; state,
epoch and exclusive version-two coverage share one consistent SQLite snapshot.
An explicit forced rebuild retains the raw epoch and old durable rows.

Sixteen accepted aggregate cases and thirteen refusal cases compare with the live
pinned Python BDK binding, including duplicate-preserving lists, stable anchor
ordering, Python hex byte boundaries and every extended key's network kind.
The state oracle also checks restarts, incompatible versions/identities,
force rebuilds and edited/shared/excluded provenance against real disposable
Python books; complete pre-existing dumps remain unchanged. These are read
comparisons, **not post-sync book-dump parity**. A separate guarded driver is
built in the serial-daemon lane. No dependency, schema, egress or product
registration is added; Python product code is unchanged.

This is a verified library checkpoint within step 1, not completion of that
step or service-slice acceptance. Native BDK wallet reconstruction and its full/
revealed-script sync, compatibility fetch-cache reuse, canonical/reorg/retraction
authority, atomic application and all four sync registrations remain. The
required native BDK wallet and Electrum crates are absent from this run's offline
Cargo cache; no registry source was side-loaded and no compatibility substitute
was introduced.
Full details, mutation entries and verification limits are in the
[state/reuse checkpoint](../reference/rust-shell.md#bitcoin-observer-state-and-record-reuse-r5-partial-step-1).

### R5 Bitcoin sync: native BDK execution/cache checkpoint (step 1)

The offline cache now supplies the Python-matching native state crates:
`bdk_wallet =3.0.0`, `bdk_chain =0.23.3`, `bdk_core =0.6.3`. The native wallet
is reconstructed from private version-one JSON, runs full/revealed-script
Electrum scans with bounded widening, applies native graph/index updates and
merges staged changes into its aggregate. BDK's connector/file-store crates are
not linked: existing consent-bound Kassiber clients fetch protocol data and the
translator follows the pinned connector sources. Esplora retains Python's
compatibility route. Compatibility checkpoint/history/header/membership reuse
is prepared under the same immutable exact-action consent, offline and proxy
boundaries. Dependencies, lockfile and license reasons are recorded together.

This completes the execution/cache work of step 1, subject to the verification
record in [rust-shell.md](../reference/rust-shell.md#native-bdk-execution-and-compatibility-caches-r5-step-1).
The separate Python reference fix stops fixed-descriptor widening when BDK's
actual revealed index does not change; its regression failed before the fix.
Native aggregate/restart and compatibility cache differential oracles supplement
the ten inherited acquisition fixtures. They do not establish persisted
book-dump or product-sync parity. No command, desktop kind or AI projection is
newly registered, and all four product-sync served-by counts remain zero.

Next are numbered steps 2 (canonical/reorg/retraction authority), 3 (atomic
facts/provenance/graph/inventory/coverage/freshness/journal application) and 4
(scoped routing and zero-fallback full-dump parity). Disconnected/behind native
state refuses until step 2. Product delivery remains blocked by those incomplete
steps, even when this library checkpoint's checks pass.

### R5 Bitcoin sync: canonical authority preparation (step 2)

The continuation at `f9ff09d0` now prepares Python-equivalent BDK canonical
membership and per-source retractions. A disconnected persisted tip triggers a
fresh full scan and aggregate; force-full rebuilds retain the identity-local
absence baseline and reject lagging backends. Point-of-agreement recovery uses
uncached remote checkpoints, with the pinned dependency's separate semantics
for a backend that falls behind after preflight. Canonical transaction selection,
eviction/reappearance and replacement remain BDK decisions. Esplora's named
compatibility route now prepares source-bound complete-history retractions,
retaining partial/omitted-script positives and preserving membership across
force-full scans. Compatibility Electrum caches still grant no independent
withdrawal authority.

This is the clean boundary after numbered step 2. The eight BDK and six Esplora
three-round authority sequences compare private aggregate/checkpoint state and
canonical/retraction sets to Python, supplementing the ten acquisition fixtures
on both protocols. They do not establish post-sync book-dump parity. No Python
product code, dependency pin, schema, product route or AI projection changes.
The BDK/cache mutation catalog grows from 89 to 114 entries, with named tests
for all 25 new authority mutations; the inherited tip entries are updated to
pin the completed preflight/rebuild rules.

Step 3 must still compose normalized records and complete outputs/coverage,
veto retractions contradicted by another live observer, and atomically apply
all facts plus freshness/journal/report effects under scope/epoch fencing.
Step 4 must register the four scoped product paths, retain the hybrid's Python
routes for unsupported wallets/backends, and prove full normalized book dumps
and zero in-scope Python fallbacks. Successful native served-by counts remain
zero for each of `wallets.sync`, `ui.wallets.sync`, `ui.freshness.run` and
`ui.workspace.freshness.run`. See the
[authority checkpoint](../reference/rust-shell.md#bitcoin-canonical-authority-preparation-r5-step-2)
for the fixture matrix, exact mutation/test mapping, verification and limits.

### R5 Bitcoin sync: normalized record composition

The continuation at `0da3346b` completes step 1 of the next handoff: pure
Bitcoin record normalization and cross-observer retraction vetoes. This splits
the composition portion out of the preceding checkpoint's remaining step 3;
atomic application and routing remain open.

`kassiber-wallets::bitcoin_records` ports the reference's graph normalization
into exact decimal record text, preserving collaborative wallet deltas, fee
attribution, occurrence/confirmation times, graph-scoped scripts, privacy and
unique-outpoint HTLC evidence. Native BDK canonical history now projects records
and release-bound fingerprints, with reuse disabled on every full/reorg scan
and missing owned-spend fees refused. `composition` merges family graphs and
complete output snapshots, rejects inconsistent inputs/prevouts and mixed
observer routes, combines checkpoints, vetoes retractions contradicted by live
records and prepares private structural provenance mappings. These are local
JSON-only preparation services with no product dispatch or persistence.

Complete normalized record JSON compares with Python across all ten acquisition
fixtures on Esplora and Electrum, cached compatibility rounds, native BDK's
eight fixtures/restarts and eight three-round canonical sequences. Sixty-seven
frozen vectors pin the pure normalizer/composer. This does **not** satisfy full
normalized post-sync book-dump parity. Successful served-by counts remain zero
for all four requested product paths; in-scope fallback acceptance is unmet.

Two Rust serialization differences were corrected: cached Electrum fresh
graphs no longer synthesize a `txid`, and raw decoder Decimal metadata retains
Python's float JSON spelling while accounting math remains integral. Python
product code is unchanged; no reference bug was found. The workspace's existing
decimal crate becomes a direct internal dependency; no upstream dependency,
schema or runtime requirement is added. The
[record checkpoint](../reference/rust-shell.md#bitcoin-normalized-record-composition-r5-continuation)
records mutations, verification and the remaining numbered steps.

All 340 sync-domain mutations are killed (83 new record/composition rules and
257 inherited rules). Offline/locked Rust formatting, workspace/harness clippy,
workspace/net/scanner/harness tests and harness/example builds pass. The exact
requested shell/hybrid/CLI-runner/rust-port command passes 63 tests and 178
subtests; the expanded protocol/reference run passes 209 tests and 79 subtests.
Inventory, source contracts, shard validation and vector drift checks pass.
The handoff records retries for disposable storage, shared mutation artifacts
and Python reference SQLCipher timeouts. This verifies the step-1 preparation
boundary; atomic persistence and product-sync acceptance remain open.
