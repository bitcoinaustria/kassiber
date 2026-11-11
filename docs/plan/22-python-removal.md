# Removing Python

**Status:** Phase 1 (inventory and plan), 2026-11-10. The owner decided on
2026-11-10 that Kassiber is Rust only. Nothing here is done yet. Phase 2 waits
for the [open decisions](#open-decisions).
**Base:** `t3code/stack-b8-switch` at 28a7c03e2. B8 already ships the Rust binary
as the CLI and the desktop sidecar. This is stage R7's "delete Python" step in
[plan 21](21-rust-backend.md#stages).
**Evidence:** counts come from `git ls-files` on the base. Test counts and
timings come from the test-debloat agent's collection on this commit
(21,198 tests) and its per-test junit timings (`tables/lanes.tsv`,
`modules-before.tsv`). The per-module grouping is in
[22-python-removal-tests.tsv](22-python-removal-tests.tsv).

## What Python is today

| Area | .py files | Lines | Role |
| --- | ---: | ---: | --- |
| `kassiber/` | 295 | 237,660 | the old backend; nothing ships it since B8 |
| `tests/` | 856 | 328,438 | pytest: 638 test modules, 218 helpers, oracles, harnesses |
| `crates/*/scripts`, `crates/*/tools` | 298 | 33,741 | vector generators, mutation wrappers, Python bug repros |
| `scripts/` | 23 | 7,425 | release tools, benchmarks, shard runner, PyInstaller entry |
| `tax-engine/` | 5 (+ the PyO3 crate) | 3,737 (+151) | RP2 fixture generators, the shared mutation runner |
| `dev/regtest/` | 7 | 4,736 | regtest stack seeding (BTCPay, Bark, Lightning, source of funds) |
| `docs/reference/tax-country-reference/` | 10 | 2,270 | independent reference calculators for country vectors |
| **Total** | **1,494** | **618,007** | |

Python dependencies besides that: `pyproject.toml` and `uv.lock` (54 packages),
the PyO3 crate `tax-engine/python` (workspace member, the only `pyo3` user),
29 CI jobs that install Python, and `.venv` lookups in the desktop shell and
the Vite bridge.

## Inventory and actions

### Delete

- **The backend:** all of `kassiber/` except the files listed under *move*.
  This includes `kassiber/cli/main.py`, the `kassiber` console script, and
  `python -m kassiber`.
- **The PyO3 binding** `tax-engine/python` and its workspace entry. This
  removes `pyo3` and its tree from `Cargo.lock` and from `THIRD_PARTY_LICENSES.md`.
- **Packaging:** `scripts/build-macos-arm64-app.sh` (the PyInstaller app) and
  `scripts/kassiber_pyinstaller_entry.py`; `pyproject.toml` package data;
  `uv.lock` (or shrink it to a dev-only test project, see D1).
- **Desktop fallbacks:**
  - In `ui-tauri/src-tauri/src/supervisor.rs`: `KASSIBER_PYTHON`
    (`env_python`), `default_python()` and `python_fallback`. A missing sidecar
    becomes a clear error. 28 strings in the supervisor and `lib.rs` say
    "Python daemon" and become "daemon".
  - **Vite bridge** (`ui-tauri/vite.config.ts`, `bridgePythonCommand`): it
    still starts `python -m kassiber daemon` first on the base. It must start
    the Rust binary (`KASSIBER_BIN` or `target/{release,debug}/kassiber`), not
    just lose a fallback.
- **CI:**
  - In `ci.yml`: `Preflight`'s Python steps, `python-tests` (9 shards),
    `tax-shadow` (9), `custody-shadow` (9) and `cli-smoke`.
  - The uv steps in `integration.yml`, `chain-observers` and
    `credential-platforms.yml`.
  - The `python-binding` Cargo cache key.
  - `setup-python` in the release workflows, once their scripts are ported (D3).
- **Python-only tests:** 274 modules, 7,080 tests (groups `dies`,
  `dies-binding` and `dies-generator-check` below).
- **Bug repros:** 30 `repro_*` scripts in `crates/*/scripts` and 46 in
  `tests/rust_port`. They reproduce Python flaws. The Rust pin tests and the
  `rust-shell.md` divergence notes stay.
- **Benchmarks and campaigns that drive Python:**
  - `scripts/benchmark-*.py` and `journals-bench-*.py`.
  - `check-journal-*-campaign.py` and `check-journal-prebuild.py`.
  - The shadow and census switches (`KASSIBER_TEST_TAX_ENGINE`,
    `KASSIBER_TEST_CUSTODY_ENGINE`) and `scripts/python_test_shards.py`.
- **Mutation entries that mutate Python files:** 108 of the 10,226 entries in
  13 catalogs.

### Move

Rust reads these from under `kassiber/`. Each move must leave the binary's
output byte-identical.

| Asset | From | To | Callers to update |
| --- | --- | --- | --- |
| AI skill references (10 files, 3,051 lines) | `kassiber/ai/skill_references/` | `crates/ai/skill_references/` | 10 `include_str!` in `crates/ai/src/skills.rs`; `docs/reference/ai.md`, `r6-ai-operator-port.md`, `rust-packaging.md`; `AGENTS.md`; `skills/kassiber/SKILL.md` links; the `prerelease-binaries.yml` path filter. The allowlist (`SKILL_REFERENCE_NAMES`) and the catalog's index are names only and don't change. |
| Kraken rates (4 CSV, 15 MB, 2 README) | `kassiber/data/rates/kraken/` | `crates/rates/data/kraken/` | 4 `include_str!` and `bundled_path()` in `crates/rates/src/csv.rs`; `scripts/build-bundled-hourly-rates.py` |
| Provider-broker bundle (build output, gitignored) | `kassiber/ai/provider_broker/index.mjs` | `ui-tauri/provider-broker/dist/index.mjs` | `ui-tauri/package.json` `broker:build`; `prerelease-binaries.yml` (7 sites); `build-linux-x64-rust-artifacts.sh`; `build-macos-arm64-rust-app.sh`; `kassiber-cli.spec`; `.gitignore` |
| Python-core rules | `kassiber/AGENTS.md`, `kassiber/CLAUDE.md` | `crates/AGENTS.md`, `crates/CLAUDE.md` | rewritten for the Rust crates; the root `AGENTS.md` links |
| Journal semantics pin | `crates/store/tests/semantics_reference.rs` reads `kassiber/db.py` | the Rust constant is the authority | delete the test; the stored-semantics goldens keep it pinned |

`bundled_path()` is compiled in and shows up as `path` in
`ui.rates.kraken_csv.import` and in the freshness job's arguments. The path
already differs per build host, and it is not a stored commitment. Phase 2
still checks that no table stores it before the move.

Report templates and fonts are already Rust-owned (`crates/reports/templates`,
`crates/pdf/fonts`), so they don't move. 45 Rust files read 54 fixtures under
`tests/fixtures/`. I recommend keeping that path as the Rust-owned fixture
directory; moving it would rewrite every `include_str!` and the test-data lock
for no gain.

### Port to Rust

| Tool | Today | Port |
| --- | --- | --- |
| Mutation runner | `tax-engine/scripts/mutation_check.py` (360 lines) plus 75 wrappers (2,800 lines, several with custom runners). 120 catalogs, 10,226 entries. | One dev-only binary, e.g. `crates/mutation-check`, with `publish = false` and no new dependencies (serde_json is already in the tree). It covers `--check` (catalog validation), runs on one shared scratch build dir, and reads the per-catalog package and feature sets from the catalog files instead of wrappers. |
| Preflight contracts | `test_connection_catalog_drift`, `test_dependency_drift`, `test_workflow_pins`, `test_test_data_lock`, `test_report_contract_drift` (about 1,200 lines) | Rust tests in a `crates/workspace` test target. Lock integrity can lean on `fetch-test-data.sh --check`. |
| Test classifier, shard map | `tests/rust_port/classify_tests.py`, `scripts/python_test_shards.py`, `test_ci_shards`, `test_inventory`, `test_contracts` | They exist to classify and shard Python tests, so they end with them. If D1 keeps some pytest, a 50-line shard list replaces them. |
| Chokepoint lint | already Rust (`crates/net/tests/chokepoint.rs`) | only its mutation wrapper moves to the runner; drop the `.venv` skip |
| Write-transaction lint | Rust (`crates/store/tests/write_transactions.rs`) and Python (`test_write_transaction_locks.py`, which scans Python code) | the Python half goes |
| Docs checks | `scripts/tax-country-vectors.py` and `docs/reference/tax-country-reference/*.py` recompute the hand-written vectors | the vectors JSON stays and `crates/reports/tests/tax_country_vectors.rs` keeps checking Rust against it (D6) |
| `check-rust` over custody recordings | `tests/custody_snapshot.py` recomputes the Python stages live, then compares Rust through the binding | Freeze the Python stages into the recordings once (`refresh`). A Rust example then replays a recording and compares every stage. The final gate needs this. |
| `check_ledger_features.py` (CI `Rust crates` job) | 20 lines of Python | a Rust test or 10 lines of shell |
| Release tooling | `release_manifest.py` (imports `kassiber.release_verification`), `render_{aur,homebrew,nix}.py`, `macos_release.py`, `prepare_macos_release.py`, `release_notes.py`, `write_build_info.py`: about 1,700 lines and 8 test modules | D3 |
| Generated Rust sources | `crates/cli/src/spec_data.rs` (from `kassiber/cli/main.py`), `crates/daemon/src/kind_order.rs`, `crates/custody/src/interpretation/printable.rs`, `crates/pytext/src/{casefold,unicode_tables}.rs`, `crates/ai/src/data/catalog.json` | They become hand-maintained source. Update the headers and delete their generators. The Unicode tables stay pinned to CPython 3.11 / Unicode 14 on purpose, because stored bytes depend on them. |
| Python as a test helper in Rust tests | the supervisor's `stub-daemon.py`; `python3` fake brokers in `crates/ai/src/service.rs`, `crates/ai/tests/broker.rs`, `crates/net/src/broker.rs`, `crates/net/src/adapter_tests.rs`; `.venv/bin/python` references in `crates/kassiber/tests/{legacy_privacy_transport,privacy_reports,psbt_privacy}.rs` | stubs become `sh` scripts or a small test binary; the `.venv` references switch to frozen goldens |
| macOS/Windows credential coverage | `credential-platforms.yml` runs nine Python operator and remembered-unlock test modules on macOS and Windows. Rust's `kassiber-credentials`, `-secrets` and `-operator` tests run on Linux only. | Add `cargo test -p kassiber-credentials -p kassiber-secrets -p kassiber-operator` to that job before the Python step goes, or keychain coverage on those platforms is lost. |

### Generators become frozen goldens

There are 178 generators in `crates/*/scripts`; 157 of them import `kassiber`
and make oracles by running Python. There are also the RP2 runners in
`tax-engine/scripts`, the tax oracle (`tests/tax_oracle`, 7,404 lines) and
`tests/fixtures/historical/build_pre_sync_fixture.py`. Their outputs are
already files:
- 399 files (707 MB) on `t3code/test-data`, pinned by `tests/test-data.lock`;
  Rust tests read 251 of them;
- 28 JSON files in the tree;
- the 5 generated Rust sources above.

After removal, those files are frozen goldens nothing can regenerate. The
last Python commit stays reachable through a tag (D4).

The new flow for a deliberate change:
1. Each golden-reading Rust test gets a write mode behind one workspace-wide
   switch, `KASSIBER_GOLDEN_WRITE=1`. In that mode the test writes its actual
   output to the golden's path instead of asserting. A test refuses the switch
   when `CI` is set, so goldens can't update themselves in CI.
2. Make the change, run the failing tests, regenerate with the switch, and
   review the diff with a structural JSON diff. That diff is a Rust port of
   `tests/rust_port/json_diff.py`; it names every changed field.
3. Every changed golden must be explained by the change. Under the parity
   policy, tax and journal numbers, stored data and exports need the owner's
   sign-off.
4. Run `scripts/fetch-test-data.sh --publish`. It commits to the data branch
   and rewrites the lock; the lock bump goes in the same PR as the code.
   Then rerun the touched crates' mutation catalogs.

Goldens that recorded product sessions follow the same flow with Rust
tools, never the Python capture tools that made them:
- **The recorded parity sessions** (`tests/data/sessions/*.tar`) come from
  the session recorder. The recorder needs the pytest suite, so after
  Python they are re-made by replaying: `scripts/replay-sessions.sh` names
  every session that differs, and after review a deliberate change
  re-records those with `session-replay replay --update`, which writes the
  new run's output, exit and state into each differing session (same argv,
  environment and starting state). Review the diffs, archive with
  `session-replay archive`, publish, bump the lock.
- **The porters' recorded goldens** (P2's freshness sessions and tapes,
  P3's CLI sessions and books, P1's routing replays) are rewritten by their
  own Rust tests under `KASSIBER_GOLDEN_WRITE=1`. P4's black-box
  transcripts are rewritten under `KASSIBER_BLESS=1`. Both switches are
  refused under CI. A golden that needs new network traffic is re-taped
  against a regtest stack with `KASSIBER_TEST_NET_TAPE=record:<file>`.
