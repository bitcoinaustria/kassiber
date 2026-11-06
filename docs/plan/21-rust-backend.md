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
| `bdkpython`, `lwk` | `bdk_wallet`/`bdk_chain`/`bdk_core` through Kassiber's gated transports (no BDK networking crates); `lwk_wollet` for Liquid sync, with Kassiber's own persisted state formats (not yet adopted: Python uses LWK only for Liquid sync, which Rust has not ported; the other Liquid paths port embit, see [the dependency review](../reference/rust-shell.md#dependency-review-lwk-sshsig-and-http-plan-21-2026-11-04)) |
| `embit` | `bitcoin`, `miniscript`, `elements`, `elements-miniscript`, `secp256k1`; the hand-written PSBT v2 reader stays hand-written |
| `XlsxWriter`, `openpyxl` | `rust_xlsxwriter`, `calamine` |
| `reportlab` | a rewrite, most likely Typst as a library with the bundled fonts; the hand-written PDF 1.4 report ports directly |
| `pyrage`, `age`/`rage` binary | the `age` crate |
| `keyring` | the same keyring stores the desktop shell uses, with the item formats Python wrote |
| `urllib`, the SOCKS5 client, the Electrum client | one transport layer (`reqwest` or `ureq` with no environment proxy and no redirects, SOCKS with remote DNS) |

| `urllib`, the SOCKS5 client, the Electrum client | one transport layer with no environment proxy and no redirects, SOCKS with remote DNS: `kassiber-net`'s own HTTP/1.1 client over `rustls` and `httparse`, kept after evaluating `ureq` and `hyper` ([Rust HTTP client](../reference/rust-http-client.md)) |
| `ssh-keygen -Y verify` | the `ssh-key` crate (done: in process, over OpenSSH's own armour and field parsing) |
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

Main keeps moving while the stack grows. Each merge of main into the stack
inventories the Python changes the port must follow and slices them;
[main sync 1](../reference/rust-main-sync.md) covers #672 to #714.

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
   contracts are implemented. As in Python, Rust offline mode still permits an
   explicitly local AI provider at a loopback address.

   contracts are implemented. Offline mode blocks AI too, except a provider
   marked local on a loopback URL, as in Python.
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
   Brought into scope on 2026-11-02; the
   [R6 port design](../reference/r6-ai-operator-port.md) holds the census,
   R5 dependencies, slices and owner decisions.
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

### R5 XLSX owner decision (2026-10-31)

The owner chose `rust_xlsxwriter` 0.102.0 for native XLSX now. PDF remains
Python. XLSX acceptance compares workbook content, not ZIP bytes: ordered sheet
names, typed cells, formulas and cached values, meaningful styles and dimensions,
merges, panes and defined names. Document properties must be deterministic.
Existing saved digests remain unchanged; new completions hash the Rust bytes.

`crates/xlsx` isolates in-memory workbook rendering from report computation and
publication. It disables every optional writer feature: no clock integrations,
autofit, alternative decimal library, constant-memory scratch files, C zlib,
WASM, or serialization macros. The writer's mandatory `zip/deflate` transitives
are pure Rust (`flate2` with `zlib-rs`, plus Zopfli). The existing pinned
`flate2` 1.1.9 is retained. Zopfli requires the lockfile's checksum-library
updates. A direct dependency on the same ZIP crate repairs generated row
heights and numeric representations in RAM; no new dependency family is added.
The standard library and existing workspace have no OOXML renderer.

The renderer requires an explicit creation timestamp and writes it to both
creation and modification properties. Python's XlsxWriter reads wall time;
the test recorder pins that independent clock to the supplied test timestamp.
ZIP member timestamps are fixed at 1980-01-01. ZIP compression, XML ordering,
shared-string/style indices and container bytes intentionally differ from
Python. These are not workbook content differences. Python's 16-significant-digit
numeric cells, full-precision formula caches and point heights are preserved.

This decision and renderer foundation do not by themselves enable a command.
Native projection, export completion and CLI/daemon routing require their own
parity and refusal evidence before registration. The generic-ledger import
template and XLSX reading remain outside the report-export slice.

The transaction continuation registers `transactions export`'s XLSX format
(including its default format) and `ui.transactions.export_xlsx`. It reuses
the CSV transaction projection and native admission/publication path; observed
lists create neither saved nor filed markers. The native service oracle covers
all 74 inherited transaction export cases plus eight generated edge books.
CLI/daemon differentials compare content, metadata and every database table;
18 CLI invocations and one daemon session with nine XLSX requests are served by
Rust with zero fallbacks. The [checkpoint](../reference/rust-shell.md#transaction-xlsx-transport-r5-partial)
records the remaining report projections and acceptance gates.

### R5 Austrian XLSX service continuation (partial)

The twelfth reports continuation starts from integrated tip `dd433e4e`, applies
the owner-specified Liquid/commercial and XLSX renderer/transaction commits,
and ports Austrian E1kv XLSX through CLI and desktop. The native annual report
feeds the deterministic renderer; existing export admission spans publication,
actual-byte hashing, saved registration and dependency capture. Explicit filing
retains the sealed saved identity, summaries and digest. It adds no tax rules,
Python product changes, schema or unapproved dependency.

Forty-seven annual fixtures pin workbook projections and real exports, including
pre-render refusals. Four CLI sequences and nine desktop requests compare
openpyxl content, wire metadata, all database tables and filing, with explicit
mappings only for generated ids and independently verified new XLSX digests.
The [Austrian XLSX checkpoint](../reference/rust-shell.md#austrian-xlsx-service-checkpoint-r5-twelfth-reports-continuation-partial)
records verification, routing and the exact remaining work. This is not full
reports or XLSX acceptance: all remaining report reads, other report
workbooks, platform CSV formats and other artifacts still need native service
and transport proof. PDF and PDF-containing source-funds bundles remain Python
by owner decision; AI projections remain Python.

### R5 full-report XLSX service continuation (partial)

The thirteenth reports continuation starts at `e193e6b10` and ports
`reports export-xlsx` and `ui.reports.export_xlsx`. The admitted full-report
projection is shared with CSV; native verification adds the whole-profile
acquisition/disposal ledgers, per-asset controls, evidence and quarantine
disclosures. Wallet filters affect the value sheets while verification keeps
the reference's pooled scope. Formula text, cached results, editable tolerance,
conditional formatting and provenance retain Python's content.

Ninety-one frozen fixtures include 61 workbook documents and 30 pre-render
refusals. The real service/process oracle compares openpyxl content, normalized
wire bytes and canonical bytes of all 112 database tables. Completion hashes
the actual Rust bytes, preserves sealed dependencies and saves without filing;
repeated explicit filing retains that capture. Thirty-one new mutation entries
have named projection, tolerance, request and completion tests.

The [full-report checkpoint](../reference/rust-shell.md#full-report-xlsx-service-checkpoint-r5-thirteenth-reports-continuation-partial)
records the final gates, routing counts, unchanged-Python verification-banner
bug and exact remainder. Exit tax is next; the other reads, workbooks, platform
CSV formats and non-PDF artifacts remain open. No tax rules, Python product
code or dependencies change. PDF-containing exports and AI projections stay
Python under the existing owner decision.

### R5 exit-tax service continuation (partial)

The fourteenth reports continuation starts at `bcde492ff` on
`t3code/r5-reports-14`. Native `reports exit-tax`,
`reports export-exit-tax-xlsx`, `ui.reports.exit_tax_preview` and
`ui.reports.export_exit_tax_xlsx` share the stored-journal estimate and the
approved in-memory XLSX renderer. Exact msat inventory, explicit regime markers,
consumed engine basis, cached/manual-rate ordering, missing prices, destination
classes and review warnings retain Python's behavior and readiness gates.

The [exit-tax checkpoint](../reference/rust-shell.md#exit-tax-service-checkpoint-r5-fourteenth-reports-continuation-partial)
records frozen projections, openpyxl workbook comparisons, other wire bytes,
all 112 database tables, zero in-scope hybrid fallbacks and named mutation
kills. The mirror now follows the corrected Python rounds 6/6b/6c/6d: configured
method, instant-based civil departure cutoff, closest fresh local quote,
historical wallet context, consumed-lot regimes before disposal overrides,
nullable incomplete totals, complete workbook disclosures, offset-required timed
departures, and version-2 journal semantics with an audited retained-outflow
migration. Exit-tax estimates remain
outside ordinary realized-report snapshot/capture registration pending a
replayable valuation design. Other report families continue in their checkpoints;
PDF publication and AI projections remain Python.

### R5 gain explanation continuation (partial)

The fifteenth reports continuation starts at `e5b555440` and ports retained
capital-gain explanations through the native CLI and daemon. Compact-v2 and
legacy calculations retain exact reconciliation, book/input-version binding,
readiness barriers and bounded inherited-basis/custody context. Frozen vectors
and service/process differentials compare response bytes and all database
tables, with named mutation tests and zero explanation fallbacks.
The native inventory is now 152 CLI leaves, including 20 report commands,
and 13 report daemon kinds plus the two transaction export kinds.

Desktop gains computation is ported as a core projection, but its daemon kind
remains unregistered. Python automatically rebuilds stale journals before that
read; the native R4 producer is not accepted at this base. This is a partial
boundary within the first requested family, not completion of that family.
Native refresh/registration remains pending on that accepted R4 producer;
the owner has authorized the independent report families to continue. The
[gain explanation checkpoint](../reference/rust-shell.md#gain-explanation-checkpoint-r5-fifteenth-reports-continuation-partial)
records the exact boundary, reference repro and verification. Python product
code, AI projections, tax rules, schema and dependencies are unchanged.

### R5 cross-check continuation (partial)

The sixteenth reports continuation starts at `be4aac52d` and ports
`reports cross-check` (including `--xlsx`), `ui.reports.cross_check` and
`ui.reports.export_cross_check`. The native inventory contains 153 CLI leaves,
including 21 report commands, and 15 report daemon kinds plus the two
transaction export kinds. The shared service reads stored observations and
journals without rebuilding, repricing or acquiring data. Cross-check remains
a diagnostic: stale journals disable type comparison with a notice, and neither
read nor export gains a final-accounting readiness barrier or saved snapshot.

CoinTracking/Blockpit parsing, integer-msat matching, exclusions, clock offsets,
cash/type/year differences, filters, totals and wallet pairing retain the
unchanged Python reference. The approved native renderer produces the workbook;
openpyxl content, response bytes and all 112 database tables are compared.
The [cross-check checkpoint](../reference/rust-shell.md#cross-check-checkpoint-r5-sixteenth-reports-continuation-partial)
records routing, named mutation kills and reference-bug repros.

Desktop gains refresh/registration stays pending on R4; the exit-tax port is
unchanged while Python fixes proceed elsewhere. This branch next owns Lightning
reads/CSV and non-PDF source-funds reads. The parallel `t3code/r5-reports-b`
branch owns year-handoff XLSX, CoinTracking/Blockpit CSV and other permitted
artifacts, and privacy hygiene/mirror/PSBT reads. PDF-containing exports and AI
projections stay Python. No Python product, tax-rule, schema, dependency or
lockfile changes are included.

### R5 source-funds stored-list continuation (partial)

`t3code/r5-reports-sof` starts at `9338364db`. The first boundary adds native
source, link, saved-case and recipient listings through both CLI and daemon,
and the daemon's paginated evidence listing. One shared report service reads
stored provenance under a database snapshot without journal rebuilding,
accounting admission, evidence-file reads or external I/O. Stored source
denominations and exact msat fields remain separate from ownership and tax
authority. Case recipient metadata remains frozen at save time.

The inventory becomes 157 native CLI leaves, including 21 report commands and four
source-funds listing commands. Daemon registration adds five source-funds
listing kinds alongside the existing 15 report and two transaction export
kinds. PDF and PDF-containing bundle routes remain Python by owner decision.
There are no source-funds workbook exports in this stored-list boundary.

This is not completion of the read family. The diagnostic continuation on
`t3code/r5-sof-reads` preserves the approved native-stack contract: uncaught
source-funds daemon failures carry an empty native `error.debug` string, typed
refusals retain null, and Rust invents no Python frames or stderr traceback.
The two former strict-xfail witnesses now compare every non-debug response byte,
pin debug presence and reference redaction, and verify all 112 tables and native
routing. Only the debug value and Python-only stderr traceback differ.
The diagnostic continuation passes 239 parity tests, including 148 native CLI
calls and 214 source-funds requests across six native daemon sessions with
zero fallbacks. Five new diagnostic mutations are killed by named tests;
the 24 stored-list mutations retain their prior evidence. See the
[diagnostic checkpoint](../reference/rust-shell.md#source-funds-native-diagnostic-checkpoint-r5-partial).
The read continuation owns report traversal/disclosure and diagrams,
review-context/request-input, coverage and suggest/assemble dry runs, in that
order. The parallel `t3code/r5-sof-mutations` worker owns case saving, source
mutations, link mutations including batch review and persisted suggest/assemble,
and recipient mutations. Each family requires its own verified commit. Mutation
families must retain consent, once-only execution and SQLite/SQLCipher rollback.
The Lightning and year-handoff/platform/privacy workers retain their separate
ownership. The [stored-list checkpoint](../reference/rust-shell.md#source-funds-stored-list-checkpoint-r5-partial)
records the verification and preserved Python quirks. Python product code,
dependencies, lockfiles, schema and tax rules are unchanged.

### R5 source-funds mutation persistence foundation (partial)

`t3code/r5-sof-mutations`, based on `a99fa77c8`, adds a native frozen-case
persistence helper with the reference's two identities, timestamp, exact msat,
frozen recipient/external metadata, JSON bytes and snapshot commitment. Both
rows share a savepoint; late insert/commit failure rolls back the save, while
an enclosing reviewed operation retains its transaction and commit authority.
The unchanged Python primitive's missing local rollback has a synthetic repro.
Three focused Rust tests and all 74 SQLite/SQLCipher process comparisons pass,
with exact receipt encodings and all 112 tables. All 21 new mutation entries
are killed by named tests. Formatting, report all-target clippy, the required
classifier update and all 57 CI preflight tests pass.

This does not complete case saving or enable a mutation route. Native report
composition, diagrams and fingerprint recomputation are dependencies owned by
the parallel read branch. CLI/daemon registration, consent and once-only
execution remain pending. Newly served mutation calls/requests are both zero;
the inventory remains 157 CLI leaves and five source-funds listing kinds.
No fallback contributes mutation acceptance. The ordered remaining work is
case saving end to end, source mutations/attachment, link mutations/attachment,
batch review and persisted suggest/assemble, then recipient mutations.
The [persistence checkpoint](../reference/rust-shell.md#source-funds-frozen-case-persistence-foundation-r5-partial)
records the exact boundary and verification. PDF-containing bundles stay Python;
no dependencies, lockfiles, schema, tax rules or Python product code change.

### R5 source mutation transport continuation (partial)

`t3code/r5-sof-mutations-2`, based on `62d4cd51a`, adds source creation and
managed-id evidence association to the shared native report service, CLI and
daemon. Exact source/fiat/timestamp validation, attachment scope and duplicate
associations match Python. The complete write and response projection share a
transaction; both storage bindings retain outer pending work and roll back
late statement, busy-commit and deferred-commit failures. Files and their
hashes remain unchanged; provenance grants no accounting authority.

The 144-case corpus and all 288 SQLite/SQLCipher process comparisons pass with
complete helper envelopes and all 112 tables. Actual process acceptance covers
11 native CLI mutation calls and ten native daemon mutation requests with
zero fallback, plus one source-list daemon read. The supported inventory is
159 CLI leaves and seven source-funds daemon kinds. The new Python pending-row
repro preserves the reference bug and the existing case repro is untouched.

This remains partial: Rust AI consent/once-only admission and internal-error
traceback/debug wire parity are unaccepted. No Python AI call counts as native
execution. Link mutations, batch review, persisted suggest/assemble, recipient
mutations and full case saving remain pending; the last still depends on the
parallel report/review producer. The [source mutation checkpoint](../reference/rust-shell.md#source-funds-source-mutation-checkpoint-r5-partial)
records the boundary. No schema, dependency, lockfile, tax-rule or Python product
logic changes are included.

### R5 dependency traversal integration (partial)

The `t3code/r5-sof-reads-2` continuation starts at `b9b050083` and adds the
bounded dependency planner as preparation for report composition. It preserves
Python's dependency layers, frontier budgets, retained-row ordering and cycle
behavior. A 203-case frozen corpus includes production-size limits and runs
natively in plaintext and SQLCipher with byte comparisons and every table
unchanged. This registers no report service, CLI leaf or daemon kind: the
mutation branch retains 159 native CLI leaves and seven source-funds daemon
kinds; this helper adds no transport registrations. Full report/disclosure/diagram transport acceptance is
still open. The [preparation checkpoint](../reference/rust-shell.md#source-funds-dependency-traversal-preparation-r5-partial)
also records an unchanged Python privacy-boundary fee-attribution reproduction;
the port must not silently treat it as an accepted authority change.

### R5 recipient mutation continuation (partial)

The mutation continuation adds recipient create/update/soft-delete through the
shared native report service and both transports. It preserves exact scoped
references, inactive-id lookup, partial-update semantics, one timestamp per
write and immutable saved-case recipient fields. These local settings convey
no ownership or tax authority and perform no evidence-file or network I/O.
Python exposes no recipient AI tools; this port adds none.

The 143-case corpus covers full helper envelopes and all 112 tables on SQLite
and SQLCipher, including the reference's binding-specific constraint errors.
Native process checks compare 13 CLI mutation calls and 13 daemon mutation
requests plus two daemon reads, with zero fallback and unchanged file hashes.
Late statement, deferred-commit and busy-commit refusals restore all rows on
both bindings; nested operations retain unrelated pending work. The inventory
becomes 162 native CLI leaves and ten source-funds daemon kinds.

The [recipient checkpoint](../reference/rust-shell.md#source-funds-recipient-mutation-checkpoint-r5-partial)
records verification and named mutation kills. The synthetic constraint-error
reproduction preserves Python unchanged. Link mutations, batch review,
persisted suggest/assemble and complete case saving still remain; the latter
awaits native report/diagram/fingerprint production. Source AI consent/once-only
admission and internal-error traceback/debug process parity remain unaccepted.
PDF-containing exports remain Python; there are no dependency, lockfile, schema
or tax-rule changes.

### R5 authored link mutation continuation (partial)

The mutation continuation adds link creation, review and managed-id evidence
association through the native report service, CLI and daemon. Creation checks
endpoint exclusivity, dates, same-asset self-transfer rules and allocation
bounds before returning a duplicate. Review preserves omitted stored msat,
rejects allocation edits on rejection and checks supplied inspection amounts
before any write. These provenance records establish no ownership or tax facts.

A 308-case frozen corpus compares complete helper envelopes and all 112 tables
on SQLite and SQLCipher, with unchanged attachment bytes/hashes. Process tests
compare 20 CLI mutations and 17 daemon mutations plus two reads with zero
fallback. Busy and deferred commits and late statement/attachment refusals
restore all rows; nested statement failures retain unrelated pending work.
The inventory becomes 165 native CLI leaves and 13 source-funds daemon kinds.
The [link checkpoint](../reference/rust-shell.md#source-funds-link-mutation-checkpoint-r5-partial)
records named mutation kills, verification and the fractional inspection-input
Python reproduction. Python product code is unchanged.

Batch review, persisted suggest/assemble, native AI consent/once-only admission,
internal-error diagnostics and complete case saving remain separate boundaries.
The report producer branch has no new commits available at this checkpoint;
traversal alone cannot authorize case saving. No dependencies, lockfiles,
schema or tax rules change.

### R5 source-funds honest-native diagnostics continuation (partial)

Registered source-funds daemon failures now retain an empty native debug
string for mapped unhandled errors; typed refusals retain null. The CLI keeps
null without `--debug` and an empty string when requested. This applies the
approved diagnostics contract without synthesizing Python tracebacks. All
non-debug response bytes remain identical, including daemon secret redaction.
The two stored-read internal-error witnesses are passing assertions instead
of expected failures.

The transport matrix covers SQLite and SQLCipher, both transports and both
debug modes, every one of the 112 tables, unchanged managed evidence bytes and
hashes, late attachment refusal rollback and zero fallbacks. Infinite expected
msat inputs retain Python's unhandled overflow envelope; NaN retains validation.
The link corpus grows from 308 to 312 cases. Named mutation tests cover debug
presence, kind/code scope, actual dispatch, redaction, infinite preconditions
and late response projection rollback.

The unchanged Python source/link attachment APIs commit before response
projection. Corrupt stored amounts can produce an error after committing the
association; Rust rolls back that failure. A local reproduction records this
reference discrepancy and the CLI's unredacted error message. This path has
separate native rollback tests and is excluded from parity acceptance, rather
than being normalized into a successful comparison. See the
[diagnostics checkpoint](../reference/rust-shell.md#source-funds-diagnostic-wire-checkpoint-r5-partial).

Batch review, persisted suggest/assemble, source AI consent/once-only admission
and complete case saving remain pending. The native AI execution loop and
report/diagram/fingerprint producer are still unavailable at this checkpoint.
There are no new routes, dependencies, lockfiles, schemas or tax rules.

### R5 source-funds batch-review continuation (partial)

The native CLI and daemon now approve bounded, target-scoped deterministic
suggestions under writer admission. Eligibility rechecks current custody
projections or authoritative wallet-scoped structural lineage; empty selections,
rejected pairs, privacy boundaries, stale versions and ambiguous quantities
grant no approval. CLI dry runs restore all state and retain their response
marker. Provenance never grants ownership or tax authority.

The 131-case batch oracle and 21 structural vectors pin exact responses, all
112 tables, unchanged evidence hashes and late rollback on SQLite and SQLCipher.
Transport checks require zero fallbacks, and 31 named mutations cover admission,
eligibility, exact-msat lineage and adapters. Python's batch post-commit
projection bug has a separate reproduction and stronger native rollback test.
See the [batch checkpoint](../reference/rust-shell.md#source-funds-batch-review-checkpoint-r5-partial).

The inventory becomes 166 CLI leaves and fourteen source-funds daemon kinds.
Persisted suggest/assemble, native AI consent/once-only execution and complete
case saving remain pending. The report producer is now available on the reads
branch for the next cherry-pick. PDF rendering and PDF-containing bundles remain
Python; dependencies, schema and tax rules are unchanged.

The `t3code/r5-sof-reads-5` continuation starts at `6e63d400a`, after the
complete report producer, and ports review-context/request-input through the
shared native service, CLI and daemon. Canonical recipes, inspection caps,
fingerprints, current deterministic bulk eligibility and advisory handoffs
retain the Python contract. The
[review checkpoint](../reference/rust-shell.md#source-funds-review-and-input-checkpoint-r5-partial)
records byte/all-table comparisons, native routing and the 34 new mutation
kills. Coverage and suggest/assemble dry runs remain separate read boundaries;
Python has no assemble dry-run CLI/daemon contract at this base. Case saving
and persisted provenance operations stay with the parallel mutations worker.
PDF and PDF-containing bundles remain Python.

The same continuation then adds native historical inbound coverage through
CLI and daemon. Classification delegates to canonical report gates and retains
all six exhaustive buckets, exact unbounded totals, lazy request-bound inputs,
CLI/daemon limits and Python exception families. Its
[coverage checkpoint](../reference/rust-shell.md#source-funds-coverage-checkpoint-r5-partial)
records the 111-case oracle and 26 new mutation kills, plus the preserved Python
truncated-summary display bug. Suggest/assemble dry runs remain separate.

The following suggestion boundary admits only CLI `source-funds suggest
--dry-run` through the native report service. Current stored custody and the
existing structural deriver supply candidates; deterministic ordering, provider
confidence, existing-link vetoes and target-frontier behavior retain Python's
contract. The report savepoint rolls temporary rows and attachment cascades
back on success and failure. The
[suggestion checkpoint](../reference/rust-shell.md#source-funds-suggestion-preview-checkpoint-r5-partial)
records its frozen oracle, native routing and mutation evidence. Persisted
suggest, daemon suggest/assemble and case saving stay with the mutations
worker. Assemble's shared preview contract is still required: Python has no
such CLI/daemon surface and commits inside its assembly passes.

### R5 source-funds persisted passes and route ownership

Continuation 5 integrates the finished read line with case saving and ports
persisted suggestion and assembly through the shared Rust service. Assembly
freezes its inputs once and commits suggestion and review stages separately
within each pass, retaining completed stages after later failures. Preview,
exact allocations, scope, caps, deterministic eligibility and reviewed-save
fingerprints retain their existing contracts. The 269-case pass corpus checks
both bindings, complete helper envelopes, all 112 tables, attachment hashes and
late rollback; process comparisons require zero native fallbacks.

The [route audit](../reference/source-funds-rust-route-audit.md) accounts for
19 native CLI leaves and 21 native daemon kinds. The owner assigns AI chat/tool
execution, source admission and case saving to Python under R6. Its tool layer
calls Python's scoped services directly and retains consent, duplicate-call
suppression, book/input binding and redaction. Direct native CLI/daemon services
preserve all checks enforced outside that loop. PDF rendering and
PDF-containing bundles remain Python-owned through both transports.

The [persisted-pass checkpoint](../reference/rust-shell.md#source-funds-persisted-pass-checkpoint-r5)
records rollback, projection-bug repros, process routing, named mutation kills
and final verification. This completes the source-funds CLI/daemon mutation
boundary without adding a native AI loop or changing Python product behavior,
dependencies, schemas, egress or accounting rules.

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

### R5 exchange file-import batch 1: JSON checkpoint (partial)

The original checkpoint below is retained as history. Rebasing `2779162e` onto
`1e6565c8` also ports the current reference's finite-fiat validation, including
legacy float overflow and derived totals. The regenerated oracle has 589
cases (444 JSON), with four more named mutations (115 wallet entries). Python
production code and the newer stack's catalog/service boundaries are preserved.


`wallets import-json` now runs in Rust through the shared local file service.
The native adapter preserves CPython decoding and typed scalar coercion,
exact decimal/msat conversion, raw provenance, dedupe, import-run membership,
rollback and journal invalidation. Shared identity handling includes idless
row promotion, fee-relaxed execution evidence and cumulative routing receipts;
it does not grant file inputs observer authority or tax normalization authority.

There are 426 JSON oracle vectors, three additional native persistence/refusal
tests and 28 new mutation entries (111 wallet entries total). Hybrid comparisons
cover complete typed book dumps for every JSON vector and the existing 15
JSON-using review regressions; wire/state cases also cover both SQLite and
SQLCipher through rollback. See the [JSON checkpoint](../reference/rust-shell.md#json-file-ingestion-checkpoint-r5-importers-batch-1-partial)
for verification, routing counts and named mutation killers.

JSON routing was 443 Rust calls and zero Python fallbacks. All 111 mutations
were killed. Native fmt, clippy, workspace/harness tests and builds passed;
the requested Python suite passed 70 tests and 754 subtests, including the
inventory and contract checks, plus 15 existing JSON review regressions.

This checkpoint stops after JSON and does **not** complete batch 1. River,
Bull/Bullbitcoin, Coinfinity, 21bitcoin, Pocket/Pocketbitcoin and Strike remain
unported. Batch 2, exchange API syncs, wallet sync and AI projections remain
outside the checkpoint. Python production code is unchanged; no reference bug
was found and no dependency or egress path was added.

### R5 exchange file-import batch 1: River continuation (partial)

`wallets import-river` now uses the shared native ingestion service. Original
CSV heading order preserves Python's duplicate/colliding alias behavior;
247 frozen cases cover accepted/refused encodings, dialect, header, numeric,
fee, cash/BTC leg, timestamp, identity and provenance branches. Notes/tags,
import runs, rollback and journal invalidation remain in the same savepoint.
Priced repeats preserve Python's refresh behavior. The current JSON reference
has 444 cases, including its finite-price validation correction on this base.

The continuation adds 54 named River mutations to the 115-entry JSON/rebase
catalog. Native vectors and metadata atomicity tests, hybrid full typed book
dumps, SQLite/SQLCipher wire/state/rollback comparisons and final verification
are recorded in the [River checkpoint](../reference/rust-shell.md#river-file-ingestion-continuation-r5-importers-batch-1-partial).

Final verification passed: fmt/clippy, workspace/harness tests, vector drift,
the requested Python suite (80 tests and 1,035 subtests), 25 selected hybrid
regressions, inventory/contracts and all 169 wallet mutations. Importer routing
totals 461 JSON and 248 River Rust calls, with zero importer Python fallbacks.

This is a partial batch boundary. Bull/Bullbitcoin, Coinfinity, 21bitcoin,
Pocket/Pocketbitcoin and Strike remain unported. Batch 2, exchange API syncs,
wallet sync and AI projections remain out of scope. The separate reference fix
`86d364cf` corrects overlapping `LBTC`/`BTC` suffix removal and has a failing-before
buy/sell regression. River also preserves context-sensitive lowercase across
the whole provider tag code. No dependency or egress path is introduced.

### R5 exchange file-import batch 1: Bull continuation (partial)

`wallets import-bull` and `import-bullbitcoin` now port the commercial-evidence
coordinator as well as the parser. Relevant mode enriches unique profile-wide
matches without inserting rows; full mode creates excluded evidence with
matched/gap/ambiguous reconciliation flags. Fees, authored metadata and closed
observation commitments remain separate from provider execution evidence.
Default-wallet creation and its commit-before-file-read behavior match Python.

The 206 frozen parser cases run through both modes with complete typed book-dump
comparison and native served-by assertions. Coordinator, SQLite/SQLCipher
wire/state/rollback and failure-injection tests cover the surrounding paths.
The 59 added mutations pin parsing, mapping, matching, reconciliation, defaults,
refusals and invalidation; shared CSV/decimal/dedupe/run/atomicity mutations
remain applicable. See the [Bull checkpoint](../reference/rust-shell.md#bull-file-ingestion-continuation-r5-importers-batch-1-partial)
for the named killers and final verification.

Hybrid acceptance totals 425 Bull Rust calls and zero Bull Python fallbacks:
412 mode/vector combinations, six coordinator calls and seven existing fixture
imports. Ten SQLite/SQLCipher import cases also compare wire bytes, complete
state and rollback. Final fmt/clippy, 1,309 workspace tests and 14 harness tests
passed. The requested Python suite passed 83 tests and 1,457 subtests; after the
final error-order correction, native checks and all affected Bull parity/fixture
tests passed again on the rebuilt harness.
All 228 wallet mutations were killed after 43 unmutated named baselines passed;
the temporary mutation source/build trees were removed.
The final classifier update and preflight also passed (57 checks).

Coinfinity, 21bitcoin, Pocket/Pocketbitcoin and Strike still need their own
verified importer commits. Batch 2, exchange API syncs, wallet sync and AI
projections remain out of scope. Python remains the reference.

### R5 exchange file-import batch 1: Coinfinity continuation (partial)

Native `wallets import-coinfinity` includes relevant/full coordinator paths,
provider-wallet creation/reuse, reference-first matching and conservative
profile-wide economic matching when both transaction id and invoice are absent.
The inclusive two-day window preserves Python's timestamp overflow errors;
ambiguous rows never become guessed ownership. Full imports remain excluded
commercial evidence with rollback-capable runs and reconciliation flags.

The 248 frozen Python files pin UTF-8-sig/BOM behavior, CSV/header branches,
broker-side trade reversal, exact fiat/fee arithmetic, integer msat, references,
timestamps and provenance. Complete typed book dumps compare in both modes;
additional scenarios cover coordinator admission, dedupe and rollback.
Verification, 505 native hybrid calls with zero fallbacks, and the 53 added
mutations (281 wallet mutations killed in total) are recorded in the
[Coinfinity checkpoint](../reference/rust-shell.md#coinfinity-file-ingestion-continuation-r5-importers-batch-1-partial).
21bitcoin, Pocket/Pocketbitcoin and Strike remain before batch 1 is complete.

### R5 exchange file-import batch 1: 21bitcoin continuation (partial)

Native `wallets import-21bitcoin` imports the full active custodial ledger and
keeps provider IDs separate from chain identities. It preserves BTC/LBTC legs,
net fiat sale proceeds, exact fees/prices, UTC and dotted-date variants, blank
row filtering, UTF-8/BOM behavior, provenance, dedupe and rollback-capable runs.
Relevant mode refuses before wallet/file admission. Reimports clear legacy
reconciliation tags and reactivate only the corresponding tagged rows; lookup
fallbacks bind wallet, provider reference, direction, asset and exact msat.

The 209 frozen Python files compare complete book dumps in full mode. Native
coordinator, hybrid and encrypted/plaintext wire/state tests cover updates,
legacy flags, default-wallet admission, rollback and refusals. The 67 added
mutations and verification are recorded in the
[21bitcoin checkpoint](../reference/rust-shell.md#21bitcoin-file-ingestion-continuation-r5-importers-batch-1-partial).
Pocket/Pocketbitcoin and Strike remain before batch 1 is complete.

### R5 exchange file-import batch 1: Pocket continuation (partial)

Native `wallets import-pocket` / `import-pocketbitcoin` preserve gross-to-net
withdrawal pairing, original header insertion order, stable date ordering,
single-use candidates, BTC/LBTC aliases, exact Decimal prices/fees, integer
msat, delivery versus execution timestamps and nested provenance. Both aliases
share relevant/full evidence admission, default-wallet reuse, conservative
profile-wide economic matching, excluded full rows, dedupe, runs and rollback.

The 262 frozen Python files compare complete book dumps in both modes.
Coordinator and encrypted/plaintext wire/state tests pin matching, ambiguity,
window boundaries, reimports, wallet admission and atomic rollback. The 52
added mutations and verification are recorded in the
[Pocket checkpoint](../reference/rust-shell.md#pocket-file-ingestion-continuation-r5-importers-batch-1-partial).
Strike remains before batch 1 is complete.

### R5 exchange file-import batch 1: Strike and completion

Native `wallets import-strike` imports the active custodial ledger in full mode,
with provider-wallet admission and relevant-mode refusal before file access.
It preserves exact Decimal/msat amounts and fees, fiat-column precedence,
English-month/Unicode date rules, trade and transfer kinds, separate Lightning
payment hashes/on-chain IDs, conditional pricing provenance, dedupe, updates,
runs, journal invalidation and rollback.

The 307 frozen Python files compare complete typed book dumps. Native,
coordinator and encrypted/plaintext wire/state tests verify surrounding paths.
The 70 added mutations and final verification are recorded in the
[Strike checkpoint](../reference/rust-shell.md#strike-file-ingestion-continuation-and-batch-1-completion-r5).

Batch-1 command coverage is complete. Workspace/harness fmt and strict clippy,
1,523 workspace tests, 14 harness tests, 95 requested Python tests and 3,027
subtests, inventory/contracts and 57 preflight checks pass. All 470 wallet
mutations are killed. The following separate reference correction closes the
legacy 21bitcoin readiness issue reproduced during review.

### R5 reference correction: legacy 21bitcoin journal readiness

A withdrawal-only regression failed before the Python fix: clearing old
reconciliation flags reactivated an excluded observation but left the journal
input version and readiness unchanged. Python and Rust now invalidate once on
reactivation, preserve the existing invalidation for priced updates, and retain
readiness for tag-only cleanup on already-active rows. Failed invalidation
rolls back the cleanup. This changes readiness, not provenance, custody review,
tax normalization or the observation's imported amount/fee.

Native plaintext/encrypted tests, the Python regression and three additional
hybrid full-book comparisons pin the correction. Eight new mutations
cover both implementations; the wallet catalog now has 478 entries. Final
verification and named killers are recorded in the
[readiness checkpoint](../reference/rust-shell.md#legacy-21bitcoin-reactivation-readiness-correction-r5).
The corrected source passes fmt/strict clippy, 1,525 workspace tests, 14 harness
tests, 96 requested Python tests and 3,027 subtests, the final inventory update,
contracts and 57 preflight checks. All 478 mutations are killed. Batch 1 is
complete for these file-import commands, with zero importer fallbacks.

### R5 file-import batch 2 sizing

Batch 2 is nine provider/format slices, with separate acceptance and commits.
These are relative implementation sizes, including parser/coordinator parity,
edge-file dump comparisons, rollback and named mutation evidence, rather than
elapsed-time promises. CoinTracking/Blockpit share tax-platform normalization,
including the generic-ledger semantics their parsers use. That shared core can
precede the complete generic-ledger file loader.

| Importer | Size | Main work beyond batch 1 |
| --- | --- | --- |
| `ledger-live` | Small | Movement CSV, operation aliases, exact fees, xpub redaction; informational fiat values remain non-authoritative. |
| `ledger` | Large | CSV/TSV/XLSX loading, column-map validation/inference, locale/type mapping, per-row refusals and empty-file rules; XLSX needs a native format strategy because the current Rust manifest has no spreadsheet/ZIP/XML reader. |
| `cointracking` | Medium | Shared tax-platform normalization, dedicated migration-wallet admission, standalone fee/cross-asset refusals, row identities and revision/history behavior. Prior tax reports remain evidence. |
| `blockpit` | Medium | Same migration coordinator plus localized type/header families, standalone BTC fee legs and non-taxable expense mapping. |
| `binance-supplemental` | Medium | AutoInvest and dividend variants, provider references and evidence matching, without duplicating authoritative wallet observations. |
| `wasabi` | Large | Sanitized JSON/bundle loading, wallet metadata, labels and transaction/privacy provenance with sensitive fields discarded. |
| `samourai` | Large | Parent/child wallet sets, deposit/badbank/premix/postmix/ricochet account plans, descriptors and privacy metadata; retain watch-only and no-contact admission. |
| `btcpay` file | Medium | CSV/JSON net wallet movements, confirmation aliases, import-authored comments/tags and raw provenance. Greenfield/API synchronization is a separate network slice. |
| `document` | Largest | Draft validation and confidence/quarantine rules, local PDF/image/OCR preparation, explicit provider-contact consent, bounded input limits, attachment storage and cross-resource rollback. |

This is substantially larger than the four-importer continuation: one small,
four medium, three large and one largest slice. Reuse the exact Decimal/msat,
CSV, evidence and active-ledger coordinators without merging provenance,
commercial matching, reviewed custody or tax authority. No batch-2 command is
enabled by batch-1 completion.

The eleventh reports continuation ports the graphless Liquid parse-success
boundary used by saved dependency capture, with 5,634 Python physical-scope
vectors and fifteen killed mutations. Decoding supplies only the existence
of a partial public graph; stored identity and authoritative observation
checks still decide dependencies. Twelve whole export sequences compare
52 actions, 75 CSV files and all 112 tables. No dependency or Python product
change is needed. See the
[Liquid checkpoint](../reference/rust-shell.md#graphless-liquid-report-dependencies-r5-eleventh-continuation-partial).
The same continuation ports the reviewed commercial subledger read and CSV
export through two CLI leaves, with fourteen Python vectors and nine killed
mutations. Their evidence-only projection preserves Python's behavior with
stale journals and excluded sources; no daemon kind or saved marker is added.
See the [commercial checkpoint](../reference/rust-shell.md#reviewed-commercial-subledger-r5-eleventh-continuation-partial).

### R5 file-import batch 2a: Ledger Live checkpoint (partial)

Native `wallets import-ledger-live` ports the first batch-2a importer, including
its existing wallet-local coordinator paths. It retains movement-only
BTC/LBTC mapping, Decimal/msat amounts and outgoing fees, indexed missing-hash
identities, Python timestamps and refusals, sanitized provenance, dedupe,
empty-metadata enrichment, import runs, journal invalidation and rollback.
Informational countervalues do not establish exact pricing or tax authority.

There are 168 frozen Python files for parser and complete normalized book-dump
comparison, hybrid routing assertions, plaintext/encrypted wire-and-state
comparisons and native coordinator/late-failure tests. The
[Ledger Live checkpoint](../reference/rust-shell.md#ledger-live-file-ingestion-r5-batch-2a-partial)
records verification and the 34 added named mutations. The exact requested base
contains 470 wallet mutations, so this checkpoint's catalog has 504 entries.
All 504 mutations are killed after 67 passing baselines; the hybrid Ledger Live
scenarios record 175 Rust-served imports and zero Python fallbacks. Workspace
and harness formatting/clippy, 1,526 workspace tests, 14 harness tests, the
requested Python suites (98 tests and 3,207 subtests), inventory/contracts and
57 preflight checks pass.

CoinTracking, Blockpit, Binance supplemental and BTCPay file import remain in
batch 2a. Generic ledger, Wasabi, Samourai and document ingestion stay deferred.
The shared tax-platform normalization and dedicated migration-wallet
reconciliation are prerequisites within the CoinTracking/Blockpit slices;
prior tax reports remain evidence. No sync/API/AI path is enabled here.

### R5 file-import batch 2a: CoinTracking core/CLI checkpoint (partial)

The continuation includes the legacy 21bitcoin readiness reference correction
and ports CoinTracking parsing, private row identities, dedicated migration-wallet
admission and transactional overlap reconciliation to the native file service.
It retains unsupported cross-asset/type/fee barriers, original rows, audited
inclusion choices, import runs, enrichment, reimport readiness and rollback.
There are 301 frozen Python edge files, full typed book-dump CLI comparisons,
plaintext/encrypted wire/state checks and 38 new named mutations. The
[CoinTracking checkpoint](../reference/rust-shell.md#cointracking-core-and-cli-ingestion-r5-batch-2a-partial)
records final verification and served-by counts.

The continuation accepts core and CLI importer checkpoints independently of
the [shared file-import daemon follow-up](../reference/rust-shell.md#shared-file-import-daemon-parity).
That shared item owns the pending transport contract for every importer.

### R5 file-import continuation: Blockpit core/CLI checkpoint

Native Blockpit ingestion extends the shared tax-platform core with required
and optional header families, English/German labels, standalone Bitcoin fee
legs and non-taxable outflow review kinds. Dedicated-wallet admission, private
row identities, original evidence, canonical-id/economic reconciliation,
provider-isolated tags, authored inclusion choices, enrichment, ID promotion,
reimports, readiness and atomic rollback retain Python behavior.

There are 365 frozen Python files. Native normalized-field/fingerprint vectors,
hybrid full-book comparisons with served-by assertions, and raw CLI wire/full
typed book-dump comparisons for every file on SQLite and SQLCipher cover the
slice. Coordinator sequences and encrypted/plaintext fault tests supplement the
file corpus. The shared bare-CR reference bug is retained with a Blockpit repro;
Python production code remains unchanged. The
[Blockpit checkpoint](../reference/rust-shell.md#blockpit-core-and-cli-ingestion-r5-continuation)
records mutation killers and final verification.

The fresh hybrid scenarios record 373 Rust-served Blockpit imports and zero
importer fallbacks. The mutation catalog adds 61 named Blockpit entries to the
retained 550 (611 total).

Remaining core/CLI slices, each requiring its own verified commit, are Binance
supplemental, BTCPay files, generic ledger, Wasabi, Samourai and document
ingestion. No dependency, egress, sync route or AI projection is added.

### R5 file-import continuation: Binance supplemental core/CLI checkpoint

Native `wallets import-binance-supplemental` parses autoinvest and dividend
CSV files through the shared local file service. Exact fiat-funded buys,
fee-inclusive acquisition value, BTC/LBTC normalization, dividend millisecond
timestamps and mining/income classification follow Python. Both full and
relevant modes retain profile-scoped commercial matching, fee precedence,
provider tags, excluded evidence rows, enrichment, import runs and rollback.
Supplemental identifiers stay namespaced source references; they never promote
a transaction into an authoritative chain identity.

The reused enrichment coordinator also corrects an inherited response-key order
mismatch. Bull, Coinfinity and Pocket now have relevant-mode wire regressions
on both codecs; NaN timestamp payloads retain Python's exact refusal.

The corpus freezes 253 Python-derived files. Native field/fingerprint tests,
hybrid full-book comparisons and every file's raw CLI bytes/full typed dumps
on SQLite and SQLCipher cover the boundary. Coordinator sequences exercise
overlap, identity-promotion refusal, enrichment, reimports, price/schema/input
refusals, default wallet creation and rollback. Python's conflicting-base-symbol,
row-index and Decimal-scale identity quirks are retained with an offline repro.
The [Binance checkpoint](../reference/rust-shell.md#binance-supplemental-core-and-cli-ingestion-r5-continuation)
records served-by counts, mutation killers and final verification.

The hybrid run records 261 native Binance imports and zero importer fallbacks.
All 66 native file-import tests pass. The catalog contains 648 entries; the
fresh audit passes 11 named baselines and kills all 37 new entries plus the
shared atomic-import rule, without survivors or build errors.

Formatting, strict workspace/harness Clippy, all 14 harness tests and all 57
CI preflight tests pass. The inventory records 5,722 Python tests. The workspace
run has 1,485 passes, 58 sandbox socket denials and one backup assertion that
passes both isolated and original-binary rechecks; the shell checkpoint records
the complete result and retained diagnostics rather than claiming a green gate.

The remaining core/CLI importers are BTCPay files, generic ledger, Wasabi,
Samourai and document ingestion, each requiring its own verified commit.

### R5 file-import continuation: BTCPay core/CLI checkpoint

Native `wallets import-btcpay` accepts CSV and JSON files through the shared
local file service. Exact net amounts, outbound fee inclusion, confirmation
aliases, raw provider evidence, deposit/withdrawal kinds, imported notes
and normalized tags follow Python. Scoped dedupe, metadata-only invalidation,
reimports, import runs and rollback retain the existing coordinator contract.
The reference boolean-identity metadata bug is preserved with an offline repro.

The corpus contains 311 Python-derived files, with native field/fingerprint
vectors, hybrid complete-book comparisons and raw CLI bytes/full typed dumps
on SQLite and SQLCipher. Coordinator and late-failure tests cover both codecs.
Thirty-eight new mutation entries bring the wallet catalog to 686; all new
entries and the shared atomic-import rule have named kill evidence. The
[BTCPay checkpoint](../reference/rust-shell.md#btcpay-core-and-cli-file-ingestion-r5-continuation)
records the audit runs, routing counts and verification results.

The expanded hybrid run records 318 native imports and zero importer fallbacks.
All 73 native file-import tests pass. Formatting, strict workspace/harness
Clippy, all 14 harness tests and all 57 required CI preflight checks pass. The
classifier records 5,729 Python definitions. The workspace suite has 1,493
passes and the 58 known sandbox
socket denials, with no other failures; exact names are in the shell checkpoint.

Python production code, dependencies, egress, sync routes and AI projections
are unchanged. Remaining core/CLI importers are generic ledger, Wasabi,
Samourai and document ingestion, each requiring its own verified commit.
### R5 file-import continuation: Wasabi core/CLI checkpoint

Native `wallets import-wasabi` ingests sanitized history/RPC bundles, safe wallet
metadata, key-state/payment counts and output inventory. Shared transaction
normalization keeps exact quantities, source provenance, wallet-scoped matching,
classification promotion, authored notes and privacy review barriers. Empty coin
snapshots and omitted sections remain distinct; late failures roll back every
write, including journal invalidation.

There are 354 frozen Python-derived files, complete native bundle/transaction
vectors, hybrid full-book comparisons and CLI raw-wire/full typed dump parity on
SQLite and SQLCipher. Reference quirks, including representation-based quantity
units and the missing Wasabi import run, are retained with an offline repro.
The [Wasabi checkpoint](../reference/rust-shell.md#wasabi-core-and-cli-ingestion-r5-continuation)
records routing counts, named mutation killers and final verification.

The final hybrid run records 369 native imports and zero importer fallbacks.
All 72 native file-import tests, 14 harness tests and 57 CI preflight tests pass.
The catalog contains 698 entries; all 50 new rules and two affected shared rules
have verified named killers, without survivors or build errors. Formatting and
strict workspace/harness Clippy pass, and the inventory records 5,728 Python
tests. The earlier workspace audit records 1,502 passes and the 58 known
sandbox loopback denials; final shared-error, inventory and refusal-order changes
have focused verification recorded in the shell checkpoint.

The subsequent Samourai checkpoint supersedes that importer's pending state.
Document ingestion remains separate; BTCPay files and generic ledger remain
assigned to the parallel importer worker.

### R5 file-import continuation: Samourai core/CLI checkpoint

Native `wallets import-samourai` creates watch-only source groups and descriptor
children from public account keys or explicit descriptors, with the same core
entry point for inline public payloads. Account maps, networks, origins, gaps,
safe provenance, private-material refusals and redacted
wallet replies match Python. Group creation is atomic and does not create
observations, ownership evidence, custody decisions or tax inputs.

There are 544 frozen Python-derived files, including twenty-six supplemental
cases, and complete SLIP132 plan/address/fingerprint comparisons. Every file has raw
CLI wire and complete typed book-dump parity on SQLite and SQLCipher. Native
coordinator triggers cover late-child rollback; hybrid scenarios require native
routing and exact whole-book state after each invocation. The checkpoint
retains Python's synthesized-branch acceptance, unchecked explicit script
declarations, whitespace round references, SLIP132 network-family exception,
ignored xpub labels, differing parent/child minimum gaps and absent
import-run/readiness invalidation with an offline reproduction. See the
[Samourai checkpoint](../reference/rust-shell.md#samourai-core-and-cli-ingestion-r5-continuation)
for served-by counts, mutation killers and verification.

The CLI catalog contains 158 native entries. `ui.wallets.import_file` remains
unregistered until native journal processing and shared daemon acceptance land.
The document CLI continuation below completes that registration; the bare-CR
correction stays deferred.

### R5 document ingestion: core and CLI

`kassiber-wallets::file_import::document` normalizes OCR rows and reconstructs
observations from the visible reviewed fields. It enforces exact integer msat,
structural and confidence quarantine, source-bound stable identities and the
source/aggregate evidence budgets. A stable local snapshot and one savepoint
coordinate imported observations, journal invalidation and managed evidence;
late failures roll back the database and remove copies. Evidence grants no
ownership, custody, basis or tax authority. The implementation reuses the
existing transaction attachment service and adds no third-party dependency.

Python-derived frozen files cover 287 draft cases and 128 reviewed selections.
The disposable-book driver compares core response bytes, all typed book tables
and schema objects, and managed evidence on SQLite and SQLCipher. It explicitly
asserts successful imports. Generic
database failures retain the shared Rust store's redaction instead of Python's
raw exception message; this is a pinned privacy deviation because raw database
diagnostics can expose source values or local paths.
Native managed-digest failures also remove the copy that Python leaves orphaned;
this pinned privacy deviation avoids retaining untracked sensitive evidence.
Cleanup validates canonical managed paths; native regressions and an offline
reference reproduction preserve the distinction from exact Python parity.
Reference quirks, named mutation killers and verification are recorded in the
[partial document checkpoint](../reference/rust-shell.md#reviewed-document-core-r5-partial).

The [provider continuation](../reference/rust-shell.md#document-provider-transport-checkpoint-r5-partial)
adds configuration bootstrap, model discovery and OCR through `kassiber-net`,
with exact-action consent and the Python local-AI offline/no-egress exception.
The PDF continuation uses the same Poppler tools, arguments, page/DPI/image
limits and 30-second timeouts as Python for OCR preparation, including private
temporary images and cleanup. PDF report/export rendering stays in Python.
The [CLI continuation](../reference/rust-shell.md#document-cli-registration-r5)
adds interactive review, runtime wiring and both native CLI registrations.
Its SQLite/SQLCipher acceptance compares exact process output, complete typed
books, managed evidence and outgoing provider requests across Python, raw Rust
and the hybrid. All 64 hybrid document calls run in Rust with zero fallback;
the native catalog now contains 160 entries. The shared daemon importer has
no new native calls.
All fifteen document CLI mutation recipes are killed by their named tests;
the linked checkpoint records their mapping and the local verification scope.
`ui.wallets.import_file` stays unregistered until native `journals process`
and shared daemon acceptance land. R6 document AI projections and session/token
flows remain on Python. PDF report generation also remains Python.

### R5 desktop wallet onboarding

The desktop kinds a user needs to add a wallet move to Rust in groups. Local
identification (`ui.wallets.identify`), the uploaded-ledger preview and
script-type detection are native. Detection is not a local read: like Python
it probes the selected backend, so it runs only as the user's explicit action
through `kassiber-net`, with a grant bound to that action. Identification and
the preview make no network calls. Raw JSONL and all 112 tables match Python
on SQLite and SQLCipher; 34 mutation entries are killed by named tests. See
[the shell reference](../reference/rust-shell.md#desktop-wallet-onboarding-r5).

On-chain identification streams from a worker that never touches the book;
its lookups are the explicit "Verify on chain" action through `kassiber-net`,
and cancellation follows Python's registry, between lookups only. Desktop file
imports, Samourai source sets and file analysis are native; each import ends
with the native local journal step. The Bull Bitcoin wallet-history format
still refuses with `not_implemented`. Document staging, local-OCR previews and
reviewed imports run natively behind daemon-held grants; a document grants no
ownership, basis or tax fact. Staging into a review case stays refused.
The descriptor reveal keeps Python's fresh-authentication guard on every
request and returns only pasteable material; no AI tool reaches it.

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

The [R4 design](../reference/r4-journal-pipeline.md) (2026-10-30; acceptance
gates remain open, with the R4-11 checkpoint below) profiles Python's `journals process` by phase and plans the Rust binary's
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
and a ratio gate in CI and a local 20,000-transaction gate. The owner decisions
on production routing, performance targets, automatic pairing and persisted
shapes are recorded in the design. Book-open cost and allocator decisions
remain open.

### R4-9 native loader acceptance

The [loader checkpoint](../reference/r4-native-loader-checkpoint.md) compares
corruption refusals at the CLI/daemon code and rendered-message boundary.
Loader-7 closes 216 of the 232 non-BLOB composed audit gaps: component integer
messages, early amount/fee admission and component timestamp sorting. Native
loading retains database scalars for the reference-equivalent consuming stage;
the JSON adapter keeps its strict signed-64 quantity contract. Infinite rates
retain the exact downstream finite-decimal validation message.

The BLOB continuation closes all 4,446 previously unresolved BLOB outcomes.
The full composed audit passes all 22,528 cases, including 6,144 actual BLOB
attempts; the separate 16,384-case non-BLOB rerun also passes. Both record
actual `typeof()`, zero writes and zero native admissions of Python-refused
projections. Typed raw bytes reach each consumer's Python-equivalent conversion,
ignore or serialization policy, including exact CLI/daemon TypeError envelopes.

Loader-9 completes the [exhaustive branch map](../reference/r4-native-loader-branch-map.md)
and kills all **308 loader recipes** and **2,292 custody recipes**. Three custody
recipes are re-anchored to their stated contracts and killed in fresh shards;
all other original kills are retained with their source/catalogue fingerprints.
The final 24,000 generated books and 1,030 permutations, 159 retained requests
and their permutations, and focused census/shadow pass. Each focused lane also
proves 53 exact native-loader byte matches and replays 53 fresh requests and
permutations. The [acceptance record](../reference/r4-native-loader-checkpoint.md#loader-9-acceptance)
owns the complete counts, re-anchor review and resumable campaign provenance.

The integrated R4-9 stage is **fully accepted, with no pending exceptions**.
The port-branch Python reference and native loader reject incompatible revision
reference families with one typed `validation` message before revision sorting
or queries. Exact matching replaces measured variant-set acceptance. Fresh
22,528-case composed and 16,384-case non-BLOB audits pass; the forty historical
message cases are stable in fresh Python processes under eight hash seeds.
All 357 combined loader mutations are killed, including the new reference guard;
the inherited full 2,292-entry custody campaign remains complete. Fresh 24,000
books/1,030 permutations, 159 retained requests and permutations, focused
census/shadow and static gates pass. The
[current acceptance record](../reference/r4-native-loader-checkpoint.md#deterministic-revision-reference-acceptance)
distinguishes the fresh evidence from inherited campaigns and timing.

The BLOB same-host timing comparison remains within 5% of the retained loader-8
baseline; this determinism continuation does not claim a fresh benchmark.
Python retains production journal authority, and the native loader remains
behind `test-harness` until its separate integration/authority gates. Shipped
Python was audited separately and is unaffected by the port-branch reference fix.

> Inherited from `t3code/r4-13-journals-process-6@684c76bad`; combined R10 results are in the [R10 audit](../reference/rust-backend-r10-integration.md).
Loader-8's paired 20k CPU totals including admission remain 0.288331 s generic
and 0.428645 s multi-wallet, both within 5% of loader-7. Those timings and the
16,384-case non-BLOB audit are inherited unchanged; the final campaign results
above belong to loader-9. The BLOB continuation closes the 4,446 storage
gaps and extends the pending message set to forty; merged verification is
recorded in the R4-13 checkpoint.

> inherited from t3code/r5-transfers-17@59e1476a4; combined R10 results are in the [R10 audit](../reference/rust-backend-r10-integration.md).
16,384-case non-BLOB audit are inherited unchanged; the loader-9 campaign results
above belong to that branch. The BLOB continuation closes the storage-class
gaps and expands the measured seed-dependent message set to forty. Combined
transfer-branch verification is recorded in the
[transfers-14 handoff](../reference/rust-shell.md#accepted-loader-integration-r5-transfers-14).

### R4-0 integrated stack

The local `t3code/rust-backend-r6` stack integrates the R2 custody, projection
review, performance and fusion lines with R5 reads, diagnostics and the shipped
Python corrections. The [integration audit](../reference/rust-backend-r6-integration.md)
records the linear picks, skips, conflict resolutions, custody campaigns,
hybrid parity and full mutation rerun. The round-4 local
`t3code/rust-backend-r7` stack adds the transaction page index, shipped Python
fixes, transfers/review, pairing library, report exports, sync/importer
checkpoints, owner decisions, bench harness, rendering and compact storage.
Its [integration audit](../reference/rust-backend-r7-integration.md) records
the ordered replay and verification. R4-0 is complete; the remaining R4
acceptance gates and production journal authority switch stay open.

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

### R5 Bitcoin sync: observer persistence prerequisite

The unregistered `wallets::observer_apply` writer persists private observer JSON
and version-two coverage through `store::observer`. It requires the caller's
transaction, validates all coverage before writing and compares the epoch
captured during preparation. Replacements preserve creation time, increment
the epoch and replace the complete coverage snapshot, including an empty one.
Frozen row/error vectors are generated by unchanged Python persistence code.

This is **not completion of atomic persistence (step 2)**. Transaction import,
retractions, provenance/graph writes, output inventory, durable policy coverage,
saved-report effects, wallet sync metadata, freshness and journal invalidation
still need a scope-fenced wallet coordinator and full post-sync book parity.
No product route or `IMPLEMENTED` entry changes. See the
[storage checkpoint](../reference/rust-shell.md#bitcoin-observer-persistence-prerequisite-r5)
for verification and the remaining acceptance work.

### R5 Bitcoin sync: atomic publication checkpoint

The unregistered `wallets::publication::wallet::apply` publishes a prepared
Bitcoin descriptor-wallet fetch under one rollback-on-drop savepoint. It fences
captured wallet configuration, book network, the supported single-wallet
profile shape and each observer's captured epoch. It writes observer/checkpoint
state and policy coverage, composes facts, retracts and inserts/merges records,
persists closed provenance, appends saved-report impacts, refreshes inventory,
and updates wallet/freshness and journal staleness together. All publication
writers require an existing transaction. Other profile wallets, enrichment,
alternate source routes and unported negative-balance repairs refuse.

Full normalized books match unchanged Python for ten fixtures and eight
three-round sequences on Electrum and Esplora, including subprocess restarts.
Reviewed-book witnesses add saved-report and current-journal cases. These are
**publication proofs using Python-prepared fetches**; the Rust replay also runs
offline from frozen full-book vectors. The original 78 rounds regenerate
unchanged; the expanded corpus has 102 successful rounds. Every row-write
position in that corpus is injected, including repeated writes, foreign-key
cascades and graph triggers: 2,447 failures restore all logical rows. Separate
tests cover cancellation, panic, stale epochs, outer transactions and the
SQLCipher logical/physical rollback distinction.

The reviewed Esplora reorg case also reproduces a Python reference cache bug:
unchanged script statistics can hide changed confirmation evidence until a
force-full refresh. A checked-in disposable repro and TODO record the bug;
Python remains unchanged and publication parity preserves its current outcome.

No sync route or `IMPLEMENTED` entry changes. Native prepared-fetch assembly,
complete native output/coverage facts, shared CLI/daemon freshness routing,
consent/scope and unsupported-route checks, a native end-to-end book proof and
zero-fallback hybrid workloads remain before registration. See the
[publication handoff](../reference/rust-shell.md#bitcoin-atomic-publication-verification-r5)
for exact verification and mutation results.

### R4-13 process composition and route registration (partial)

The [pipeline checkpoint](../reference/r4-journal-process-checkpoint.md) composes
the native loader, custody kernels, fusion/engine, rendering and persistence
under one writer transaction. Production now binds the shared scope resolver
and `SystemSources`, and registers CLI `journals process` with optional automatic
pairing. Its hybrid launcher replays a refusal through Python only after
byte-identical rollback; successful admitted invocations stay native. R4-10's
ordered pre-build is integrated. Automatic pairing preserves Python's separate
first-rebuild commit and atomic pairing/second-rebuild transaction.
Desktop `ui.journals.process` uses the same scoped refresh service and foreground
execution slot as the native freshness journal job, with automatic pairing,
ordered progress, cancellation and connection-owner retirement. Its direct
processing preserves Python's table mutations without queueing a freshness job.
The [seventh continuation](../reference/r4-journal-process-checkpoint.md#seventh-continuation-desktop-registration)
records the separate verification boundary. Transitional JSON boundaries
remain open. The assistant's internal tool, whose consent covers a rebuild
without automatic pairing, has a verified native journal step on the same
service; Python's chat loop serves it until R6 ports `ai.chat`. The desktop
journal reads, the explained quarantine page and quarantine resolution are
also native and registered, so every journal desktop kind is served natively. Registered-route
campaigns, census/shadow, inherited mutation and performance acceptance are
still required. Python retains production authority until R4-16.

### R4 end-to-end performance (prepared-book pipeline)

The [end-to-end performance record](../reference/r4-e2e-performance.md)
measures the stack's prepared-book pipeline against Python's `journals
process`. At `06e253ea6` generic 20k was 3.50×, below the 4× target. With
the finalized projection and quantity stage read through views instead of
rendered JSON and the in-process engine on the calling thread, every bench
book is at least 4.68× (median of eleven pairs), with stored books
byte-identical.

### R4-11 rendering checkpoint (partial)

`crates/journals` renders the typed tax-engine `LedgerResponse` and joins the
finalized custody facts into the eighteen fields of Python's pre-persistence
`build()` state. A replaceable JSON boundary accepts today's custody stages;
the caller supplies entry UUIDs. A test-only Python hook captures complete
canonical states and explicit UUID mappings. Production remains Python, with
no command or daemon registration and no persistence changes.

The [checkpoint evidence and R4-12 handoff](../reference/r4-journal-render-checkpoint.md)
record sampled byte parity, mutation coverage and back-to-back timings.
The [performance follow-up](../reference/r4-journal-render-performance-checkpoint.md)
meets the earlier wall-clock render budgets. The
[boundary continuation](../reference/r4-journal-render-boundary-checkpoint.md)
repairs integer refusals, holding keys and nonfinite detail/summary values.
The [native ledger continuation](../reference/r4-journal-render-scalar-checkpoint.md#native-ledger-continuation)
adds a lazy native-value ledger reader checked against 990 Python results.
The [native builder assembly continuation](../reference/r4-journal-render-scalar-checkpoint.md#native-builder-assembly-continuation-partial)
checks 1,254 complete builder outcomes, including all 234 merged branch witnesses
and 30 new scalar captures. The
[native ownership graph continuation](../reference/r4-journal-render-scalar-checkpoint.md#native-ownership-graph-continuation)
replaces its serde graph adapter and checks 1,435 outcomes, including 181 new
scalar graph captures. The
[lossless probe handoff](../reference/r4-journal-render-scalar-checkpoint.md#lossless-probe-handoff-continuation)
closes all four end-to-end surrogate repros and matches all 1,435 frozen cases
through the executable, including cached reads. The
[nonfinite response continuation](../reference/r4-journal-render-scalar-checkpoint.md#nonfinite-response-and-diagnostic-continuation-partial)
adds 1,776 Python builder/ledger outcomes and repairs diagnostic float sums,
wallet mapping lookup and JSON NaN holding-key identity. It covers ordinary
and generic transfer response nodes; complete custody/Austrian shape coverage,
successful lazy decoding, shape/refusal/UUID campaigns and final acceptance
remain open.
R4-11 remains incomplete: lossless native scalar fidelity, exhaustive
rendering/join mutations and full inherited campaign gates still need completion.
The [borrowed-pricing continuation](../reference/r4-journal-render-boundary-checkpoint.md#sixth-continuation-borrowed-pricing-and-entry-identities)
meets the isolated 0.15 s render-and-join CPU budget on both 20k books; final
fidelity and integration changes require a fresh measurement.
The [legacy-float continuation](../reference/r4-journal-render-boundary-checkpoint.md#seventh-continuation-legacy-transfer-floats-and-carry-refusals)
adds native audit-float consumption, nonfinite transfer quantities and exact
carried-rate coercion/refusals. Thirty-nine new witnesses supplement the
unchanged 383-case corpus. Surrogate and malformed-response fidelity remain
open, and fresh CPU pairs do not recertify both 20k budgets.
The [controlled CPU comparison](../reference/r4-journal-render-boundary-checkpoint.md#eighth-continuation-controlled-cpu-comparison)
reestablishes both 0.15 s budgets on unchanged native source in seven alternating
pairs with exact state bytes. Fidelity, integration and full acceptance remain open.
The [ordered-refusal continuation](../reference/r4-journal-render-boundary-checkpoint.md#ninth-continuation-ordered-response-refusals)
adds a cold response traversal, rejects malformed response lists and wallet
mappings, and pins 550 refusals with exact UUID consumption. Seven new mutations
are killed. Successful lazy decoding, lossless scalars and the full acceptance
campaigns remain open.

### R5 pairing command checkpoint

`t3code/r5-transfers-8` starts at `f2464d20` and imports R4-14's pairing library
from `d8f3bac6`. Suggestions, bulk pairing and rule application now register
through CLI and daemon, bringing this subset to **22/29 CLI leaves and
21 desktop kinds**. The adapters reuse the pairing module, preserve CLI dry
runs and desktop coercion/authorship, and own only their outer transaction.
Fixed-source ordered-output and complete-database oracles exercise command
dispatch; process tests pin wire bytes, hybrid routing and Python projections.
The [shell checkpoint](../reference/rust-shell.md#pairing-command-checkpoint-r5-transfers-8)
records the evidence and remaining work. Review cases/plan/apply still require
current ledger composition, whose interpretation/quantity/finalized stages
are absent on this base; they must not be approximated from stored journals.

### R5 component read checkpoint

The transfers-8 continuation registers component list/show and the safe desktop
list/get kinds, bringing the subset to **24/29 CLI leaves and 23 desktop kinds**.
The shared authoring materializer supplies all validation and effective-state
rules. An explicit CLI flag can include local evidence; desktop reads always
redact it. Python oracles pin ordered views, profile/reference/filter refusals,
effective filtering before pagination and complete unchanged database rows
across 51 books and 1,190 reads. Real CLI/daemon comparisons require zero hybrid
fallbacks. The [shell checkpoint](../reference/rust-shell.md#component-read-checkpoint-r5-transfers-8)
records counts and mutation witnesses. Gap snapshots and the three current-ledger
review operations remain disabled at this boundary; AI projections stay Python.

### R5 gap read checkpoint

The transfers-8 continuation adds gap list/review and
`ui.custody.gaps.list|review_context`: **26/29 CLI leaves (all 25 transfers,
one historical review receipt) and 25 desktop kinds**. The existing pure gap
search and authored component materializer supply candidate and reviewed state.
Snapshots preserve historical, reopened, conflicting and evidence-drift states,
canonical quantity blocker precedence, journal freshness, conflict-deduplicated
residual totals and version/ordinal/identity-bound cursor pages. Desktop reads
retain their narrower page cap, safe fields and exact integer strings.

The fixed-source oracle covers 94 books and 1,351 reads, comparing ordered
results/refusals and complete unchanged application rows. The component oracle
expands to 1,394 reads over its existing 51 books to cover surrogate identifiers:
invalid component references retain Python's encoding refusal; gap and cursor
identifiers cannot alias a real ID after code point loss. Product Python and AI
projections remain unchanged. The [shell checkpoint](../reference/rust-shell.md#gap-read-checkpoint-r5-transfers-8)
records process, mutation and final gate evidence.

This is a verified partial boundary, not completion of the service slice.
`review cases|plan|apply` and `ui.review.cases|plan|apply` remain disabled. Each
requires `review_workflow._build` and a fresh `custody_journal.build_ledger_state`,
including live before/after simulation and locked apply revalidation. This
base's Rust custody API explicitly skips interpretation, quantity and finalized
projection; reading stored journals would violate the current-state contract.

### R5 transfers on the R7 integration base (partial)

`t3code/r5-transfers-9` starts at `dd433e4e`, skips the already integrated
pairing-library import and applies the transfers-8 pairing registration,
component reads and gap snapshots. It serves **26/29 CLI leaves and 25/28
corresponding daemon kinds**. Integrated custody/wallet/journal work and all
test-inventory entries are retained.

R7 supersedes the transfers-8 statement about absent pure custody stages:
custody, fusion, tax computation and a render checkpoint are now present.
The shared native SQL snapshot/ownership loader (R4-9), complete canonical
build-state assembly/render acceptance (R4-11), and journal persistence
(R4-12) still remain. `review cases|plan|apply` and their three daemon kinds
are not implemented; their fresh simulation and atomic verified persistence
cannot use stored journal projections. The [integration checkpoint](../reference/rust-shell.md#r7-transfer-route-integration-r5-transfers-9-partial)
records evidence and the precise remaining boundaries. This does not complete
the transfers/review service slice. Python and AI projections are unchanged.

### R5 transfers: native review-build assembly (partial)

`t3code/r5-transfers-10` starts at `b5eb38ea8`, which includes the R4-9 native
loader and R4-12 writer checkpoints. It repairs their integration with the typed
custody snapshot and joins native loading, custody, fusion, tax computation and
rendering into a complete review build. The loader now supports caller-owned
read snapshots separately from its existing writer-only entry point. Planning
backup uses the same SQLCipher binding, an in-memory destination and an injected
ephemeral key; pending source writes are refused without committing them.

The assembler remains gated to tests/`test-harness`. Its sampled byte comparisons
include every build invoked by mixed reviews, acquisition reviews and sibling or
interleaved unpairs. Complete decisions determine transfer basis eligibility;
suggestions and graph evidence gain no reviewed-custody authority. Canonical
quarantine deduplication and live-profile anchor filtering precede case use.
The [checkpoint](../reference/rust-shell.md#native-review-build-assembly-r5-transfers-10-partial)
records exact coverage and reproduction commands.

Counts remain **26/29 CLI leaves and 25/28 corresponding daemon kinds**. The
three review commands remain unregistered. This is a build/clone checkpoint,
not review workflow acceptance: lossless/refusal admission, cases/cursors,
portable artifacts and policy-bound effects, ordered operation application,
idempotent receipts, canonical writer integration, filed impacts, rollback and
CLI/daemon/AI consent parity still require implementation and proof. There is no
new hybrid fallback or Python product change.

### R5 transfers: native case-reader checkpoint (partial)

The transfers-11 continuation starts at `4af9249bf` and implements the case
reader under `test-harness`: scope/version/processed-time cursors, current-build
case paging, named waiting roots, exact observed pair evidence and fingerprints
that bind allocations, plus bounded historical receipt summaries. It preserves
owned and caller-owned transactions, including build failures/unwinding, and
proves consistent reads during a concurrent writer. Reviewed custody remains
distinct from observations, suggestions and evidence.

The [checkpoint](../reference/rust-shell.md#native-review-case-reader-r5-transfers-11-partial)
records 56 complete native/Python build matches, 2,850 case-result matches on
plaintext/SQLCipher and 36 independent waiting-root witnesses. The comparison
drivers supply actual native build quarantines to the reader as separate test
seams; live callback integration remains pending. Counts stay **26/29 CLI
leaves and 25/28 daemon kinds**. Normalized operations, artifacts and policy
effects, planning/validation, atomic apply/receipts, journal and filed-impact
storage, loader admission and CLI/daemon/AI consent parity remain open. R4-9
owns its acceptance record; at this checkpoint the integrated BLOB continuation
left forty measured seed-dependent messages pending Python. The deterministic
refusal fix integrated by transfers-16 closes all forty.
Python product code and route registration are unchanged.

### R5 transfers: economic effects and artifact primitives (partial)

Transfers-12 subsequently joins cases directly to the live native assembler and
pins 216 operation-normalization vectors. Transfers-13 starts at `f2092193a` and
adds full economic digests and bounded previews over typed canonical journal
rows. Acquisition effects bind exact accounting totals, the current profile's
policy and the database instance identity. Fourteen live books exercise native
assembly and effects in SQLite/SQLCipher and owned/caller transactions; 33
complete build states and 80 synthetic cases cover the effect projection,
including nonfinite tax-summary refusals before JSON serialization.

Portable artifact construction and pre-transaction admission also match the
Python content/refusal contract, including complete digests, scope, canonical
operations, Unicode idempotency-key bounds and the exact encoded size limit.
Their isolated oracle stubs accounting during construction and stops apply
before transaction ownership is read. It does not prove planning or apply.
See the [checkpoint and remaining handoff](../reference/rust-shell.md#review-economic-effects-and-portable-artifact-primitives-r5-transfers-13-partial).

Ordered domain operations, isolated plan/validation, atomic apply/receipts,
journal/filed-impact storage, caller cache/transaction restoration, final loader
acceptance and CLI/daemon/AI consent parity remain required. Counts remain
**26/29 CLI leaves and 25/28 daemon kinds**, with all six review routes still
unregistered. At transfers-13, both loader refs remained at `c5966c1ff`; that
checkpoint closed no inherited lossless, refusal or explicitly pending exception.

Transfers-14 integrates accepted loader-9 and BLOB work with their missing
prerequisites, retaining raw database admission alongside typed assembly and
sharing their downstream stages. The BLOB gap is closed. Transfers-16 also
integrates deterministic revision-reference refusals, closing all forty former
seed-dependent loader exceptions. The review assembler's
strict typed conversion still needs its own lossless/refusal closure. The
[integration handoff](../reference/rust-shell.md#accepted-loader-integration-r5-transfers-14)
records fresh verification separately from inherited campaign evidence. At that
checkpoint, ordered planning, atomic apply and all six route registrations
remained pending; counts were unchanged.

### R5 transfers: ordered planning and validation (partial)

Transfers-15 starts at `59e1476a4`. Its unregistered review workflow now simulates
ordered metadata, acquisition, component and unpair operations on a same-binding
RAM snapshot. It reuses the existing native metadata and custody services,
prechecks confirmed pair fingerprints, preserves sibling-unpair baselines and
refreshes canonical decisions and fingerprints after intervening operations.
Validation recomputes the complete artifact against current evidence and policy.
The existing workspace metadata crate is the only added dependency edge; no
third-party package or runtime requirement changes.

Complete Python oracles pin planning on plaintext/SQLCipher in autocommit and
caller-read modes, plus recomputing validation on both bindings. Pending writers
refuse and full source rows stay unchanged. See the [checkpoint](../reference/rust-shell.md#ordered-review-planning-and-validation-r5-transfers-15-partial)
for named mutation evidence. At that checkpoint, atomic apply,
receipt/journal/filed-impact parity, caller transaction/cache restoration,
assembler lossless/refusal closure and CLI/daemon/AI consent parity remained
open. All six review routes remained unregistered; counts stayed **26/29 CLI
leaves and 25/28 daemon kinds**. The determinism continuation integrated by
transfers-16 closes the forty historical
loader message exceptions; Python's residual-fee refusal remains unchanged.

### R5 transfers: atomic review apply (partial)

Transfers-16 starts at `7a3dd1540` and integrates the deterministic loader refusal
from `aaf6216e3`. The unregistered apply service owns its writer reservation,
nested savepoint, canonical before/after rebuild, journal persistence and durable
receipt as one operation. Historical retries precede freshness checks; conflicting
keys refuse. Explicit `commit=False` preserves the caller's pending edits and
transaction, while failure rolls back the review and restores the cache. Late
commit failure rolls back even after the operation's savepoint was released.

The Python oracle compares 72 complete apply outcomes and all logical database
rows with explicit generated-identity bijections on SQLite and SQLCipher. It
includes metadata/component order, sibling unpairs, execution authorship,
canonical journals and saved/filed-impact readiness. Named mutations pin these
contracts; the [handoff](../reference/rust-shell.md#atomic-review-apply-r5-transfers-16-partial)
records the verification and remaining work. The metadata writer now uses one
grouped UPDATE, matching Python's chain-index trigger effects, and sibling-pair
revisions validate their author source.

Product envelope/AI once-only consent parity
and zero-fallback registration proofs remain required. All six review routes
remain unregistered; counts stay **26/29 CLI leaves and 25/28 daemon kinds**.
There are no retained seed-message exceptions. Python's residual-fee refusal and
its existing repro remain unchanged. No dependencies or Python product behavior
are changed beyond the explicitly requested deterministic loader fix.

### R5 transfers: raw review composition and authoring refusals (partial)

Transfers-17 starts at `66d41ec59`. The accepted native raw loader now feeds
`kassiber_custody::run_loaded` directly through journal fusion and rendering.
The JSON snapshot API retains its separate typed admission. Fusion shares the
lossless operand adapter and returns explicit refusals instead of assuming that
every finalized fact can be serialized. Byte facts unused by the request builder
remain available to their actual consumers.

Shared transfer and transaction row readers and canonical journal storage retain
SQLite BLOB identity. The apply corpus includes all successful raw-storage plans
and records byte storage explicitly in its complete-row oracle.
Delegated metadata pricing uses Python-compatible integer conversion, and a
truthy invalid stored currency refuses before fallback or metadata comparison.
Authoring reasons retain their exact code points; unsupported kind admission
remains unchanged.
Metadata validates source and reason length before detecting no-ops; SQLite
encoding applies only to writes. Component reasons, placeholder-wallet lookup
and revision preimages retain exact UTF-8 refusal positions.

The [checkpoint](../reference/rust-shell.md#raw-review-composition-and-authoring-refusals-r5-transfers-17-partial)
records reference corpora, named mutation witnesses and verification. Product
CLI/daemon envelope and authorship proofs, AI once-only revalidated consent,
zero-fallback workloads and all six registrations remain pending. Counts remain
**26/29 CLI leaves and 25/28 daemon kinds**. The AI chat loop and Python's
residual-fee refusal remain unchanged. No dependencies or Python product code
change at this boundary.

### R5 review workflow routes

`t3code/r5-review-1` registers the review routes over the transfers-10 to
transfers-17 library, now built in production. `review cases`, `review plan`,
`review apply` and `review receipt`, with `ui.review.cases`,
`ui.review.request_input`, `ui.review.plan`, `ui.review.apply` and
`ui.review.receipt`, share one composition root behind `Book::review`. Counts
reach **29/29 CLI leaves and 28/28 daemon kinds**, plus the input request.
Plans stay bound to the input version and run on an in-RAM copy. Apply records
operations, journals and the receipt atomically, under the stored idempotency
key and authored `cli` or `gui`.
Daemon and CLI sessions match Python byte for byte on all 112 tables,
including refused applies and repeated keys, with zero fallbacks. All 19
route mutations are killed. Process comparison also fixed the receipt key
order. The AI tools and their once-only consent stay in Python until R6. See the
[checkpoint](../reference/rust-shell.md#review-workflow-routes-r5-review-1).

### R5 Bitcoin sync: native fetch assembly checkpoint

The unregistered `wallets::fetch::Fetch` captures the supported single-wallet
Bitcoin scope, local discovery horizon, freshness checkpoint, observer epochs
and closed record-reuse evidence before a request-local grant can be created.
It assembles ordinary native BDK Electrum updates and compatibility Esplora
fetches through `kassiber-net`, with no dependency networking crates. BDK facts
include complete outputs, policy coverage, private aggregate/fingerprints and
source-bound canonical membership even when records are reused. Esplora keeps
the reference's unchanged-cache behavior and named `http_route_policy` route.

Rust independently prepares and publishes all 102 rounds of the publication
corpus on both protocols, matching serialized Python fetches and every table's
canonical logical rows. Tests reopen the Rust process and book each round and
freeze only BDK's public request clock; a backend race is replayed separately
for each implementation. The fixed-descriptor witness also pins bdkpython's
descending chain-position record order, which the raw Rust iterator does not
provide. Missing/foreign/revoked grants, changed scope, offline and cancelled
actions refuse without traffic; unsupported source shapes refuse locally.

This checkpoint does not register any of the four sync routes. Shared
CLI/daemon wallet/freshness services, product request cancellation and scope
fences, unsupported hybrid fallback and a zero-fallback hybrid workload remain.
The Python same-statistics Esplora reorg-cache bug remains an external
dependency; this checkpoint preserves its existing stale-evidence outcome.
See the [native fetch handoff](../reference/rust-shell.md#bitcoin-native-fetch-assembly-r5).

### R5 Bitcoin sync: compatibility tip-cache continuation

The separate Python reference fix `ec49bf25` is integrated. Rust compatibility
fetches now bind cache reuse to Esplora's canonical tip hash or SHA-256 of
Electrum's full header. Changed, missing or malformed anchors refresh evidence
when statistics/status/heights alone are unchanged. Electrum discards stale
header timestamps and reprocesses history entries while retaining validated
stored transaction graphs. Optional anchor reads use the existing gated
clients inside exact-action acquisition consent.

Publication vectors and reviewed-book assertions now require the corrected
same-statistics Esplora reorg outcome. The native preparation/publication and
compatibility oracles compare against this updated reference, including
restart checkpoints and legacy checkpoints without anchors. Thirteen additional
named mutation recipes cover the new cache rules and idempotent integer-sat
normalization of reused Electrum graphs. See the
[tip-cache handoff](../reference/rust-shell.md#bitcoin-compatibility-tip-cache-continuation-r5)
for verification and remaining product-service work. The four sync routes are
still unregistered; this is a cache-parity boundary, not R5 slice acceptance.

The cache boundary passes 1,508 workspace tests, all 206 tests in the seven sync
parity suites, network/strict observer checks, the wallets-sync and serial-network
shards, and all 57 preflight tests after inventory and source-location snapshot
refresh. All 501 sync mutations (452 inherited, 36 fetch, 13 new) and the separate
twelve descriptor mutations are rerun and killed. No product-route acceptance
is claimed; shared services, product fences, pre-bootstrap fallback and all four
registrations remain pending as detailed in the handoff.

### R5 Bitcoin sync: wallet-service checkpoint (partial)

`crates/wallets::sync` now owns selection, request-scoped authorization, ordered
preparation and atomic publication. Explicit selection and `--all` preserve
label ordering, deprecated-wallet filtering, single-wallet errors, isolated
batch errors, successful freshness markers and journal staleness. The service
revalidates the admitted connection, database identity, profile, wallets,
network, resolved backends, observer epochs and checkpoints before traffic and
publication. Missing/foreign/revoked/replayed consent, offline/no-egress and
cancellation remain fenced; local publication checks revocation, cancellation
and offline policy through the commit stage.

Backend resolution includes saved custom backends and the active default.
The original named/default selection is retained for revalidation, so changing
the stored default refuses an existing action before traffic.

An inactive CLI/book adapter calls this shared service with the persisted
offline policy. The live service oracle invokes Python's complete CLI sync
handler and the independent native service on disposable loopback backends,
comparing full envelopes and every normalized table across both protocols.
This advances beyond the prepared-fetch/publication helper boundary, but is
not registered CLI/daemon product acceptance.

At this service checkpoint, freshness coordination, progress events, default
local journal refresh, unsupported hybrid selection and all four registrations
remained unfinished. The [CLI continuation below](#r5-bitcoin-sync-cli-registration-checkpoint-partial)
supersedes the CLI registration and hybrid-selection status. The native `Book`
interface still has no journal
refresh operation, and the hybrid probe currently resolves only command-line
shape; it cannot inspect saved wallet/backend capabilities without opening a
book. Daemon route activation must resolve those boundaries before claiming native
served-by counts. See the wallet-sync service checkpoint at the end of
`docs/reference/rust-shell.md` for verification and the remaining handoff.

### R5 Bitcoin sync: CLI registration checkpoint (partial)

The native `wallets.sync` entry is registered. It calls the shared
request-bound sync service through the admitted `Book` and preserves Python's
CLI contract: sync publishes observations and journal staleness, without
processing journals. Python's default daemon `ui.wallets.sync` instead runs
freshness jobs, then `run_local_journal_refresh` with current-journal skipping
and automatic pairing. That daemon default remains on Python until its queue,
progress and refresh integration can preserve the same accounting effects.
No native journal-processing claim is made.

The test-only hybrid runner keeps its inert command-shape probe. For
noninteractive sync without a passphrase FD, it then opens the admitted book
and constructs an inert `Sync` to inspect saved wallet/backend capabilities.
Only a silent pre-consent handoff may select Python. No grant or network
client exists at that decision. Native failures after admission, including
later `native_fallback_required` errors, never retry. Interactive and
passphrase-FD hybrid sync calls choose Python before bootstrap so no unlock
input is consumed twice. The product binary has no Python retry mechanism.

The live registered CLI oracle now compares complete Python/native CLI
responses, empty progress streams and every normalized table on disposable
Electrum and Esplora backends. Fixtures, restart sequences, reviewed journals,
saved reports, force-full, active defaults and saved custom backends traverse
the hybrid candidate with zero supported-shape fallbacks. Source-less local
wallets also retain skip results and label/deprecation handling. See the CLI
registration checkpoint at the end of `docs/reference/rust-shell.md` for
execution counts and verification.

Remaining work is the three daemon registrations: freshness source policy,
queue/recovery/dispatch/state and wire snapshots; request progress,
cancellation/book-switch/revocation wiring; and either a narrow existing-Python
journal-refresh handoff or explicitly restricted no-refresh variants. Require
full live daemon envelope/event/book parity on both protocols, zero in-scope
fallbacks and named mutation kills before enabling any of those entries.

### R5 Bitcoin sync: pre-dispatch freshness queue checkpoint (partial)

`crates/wallets::freshness` now implements the local SQLite queue prerequisite:
policy reads/writes and legacy defaults, source-state transitions, checkpoint
preservation/reset, single-flight and forced replacement, journal payload
merging, pause/resume/cancel, interrupted-job recovery, due and exact selected
job eligibility, selection ordering, snapshots and workspace profile ordering.
Checkpoint/result redaction stays distinct from private checkpoint storage.
The caller owns connection admission, execution serialization and commits.
The helper has no network client, consent grant, worker or product registration.

This does not implement job execution or finish a daemon route. Success/error
and backoff transitions, handler/source-spec construction, UI text redaction,
live progress and request controls, graph handoff, and journal refresh remain.
All three daemon sync/freshness routes continue on Python, including explicit
no-journal variants. R4-13 blocks native default journal processing but is not
the only outstanding prerequisite. A future registration must either provide
the narrow existing-Python journal operation or explicitly admit only no-refresh
shapes, and prove live daemon envelope/event/book parity with zero fallbacks.
See the queue checkpoint in `docs/reference/rust-shell.md` for verification and
the exact handoff.

### R5 Bitcoin sync: source specification and admission checkpoint (partial)

`crates/wallets::freshness::sources` now constructs the Python daemon's local
source specifications, including wallet/file/deprecation precedence, BTCPay
wallet/provenance/account routes, rates, journals, force-full intent and policy
filtering. Discovery does not grant permission to execute those sources.

The new `Book::freshness_plan` seam admits a complete active-book or workspace
selection without queue writes, recovery, transport or consent. A plan owns the
foreground execution slot until dropped and captures connection/database,
context, ordered profiles, wallets, policy and native sync inputs for later
revalidation. Admission requires an explicit journal opt-out and refuses enabled
rates, BTCPay, unsupported wallet/backend shapes and ambiguous ID/label matches.
Defaults remain on the existing Python path; no journal subprocess bridge is
introduced. The guarded daemon test kind verifies the real admitted-book seam
but is absent from production builds and desktop/AI allowlists.

No product route is registered by this checkpoint. Job execution, retry/backoff,
UI redaction and aggregation, stream/control/graph wiring, hybrid request-shape
selection and full live daemon parity remain. See the new checkpoint at the end
of `docs/reference/rust-shell.md` for verification and the continuation.

### R5 Bitcoin sync: job execution checkpoint (partial)

`crates/wallets::freshness::execution` consumes the admitted plan while its
foreground slot remains held. It ports running/progress/success/error state,
requested-job dispatch, retry-after and SHA256-jittered capped backoff,
database-busy rollback, source checkpoints, wallet-result aggregation and
active-book/workspace snapshots. UI rendering adds the daemon's free-text
URL/host and operational redaction to the queue's structured redaction.

`Book::freshness_execute` connects supported on-chain jobs to native fetch and
atomic publication. An explicit request-local grant is bound to the admitted
plan; persisted jobs carry no transport authority. Native publication retains
the CLI's existing sync marker and the daemon's separate completion marker.
All transport goes through `kassiber-net`; no networking dependency was added.
Only explicit journal opt-outs remain admitted. The guarded fixtures and
daemon test kind are absent from production dispatch and desktop/AI allowlists.

Terminal-envelope and full logical-book comparisons exercise both protocols,
checkpoint restarts, reviewed force-full refreshes and workspace aggregation.
Their fixed clock isolates state parity from progress timing. They do not
establish live progress-stream parity. Source cancellation during fetch,
request lifecycle/replay controls and after-response graph grants still need
the next handoff step. The three product daemon routes remain on Python,
including no-refresh shapes. See the final checkpoint in
`docs/reference/rust-shell.md` for verification and the exact continuation.

### R5 Bitcoin sync: fetch progress and batch controls (partial)

Native freshness execution now forwards discovery, BDK scan/decode and Esplora
stats/history/decode/UTXO progress. Active-book runs prefetch selected on-chain
jobs before running-job events; workspace runs fetch inside their job context,
matching Python's distinct stream ordering. Prefetched outcomes remain bound to
the exact job and are revalidated before publication. A captured fetch failure
is recorded by its job without repeating transport.

Protocol boundaries now run synchronous checks on the admitted connection for
persisted cancellation, book scope, wallet inputs and backend revisions, in
addition to the atomic cancellation flag and `kassiber-net` permission checks.
Parallel discovery workers retain only the atomic flag; connection checks run
before and after their batch on the owning thread. Failed progress callbacks
stop acquisition and publication. The guarded daemon kind streams ordered,
request-bound envelopes through the real `Book` seam.

This is an internal continuation of step 2. Product request-loop lifecycle
controls, early request cancellation and after-response graph grants with a
stoppable, connection-owning worker remain outstanding. Step 3 validation,
inert no-refresh hybrid selection and product registration have not started.
The explicit no-refresh admission choice remains; default journal processing
continues through Python. All three product daemon routes remain unregistered.

### R5 Bitcoin sync: request-loop lifecycle (partial)

The guarded native daemon now executes an admitted freshness request on a
dedicated connection while its request loop services cancellation, source
pause/resume/cancel and offline changes. Request grants remain once-only and
bound to the admitted plan; request-ID replay cannot create a new grant.
Early cancellation is bounded and expires. Book switching, lock, shutdown and
EOF cancel/revoke and join the runner before changing context or releasing
ownership. Terminal responses are written by the request loop after the runner
has closed its connection. An output failure also stops and joins the request.
Encrypted execution uses the unlocked session to open the dedicated connection
without putting a passphrase in a request or retaining another worker copy.

Python pause/resume currently returns a private checkpoint in its result
container. A new repro pins that reference bug; Rust keeps the UI redaction,
and all other control fields and complete logical books are compared. Python
is unchanged. The earlier cancelled-prefetch privacy deviation also remains.

The next boundary is after-response graph grants and the bounded, stoppable
graph worker. Step 3 keeps the explicit no-refresh choice: implement product
validation and inert whole-shape selection while defaults remain on Python.
Full product daemon parity and zero served-shape fallbacks must precede route
registration. This checkpoint registers none of the three product routes.

### R5 Bitcoin sync: private graph handoff (partial)

Successful native freshness jobs now carry private graph follow-up grants out
of the guarded request runner separately from their sanitized response. The
main loop publishes them only after the terminal record is written and flushed;
fallback error serialization and connection-close failures do not publish.
The mailbox is bounded and replaces an older grant for the same wallet.
Request cancellation/revocation and context lifecycle boundaries discard grants.
Local validation binds the exact completed job, database identity, active book,
network binding, wallet signature and effective/stored backend configuration.

This completes the private handoff prerequisite only. Rust still needs the
connection-owning, stoppable graph worker and bounded transaction-reference
lookup service, their unsolicited events, parity and graph mutation acceptance.
The mailbox has no consumer and creates no graph traffic or graph-cache writes.
Product validation/whole-shape selection, full product daemon parity and route
registration remain open; all three product routes still use Python.

### R5 Bitcoin sync: no-refresh daemon registration

The native daemon now registers admitted explicit no-refresh variants of
`ui.wallets.sync`, `ui.freshness.run` and `ui.workspace.freshness.run`, plus
source cancel/pause/resume. Product requests automatically arm graph consumption
after the terminal-flush barrier. The guarded whole-invocation hybrid selector
is inert on already opened books and declines unopened/locked books, default
refresh, unsupported protocol IDs and unported source selections before consent.
Native errors never authorize a Python retry.

Acceptance compares full product envelopes, ordered progress/background events
and every normalized table over both loopback protocols, fixture/restart and
reviewed-book scenarios, workspace ordering and real automatic graph lookup.
Product cancellation, offline/no-egress and scope fences remain in place. The
three previously recorded reference deviations are retained with their repros;
Python is unchanged. The `all:false`/missing-wallet input also has a reference
repro and remains outside the served selection.

All 248 product cases pass across the main and continuation runs, without
skips. The oracle witness records 107 wallet-sync, 109 active-book freshness
and 111 workspace freshness native round trips, with zero served-shape
fallbacks. The two default enqueue-only hybrid cases select Python before
execution. Live source-control comparisons preserve the documented earlier
stop after prefetch cancellation while matching terminal results and complete
normalized books.

All 18 final product mutations are killed after 16 named baselines pass; the
stored-state error recipe specifically reaches Electrum reconstruction.
All 225 affected Rust tests and 13 inherited caller regressions pass, with
two subtests. Formatting and production/harness clippy pass. Required test
classification records 5,819 definitions, and CI preflight passes all 57 tests.

Default refresh is still open. Once native `journals process` from
`t3code/r4-13-journals-process-2` is registered, adapt it through the worker-owned
`Book::freshness_execute` handler, extend journal admission and caller auto-pair
authority, preserve wallet sync's separate follow-up and freshness/workspace
job/summary ordering, and prove defaults before removing either journal guard.
Enabled unported rate sources still decline. No Python journal bridge is
introduced. The [product handoff](../reference/rust-shell.md#bitcoin-wallet-sync-product-routes-r5-no-refresh)
records the exact integration boundaries and verification.

R4-11 remains incomplete: the 0.15 s generic-20k budget, lossless legacy edge
values, exhaustive rendering/join mutations and full inherited campaign gates
still need completion.

### R5 file-import continuation: generic-ledger core/CLI checkpoint

Native `wallets import-ledger` reads CSV/TSV and bounded XLSX/XLSM files, with
native headers, column inference, explicit maps, exact Bitcoin/sats and cash
fees, execution values and swap-linkage fields. Dry-run previews read no book;
import runs preserve the original supplied map, scoped dedupe, journal
invalidation and atomic rollback.

There are 341 frozen Python-derived files, including 35 spreadsheets. Native
field/fingerprint and preview vectors, hybrid full-book routing assertions,
raw CLI bytes/full typed dumps on SQLite and SQLCipher, and coordinator/late
failure cases cover the boundary. Five reference quirks have an offline repro.
The catalog adds 44 named mutation entries (730 total). The
[generic-ledger checkpoint](../reference/rust-shell.md#generic-ledger-core-and-cli-ingestion-r5-continuation)
records routing counts, mutation killers and final verification. The expanded
hybrid run records 689 native imports/previews and zero importer fallbacks;
all 80 native file-import tests pass. The 44 new mutations and shared atomic
rule are killed across the scratch audit and early-limit witness recheck.
Formatting, strict workspace/harness Clippy, all 14 harness tests and all 57
required CI preflight checks pass. The final workspace run passes 1,500 tests,
excluding the 58 previously verified loopback cases, with no other blocks or
failures. The classifier records 5,739 Python definitions.

The only dependency change is a wallets-to-flate2 edge, reusing rates' exact
1.1.9 pin and pure Rust backend. Cargo.lock retains the same 370 packages and
versions; the existing license entry records the added use. Python production
code, schema, egress, sync routes and AI projections are unchanged. Remaining
core/CLI importers are Wasabi, Samourai and document ingestion in the separate
worker's scope.

### R5 year-handoff XLSX continuation (reports worker B)

The owner split remaining reports after `be4aac52d` into independent workers.
`t3code/r5-reports-b` ports year-handoff XLSX through the native service, CLI
and desktop daemon. Exact observed movements and journal holdings remain
separate; source freshness, current journals and prior-year quarantines block
export. The eight-sheet projection preserves pooled basis, asset identity,
classification, timing notes and live formula caches without running tax or
custody computation. Saved completion hashes the actual native bytes;
explicit filing retains its frozen capture and existing amendment history.

The [year-handoff checkpoint](../reference/rust-shell.md#year-handoff-xlsx-checkpoint-r5-reports-worker-b-partial)
records openpyxl content, exact wire metadata, all 112 database tables,
zero in-scope fallbacks, mutation evidence and reproducible reference quirks.
The native inventory is 153 CLI leaves (21 reports) and 14 report daemon
kinds plus two transaction exports. Cross-check, Lightning and non-PDF
source-funds belong to the other reports worker. The platform CSV continuation
below completes CoinTracking/Blockpit.
Privacy hygiene/mirror/PSBT reads continue independently; the artifact
inventory finds no other unported non-PDF artifact in this worker's scope.
PDF-containing exports and AI projections remain Python; dependencies, tax
rules, schema and Python product code are unchanged.

### R5 platform CSV continuation

`t3code/r5-reports-platform-csv` starts at `0475d3e49` and completes
CoinTracking/Blockpit through the native report service, `transactions export`
and `ui.transactions.export_platform_csv`. Byte-exact UTF-8/CRLF artifacts,
wire metadata/errors and all 112 database tables compare against unchanged
Python, including its no-snapshot-registration boundary. Exact msat quantities,
booked values, current journals, source freshness, wallet/date scopes,
quarantines and unsupported mixed classifications retain the reference rules.
The 156 frozen projections and all 163 live service/process comparisons pass;
64 CLI calls and three daemon sessions have no in-scope fallback. All 46 new
mutations and the reshaped inherited format-admission mutation are killed by
named tests. The final selected regression passes 445 tests and 1,096 subtests;
required CI preflight passes all 57 tests after the classifier update.

The [platform checkpoint](../reference/rust-shell.md#platform-csv-checkpoint-r5)
records routing, killed mutations, verification and disposable reference-bug
repros. No additional CLI leaf is needed: the existing native transaction
export now admits all four formats. The native inventory is 153 CLI leaves
(21 report commands), 14 report daemon kinds and three transaction export
kinds. Privacy reads continue on the independent privacy branch; there is no
additional unported non-PDF artifact in this worker's family. Cross-check,
Lightning, source-funds, PDF-containing artifacts and AI projections retain
their separate ownership and checkpoints. Python product code, schema,
dependencies and lockfiles are unchanged.
### R5 privacy hygiene continuation (partial)

`t3code/r5-reports-privacy` starts at `0475d3e49`, the year-handoff worker B
tip. It ports the configuration-posture privacy hygiene service and registers
`reports.privacy-hygiene` and `ui.reports.privacy_hygiene`. The service reads
local database headers and stored aggregate facts without probing endpoints,
refreshing journals or exposing private configuration. Endpoint posture retains
Python 3.11's URL/IP rules, proxy-kind allowlist and infrastructure-owner
precedence. Provider, watch-only material, inventory and privacy-marker counts
retain their reference scope and truthiness. Stale journals are reported as a
fact rather than admitted as tax reports.

The [privacy checkpoint](../reference/rust-shell.md#privacy-hygiene-checkpoint-r5-reports-privacy-partial)
records exact wire comparisons, every one of the 112 tables, native routing,
named mutations and retained reference quirks. Native inventory increases to
154 CLI leaves (22 reports), with 15 report daemon kinds plus two transaction
exports. The parallel platform CSV worker's registrations are separate.
The initial checkpoint retained a corrupt stored-journal-marker diagnostic gap.
The [native diagnostic continuation](../reference/rust-shell.md#corrupt-marker-daemon-diagnostics-r5-privacy-continuation)
closes it under the native runtime contract below: typed errors retain null,
unhandled debug strings are empty, and every other response byte matches Python.

This is a partial handoff. Privacy Mirror still needs the shared immutable
Chain Analysis index, observer projection, clustering, patterns, attribution,
conditional entropy and process-bound handles. The reduced PSBT report still
needs the shared v0/v2 parser, categorical features and inventory/linkage
overlay. Both CLI/daemon reads remain unregistered and fail closed; no separate
graph interpreter or PSBT signing authority is introduced. AI projections and
all Python files remain unchanged. No dependency, lockfile or schema changes.
### R5 Lightning profitability preparation (incomplete)

`t3code/r5-reports-lightning` starts at `9338364db`. The native reports crate
now prepares profitability payloads, CSV rows and direct CSV publication from
an already curated open-channel/routing projection. Scoped connection lookup
and daemon window coercion retain Python behavior. Amounts remain unbounded
integer sat at this report boundary; binary64 appears only in the reference's
CSV BTC formatting. No tax, journal, adapter persistence or readiness behavior
changes. The [preparation checkpoint](../reference/rust-shell.md#lightning-profitability-preparation-checkpoint-r5-incomplete)
records byte/table comparisons, mutation evidence and reference-quirk repros.

This does not complete the requested Lightning read/export family. Both public
reads fetch live snapshots in Python. Native LND and CLN report acquisition
now have unregistered preparation boundaries, using the net gate, action-bound consent,
minimal report projections and disposable loopback comparisons with unchanged
Python. These are report-only clients. CLN uses a fixed read-only subprocess API,
curated resource rows and ancillary-field refusal checks; its child transport
is gated at launch and stopped on policy change or deadline. Complete LND
snapshot/error/TLS/proxy parity, backend/capability service handling,
CLI/daemon registration and full zero-fallback product acceptance remain
required. The [LND checkpoint](../reference/rust-shell.md#lnd-profitability-acquisition-checkpoint-r5-incomplete)
records the retained quirks, transport mutation evidence and Python error
disclosure repro. No public route is added:
the inventory remains 153 CLI leaves (21 reports), 15 report daemon kinds and
two transaction export kinds. New Lightning native CLI/daemon served-by counts
are zero; no hybrid acceptance is claimed. Non-PDF source-funds reads belong
to `t3code/r5-reports-sof`; the separate reports-b ownership is unchanged.

### R5 Lightning CLN acquisition continuation (incomplete)

`t3code/r5-reports-lightning-3` starts at `69955f7a0`. CLN report acquisition
now runs through the net gate with immutable process configuration and exact
action binding. The frozen Python corpus and loopback process comparisons
retain window-label-only behavior, symmetric rounding, rebalance fee deduping,
channel filtering and full-snapshot ancillary refusals. The
[CLN checkpoint](../reference/rust-shell.md#cln-profitability-acquisition-checkpoint-r5-incomplete)
records verification and mutation evidence. Python disclosure quirks have
synthetic repros; native errors exclude raw node output and command details.
The native route inventory is unchanged and successful product Lightning
served-by counts remain **0 CLI calls and 0 daemon requests**. LND ancillary,
error/TLS/proxy parity, shared backend/capability services, public wiring,
registration and product wire/CSV/all-table acceptance remain required.

### R5 Lightning LND ancillary and transport continuation (incomplete)

The first boundary on `t3code/r5-reports-lightning-4`, based on `8b63484d6`,
retains LND ancillary refusals and their resource order, including checks on
unused snapshot fields. Its 535-case corpus and loopback process TLS/proxy
matrix preserve the Python projection and recorded quirks while keeping raw
responses out of native errors. The
[continuation checkpoint](../reference/rust-shell.md#lnd-ancillary-and-transport-continuation-r5-incomplete)
records evidence and explicit privacy differences. Shared backend/capability
and grant services, CLI/daemon registration and zero-fallback product
wire/CSV/all-table acceptance remain required; product served-by counts stay
zero.

### R5 Lightning profitability service completion

`t3code/r5-reports-lightning-5`, based on `d1178136a`, completes the live
profitability read/CSV family. Shared backend resolution, explicit adapter
capabilities and request-local grants now feed the native CLI read, CLI CSV
export and desktop profitability kind. Grants bind the exact operation and
book/profile/connection/window/export/backend configuration; the service checks
current book, context and configuration before each resource and publication,
and revokes grants on completion or failure. Configuration and display reads
remain inert, and offline/no-egress apply to loopback and CLN subprocesses.

The [product checkpoint](../reference/rust-shell.md#lightning-profitability-product-wiring-r5)
records exact Python wire/CSV comparisons, all 112 unchanged tables, zero
in-scope hybrid fallbacks and 36 successful native CLI reads, 36 CSV exports and
20 desktop profitability requests in its core acceptance workload. The final
verification continuation records service/transport mutation kills and the
product TLS/proxy/error matrix. Native inventory is 155 CLI leaves (23 report
commands), 16 report daemon kinds and two transaction export kinds.

This completes Lightning report reads/CSV only. Full node-snapshot and backend
probe routes, Lightning wallet sync and AI projections retain their separate
porting work. Non-PDF source-funds and reports-b ownership remain unchanged.

### R5 privacy: shared public-feature prerequisite

The continuation from `4014edb6f` ports the shared public transaction-feature
extractor, closed persisted-feature sanitizer and evidence-bound rule evaluator
to `kassiber-wallets::analysis_features`. It reuses that crate's existing Bitcoin,
exact integer and decimal primitives without adding dependencies. Stored
observations and future PSBT adapters can consume the same pure implementation;
there is no separate report-specific graph or ownership interpreter.

The [feature checkpoint](../reference/rust-shell.md#public-feature-primitives-r5-privacy-continuation-partial)
records 1,330 frozen unchanged-Python vectors, canonical byte comparisons,
all-table read invariants and 47 named mutation recipes. This completes a
prerequisite, not Mirror's shared analysis engine or either report's process
acceptance. CLI/daemon registrations and served-by inventory remain unchanged.
The immutable observation index, observer projection, traversal, clustering,
patterns, attribution, conditional entropy and process/book-bound handles still
precede Mirror acceptance. PSBT still needs its v0/v2 parser, validation and
inventory/linkage overlay. The separate legacy scorer and AI projections remain
Python. No schema, lockfile or Python product code changes.

### R5 privacy: corrupt-marker daemon diagnostics

The configuration-hygiene daemon now preserves Python's debug-field presence
for corrupt journal markers. Its native diagnostic is an empty string, with no
invented Python traceback, frame paths or stderr trace. Python includes this
field for these unhandled daemon errors with or without global `--debug`;
typed argument refusals retain null in both modes. The conversion is scoped to
`ui.reports.privacy_hygiene` internal errors. Other kinds and CLI diagnostics
retain their existing contracts.

The [diagnostic checkpoint](../reference/rust-shell.md#corrupt-marker-daemon-diagnostics-r5-privacy-continuation)
records six sessions and 18 requests, all non-debug bytes including line endings,
all 112 tables and zero hybrid fallbacks. Three mutations are killed by named
native tests, bringing the report catalog to 758 applicable entries. The
existing hygiene acceptance also passes: 208 service cases, 48 CLI invocations
and five ordinary daemon sessions. Combined routing is 48 CLI invocations and
11 daemon sessions carrying 58 hygiene requests, all native. Product inventory
remains 154 CLI leaves (22 reports), with 15 report daemon kinds plus two
transaction exports. Mirror, PSBT and the legacy scorer remain unregistered;
AI projections remain Python. No egress or persistence authority is added.

### R5 privacy: bounded PSBT parser prerequisite (partial)

`t3code/r5-privacy-psbt` starts at `3291f143a` and adds the native shared
PSBT v0/v2 decoder, transient inventory structure adapter and redacted public
analysis. It preserves framing, byte/count limits, required-locktime resolution,
zero sequences, supplied-UTXO hashes/indices/amounts/scripts and signature
encoding checks. Bitcoin parsing reuses existing workspace dependencies.
No schema, lockfile, egress or spending authority is added.

The [parser checkpoint](../reference/rust-shell.md#psbt-parser-prerequisite-r5-privacy-psbt-partial)
records 1,821 unchanged-Python vectors, complete 112-table primitive read
invariants, named mutations and the retained embedded-decoder reference bug.
The JSON driver exercises both offline-switch states with no network consent
or fallback path. These are parser proofs; they do not accept a book-service
or transport route. Product inventory remains 154 CLI leaves (22 reports),
15 report daemon kinds plus two transaction exports, with zero newly accepted
PSBT or legacy-scorer product reads. Inventory/linkage overlays, report
redaction, CLI/file/daemon acceptance and registration remain outstanding, as
does the separate legacy scorer. Mirror remains the parallel worker's scope.

All 39 parser mutations are killed by six named native selections; compiler
errors and empty selections are not accepted as kills. The retained Python
oracle runs five cases, including both offline-switch states. The helper used
by the required preflight preserves account `HOME` and passes an explicit
disposable data root. The classifier now records 5,716 definitions, including
four retained PSBT oracle definitions. No product served-by count changes.

### R5 privacy: reduced PSBT inventory/linkage overlay prerequisite (partial)

`t3code/r5-privacy-psbt-2` continues from `fff419a68`. It ports input/output
inventory matching and the conditional proposal overlay over an admitted
historical graph, plus active/bound report scope redaction and table rows. The
report resolves scope and invokes its native baseline callback inside one
SQLite snapshot. Observer-only components, branch evidence, unknown/bounded
coverage, exact msat fee availability, adversary deltas and hypothetical
scenarios retain the Python contracts. No signing or spending authority is
added, and Python product code is unchanged.

The [overlay checkpoint](../reference/rust-shell.md#psbt-inventorylinkage-overlay-prerequisite-r5-privacy-psbt-partial)
records 171 unchanged-Python composition vectors, complete 112-table checks,
snapshot concurrency, a guarded native service driver and named mutations.
Its 342 offline-on/off fixture calls have no fallback dispatch or attempted
egress. The historical graph is injected for this composition oracle; native
inventory admission and database graph construction remain the Mirror
workstream's prerequisite. This does not accept CLI/file/daemon transports or
register product reads. PSBT and legacy-scorer served-by counts remain zero.
Native graph wiring, transport acceptance, routing proof, registration and the
entire separate legacy scorer remain outstanding.

### R5 privacy: native PSBT inventory admission (partial)

`t3code/r5-privacy-psbt-3` starts at `a731608b1`. The native report now has
an inventory-only baseline constructor and an explicit source-admission gate.
Scope resolution, source checks and capture share one SQLite read snapshot.
The constructor retains exact msat, normalized physical identities, ambiguity,
branch provenance, existing script-reuse hypotheses and deterministic observer
components. Admission is limited to 400 inventory rows without spend history,
bindings, occurrence-qualified sources, stored physical transaction evidence,
reference observations/cache or attribution datasets. Unsupported sources fail
`not_implemented`; missing source schemas fail closed. There is no empty-graph
substitution for a historical book.

The [inventory checkpoint](../reference/rust-shell.md#psbt-native-inventory-admission-checkpoint-r5-privacy-psbt-partial)
records 139 full-Python-graph result/error byte comparisons and 15 explicit
native source/bound refusals, all 112 tables, concurrent snapshot checks and
admission mutations. The guarded fixture uses native database capture rather
than an injected baseline. It introduces no Python fallback or network route.
Python product code remains unchanged, including a reproduced corrupt stored
dataset-manifest `KeyError`.

This is a clean partial boundary, not completion of either requested product
gate. The latest inspected Mirror tip `3910d821d` adds caller-supplied
observation merging and confirmation overlays, but its checkpoint still lists
full SQL source orchestration and historical traversal/clustering as incomplete.
Integrating that shared graph remains necessary before CLI/file and
daemon acceptance, transport mutations, hybrid routing proof and registration.
PSBT product served-by counts remain zero; the legacy scorer and Mirror remain
their separate workers' scope. Signing, finalizing and broadcasting are absent.

### R5 privacy: native PSBT historical baseline

The PSBT continuation reuses Mirror's native index and SQL source interfaces
from `4d2e0c289` and `911f5d3e7`. The reduced report now captures historical
public linkage, bounded coverage and reviewed source-funds/counterparty exposure
inside its scope snapshot. Its 408-case unchanged-Python corpus compares exact
report/error bytes and all 112 tables. Product routing remains unregistered;
transport acceptance, mutations and served-by registration follow this boundary.
See the [historical baseline checkpoint](../reference/rust-shell.md#native-psbt-historical-baseline-r5-privacy-psbt).

### R5 privacy: PSBT transport adapter acceptance

The actual native CLI/file and daemon adapters now call the full historical
PSBT report service. Literal UTF-8 files, output projections, argument validation
and refusal precedence match unchanged Python across 528 adapter calls, both
offline-switch states and all 112 tables, with zero egress or fallback dispatch.
Product registration remains unchanged; mutations and whole-process hybrid
served-by proof precede the final registration boundary. See the
[adapter checkpoint](../reference/rust-shell.md#psbt-clifile-and-daemon-adapter-acceptance-r5-privacy-psbt).

### R5 privacy: registered reduced PSBT privacy reads

The PSBT continuation completes the native historical/adversary baseline,
CLI/file and daemon adapters, transport mutations and product registration in
three verified local commit boundaries. `reports.psbt-privacy` and
`ui.reports.psbt_privacy` now execute in Rust: 192 CLI calls and 180 daemon
requests in 12 sessions match the unchanged reference with all 112 tables
preserved, zero hybrid fallbacks and no egress in both offline-switch states.
All 21 transport/registration mutations are killed; the full 408-case historical
corpus also retains unsorted JSON wire order. Inventory is now 155 CLI leaves
(23 reports), with 16 report daemon kinds plus two transaction exports.

The retained decoder exception follows the explicit native diagnostic contract;
Python remains unchanged. Mirror delivery, the legacy scorer, chain-analysis
comparison/Payjoin/entropy transports and AI dispatch are separate work. No
signing or broadcasting authority is introduced. The
[registration checkpoint](../reference/rust-shell.md#registered-psbt-privacy-reads-r5-privacy-psbt)
records routing counts, mutations, checks and the local-only verification limits.

### R5 privacy: legacy hygiene scorer prerequisite (partial)

`t3code/r5-privacy-legacy-scorer` continues from `a731608b1`. The native
Phase-1 scorer preserves transaction/inventory rules, exact msat conversion,
collaboration and domain bounds, inbound attribution, structural evidence,
coverage, risk aggregation and deterministic ranking. The
[legacy scorer checkpoint](../reference/rust-shell.md#legacy-hygiene-scorer-prerequisite-r5-privacy-legacy-partial)
records 584 Python-derived vectors, complete 112-table comparisons, 1,168
guarded fixture-driver calls, six identical hash-seed variants and 39 killed
mutations. Malformed active amounts retain the reference internal error; a
local repro records Python's uncaught `ValueError`, without changing Python.

This prerequisite registers no product route. The existing
`ui.privacy_hygiene.snapshot` still needs CLI-launched daemon acceptance,
zero-fallback routing proof and registration; no new CLI command is planned.
`reports privacy-hygiene` remains the distinct configuration report. Mirror
and PSBT product acceptance remain parallel work.

### R5 privacy: legacy hygiene daemon acceptance

The continuation after `b46684d90` serves the existing
`ui.privacy_hygiene.snapshot` natively through the CLI-launched daemon. It
preserves Python's raw options, active context, expected-scope checks, request
IDs, public projections and insertion-ordered wire bytes. Malformed stored
active amounts retain the reference message with honest native diagnostics;
the daemon rolls back and continues. Python product code is unchanged.

The [acceptance checkpoint](../reference/rust-shell.md#legacy-hygiene-daemon-acceptance-r5-privacy-legacy)
records 342 legacy requests in 14 native daemon sessions, 12 unchanged
configuration-report CLI calls, complete 112-table comparisons, zero fallbacks
and strict no-egress proofs with both offline-switch states. Eight transport
mutations are killed by three named process tests, in addition to the 39 native
scorer mutations. The existing `reports privacy-hygiene` configuration report
is unchanged; this port adds no CLI command, permission or AI tool. Native CLI
inventory remains 154 leaves (22 reports); the report daemon dispatcher adds
one existing legacy snapshot to its 15 report kinds and two transaction exports.
Mirror and PSBT product acceptance remain the parallel workstreams' scope.

### R5 privacy: immutable-index and occurrence prerequisite (partial)

The local continuation from `3291f143a` adds
`kassiber-wallets::analysis_index`: immutable captured snapshots, scoped physical
occurrence resolution and the in-memory public observer projection. Explicit
block occurrences do not fall back to a different observation; grants do not
become physical identity. Public views withhold private ownership, custody,
labels and confidential Liquid facts before future graph analysis. No new
network, storage-write or accounting authority is added.

The [index prerequisite](../reference/rust-shell.md#mirror-index-prerequisite-r5-partial)
records 195 unchanged-Python vectors, canonical result bytes, all-112-table
invariants and 34 named mutation recipes. Two native driver runs evaluate 390
primitives with zero fallback and no consent, with offline mode off/on. This
proves the prerequisite only: observer inputs are captured Python snapshots;
native observation ingestion and durable-index synchronization are still open.
Mirror product served-by counts remain zero and both transports stay
unregistered. Inventory remains 154 CLI leaves (22 reports), 15 report daemon
kinds and two transaction exports. AI projections and Python product code stay
unchanged; the separate PSBT/legacy worker owns its continuation.

A local repro and eight frozen hash-seed results record a Python reference bug:
ambiguous-occurrence findings have seven different orders while retaining the
same snapshot ID. Native ambiguity collections are ordered. The future native
builder must consume them deterministically; this branch does not fix Python.

At the initial checkpoint, the first requested Mirror boundary still needed
observation ingestion, confirmation overlays, durable-index synchronization
and bounded SQL reads.
Subsequent boundaries remain traversal/clustering/patterns;
attribution/datasets/entropy; process/book-bound handles and report composition;
then CLI/daemon acceptance and registration. No full Mirror acceptance is claimed.

The continuation from `f94d5b16d` ports the observation merge kernel and
confirmation overlay as a further prerequisite within that first boundary.
Stored/cached/acquired rows and inventory use the native occurrence resolver,
exact msat arithmetic, ranked conflict barriers, complete-shape checks and
observer projection. Custody relations retain caller-supplied freshness and
remain separate from physical observations. The native builder accepts JSON
rows; legacy mappings containing non-finite numbers or lone surrogates refuse
instead of silently losing fields. Full SQL source orchestration, those legacy
representations, durable synchronization and the bounded read view remain open.
The [continuation checkpoint](../reference/rust-shell.md#mirror-observation-merge-and-confirmation-prerequisite-r5-partial)
records the 339-case unchanged-Python corpus, exact bytes/all-table invariants
and mutation verification. Mirror remains unregistered; AI projections stay
Python. This does not complete any of the five requested Mirror boundaries.

The initial prerequisite's 34 mutations are killed with four passing unmutated
selections. Its local Linux verification passes formatting,
production/test-harness workspace Clippy
with warnings denied, all 159 wallet unit tests (19 socket-dependent cases need
an isolated loopback-permitted rerun), and all five retained index-oracle tests.
The required classifier update records 5,716 definitions; CI preflight passes
57 tests without skips. Build trees are removed after summaries are retained.
Full workspace runtime, full quality-gate and packaged/platform acceptance are
not established by this partial checkpoint.

### R5 privacy: merged analysis and legacy JSON (partial)

`t3code/r5-privacy-mirror-4` starts at `911f5d3e7` and merges the analysis line
through `4d2e0c289`. SQL sourcing and the traversal, clustering, patterns,
attribution/dataset and entropy kernels now share one immutable native index.
The [legacy JSON checkpoint](../reference/rust-shell.md#mirror-legacy-json-compatibility-r5-partial)
adds exact non-finite/surrogate source handling, commitments and observer aliases
with a 512-case unchanged-Python corpus. Typed physical identities still fail
closed. The reference manifest-encoding failure has a separate reproduction;
Python product code is unchanged.

Durable triggers/contributions, incremental synchronization and SQLite/SQLCipher
kill recovery, bounded observer-scoped SQL views, process/book-bound handles,
the Mirror report and CLI/daemon acceptance/registration remain open. Mirror
product served-by counts are zero. AI projections remain Python; this adds no
schema, dependency or egress permission and makes no full Mirror acceptance claim.

### R5 privacy: bounded SQL views (partial)

The [SQL view checkpoint](../reference/rust-shell.md#mirror-bounded-sql-views-r5-partial)
adds observer-scoped bounded reads over an already synchronized index, sharing
the captured traversal and analytics kernels. It preserves caller transactions
and all 112 tables, with 228 unchanged-Python cases and guarded native replay.
Durable synchronization/recovery stays on the parallel 5a branch. Handles, the
Mirror report and transport registration remain subsequent boundaries. Product
served-by counts are zero, and AI projections remain Python.

### R5 privacy: process and book-bound handles (partial)

The [handle checkpoint](../reference/rust-shell.md#mirror-process-and-book-bound-handles-r5-partial)
adds HMAC references with a bounded RAM-only registry, process identity, book
identity and current-view subject revalidation. Forty-eight unchanged-Python
replay vectors and actual process-restart tests preserve bytes, all 112 tables
and the no-egress boundary. This does not register a product entry point or port
generic AI projections. Report composition and durable integration follow.

### R5 privacy: Mirror report projection (partial)

The [report checkpoint](../reference/rust-shell.md#mirror-report-projection-r5-partial)
composes SQL traversal, clusters, patterns, attribution, structural features and
conditional entropy. Private ownership only selects relevance; every observer
finding remains public. The 142-case unchanged-Python report corpus checks both
local and redacted report bytes, all 112 tables, budgets and no-egress. Generic
AI projections remain Python. CLI/daemon acceptance and registration still await
the durable integration boundary.

### R5 privacy: transport adapters awaiting registration

The [transport checkpoint](../reference/rust-shell.md#mirror-transport-adapters-awaiting-registration-r5)
adds report scope/table mapping, CLI scope/format forwarding and daemon argument
validation. Six wrapper cases retain byte and all-112-table parity; three
adapter mutations are killed. Product registration remains deliberately last,
after durable integration and SQLite/SQLCipher recovery acceptance, with
whole-process served-by evidence still required.

### R5 privacy: durable read integration awaiting final registration

The [integration checkpoint](../reference/rust-shell.md#mirror-durable-read-integration-awaiting-recovery-and-registration-r5)
combines 5a triggers/contributions and incremental synchronization with the read
side. A dedicated report savepoint retains successful generation publication
and preserves caller rollback. All 37 native index tests pass; eight report
flow scenarios compare 22 canonical report states and all 112 table commitments.
Recovery acceptance and whole-process registration remain open.

### R5 Chain Analysis: local read checkpoint (partial)

The [read checkpoint](../reference/rust-shell.md#chain-analysis-local-read-checkpoint-r5-partial)
adds a native shared service and eleven CLI commands/nine daemon kinds for local
overview/trace/path queries, synchronous entropy, case list/get/compare, label
listing and dataset list/get/query. Graph reads synchronize the inherited durable
index under caller-owned savepoints; saved cases retain content-integrity checks
and exact claim queries retain public visibility and state-bound pagination.
No acquisition or watch operation is registered by this slice, and AI projections
remain Python.

This does not complete the requested Chain Analysis read inventory. PSBT
analyze/compare/entropy, dataset preview, their asynchronous `.start` operations
and `jobs.get` remain open. The pinned base has shared PSBT public-feature
primitives, but no native PSBT parser/comparison service or source/job runtime.
Those contracts need their own native implementation and process acceptance;
unsupported entry points continue to fail closed without Python fallback.

### R5 Chain Analysis: PSBT, preview and receipt completion

The [completion checkpoint](../reference/rust-shell.md#chain-analysis-psbt-preview-and-receipt-completion-r5)
closes the read inventory. It reuses the native PSBT parser and adds proposal
comparison with BIP78 commitment checks, conditional PSBT entropy, streaming
dataset preview, and staged source grants. A bounded receipt runtime serves the
`.start`, `jobs.get` and `jobs.cancel` kinds. Lock, profile switch and restore
clear grants and receipts and cancel workers. Python's hash-seed-dependent
manifest field order is recorded; the native port uses sorted order. AI
projections remain in Python, and nothing introduces egress.

### R5 chain analysis: local mutation checkpoint (partial)

`t3code/r5-chain-analysis-ops` branches from `b258fb1c9` and includes the
`e4c26e5f7` durable recovery checkpoint. The shared native analysis-operations
service registers seven CLI leaves and matching daemon kinds: label
upsert/import/delete, case save/delete and dataset revoke/discard. The CLI
inventory rises from 154 to 161. Exact appended registries keep this work
independent of the parallel read-command port; AI projections stay Python.

The [acceptance record](../reference/rust-shell.md#chain-analysis-local-mutations-r5-partial)
pins 158 unchanged-Python scenarios, 185 action snapshots, canonical envelopes
and all 112 tables. It includes caller rollback, SQLite/SQLCipher DELETE/WAL
SQL failures, strict no-consent replay with offline off/on, whole-process CLI
and daemon parity and eleven named mutation recipes. Saved cases recompute
through the native durable index; labels retain revisioned history and imports
are atomic. Incomplete dataset cleanup retains Python's bounded commit policy.
All eleven mutations are killed with passing baselines; 41 native unit tests,
17 Python cases and the required 57-test CI preflight pass. Formatting,
production/harness Clippy and the required classifier update also pass.
A separate repro records Python's Liquid legacy address-validation bug without
changing Python product code.

The operations assignment remains incomplete: watches preview/create/list/
configure/delete/inbox/acknowledge/evaluate, sources plan/authorize/list/revoke/
run, dataset import, acquisition plan/apply, their daemon kinds, dataset
import/discard async starts and job cancellation. Exact-action consent,
once-only source authorization and gated acquisition egress with revocation
and cancellation still need implementation and loopback acceptance. No new
network capability or accounting authority is introduced by this checkpoint.

### R5 chain analysis: evidence watches (partial)

`t3code/r5-chain-analysis-ops-2` continues from `dfd7b7e86` and ports all eight
`watches` leaves and daemon kinds through the shared native service, raising
the CLI inventory from 161 to 169. Watches remain local-only and require a
keyed SQLCipher book. The [acceptance record](../reference/rust-shell.md#chain-analysis-evidence-watches-r5-partial)
pins 18 unchanged-Python scenarios with all 112 tables, SQLCipher DELETE/WAL
disk replay, encrypted-book CLI/daemon parity with offline off/on and nine
killed mutations. The daemon's background watch tick, sources, dataset import,
acquisition, async starts and job cancellation remain open.

### R5 chain analysis: source grants (partial)

The five `sources` leaves and daemon kinds now run natively, raising the CLI
inventory to 174. Plans fingerprint the saved Bitcoin Core route without
credentials; authorization, revocation, expiry and the queueing refusal while
offline match Python byte for byte. See the
[acceptance record](../reference/rust-shell.md#chain-analysis-source-grants-r5-partial).
The authorized runner that actually reads blocks remains open.

### R5 chain analysis: acquisition plan and apply

`acquire.plan` and `acquire.apply` and their daemon kinds now run natively (CLI
inventory 176). Apply recomputes the reviewed plan before minting a
request-local `kassiber-net` grant for exactly that backend route, mirrors
Python's request, byte and deadline budget and offline/`KASSIBER_NO_EGRESS`
checks, and saves sanitized observations atomically after revalidating the
plan. Loopback fakes prove identical request sequences, bytes and tables, and
that refused or stale actions send nothing; see the
[egress record](../reference/rust-port-egress.md#chain-analysis-acquisition-apply-r5).

### R5 chain analysis: dataset import (partial)

The CLI's local attribution import runs natively (CLI inventory 177), sharing
the read port's strict parser and matching Python's staging, activation,
progress and cancellation byte for byte. The daemon import and cleanup job
starters run on the read branch's job and picker-token runtime, carried here
unchanged; the integration steps are recorded in the
[import checkpoint](../reference/rust-shell.md#chain-analysis-dataset-import-r5-partial).

### R5 chain analysis: background worker

The daemon's local-evidence worker runs natively: authorized source runs with
durable quota, lease and revision fencing (subject and block-range modes with
reorg handling), then due watch evaluation and the generic wake-up event. Both
daemons' workers produce identical events, loopback request sequences and
tables; see the [source runner record](../reference/rust-port-egress.md#chain-analysis-source-runner-r5).
The operations assignment is complete apart from integration with the reads
branch.

### R5 unassigned desktop projections: resolver checkpoint (partial)

`t3code/r5-desktop-projections` starts at `b10e90e54` (`t3code/rust-backend-r9`).
Only `ui.transactions.resolve` is newly registered: **1/15 requested desktop
kinds**, with no additional CLI or AI registration. The transaction group is
incomplete; transfers/custody, reports/export preview, review/audit and rates
have not been started. The [checkpoint](../reference/rust-shell.md#desktop-transaction-resolution-r5-partial)
records per-kind served-by evidence, exact envelope/all-table comparisons,
named mutation tests and the remaining kinds.

Resolution reuses the native desktop row renderer and current-journal gate.
IDs precede external aliases, duplicate aliases retain Python's SQL ordering,
and a focused leg keeps its observed principal and fee while pair status comes
from the book's review queue. A shared transfer helper binds a uniquely matched
review to its observations and allocations; ambiguous or missing reviews get
no confirmation fingerprint. Reads grant no custody or tax authority and make
no network request. Python product code remains unchanged.

### R5 unassigned desktop projections: continuation (partial)

`t3code/r5-desktop-projections-2` rebases the resolver checkpoint onto
`4844ba8b8` and serves 10/15 requested desktop kinds natively; see the
[continuation checkpoint](../reference/rust-shell.md#desktop-projections-r5-continuation).
It corrects the resolver acceptance, which had compared Python with Python, and
adds Python's report-read journal rebuild to the daemon for all report-like
reads. Graph, transaction review context, evidence summary,
report blockers and review input requests remain.

### R5 desktop settings and sync status (partial)

`t3code/r5-desktop-settings-1` starts at the r10b integration tip `4844ba8b8`
and serves seven more desktop kinds natively: freshness status/configure,
maintenance settings/configure, the active-profile backend list, the Lightning
backend test and the wallet UTXO inventory. See the
[checkpoint](../reference/rust-shell.md#desktop-settings-and-sync-status-r5).
Reads and configuration writes start no job, probe or connection; the Lightning
test is the one explicit egress, granted per request through `kassiber-net`.
`ui.maintenance.run` stays `not_implemented` until report blockers, the
report-read journal prelude and report-read wallet sync are native. Python
product code is unchanged; one reference crash is recorded with a repro.

### R5 workspace shell

The desktop's first screens no longer need the Python daemon. Native
`ui.projects.list/create/select`, both overview snapshots, health, next
actions, review badges, `ui.onboarding.complete`, `ui.secrets.init` and the
`health`/`next-actions` CLI leaves run through `kassiber-setup`, the new
`kassiber-workspace` crate and the daemon's project, workspace-read and
secrets modules. The generic `cancel` control keeps Python's unclassified
refusal. Every read is local and egress-free; project switching preserves
Python's ownership-before-probe and close-before-release order.

Evidence: 55 frozen Python books replayed byte for byte, process oracles over
daemon and CLI bytes with all 112 tables (SQLite and SQLCipher), mutation
entries in `crates/daemon/scripts/shell_mutations.json`, and two recorded
Python reference bugs. The
[Rust shell reference](../reference/rust-shell.md#workspace-shell-r5) owns the
served-by counts and limits.

The daemon's diagnostics (`ui.logs.snapshot` over a bounded, redacted RAM log
ring and `ui.egress.snapshot` over the process-wide `kassiber-net` ledger) and
the `update`/`verify-download` CLI leaves are native too. The new
`kassiber-release` crate verifies release signatures in process with the
`ssh-key` crate, accepting and refusing what Python's `ssh-keygen -Y verify`
does over 106 frozen signatures (it no longer needs OpenSSH, a documented
difference), and contacts GitHub only after an explicit `update --check`. The
round-5h redaction-scanner delta (`a12f3ad85`) is still unmerged on both
runtimes.

### R5 unassigned desktop projections: completion

`t3code/r5-desktop-projections-3` brings the requested desktop kinds to
**14/15** native: `ui.report.blockers`, `ui.transactions.review_context`,
`ui.audit.evidence.summary` and `ui.transactions.graph`, plus
`ui.maintenance.run`, with report-read wallet refresh behind the book's
freshness policy and narrowly scoped `kassiber-net` consent; see the
[completion checkpoint](../reference/rust-shell.md#desktop-projections-r5-completion).
It reuses the workspace-shell readiness library and the settings slice's
maintenance state instead of porting them twice, and keeps
`ui.review.request_input` with the review workflow slice. The domain reads
live in `kassiber-journals`, the highest domain crate that can reach the
owned index, custody components, transfer matching and the workspace health
without a dependency cycle. Graph public lookups, report-read refresh of
rates/BTCPay/Lightning/Silent Payments sources and the no-backend refresh path
refuse with `not_implemented`. All 21 mutation entries are killed by named
tests. Python product code is unchanged.

### R6-1 local IPC, operator protocol and ownership

`crates/ipc` (the only crate allowed `unsafe`) holds the `AF_UNIX`
peer-credential, named-pipe, explicit-inheritance launch, signal and
pipe-wait primitives. `crates/operator` ports Python's frames and limits,
the challenge, the passwd-home runtime directory, the endpoint election,
build identity and the client for all 14 actions. `crates/ownership` now
writes Python's record bytes, names the holder's pid, ports the seven-day
janitor and the Windows `OPEN_ALWAYS` probe, and hands out inheritable
duplicates. Nothing is registered yet.

Parity rests on a Python-generated corpus of frames, refusals and per-action
client bytes, replayed in Rust and checked for drift. Python's client also
talks to a Rust listener, and the Rust client to the Python broker fixture.
A store-interop case shows a Rust-launched child's inherited locks blocking
Python. The 74 Python protocol, client and project tests are ported or mapped.
All 29 mutation entries are killed.

The Windows code is not compiled here because no Windows target is
installed. The [checkpoint](../reference/rust-shell.md#operator-protocol-local-ipc-and-ownership-r6-1)
records its tests, reviewed differences and what R6-2 builds on it.

### R6-2 broker service, server and children

`kassiber --operator-broker-server` runs the Rust broker. `crates/operator`
ports the service (leases, per-project queues, the operation state machine,
retention and tombstones, replay, single-use admin authorization, auth
backoff, the janitor, idle restart and close), the runner and the server
with its 64-client limit, signal stop, detail filter and Linux login guard,
read from systemd's user record. Storage, modes, classification and paths
come through a backend seam the binary implements; the CLI gains a
`classify` API and the broker-child mode. Touch ID answers
`native_auth_unavailable` until R6-4.

Parity: 93 service, 31 server and 10 mode tests port the Python suites with
fakes; nine binary tests run real children; the Python integration suite
passes five of six scenarios against the Rust broker through a fixture
switch; and a differential driver compares a whole scenario of replies,
errors, operation records and child envelopes with the Python broker. It
found that the store's `unlock_failed` lacked Python's hint and driver
error, and a brokered rotation failed on its own inherited owner lock; both
are fixed. 39 new mutation entries cover admission order, capabilities,
scope, replay, retention, expiry, admin authorization, result states,
pinning and the guards. The
[checkpoint](../reference/rust-shell.md#operator-broker-service-server-and-children-r6-2)
records the details and what R6-3 and R6-4 take on.

### R6-3 operator CLI, brokered routing and agent access

The seven password `operator` leaves and `--operator-auth-fd` are native and
talk to the per-user broker with this binary's build identity. Brokered
routing runs once in the CLI, in Python's place, for every native command:
it pins the canonical root, sends secrets only as labelled `S` frames,
never starts a broker, writes the accepted line, waits, cancels on Ctrl-C
with exit 130 (a SIGINT-only flag in `crates/ipc`, the console handler on
Windows) and maps a result without child output to Python's typed errors.
`config/agent-access.json` lives in `kassiber-setup`; the daemon registers
the five `ui.agent_access.*` kinds and ticks the desktop's agent lease in its
request loop, ending it with the desktop session. macOS agent labels ask
`/bin/ps`. Touch ID stays R6-4 and `mcp` R6-8.

Parity: the four Python suites are ported as Rust tests (with seams for the
broker, clock, preference file and labels) or covered end to end through a
real broker and daemon, and a new differential driver compares 39 CLI calls
and 24 daemon requests per language, each through its own broker, byte for
byte after declared mappings, with the hybrid log showing Rust served them.
The MCP integration scenario still waits on R6-8's `mcp call`. 29 new
mutation entries cover the agent-access file, the agent-session checks, the
desktop lease's intervals and conditional lock, and the CLI's broker choice,
build binding, exit codes and mapping. The
[checkpoint](../reference/rust-shell.md#operator-cli-brokered-routing-and-agent-access-r6-3)
records the details and what R6-4 and R6-8 take on.

### R6-5a AI tool executor and read tools (partial)

`t3code/r6-5a-ai-tools` starts at the r10b integration tip `809b168c6`. It adds
`crates/ai`, which holds:

- the embedded 161-tool catalog, with a drift test against Python;
- per-turn tool selection and the schema validator;
- argument and result redaction and consent summaries;
- the consent state machine;
- the read executor;
- the auto-read planner and context trimming;
- provenance and the privacy receipt;
- the embedded skill references and the read-side projections, including the
  chain-analysis `ca-ref:` projection.

The daemon serves the executor's database work on its request loop. Only the
test-harness kinds `test.ai_tool` and `test.ai_auto_read` drive it until chat
and MCP are registered (design §7 item 1). 77 of the 100 read tools run
natively. The other 23 answer `not_implemented`, because their services are not
native. Two of them egress, and those also need a consent bound to the call.

A per-tool differential driver matches Python byte for byte on three fixture
books. It also runs inside an empty network namespace. 22 mutation entries are
killed by named tests. See the
[checkpoint](../reference/rust-shell.md#ai-tool-executor-and-read-tools-r6-5a)
for the refused tools, the reviewed differences, one Python quirk kept with a
repro, and the R6-5b remainder. No new external dependency and no Python
product change.

### R6-6a AI providers, HTTP streaming and discovery

`t3code/r6-6a-providers` starts at `6297511d1` (R6-5a's first commit) and
moves the provider store from `crates/wallets` into `crates/ai`, which now also
holds the Responses/Chat Completions client and the discovery cache.
`kassiber-net` gains a streaming reader and a socket-level cancel. The 13
provider daemon kinds and the 9 `ai` CLI leaves are registered; paths that need
a CLI agent runtime answer `not_implemented` until the provider broker client
(R6-6b). Listing, reading and editing providers contacts nothing; model
listing and connection tests contact only after the provider gates, through a
route bound to that request. See the
[checkpoint](../reference/rust-shell.md#ai-providers-http-streaming-and-discovery-r6-6a)
for the parity driver, the 19 mutation entries and the reviewed differences
(64 MiB response and 16 MiB SSE line caps, following the design's
recommendation). Python product code is unchanged.

### R6-6b AI provider broker client

`t3code/r6-6b-broker-client` starts at `40caf2ea2` (the R6-6a tip).
`kassiber-net` gains a gated process route for the TypeScript provider broker
(`Subsystem::Ai`, `Operation::Process`): the online gate and an
explicit-trigger consent bound to the program, the attempt observer on every
launch, Node lookup with Python's three environment overrides, the
`XDG_RUNTIME_DIR` temp-root checks and a new process group. `crates/ai` gains
the broker client: probes under one deadline, chat legs with keep-alive
between tool calls, per-leg watchdogs and SIGTERM-then-SIGKILL.
`ai.provider_runtime.status` probes and model listing and connection tests on
`*-cli://default` rows no longer answer `not_implemented`; nothing starts on
launch, idle, listing, editing or `cached_only` status. See the
[checkpoint](../reference/rust-shell.md#ai-provider-broker-client-r6-6b) for
the stream and provider parity drivers, the 17 mutation entries and the
reviewed differences. `ai.chat` (R6-7) is the client's first chat caller.
Python product code is unchanged.

### R6-7 AI chat

`t3code/r6-7-chat` starts at `b12e22ac5` (the R6-6b tip) and merges the R6-5a
tip `322cfb0f0`. `crates/ai` gains the chat turn (worker loop, consent,
cancel registry, provider cursors, history under the persistence policy,
provenance with a per-turn request receipt) and the embedded Kassiber
prompt; the daemon serves `ai.chat`, its controls and the five history kinds,
running each turn's book work as request-loop tasks; the CLI serves `chat`
(a daemon child unlocked over stdin) and the five `chats` leaves. All of it
sits behind the `native-chat` feature, which only test-harness builds
enable, as design §7 item 1 recommends: chat registers in product builds once
R6-5b, the 23 refused read tools and document-import staging are native. See
the [checkpoint](../reference/rust-shell.md#ai-chat-r6-7) for the stream
driver, the 17 mutation entries, the reviewed differences and one Python
quirk kept with a repro. Python product code is unchanged.
### R6-8 MCP server and chat Ctrl-C

`t3code/r6-8-mcp` merges the R6-3 tip into the R6-7 tip. The new crate
`crates/mcp` holds the dual-era stdio JSON-RPC server (single tool worker,
limits, cancellation, the 30 s drain) and the 44 read-only definitions
derived from the AI catalog; `kassiber-cli` adds the six `mcp` leaves with
the stdio claim, the direct path (agent access first, never creating a
project or book, a read-only scope pin under SQLite `query_only` that never
writes the stored context, the book opened and closed per call through the
R6-5a read executor as a remote provider) and the brokered path (the
operator client, agent sessions, 0.1 s polling, cancelling queued work,
never starting a broker). `mcp serve` and `mcp call` register behind the
`native-mcp` feature until the 13 MCP reads R6-5a refuses are native; the
other four leaves always register. `kassiber chat` now cancels a turn on
Ctrl-C and keeps its session; its daemon child leads its own process group,
where Python's dies with the terminal's Ctrl-C (a recorded repro).

Parity: a new stdio transcript driver compares every reply of Python and
Rust byte for byte on copies of the same fixture books, plus the one-shot
leaves; the in-process cases of `test_mcp_server.py` are ported as Rust
tests; the MCP scenario of `test_operator_integration.py` runs under the
Rust broker; a Rust-on-Rust test serves a brokered book through the
desktop's agent grant. 38 mutation entries cover era selection, error
codes, limits, id typing, the escaped result size, the access gate, the
brokered checks, the pin and the chat Ctrl-C. The
[checkpoint](../reference/rust-shell.md#mcp-server-and-cooperative-chat-ctrl-c-r6-8)
records the details.

### R6-5b AI tool writes, previews and projections

`t3code/r6-5b-ai-tools` merges R6-5a into the r10b integration tip
`a0b4d0342`, where workspace, journals, review, desktop projections and wallet
onboarding are native. It:

- lifts 22 of R6-5a's 23 read refusals, each after the driver matched Python
  byte for byte on the fixture books; only `ui.connections.node.snapshot`
  still refuses (no native node-snapshot route);
- adds the mutating executor for all 61 writing tools, the custody AI audit
  writer, and the consent cards with their server-side previews (review apply,
  acquisition, dataset import);
- adds the remaining projections: the review worklist and transaction review
  composites, the desktop `ui.chain_analysis.ai_context` kind, the Lightning
  profitability aggregate, file analysis, ownership identification, the
  redacted Privacy Mirror and the AI egress view;
- passes the AI-path parameters to the native routes: `ai_tool` authorship
  for metadata edits, reverts, quarantine resolutions and the review workflow,
  strict `ui.wallets.sync` without journal pairing, journal processing without
  pairing, and maintenance runs that honour the turn's refresh state.

Writes still refused: the PDF and audit-package export targets,
`ui.source_funds.export`, `ui.btcpay.provenance.review`, and `ui.wallets.sync`
for books whose sources need Python's direct-sync path. The chain-analysis
`snapshot_id` was already derived as Python derives it; its instability came
from two read paths that drew the index identity from the wrong source. Two
native journal-view gaps found by the driver (calculation semantics in view
freshness) are fixed. See the
[checkpoint](../reference/rust-shell.md#ai-tool-writes-previews-and-projections-r6-5b)
for evidence, reviewed differences and what chat still needs. No new external
dependency and no Python product change.

### R5 PDF: text reports and the shared report design

On 2026-11-02 the owner brought PDF into scope with Typst as the engine, and
on 2026-11-04 asked for one minimalist report format shared by every country
and by PDF, XLSX and CSV instead of reproducing ReportLab's layouts. Python
stays the reference for report data, not for layout.

The hand-written PDF 1.4 writer was first ported with byte parity for the
full report and exit-tax PDFs; the shared design below replaced it (see
[the checkpoint](../reference/rust-shell.md#pdf-reports-r5-partial)).

The owner approved the [report design](../reference/report-design.md) on
2026-11-04 with five decisions: Austrian books in German with Austrian number
and date formats (English otherwise), every string from one table per
language, CSV always machine format; full upstream DM Sans and JetBrains Mono
pinned by hash, amounts in DM Sans tabular figures, Liberation Sans as glyph
fallback; the new CSV format without a legacy option; Austrian sections as a
`section` column on shared Disposals and Income tables with an index on the
Kennzahl page; red only for blockers and the main chart line. The rollout
replaces one report per commit, each with data parity against Python's
model; the [rollout table](../reference/report-design.md#rollout) records
progress and every necessary envelope difference.
