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
- Python leaves the product at cutover (stage R9): no interpreter, no
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
| `tax-engine/python` | the PyO3 binding; grows a `custody` operation in R1 and is deleted in R9 |
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

1. **Compute behind PyO3** (stage R1). Custody computation moves into Rust
   behind the existing binding, as the tax engine did. Python keeps SQL,
   descriptor derivation, and persistence, so the whole Python suite keeps
   running and a shadow mode compares both implementations on every build.
2. **The binary grows** (stages R2-R8). The Rust binary opens the same data
   roots and implements commands and daemon kinds group by group. Python stays
   the reference. A development-only fallback lets the Rust binary hand a
   command it does not implement yet to the Python CLI, so black-box suites
   and recorded scenarios replay end to end while the port is partial. The
   fallback is compiled only into test builds and never ships.
3. **Cutover** (stage R9). The desktop and every release artifact switch to
   the Rust binary, the fallback goes, and Python is deleted.

## Verification

1. **Crate tests:** unit, property, and golden-vector tests per crate. Vectors
   for Python-compatible text (JSON, float `repr`, `Decimal`) come from
   CPython, as the tax engine's decimal vectors do.
2. **Shadow runs (R1):** a test-only switch runs Python and Rust custody on
   every journal build in the suite and fails on any difference. Owners can
   run the same comparison on a disposable copy of a real book.
3. **Recorded corpus (R0 onward):** a recorder hooks the Python CLI's entry
   point and the daemon's request handler during a full test-suite run. Each
   transcript holds the command or kind and arguments, the relevant
   environment, stdin, and normalized snapshots of the data root before and
   after, plus the output. Replay runs the Rust binary on the same starting
   state and compares output and end state, with normalizers only for uuid4
   values, wall-clock times, and temporary paths. A coverage report lists
   every CLI leaf and daemon kind; each stage closes its gaps with new
   scenarios, and cutover requires none.
4. **Black-box suites:** a `KASSIBER_TEST_CLI` switch points the subprocess
   tests (CLI smoke, the daemon smoke's subprocess half, CLI chat, MCP, entry
   point smoke), the packaged-binary smoke, and the regtest integration
   harness at the Rust binary. The Tauri supervisor tests and the Vitest suite
   run unchanged, because the protocol does not change.
5. **Egress proof:** today's test guard is a Python socket patch that cannot
   reach a Rust process. Every Rust connection goes through one transport
   layer whose test build refuses connections unless a test allows them, the
   binary honors `KASSIBER_NO_EGRESS` and the test no-egress modes including
   strict loopback, and CI runs the black-box suites in a network namespace
   without a route out. Every egress surface keeps a regression proving no
   traffic before consent.
6. **Packaging:** every release artifact builds, signs, and passes the
   packaged smoke on macOS arm64, Linux x86_64 on the AppImage glibc floor,
   and Windows x86_64.

## Stages

Each stage gets its own section here when it starts, moves its tasks into
[TODO.md](../../TODO.md), and lands as one or more draft pull requests on the
stack.

0. **R0 Foundations.** Pin the toolchain and form the root workspace; cache
   Rust builds in CI. Move the decimal type into `crates/pydecimal`, and build
   `crates/pyjson` against CPython vectors. Fix the
   two Python behaviors that would make an oracle unstable: the swap-dismissal
   expiry reads the wall clock inside the custody path, and the custody hash
   serializations have no golden vectors. Add the corpus recorder and replay
   harness and the `KASSIBER_TEST_CLI` switch.
1. **R1 Custody.** Define the versioned custody snapshot (below), port the
   pure custody computation, freeze the Python modules as an oracle, add the
   shadow lane, and join custody and the tax engine in one `journal`
   operation. Python keeps SQL, the pre-build repairs, descriptor derivation,
   and persistence.
2. **R2 Store.** Open, create, and migrate every database Python can, through
   `rusqlite` with bundled SQLCipher; the data-root and project model; unlock
   and keyring. Done when every fixture database reaches the same normalized
   dump through both.
3. **R3 Binary and journals.** The CLI tree, checked against
   `commands describe`; the envelope and error mapping; projects, workspaces,
   profiles, accounts, wallets, CSV import, and journal processing end to end
   in Rust, including snapshot loading, the ownership index, and persistence.
4. **R4 Imports and documents.**
5. **R5 Reports and exports.**
6. **R6 Chain and network:** sync backends, Liquid, Lightning, BTCPay, rates,
   chain analysis, and the network crate with the consent gate and egress
   ledger.
7. **R7 The remaining CLI groups:** transactions, metadata, transfers and
   components, gap reviews and the review workflow, source of funds, loans,
   attachments, secrets, backup, maintenance, diagnostics, update and release
   verification, and the small status commands.
8. **R8 Daemon, AI, MCP, and operator broker.**
9. **R9 Cutover.** Switch the desktop sidecar and every release artifact,
   delete Python and the PyO3 binding, move the remaining test coverage into
   Rust tests and the corpus, and update every doc and AI reference that names
   Python.

### R1 custody snapshot

One versioned request document, built in the journal transaction after the
source-overlap repair, the legacy component refresh, and auto-pricing:

- profile facts, including the journal input version and an explicit `as_of`
  that replaces the wall clock for dismissal expiry;
- wallets with labels, kinds, accounts, and normalized configuration facts
  (chain, network, the Samourai block), never descriptors or xpubs;
- the active transactions in their database order, with every column the path
  reads and their provenance;
- the owned index restricted to the scripts, outpoints, and txids the rows'
  graphs reference, or none when no row has provenance;
- the receipt-reconciliation sources, Lightning channel records, loan legs,
  materialized active components, evidence snapshots, migration quarantines,
  dismissed gap fingerprints, and pair dismissals;
- for books above the gap search's row limit, the SQL-selected worklist.

The response carries the finalized tax projection, the quantity state, the
interpreter quarantines, blocked and non-event ids, native reconciliations,
custody transfers, warnings, component blockers, and the gap-ignored id sets;
in the joined `journal` operation it also carries the tax ledger, the wallet
balance comparison, and the ownership-review projection.

## Risks

- **Scale.** About 218,000 lines of Python, 313 CLI leaves, 303 daemon kinds,
  91 tables. The corpus coverage gate per stage keeps this measurable.
- **A long stack against a moving `main`.** Python features keep landing on
  `main` during the port. Each merge of `main` into the stack re-records the
  corpus, so a new Python behavior shows up as an uncovered or failing
  transcript.
- **Mostly white-box tests.** About 160 of 273 test modules call Python
  internals. The recorder turns them into black-box transcripts; what it
  cannot reach is ported as Rust tests before Python is deleted.
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
- **PDF output.** No Rust library matches ReportLab's layout engine, so the
  four ReportLab reports are rewritten. Extracted text, tables, and page
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