- In every case, review the diff, publish to the data branch, and bump the
  lock in the same PR. The P2 capture tools in
  `~/.cache/kassiber-verify/port-p2/` and `tests/rust_port/freeze_session_books.py`
  go with Python.

## Tests

The pytest classification comes from the test-debloat agent when it reports.
Until then, this is a static grouping by module. It reads each module's
imports and transitive test helpers: how the module reaches Rust (the binary,
an example binary, or the binding), whether it imports `kassiber`, and whether
it compares against a reference. The per-module list is in
[22-python-removal-tests.tsv](22-python-removal-tests.tsv).

| Group | Modules | Tests | Local s | Rust mutants killed only here | Fate |
| --- | ---: | ---: | ---: | ---: | --- |
| `dies`: Python product only | 265 | 6,071 | 725 | 0 | delete |
| `dies-binding`: tax engine through PyO3 | 4 | 640 | 60 | 0 | delete; the Rust crate tests and `ledger_*` fixtures cover the engine |
| `dies-generator-check`: rerun a Python generator `--check` | 5 | 369 | 109 | 0 | delete once the goldens are frozen |
| `tooling`: CI, packaging, release, harness checks | 22 | 277 | 15 | 0 | port (see above, D3) |
| `black-box`: Rust only, no Python product | 30 | 128 | 1,772 | 2 | D1 |
| `parity-seeded`: Rust under test, Python seeds the book | 108 | 1,946 | 1,960 | 3 | D1; the seeding must change whatever D1 says |
| `parity-compare`: Rust compared with Python | 204 | 11,767 | 26,349 | 601 | freeze, then D1 |
| **Total** | 638 | 21,198 | 30,992 | 606 | |

How the 342 Rust-touching modules reach Rust:
- 153 through the binary only (`cli_runner`, `KASSIBER_RUST_BIN`);
- 49 through the binary plus an example binary;
- 37 through example binaries only;
- 94 only through helpers;
- 9 through the binding.

Facts that drive D1:
- **312 modules import `kassiber`, directly or through a test helper.** They
  all break when `kassiber/` goes, so keeping pytest is not free. Only the 30
  black-box modules would survive unchanged.
- **606 Rust mutants are killed only by pytest,** spread over 64 modules
  (`test_rust_cli_leaves`, `test_rust_freshness_*`,
  `test_rust_wallet_completeness`, `test_rust_ai_tools`, and others). Deleting
  those modules without a replacement loses kills, which the gate forbids.
- **Parity time dominates:** 26,349 of 30,992 local test-seconds. Python runs
  at least 4 times slower than Rust, so most of that time is Python.

## Session replay

Step 3 records every Rust process a parity test starts, once, at the base
where Rust and Python agree, and replays them against the Rust binary
(`crates/session-replay`). A session holds the argv, the kept environment,
stdin with its timing, the descriptors it was handed, stdout, stderr, the exit,
the network tape, and the state roots before and after. Recordings live on
the test-data branch, never in the tree.

Recording and storage rules:
- **Synthetic secrets only.** A passphrase or token enters a recording only
  if it is a literal of the checked-in test sources (the allowlist the
  recorder generates). A session handing over any other passphrase is
  refused; a secret-named variable with any other value is dropped.
- **Encrypted books are stored decrypted** with their passphrase, page size
  and page count. A replay encrypts them again with the product's
  `encrypt_plaintext`, then checks that the product opens the result with
  the recorded passphrase, that the page size matches, that it has no more
  pages than the original (the product's encryption writes a compact book,
  so an original with free pages had a few more), and that the decrypted
  contents equal the recording; any mismatch fails the session.
  About 20 sessions covering unlock, rekey, wrong passphrase and crash
  recovery keep their original ciphertext.
- **Shared pages.** Files over 64 KiB are stored as 4 KiB content-addressed
  pages, so books that differ in a few pages share the rest.
- **Exact-path dedupe.** Of the sessions that replay identically twice, one
  is kept per coverage signature (the set of Rust basic blocks it executes);
  there is no fallback to a smaller greedy cover.
- **Deterministic ids.** Test builds seed SQLite's id generator from
  `KASSIBER_TEST_SQLITE_SEED`, mixed with the process arguments
  (`kassiber_store::seed_test_prng`). The recorder hands each process a fresh
  seed and keeps it, so random row ids and hashes over them replay
  identically. Product builds compile the hook away;
  `scripts/check-no-test-tape.sh` checks that the marker is absent from them,
  as it does for the network tape.
- **A shared wall clock.** Book rows hash their own `created_at` (a pair's
  `source_row_hash`), so the recorder preloads `kassiber-clock-shim` into
  each process. The process then reads `CLOCK_REALTIME` as the whole second
  the session began plus one microsecond per read, and in real time only
  after two seconds. A replay preloads the shim with the recorded base, so a
  short process reads the same times and writes the same hashes. The shim is
  a separate library that only the recorder and the replay load; no product
  build links it.
- **The recorded environment.** The product's own variables are kept,
  including the test switches (`KASSIBER_TEST_*`, `KASSIBER_RUST_TEST_*`).
  `PATH` is kept, so a test that hides a tool still hides it; a replay uses
  the recorded entries this host has. Secret-named variables are kept only
  with synthetic values.
- **Every module with a comparison is recorded,** the porters' modules too.
  The porters hand-port only the cases that hold kills; the replay keeps the
  other cases covered.

### Comparison rules

A replay compares its stdout, stderr, exit, every file in the state roots,
and every book row by row. Before comparing, both sides go through the same
rewrite. Each rule below covers a value that differs between two correct runs.

| Rule | Why it is safe |
| --- | --- |
| uuid4 and `uuid4().hex` values become `<gen:N>`, numbered by first appearance | random by construction. The numbering is a bijection: equal values share a number and different values get different ones, so a row pointing at the wrong id still differs. |
| uuid5 values become `<gen:N>` unless an input names them or both runs printed them | a uuid5 over a random uuid4 differs between runs. A uuid5 over inputs only is printed identically by both runs, so it stays literal. |
| `chat-…-<32 hex>` and `mcp-<32 hex>` request ids, 43-character url-safe `token`/`*_token` values, 22-character `upload_id`s and analysis `snapshot_id`s become `<gen:N>` | random request ids, document tokens and upload ids; a snapshot id hashes the analysis index's random instance id. Ids the test echoes back on stdin are translated to the replay's own. |
| The random part of a temporary name inside the session's roots (`kassiber-chat-uploads-XjMTS7Hk`, `tmpzxLZfBnQ`) becomes `<gen:N>` | `mkdtemp`-style suffixes: six to eight characters mixing letters and digits or cases. Names an input gave stay literal. |
| Values that came in through argv, the environment, stdin or the starting state are pinned | they are inputs and must stay literal. Stdin ids the process printed first are echoes, not inputs. |
| Clock readings inside the run's window (±5 s, the recording's window plus 10 minutes of lookback) become `<time>` | wall-clock reads. ISO, compact (`20261110T120000Z`) and epoch forms. |
| The same at whole hours up to 72 h away, or at 7, 14, 30, 60, 90, 180 or 365 days | expiries derived from the clock, such as a 90-day dismissal. Arbitrary offsets stay literal, so a fixture dated 10 days after the run compares exactly. |
| Both sides are normalized with the same spans: the replay's window, the recording's, and the window of any run that re-recorded the session (`rerecorded_us`) | a reading restored from the starting state appears on both sides; with one-sided spans, the hour-offset rule matched it whenever the replay ran at the same minutes and seconds, so equal state differed at random. A re-recorded session keeps its first window, which still covers the frozen SQLite clock, and adds the re-run's, which covers the real clock. |
| Never a clock reading: a number under a figure key (amount, msat, sat, price, fee, gain, loss, basis, proceeds, tax, cost, value, quantity, balance, total, fiat, income, rate) | an msat amount can look like a millisecond epoch. |
| `just now` and `N min ago` style ages become a placeholder | relative to the clock. |
| Durations (`*elapsed*`, `*duration*` keys or columns, `{"type":"duration_ms","value":N}` log records, fractional `seconds`/`user`/`sys`/`cpu`/`wall` and the same with a `*_` prefix) compare as present, not by value | they measure run time. Whole numbers under those keys stay literal. |
| JSON arrays of objects keyed by a generated id compare sorted by shape | their order follows random ids. |
| Rows of a book table compare in shape order, with generated values collapsed for the sort | their insertion order follows random ids; every row and value is still compared. |
| Daemon output is grouped by `request_id`, each request's records in order | concurrent requests interleave freely. |
| A book is dumped after applying its WAL; `-wal`, `-shm`, `-journal` and lock files are not compared | the WAL state is in the dump; ownership locks name the live process. |
| Artifact file names are compared by shape | they embed generated ids or times. |
| XLSX and ZIP artifacts compare member by member: XML members as text under these rules, binary members by hash | an export's `docProps` records its creation time; every cell still compares. |

No rule touches amounts, msat values, prices, tax figures, the dates of tax
events, or any stored column other than random ids and clock readings.
`crates/session-replay/tests/figures.rs` enforces this. It runs the tax-country
vectors, the tax-engine fixtures, and the journal and report vectors through
the rewrite, and fails if any figure field changes. A synthetic envelope
checks figures that look like clock readings. A third test checks that the
numbering stays a bijection.

### Dropped sessions

A session that does not replay identically twice is dropped. Each dropped
session is listed with its reason and the Rust functions only it covered, in
`tests/data/sessions/dropped.tsv` beside the recordings. A dropped session
that is the only coverage of a tax, export or stored-data path gets fixed,
not dropped.

The recording at the base, step 3:
- 301 modules (every comparison module, the porters' included; not the
  regtest integration lane) ran under the recorder: 58,077 processes, none
  refused for a passphrase. Five modules were recorded again after fixes
  (the year handoff and E 1kv exports, the hidden-home resolver, the pairing
  commands with a longer watchdog, the journal books under `target/r4-13`).
- Exact-path dedupe kept 24,180 sessions; the Rust-blessed export goldens,
  the modules recorded again and the export or stored-data sessions fixed
  after the first pass brought the candidates to 24,346.
- Every candidate replayed twice from the archived copy (the second time
  after the last product merges): 24,315 replay identically. 65 changed only
  because the legacy-migration port reworded refusals that named Python, or
  now migrates the book it refused (`pre_432_5d232097`); they were re-recorded
  with `--update` after review and replay identically twice.
- The archives are 9 gzip streams, about 340 MB, on the test-data branch.

Of the 119 dropped sessions that were the only coverage of an export or
stored-data function:
- 38 are Rust-blessed export goldens (below): 26 kept as recorded, the rest
  recorded again after the two Rust fixes the figure check found;
- 11 aborted in the first recording runs (the session clock froze Rust's own
  clock then, and age's scrypt calibration asked for 1 TiB; fixed), and
  their modules recorded again are kept;
- 49 replay since the normalizer fixes;
- 21 remain dropped, each a recorder limit:

| Module | Sessions | Why it cannot replay |
| --- | ---: | --- |
| `test_rust_operator_cli`, `test_rust_mcp` privacy fence | 10 | the outcome depends on a live operator broker's lease in another process |
| `test_daemon_smoke` | 4 | the daemon waits for, or races, a live broker owner in another process |
| `test_rust_backup_desktop` | 3 | a concurrent owner holds the project while the session runs |
| `test_rust_backup_services` recipient discovery | 2 | the encrypted archive's size depends on its embedded creation time |
| `test_rust_backup_install` | 1 | a pre-restore directory with a random name the session never prints |
| `test_rust_liquid_compatibility` | 1 | LWK asks for transactions in a different order each run (P1 serves it from a JSON-RPC fake instead) |

Other recorder limits, not coverage-unique: interactive TTY sessions
(`test_rust_document_cli`, `test_rust_chat_interrupt`: the recorder pipes
stdin, so the product takes its non-interactive path), tests that kill or
signal the process (`test_rust_journal_commands`' killed writers), the
closed-terminal tests of `test_rust_freshness_graph` and
`test_rust_journal_desktop`, and tests that inspect the process's open files
or owner locks (`test_rust_store_interop`, the MCP privacy fence). Their
behaviour is covered by the porters' Rust tests where they hold kills, or
listed in `dropped.tsv`.

### Rust-blessed export goldens (owner sign-off required)

These goldens record Rust's output where the shared cross-country export
design changed the layout from Python: titles, sheet and file names, column
headers, file kinds (a CSV bundle instead of one stacked CSV), PDF engine and
report language. Their tests failed under the recording because they assert
Python's layout. Every figure was compared with Python's output of the same
process at the same base: each session was replayed against this checkout's
Python reference (with the test no-egress guard) from the recorded starting
state, and both sides' JSON stdout, CSV, XLSX values, PDF text and changed
book rows were reduced to multisets of normalised figures (every number,
amount, msat, rate, fiat value, gain, tax figure, count and date), layout
aside. Book rows were compared row by row with ids, clock readings and
artifact hashes masked; all were equal. Both recording runs of each process
gave identical figures, so each row covers its two sessions. They need owner
sign-off.

"Figures" counts all figure tokens / distinct values, Rust then Python. A
one-sided value is a figure the other side's layout does not print; none is a
different value for the same quantity, except the two fixed below.

| Test (process) | Artifacts | Figures compared | Result |
| --- | --- | --- | --- |
| `test_cli_smoke` `test_07a_export_pdf_report` (`reports export-pdf`) | stdout JSON, full report PDF, 2 book rows | 230/71 vs 206/68 | equal; Rust only: its balance chart's BTC axis ticks (0.02, 0.04, 0.06) |
| `test_cli_smoke` `test_07ab_export_csv_and_xlsx_report` (`reports export-csv`) | stdout JSON, CSV bundle (ZIP, 10 tables and manifest) vs one stacked CSV, 2 book rows | 259/81 vs 286/121 | equal; per-section row counts equal; Rust only: `rows` 45 (bundle rows) vs Python's 74 (stacked lines); Python only: the `sat` float columns, balance-history period ends, the transactions table's fiat value and fiat fee columns |
| `test_cli_smoke` `test_07ac_export_xlsx_self_verifying` (`reports export-xlsx`) | stdout JSON, XLSX with Verify sheet, 2 book rows | 319/89 vs 341/127 | equal; the same kinds of one-sided values as 07ab |
| `test_cli_smoke` `test_07ad_verify_xlsx_reconciles_including_income` (`reports export-xlsx`) | stdout JSON, XLSX, 2 book rows | 212/49 vs 231/63 | equal; Rust `rows` 22 vs 51; Python only: `sat` columns, period ends |
| `test_cli_smoke` `test_07ae_transactions_export` (`transactions export`, XLSX) | stdout JSON, XLSX | 41/32 vs 41/32 | equal multisets |
| `test_cli_smoke` `test_07ae_transactions_export` (`transactions export`, CSV) | stdout JSON, CSV | 41/32 vs 41/32 | equal multisets |
| `test_report_year_handoff` `test_balances_use_the_year_end_rate` (`reports export-year-handoff`) | stdout JSON, year handoff XLSX (9 sheets), 2 book rows | recorded: 445/95 vs 471/105; fixed: 466/105 vs 471/105 | **differed, fixed:** the recording's Transactions sheet held its `sats` cells as the text `Decimal('50000000')` where Python writes 50000000. Fixed in Rust (whole satoshis as counts, msat fractions as numbers); afterwards the distinct figures are equal, only the year in Python's sheet titles repeats more. Re-record both sessions |
| `test_source_funds_cli` `test_austrian_eur_basic_source_funds_pdf_context` (`reports export-source-funds-pdf`) | stdout JSON, Austrian source-of-funds PDF (German) | 138/18 vs 146/24 | equal; Python only: its 100 % source-share figure, page numbers, digit-only fragments of wrapped txids |
| `test_source_funds_cli` `test_report_options_precision_masking_and_section_omission` (`reports export-source-funds-pdf`) | stdout JSON, PDF in sats with the recipient masked | 97/11 vs 104/12 | equal; Python only: page count. Rust prints no recipient line at all, so the recipient stays masked |
| `test_source_funds_cli` `test_source_funds_review_gates_snapshot_and_pdf` (`reports source-funds`) | stdout JSON, 2 book rows | 356/53 vs 356/53 | identical to Python |
| `test_source_funds_cli` `test_source_funds_review_gates_snapshot_and_pdf` (`reports export-source-funds-pdf`) | stdout JSON, PDF | 187/25 vs 194/28 | equal; Python only: 100 % share, page numbers |
| `test_daemon_smoke` `test_daemon_report_export_kinds_write_managed_files` (`daemon`) | NDJSON stdout (full and summary PDF exports, portfolio summary, balance history), both PDFs, 4 book rows | 490/95 vs 429/84 | equal; Rust only: chart ticks, the summary's E 1kv Kennzahl table (all 0,00), 365-day weighted holding period, the period-end cost basis 5.666,67 (Python's stdout: 5666.666666666667); Python only: chart ticks, the long-term-days sentinel 9223372036854775807. The recording has no after state (the failing test removed its directory first), so the PDFs come from a replay on this checkout's binary, whose stdout equals the recording byte for byte. Re-record both sessions |
| `test_review_regressions` `test_austrian_e1kv_report_exports_summary_csv_pdf_and_xlsx` (`reports austrian-e1kv`) | stdout JSON | 200/38 vs 200/38 | identical to Python |
| same (`reports austrian-tax-summary`) | stdout JSON | 200/38 vs 200/38 | identical to Python |
| same (`--format csv reports austrian-e1kv`) | CSV | 24/17 vs 24/17 | identical content; Rust writes the file 0600, Python 0644 |
| same (`reports export-austrian-e1kv-pdf`) | stdout JSON, E 1kv PDF (German), 2 book rows | recorded: 180/55 vs 186/46; fixed: 180/54 vs 186/46 | **differed, fixed:** the Disposals table ran 3 pt past the A4 edge, so its Kennzahl 174 read as "17". Fixed in Rust (a crowded table hyphenates its headers); afterwards Rust only: section numbers 1.1–4.5 and the year 2021 in the Alt/Neu note; Python only: its table of contents and page numbers. Re-record both sessions |
| `test_report_cross_check` `test_edits_in_the_tool_show_as_differences` (`reports cross-check`) | stdout JSON | 150/46 vs 150/46 | identical to Python |
| same (`--source Kraken`) | stdout JSON | 106/40 vs 106/40 | identical to Python |
| same (`--xlsx`) | stdout JSON, XLSX | 115/44 vs 115/44 | equal multisets |

Out of scope: the 11 dropped sessions of
`test_rust_shell_parity` `test_network_configuration_and_partition_export_through_hybrid`,
`test_rust_backup_desktop` `test_export_success_and_io_refusal_match_raw_jsonl`,
`test_rust_write_transaction_locks` `test_sqlcipher_writes_wait_for_a_held_writer`
and `test_rust_backup_services` (`test_export_manifest_matches_python_without_attachment_hashes`,
`test_sqlcipher_round_trips_both_directions_preserve_every_object_and_sidecar[True]`).
Their tests failed when an encrypted export (partition split or `backup
export` with a passphrase descriptor) aborted with `memory allocation of
1099511627776 bytes failed` (SIGABRT) in one recording run, not on export
layout.

#### Re-blessed for the 2026-11-11 export decisions

Two owner decisions change export bytes: the transactions CSV ends with a
`status` column (`draft` or `final` on every row), and every design CSV writes
BTC with eight places, more only for sub-satoshi msat (`0.05000000` instead
of `0.05000000000` or `0.05`); see [report design](../reference/report-design.md#csv).
Goldens were re-blessed through the Rust path only and each diff reviewed:

| Golden | How | What changed |
| --- | --- | --- |
| `crates/kassiber/tests/black_box/golden/ai_tools/mutating_tools.txt` | `KASSIBER_BLESS=1` | fixture `reports`, `ui.reports.export_csv` (full report bundle): the tool result's `bytes` and `report_snapshot.content_sha256`, the saved `filed_report_snapshots` row's content hash and the digest of that write block. All follow from the bundle's BTC cells; with the old cell text the golden passes unchanged |

No report vector under `crates/reports/tests/data` changed: the artifact
vectors compare the transactions CSV to Python's file by value (the new
`status` column is checked against Python's draft section title instead), and
no test-data golden holds design CSV text. The recorded sessions under
`tests/data/sessions` that export a transactions, capital-gains or bundle CSV
will differ in those bytes and need re-recording.

### Pathological stored values (owner decision 2026-11-11)

The legacy-migration inventory's G5 (NaN or infinity, integers beyond 64
bits, unpaired surrogates in stored text where a migration or the gate
converts them) is not ported. Kassiber refuses such a book as
`unsupported_schema_layout`, reason `unsupported_stored_value`, with a message
that names the value and does not mention Python, and writes nothing: a book
that still needs migrating is rehearsed on an in-memory copy first. See
[stored values Kassiber refuses](../reference/database-compatibility.md#stored-values-kassiber-refuses).

## Harnesses

The regtest and demo harnesses are Python:
- `tests/integration/`: 14,046 lines, 4,992 of them `regtest_demo.py`;
- `dev/regtest/*.py`: 4,736 lines;
- `scripts/integration-harness.sh`: 1,879 lines, using uv and `.venv`;
- `skills/kassiber-regtest-mode`.

They start the product through `cli_runner`, which can already target the
Rust binary (`KASSIBER_RUNTIME=rust`). But 15 harness files also import
`kassiber` in process:
- the demo's rate refresh, `open_db`, Silent Payments descriptor validation
  and fingerprints, `sanitize_wallet_segment`, `btc_to_msat` and the
  generic-ledger column list;
- Lightning backdating;
- exchange fixture pricing;
- the chain-observer oracle, which tests Python's observer through `bdkpython`
  and `lwk`.

- **If they stay Python for a while:** they still break when `kassiber/` goes.
  The demo-organization work and the required `demo-full` run would stop.
  Making them product-free means:
  - replace those imports with CLI calls, stdlib `sqlite3` reads (the demo
    books are plaintext), and copies of three small pure helpers;
  - run them with `python3 -I` and no uv environment.

  That is roughly 1–2 days and a few `demo-full` runs. The chain-observer
  oracle must be pointed at the Rust observer or retired in favour of the
  Rust crates' live tests.
- **Porting them to Rust (or shell):** about 20,000 lines across Compose
  control, RPC seeding, the scenario engine, the BTCPay, Lightning and Boltz
  overlays, and the assertions. That is weeks of work, with a high risk of
  stalling demo work while it happens.

## CI minutes

These are local test-seconds on this shared machine. CI hardware differs, so
read them as proportions.

| Per full CI run | Today | After |
| --- | ---: | ---: |
| 11 pytest lanes | 30,992 s | 0 |
| Tax shadow (re-runs the overlay) | 20,275 s | 0 |
| Custody shadow (same overlay) | 20,275 s | 0 |
| Rust-binary tests kept (black-box, plus the Rust share of the parity tests at most 20%) | in the lanes above | ~7,400 s |
| Setup in 29 Python jobs (setup-python, `uv sync --locked` building the binding, release builds of probes in 18) | ~90–230 min | 0 |

Savings, roughly 1,150–1,300 runner-minutes per full CI run:
- about 64,000 test-seconds, or 1,070 runner-minutes;
- the Python setup in those 29 jobs.

The Rust crate tests (about 1,400 s serial locally) are unchanged.

## Order of operations

Nothing is deleted before its golden or its Rust replacement exists. Each
step is green before the next.

0. **Baseline.** At the base:
   - tag the last Python commit (D4);
   - run every generator's `--check`, the shadow lanes and `check-rust` once,
     green;
   - refresh the custody recordings so they carry every Python stage.
1. **Move the assets** (table above). Build the binary before and after, and
   compare the skill references, `ai.tools.list` and the rates outputs byte for
   byte.
2. **Port the gate tooling:**
   - the mutation runner, whose `--check` must agree with today's on all 120
     catalogs, plus one sampled run per catalog;
   - preflight contracts as Rust tests;
   - the Rust `check-rust` replay;
   - `check_ledger_features`;
   - Rust credential tests on macOS and Windows.
3. **Freeze the parity goldens.** Switch the 204 `parity-compare` modules to
   compare Rust with frozen output instead of a live Python run. Replace the
   Python seeding in the 108 `parity-seeded` modules with CLI seeding or
   fixture books. The replacements must still kill the 606 mutants.
4. **Port or keep the black-box tests** (D1). Rerun the affected catalogs: no
   lost kills.
5. **Remove the Python runtime paths:**
   - the desktop fallbacks and stub; the Vite bridge switches to the binary;
   - the PyInstaller packaging and the Python CI jobs;
   - the harnesses become product-free (D2), proven by one `demo-full` under
     Rust with 0 main-book quarantines.
6. **Delete:** `kassiber/`, the binding, the Python-only tests, the
   generators, the repros, and `pyproject.toml`/`uv.lock` (or what D1 keeps).
   Prune `Cargo.lock` and `THIRD_PARTY_LICENSES.md`.
7. **Docs:**
   - 95 Markdown files mention Python (5,408 lines; 274 lines cite
     `kassiber/*.py`). The ones that state the architecture change in step 7:
     `AGENTS.md` ("delegate to the same Python core"), `kassiber/AGENTS.md`
     (moves to `crates/AGENTS.md`), `ui-tauri/AGENTS.md`, `CONTRIBUTING.md`
     (uv setup, the gate), `README.md`, `SECURITY.md`, plans 00 and 01,
     `daemon.md`, `ai.md`, `desktop.md`, `privacy-and-security.md`,
     `testing.md`, `TODO.md`, both `skills/`, and the in-product
     `troubleshooting.md`/`onboarding.md`. Those two embed uv hints;
     their text changes, their names and allowlist don't.
   - The port's working references (`rust-shell.md`, `rust-port*.md`, `r4-*`,
     `r6-*`) are marked historical rather than rewritten.

**Final gate:**
- `cargo fmt`, clippy, workspace tests;
- every mutation catalog with no lost kills;
- the chokepoint lint;
- the Rust `check-rust` over the custody recordings;
- the desktop build and its tests;
- one `demo-full` under Rust in its own Compose project with 0 main-book
  quarantines.

## Risks

- **Goldens become hand-blessed.** A regenerated golden can hide a wrong
  number. Mitigations: the CI refusal, the structural diff, owner sign-off for
  tax, stored data and exports, and the lock bump in the same PR.
- **Lost mutant kills while tests move.** The mutation runner is ported
  first (step 2), so every later step is measured by it.
- **Platform coverage.** macOS and Windows credential tests go away unless
  Rust ones replace them first.
- **The Vite bridge is Python-first on the base.** Previews break the day
  `kassiber/` goes unless step 5 lands first.
- **Plan 21 promised a test-only Python reference for a validation period
  after cutover.** This removal ends it (D5).

## Open decisions

**D1. What happens to the pytest files that drive the Rust binary?** That is
342 modules, 13,841 tests and the 606 pytest-only kills.

| Option | Cost | Risk |
| --- | --- | --- |
| (1) Port all to Rust integration tests | the highest: about 138,000 lines of pytest to rewrite after freezing. Even with debloat's duplicate removal, several worker-weeks. | translation slips. The mutation runner catches lost kills, but the removal PR waits for the whole port. |
| (2) Keep pytest as a dev-only tool for a while | still rewrite the Python seeding in 312 modules and switch 204 parity modules to goldens; keep uv, pytest and xdist, the egress guard and a shard list; 2–3 CI lanes | Python stays in the repo as test tooling, against the Rust-only decision; two test stacks to maintain |
| (3) Mix | port by hand the 64 modules that hold the 606 kills, plus the 30 black-box modules, to Rust. Replace the other parity modules with one generic Rust golden-replay runner over recorded sessions (argv, env, stdin, expected envelopes and DB manifest), recorded once at the base, where Rust and Python agree. Delete what debloat finds duplicated by crate tests. | the replay runner is new code; sessions recorded through example binaries need the recorder extended (86 modules). |

**Recommendation: (3).** It keeps every kill and needs no Python at the end.
It also converts the bulk of the parity tests mechanically instead of
rewriting 138,000 lines.

**D2. Harnesses.** I recommend making them product-free Python now (stdlib
only, run with `python3 -I`, no uv) so `demo-full` keeps working, and porting
them later as their own project. The alternative is a full Rust or shell
port before removal.

**D3. Release tooling** (about 1,700 lines). I recommend porting it to a
small binary in `crates/release`, which already implements the release
verification `release_manifest.py` imports. The alternative is keeping it as
stdlib-only CI scripts.

**D4. Archive.** I recommend a tag at the last Python commit
(`archive/python-backend`, as with `archive/general-ledger`). The 365 Rust
comment lines that cite `kassiber/*.py:line` then stay meaningful, with one
note in `crates/AGENTS.md`, instead of a mass rewrite. The crate names
`pyjson`, `pydecimal` and `pytext` stay: they name the CPython-exact formats
the stored bytes depend on.

**D5. Validation period.** Confirm that no test-only Python reference
remains after this PR, and that ad-hoc comparison means checking out the
tag.

**D6. Tax-country reference calculators.** I recommend deleting them and
keeping their reviewed vectors JSON as frozen goldens. The alternative is
keeping them as research scripts outside the gate.
