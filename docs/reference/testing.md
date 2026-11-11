# Testing

Kassiber's default gate stays fast and hermetic, while issue #312 adds opt-in
real-node lanes for proving wallet sync and demo books against disposable
regtest infrastructure.

## Dev Environment

Use the locked `uv` environment for local tests and daemon-backed desktop
development:

```bash
./scripts/bootstrap-dev-env.sh
export KASSIBER_PYTHON="$PWD/.venv/bin/python"
uv run --locked python -m pytest tests/test_wallet_descriptors.py -q
```

The bootstrap script runs `uv sync --locked`, which fails if `uv.lock` no longer
matches `pyproject.toml` and never rewrites the lock, then
verifies imports for the packages that most often go missing in ad-hoc shells
(`embit`, `sqlcipher3`, and the tax engine's `kassiber_tax` binding). On
Debian/Ubuntu it fails early with the required SQLCipher system package command
if the development headers are not available.

`uv sync --locked` builds the [tax engine](tax-engine.md)'s binding from
`tax-engine/python` with maturin, so a Rust toolchain must be on `PATH`. The
repository root is one Cargo workspace holding the engine and the backend
crates under `crates/`, with one `Cargo.lock` and one `target/`. `rust-toolchain.toml` pins one
Rust release for every crate, including `ui-tauri/src-tauri`; rustup selects it
inside the checkout, the bootstrap script installs it, and the quality gate
refuses any other `rustc`. CI and release workflows install it through
`.github/actions/setup-rust`. uv rebuilds the binding whenever a Rust source
file changes. The scripts set `MATURIN_NO_INSTALL_RUST=1` so a missing
toolchain fails instead of maturin downloading one. The crates' own tests run
from the root with `cargo test --workspace --locked`.

## Tax engine backends

Tests run the tax calculation on production. The reference is a frozen
oracle in `tests/tax_oracle/`: the phase-3 adapter with its normalizer and
Austrian rules, running on RP2. Set `KASSIBER_TEST_TAX_ENGINE` to change
that for a whole pytest run:

| Value | Meaning |
| --- | --- |
| unset | production |
| `shadow` | production, plus the oracle on every `build_ledger_state` call; any difference fails the call and the session (`tests/tax_engine_compare.py` masks only generated entry ids and the engine identity; failures compare after two listed normalizations: RP2 error class names map to `Engine*`, and RP2's temporary INI path prefix is dropped from messages). Production's request builder may refuse a shape a finalized projection cannot have only where `tests/tax_oracle/projection_range.py` finds the same shape (LD-5); such a refusal is not compared, and any one fails the session |
| `census` | production, and records which test modules reached the engine; the session fails if one of them is missing from the shadow overlay |
| `oracle` | every call on the oracle, to show the tests' expectations hold on RP2 |

Product code reads no such variable. Under `shadow` and `census`,
`tests/_child_hooks/sitecustomize.py` installs the same mode in Python child
processes, such as the `python -m kassiber` runs and daemons tests spawn,
once they load `kassiber.core.engines` (or `tests/tax_engine_compare.py`, when
a child imports it first); `conftest.py` puts that directory on
`PYTHONPATH` whenever a mode is set, also under `KASSIBER_INTEGRATION`. A
shadow difference fails the child's command. Children under `oracle`, and
children started with an environment of their own, run production. See
the [tax engine](tax-engine.md) for how the backends relate.

### Shadow overlay

The shadow overlay is the set of test modules that reach the tax engine:
those whose source, or the source of a `tests` module they import
(transitively), calls `process_journals`, `build_ledger_state`,
`CustodyJournalBuilder`, an engine class, or `journals process` through the
CLI or the daemon, plus the modules the census found through other paths
(`_SHADOW_EXTRA_MODULES` in `scripts/python_test_shards.py`). It sits across
the CI lanes instead of being one: each module keeps its lane.

```bash
uv run --locked python scripts/python_test_shards.py list-shadow [lane]
uv run --locked python scripts/python_test_shards.py run-shadow [lane] [--ci] [-- pytest args]
```

`run-shadow` runs the overlay, or its part in one lane, with
`KASSIBER_TEST_TAX_ENGINE=shadow`; `--ci` adds the lane's xdist workers.

Under `shadow` and `census`, every production call, in the test process, its
xdist workers, and its child processes, appends a route record (test module,
capability, route, and whether it returned, raised, or differed) to a file in
the session's temporary directory; nothing from the book is recorded. Every
book with projected rows runs on the engine. At the end the session prints,
per engine capability the book exercises (`generic`, `generic_carry`, `at`,
`at_swap`, as `classify` in `tests/tax_engine_compare.py` reads them from the
request), how many calls reached the v2 engine, plus the calls that ended
before it as `not routed`: a projection without rows counts under its
country's capability (`generic` or `at`), while raw rows and books the
request builder refuses (configuration failures and LD-5 shapes) count as
`unclassified`. It also prints the number of LD-5 refusals. Shadow fails the
session on any recorded difference, even one a caller caught, on a
capability that saw calls but none on the engine, on a record of any other
route, and on any LD-5 refusal, since the suite's books are finalized
projections. A
test marked `stubs_tax_engine` replaces parts of the production engine, so
shadow runs its calls on production without comparing them and lists them in
the summary.
`tests/test_tax_shadow_harness.py` pins which tests carry that marker. It
also proves the child path with the test-only
`KASSIBER_TEST_TAX_SHADOW_INJECT_DIFFERENCE=1`, which alters production's
outcome before the comparison: a `journals process` child must fail with the
shadow error.

### Projection goldens

`tax-engine/core/tests/data/ledger_fixtures.json` stores one case per line:
a profile and finalized projection (`inputs`), the oracle's outcome on RP2
(`oracle`), the outcome production must give (`expected`), in the tagged
codec of `tests/tax_ledger_codec.py`, the engine request production's
builder makes of the inputs (`request`, `null` when the builder fails), and
the engine's response to it (`response`, `null` only where `request` is).
`expected` equals
`oracle` except where `listed_difference` names one: `LD-4` (RP2 error
classes), `LD-3` (RP2's configuration prefix too), or `LD-5`: the case has
the one shape `invariant` names, which a finalized projection cannot have,
and `expected` is the builder's refusal. The cases are the named
shapes, one LD-5 case per such shape, and seeded books of
`tests/tax_ledger_generator.py`'s `corpus()`.

```bash
uv run --locked python tax-engine/scripts/gen_ledger_fixtures.py [--accept CASE ...]
```

The generator certifies each case before writing: the inputs survive the
codec and stay inside what a finalized projection can be (an LD-5 case has
exactly its one shape), the oracle reaches the shape the case is named for,
`expected` follows from `oracle`, the builder gives the same request on
the stored and the original inputs or fails with `expected`, and production's
reader renders a stored response into `expected` exactly. It refuses to
change or drop a stored case's inputs, outcomes, listed difference, or
invariant unless that case is named with `--accept`; review the change before
accepting it. Adding cases, or a changed request or response, needs no
`--accept`; `tests/test_tax_ledger_contract.py` checks the builder against
the stored requests and the engine and the reader against the stored
responses (the engine's bytes, and the reader's outcome against `expected`),
and `tax-engine/core/tests/ledger_fixtures.rs` checks that the engine reads
each request and replays each response. Its output depends only on the code,
so two runs are byte-equal. `tests/test_tax_ledger_goldens.py` checks that
the file is the current corpus and what two fresh generator runs write,
production against `expected`, and the live oracle against `oracle` (this
catches a change in a product module the oracle shares), and runs 200 seeded
generic and 200 Austrian books of the random generator against the oracle.
One book in three uses the generator's `terms`: a generic holding
period of 1, 30, or 730 days instead of 365, or an Austrian method beyond
the three the stored seed books draw (`moving_average`, `lifo`, `lofo`). The
named cases cover the same terms: sales just before, at, and just after
such a holding period ends, and Austrian books under each of those methods.
Without RP2 its oracle checks skip, except under `KASSIBER_CI`, which CI
sets, where they fail.

### Stage comparison

`tests/test_tax_ledger_prepare.py` compares the engine's `prepare` stages with
the frozen oracle's own intermediates, field by field, so a difference shows
the stage it starts in before whole-ledger goldens can. Per asset that is the
normalized stage (the events, moves, their order, their Austrian regimes,
elections, and regime flows, normalize's quarantines, and the contamination
time) and the prepared stage (the lot engine's entries as v1 entry documents
with their notes and text, the walk's gate quarantines, and the transfer audit
with the legs' description) of the final pass, against what the oracle's
`_prepare_assets` builds on RP2, each entry its walk constructs recorded as
the v1 entry document of its arguments (`tax-engine/scripts/v1_request.py`)
with RP2's `str(transaction)` as its text; the generic Bitcoin rail carry
before it (the rows it priced, with their value and rate, the ids its blocked
pairs withhold, their quarantines, and its passes), against the oracle's
`_apply_generic_bitcoin_rail_carry_values` run with the same configuration;
and the Austrian swap-link selection over the first pass (the rows it links,
the ids its blocked pairs leave out of the second pass, and their
quarantines), against the oracle's `_select_at_cross_asset_swap_links`.
Quarantines and audit rows go through the production reader. A failed pass is
compared by the stages it reached and its error. It covers every golden book,
the first 200 seeded generic and Austrian books without a carrying-value
cross-asset pair, the first 200 seeded Austrian books with one, the first 100
seeded generic books with the rail carry, and edge books neither reaches
(non-finite stored rates and values, overflow, subnormal prices, missing
labels, an empty or unparsed timestamp, blocked groups that share
transactions, a group drained between its moves, one instant spelled two ways,
a holder label with spaces; on Austrian books, times the regime tracker reads
otherwise than RP2 or cannot read, the tracker's order at one instant, a held
move leg, a move fee across both regimes, overrides of the tracker's regimes,
wallet ids it merges, and swap pairs whose out leg is a move's, whose leg is
not a row, without an id, whose reason only the pair's transaction id finds,
whose leg's transaction has two reasons, with an unconfirmed leg, sharing a
row, and linked on a regime the second pass changes; and rail pairs whose
source has no basis and no reason, whose source has no amount, and whose
coarse in leg the carry prices). Where an entry with an infinite value fails
an earlier constructor check too, the engine fails at RP2's check. A last
check runs the whole oracle and records its own normalize and walk calls in
the carry's passes and both later ones, so the test's mirror of those calls
cannot drift.

### Comparing a real book

Owners can compare production with the oracle on their own book, locally:

```bash
uv run --locked python -m tests.tax_oracle.compare_book --data-root <copy> \
    [--db-passphrase-fd <fd>] [--profile <id or label>]
```

`<copy>` must be a disposable copy of a data directory, because opening it may
migrate its schema. The tool refuses Kassiber's default and legacy data roots,
anything inside the default state root or a registered project, a directory
without a database, and a database or SQLite side file that is a symbolic or
hard link, so it cannot reach a live book. Copy the files rather than linking
them. It reads an encrypted copy's passphrase only
from the file descriptor, never from the keyring, a remembered unlock, or the
operator broker. Once the copy is open its connection is read-only; it builds
each profile's custody projection, runs production and the oracle on it, and
prints the differences to stdout. It makes no network connection.

### Mutation checks

`tax-engine/scripts/mutations.json` names, for each tax rule a test pins, one
edit that breaks the rule and the tests that must then fail. Most of those
tests run `build_ledger_state`; the transfer-identity entries pin the helpers
in `kassiber.transfers` directly. Every book runs on the engine, so a
rule's entry edits the engine (cargo filters, including the replay of the
stored responses), the request builder, or the Python reader; the phase-3
adapter and its normalizer are left only as the frozen oracle. Three entries
edit the shadow summary's capability classifier in
`tests/tax_engine_compare.py`:

Every catalog (`crates/*/scripts/*mutations*.json` and this one) belongs to
a suite in `crates/mutation-check/catalogs.json`, which says how its cargo
tests run: packages, features, targets and environment. The Rust runner
validates or runs them:

```bash
cargo run -p kassiber-mutation-check -- check            # every entry applies, every catalog has a suite
cargo run -p kassiber-mutation-check -- list             # suites and entry counts
cargo run -p kassiber-mutation-check -- run SUITE [--only ID ...] [--keep]
```

`run` copies the working tree, plus the fetched test data the lock pins, to
`target/mutation-scratch/<suite>`, runs each named test there once
unmutated, then applies one edit at a time and fails if the mutated tree does
not build, a named test still passes, or a filter selects no test. Every
suite builds into the one shared `target/scratch-build`. Full runs are too
slow for CI; run a suite when its catalog or the code it edits changes. CI and
the quality gate run `check`.

Entries killed only by a pytest test, or that edit a Python file, are legacy
while Python is being removed ([plan 22](../plan/22-python-removal.md)):
`check` validates and counts them, `check --strict` refuses them, and `run`
refuses to start a suite that selects one. Until each gets a Rust killer, the
retired Python runner (`tax-engine/scripts/mutation_check.py` and the
per-crate wrappers) still runs them.

## Product subprocesses and recorded sessions

[Plan 21](../plan/21-rust-backend.md) verifies a Rust backend against the
Python reference at the process boundary. Two test-only variables serve it;
product code reads neither.

### The candidate switch

Every test, integration lane, and regtest seed that runs the CLI or
`kassiber daemon` as a child builds the command with `tests/cli_runner.py`
(`run`, `popen`, `argv`); `tests/test_cli_runner.py` fails on a new
`python -m kassiber` construction anywhere under `tests/`, `dev/`, or
`scripts/`, except the two scripts it lists with their reason. The working
directory defaults to the repository root.

| `KASSIBER_TEST_CLI` | The command |
| --- | --- |
| unset | `python -m kassiber` from this checkout |
| `/path/to/kassiber` | that executable |
| `["/path/to/kassiber", "--flag"]` | that argv prefix, without a shell |

A value that names no runnable executable fails the call. An argument
`cli_runner.InputFd(b"...")` becomes the number of a pipe holding those
bytes, for `--db-passphrase-fd` and its siblings; it carries synthetic test
values only. Call sites that pin the Python reference itself pass
`reference=True`, which ignores the switch; only they may pass interpreter
options (the tax shadow harness does both).
`tests/_child_hooks/` and with it the no-egress guard reach Python children
only. A Rust candidate carries the guard only when built with
`--features test-harness`; a release build exits 78 when
`KASSIBER_TEST_NO_EGRESS` is armed, so tests that start one must leave it
unset (`tests/integration/runtime_parity.py` probes this).

### Hybrid Rust candidate

While the Rust port is incomplete, `tests/rust_port/hybrid_cli.py` serves
each call from Rust when Rust implements the command, and from this
checkout's Python otherwise, on the same book:

```bash
cargo build -p kassiber --features test-harness --locked
KASSIBER_TEST_CLI="[\"$PWD/.venv/bin/python\", \"$PWD/tests/rust_port/hybrid_cli.py\", \"--hybrid-rust-bin=$PWD/target/debug/kassiber\", \"--hybrid-log=/tmp/served.jsonl\"]" \
  uv run --locked python -m pytest -q tests/test_<module>.py
```

Use absolute paths: some tests run the CLI from another directory. Each
`KASSIBER_HYBRID_*` setting can also come from the environment, which wins,
but tests that scrub `KASSIBER_*` variables from a child only keep settings
given as `--hybrid-<name>=<value>` on the candidate command line.

Each call first runs the binary with `KASSIBER_RUST_TEST_PROBE=1` (test
builds only). It parses the command line, prints the command path, and exits
0 when Rust implements the command or 3 when it does not, touching nothing.
A call carrying a global flag Rust still refuses, such as `--diagnostics-out`,
probes as not implemented.
Usage errors also run in Rust. `daemon` stays in Python unless
`KASSIBER_HYBRID_RUST_DAEMON=1`, because one session mixes kinds.

The candidate `exec`s the chosen side, so stdin and inherited descriptors
such as `--db-passphrase-fd` reach it unchanged. `KASSIBER_HYBRID_LOG` gets
one `{"command", "served_by"}` line per call. A slice proves its commands
ran in Rust from that log. `tests/test_rust_hybrid_candidate.py` pins the
routing.

Prebuilt test binaries must match the sources. `tests/rust_fixture_binary.py`
reads the dep-info file cargo writes beside each binary (`<binary>.d`) and
refuses a binary older than any workspace source it was built from or than
`Cargo.lock`, or one without dep-info. `tests/conftest.py` stops the run when
a binary named by an environment variable such as `KASSIBER_RUST_BIN` is
stale; examples found beside `KASSIBER_RUST_BIN` or under `target/` are
checked where a test runs them, and the refusal names the build to run.

Fresh sources do not prove the right features. A workspace `cargo test
--features …` unifies test features across every crate and relinks shared
binaries and examples in the same target directory, and dep-info stays fresh.
After building the test binaries, record the build beside them:
`python -m tests.rust_fixture_binary record --features "<cargo args>" BINARY…`
writes `<binary>.build.json`. A recorded binary that a later cargo invocation
relinked is refused, naming the recorded build. Binaries without a record keep
the dep-info check only. Run workspace tests in their own `CARGO_TARGET_DIR`
when test binaries share the default one.

The unregistered Bitcoin wallet-sync service has a separate guarded driver,
`cargo build -p kassiber-wallets --example sync_fixture --features test-harness
--offline --locked`. `tests/test_rust_wallet_sync_service.py` compares its full
`wallets.sync` envelope and every logical book row with Python's complete
`handlers.sync_wallet`, including both wallet selection modes, the fixture and
restart corpus, reviewed books and process consent/cancellation/no-egress fences.
This is service-boundary evidence; it does not establish CLI/daemon registration
or native hybrid served-by counts. The serial Rust-enabled lane builds the
driver explicitly. Keep its books and temporary files under a disposable root
and retain the socket guards.

The guarded `freshness_sync_fixture` additionally executes admitted Bitcoin
freshness jobs. `tests/test_rust_freshness_native.py` compares terminal
envelopes and every logical table with Python's daemon freshness coordinator
on disposable Electrum/Esplora servers. Identity and clock seams are fixed;
elapsed time is frozen in the Python reference rather than removed from the
comparison. This proves internal execution, not live daemon progress, graph
handoff or product routing. The separate `freshness_fixture` drives scripted
handlers for deterministic retry, cancellation, removal, checkpoint and
redaction cases in `tests/test_rust_freshness_execution.py`. Both fixtures
require `--features test-harness` and preserve account HOME.

The guarded `graph_fixture` drives native post-sync graph completion.
`tests/test_rust_freshness_graph.py` compares count-only completion payloads and
every normalized table against Python on disposable Electrum/Esplora servers.
The differential driver freezes the pass-admission clock in both implementations
so shared-host load cannot change cursor/budget expectations. Production uses
the real monotonic clock; lookup deadlines and backend timeouts remain real in
the driver too.
It also exercises the explicitly started daemon worker, unsolicited events,
request/lifecycle cancellation and SQLCipher session cleanup. Graph grants still
require a flushed successful-sync response; passive paths and recovered jobs
cannot start lookup. The graph mutation catalogue is
`crates/wallets/scripts/freshness_graph_mutations.json`, run through the same
`freshness_mutation_check.py` driver with `--mutations`. The driver rejects skips,
empty selections and build failures, and removes its scratch/build trees.
The reference mismatch repro in that module deliberately records Python's
acceptance of incorrect Electrum transaction bytes; native decoding refuses it.
This suite runs in the Rust-enabled serial daemon shard and does not establish
product routing or served-by counts.

For plaintext file-import scenarios, `KASSIBER_HYBRID_IMPORT_ORACLE=1` also
replays each native CSV/Phoenix call through Python's core on an in-memory
backup. Test-only identity/clock inputs keep generated rows comparable. Every
schema object, table value and SQLite storage class, and the returned payload,
must match. This requires an initialized disposable book with explicit
`--data-root`; it is test tooling only. SQLCipher imports use the separate
real-store wire/state scenarios.

For tests that migrate/encrypt books, set `KASSIBER_RUST_TEST_OWNER_ROOT` to
an existing disposable directory. The hybrid launcher's Python fallback uses
the same isolated ownership namespace as the native test build. This injection
lives only in test tooling; production ownership still uses the stable UID
account-home namespace, independently of `HOME` and XDG settings.

### Recording

`KASSIBER_TEST_RECORD_SESSIONS=<dir>` records every invocation the runner
starts, without changing what the test sees: the child gets pipes the
recorder pumps, and the test gets pipes of its own. Use a directory outside
the checkout or the ignored `.session-recordings/`:

```bash
KASSIBER_TEST_RECORD_SESSIONS=$PWD/.session-recordings \
  uv run --locked python -m pytest tests/test_cli_smoke.py -n 4 --dist loadscope -q
```

A session is everything run against one state root in the system temporary
directory: for a data root named `data`, its `data`, `config`, `exports`, and
`attachments` directories (the data-root layout); otherwise the data root, or
a temporary `HOME` and the other state directories the environment points
at. Each session directory holds a manifest (format, recorder version,
comparison policy, reference commit and dirty flag, schema fingerprint, tax
engine and product version, the tests, the roots, and the states), the events
in their global order, content-addressed blobs, and state trees. Events are
each process's argv, working directory, environment subset, stdin and
standard stream kinds, the fixture files it names in argv, stdin, or a
`*_FILE` variable (with their content, or a note that an earlier step
produced them), its process id, stdout and stderr, signals, and exit status
with the files it wrote outside the roots. Every stdin write, signal, and
spawn records how many lines each live process had printed by then, which is
how a daemon's control-channel exchanges, streaming records, cancellation,
shutdown, and restarts are ordered. The roots are captured unchanged when
the session starts and whenever it falls idle. A change the test makes itself
between processes, such as seeding through `open_db`, starts a new segment
from what the test left.

The environment subset is `KASSIBER_*`, `SATBOOKS_*`, the state-root
variables (`HOME`, `XDG_*`, `LOCALAPPDATA`, `USERPROFILE`, `APPDATA`), the
packaging variables, and locale, terminal, and time-zone variables
(`tests/session_harness/policy.py`). A path outside the temporary directory
and the checkout, such as the real home, is recorded as inherited, not by
value; `KASSIBER_TEST_NO_EGRESS` is kept for the replay's guard.

The recorder writes only under its directory and refuses, keeping only the
reason in `refusals.jsonl`: a root outside the temporary directory or a
command without one, a named path outside it and the checkout, a
secret-named product variable that is set, stdin a call site marks with
`secret_input=True`, inherited descriptors other than `InputFd`,
interpreter options, a recording while `KASSIBER_TEST_CLI` is set, and any
session in which a value of a secret-named variable from the test's
environment appears. Recordings stay out of git; a committed recording must
hold synthetic data only.

### Replay and comparison

```bash
uv run --locked python -m tests.session_harness replay .session-recordings \
  [--candidate /path/to/kassiber] [--jobs 6] [--diagnose] [--json report.json]
```

Each session starts from a fresh copy of its starting state, at the recorded
paths when the replay can claim them and at fresh temporary paths otherwise,
and receives the recorded inputs in the recorded order. The candidate is the
Python reference unless `--candidate` or `KASSIBER_TEST_CLI` names another;
the reference replays under the suite's no-egress guard. The comparison
policy (`kassiber-session-compare/1`):

- JSON is compared structurally: key order, array order, and value types
  count, the spelling of a number (`1e-05`, `1e-5`) does not; the layout
  (whitespace, separators, ASCII escaping, the trailing newline) is checked
  separately.
- Each database is dumped as its pragmas, every schema object, and every
  table's rows in primary-key order, with no column masked. An encrypted
  book is opened with the passphrases the session fed in (`InputFd`
  descriptors and daemon `passphrase_secret` fields), under the contract's
  SQLCipher parameters. Other files are compared by content
  where they are text, zip archives member by member, and by size and
  sha256 otherwise; permissions count.
- Values a run generates compare through bindings: a uuid4 (or a whole
  `uuid4().hex` value), or a wall-clock date, time, or epoch value inside the
  run's own time window, binds on first sight to the candidate's value at the
  same position, and every later occurrence must map the same way in both
  directions, in the same order. Values that came in through argv, stdin, a
  fixture, or a restored state are bound to themselves first. Ids the
  candidate printed are translated into later inputs, as a test echoing them
  would do. The `request_id` of a daemon `supervisor.*` record, the recorded
  processes' ids, and relocated paths are mapped the same way.
- Nothing else maps: hashes, uuid5 ids, and derived timestamps compare
  exactly.

Two Python runs differ legitimately in their clock readings and random ids
until plan 21's clock and entropy injection exists, so a strict replay of
most sessions still fails; the injection, not more masking, is the fix.
`--diagnose` adds comparison lanes that never decide acceptance: `clock`
keeps only the order of clock readings (including readings shifted by whole
hours, such as expiries) and ignores the creation stamps of PDFs, `entropy`
also maps any uuid and long hex run, and `order` compares
arrays, JSON stored as text, and rows with generated keys in a canonical
order. The report names the first lane in which each session is clean.
`tests/test_cli_session_replay.py` proves both: a fresh Python run of a
recorded CLI session, of a daemon session with a control-channel exchange,
and of a session that encrypts its book through descriptors is clean once
relaxed, and every lane fails on an altered output amount, exit
status, referenced id, stored amount, link between rows, and timestamp, on an
error a caught-error path reports, on a stored amount a grandchild process
changes, and on a daemon response (`tests/session_harness/inject_candidate.py`).

Limits that matter for the Rust stage: loopback services a test serves from
its own process (AI provider and backend fakes) and mocks inside the test
process do not exist on replay; a test's direct writes while a recorded
daemon is live are not replayed (the recording notes when the test opens a
session's database then); files the product reads outside the roots
without naming them, such as the machine-wide project catalog, are neither
recorded nor restored; and a wrapper candidate's own process id is what gets
mapped, not its child's.

### Frozen routing replays

`cargo test -p kassiber --features test-harness --test routing_ports` holds
the Rust replacements for the pytest parity modules whose tests were the only
killers of `crates/wallets/scripts/routing_mutations.json` entries (plan 22,
D1 option 3). Each test is named after the pytest test it replaces and
replays the `kassiber` processes that test started, from a golden under
`crates/kassiber/tests/routing_ports/goldens/` recorded once where Rust and
the Python reference agreed. A session lists:

- `setup`: what brings the state from the previous session's end to this
  one's start (books as row patches against the current-schema manifest or
  the last state, other files whole);
- `args`, `env`, `cwd`, `stdin` and `tape`: the inputs; the tape is
  `kassiber-net`'s test-observer tape of the fake servers' replies (stdin
  chunks also wait for the reply to the previous request);
- `expect`: exit status, stdout, stderr, and every row and file the process
  changed under its roots.

Paths are templated (`{tmp}`, `{repo}`). Generated uuids and 32-digit hex
ids compare by order of first appearance and are restored to the recorded
values after each session, so later sessions and orderings by id see the
recorded state; ISO timestamps inside the run's window compare as `<now>`;
wall-clock `elapsed_ms` is 0. Values over 16 KiB, and table changes of more
than 2,000 rows, compare by length or count and an FNV-1a 64 digest. A test
may mask further clock-derived numbers (`age_seconds`). A client whose
request order varies between runs is answered by a loopback JSON-RPC server
built from the tape (`replay_served`, `kassiber-net`'s `FakeJsonRpc`) instead
of the byte tape.

The replays run on strict tapes (`KASSIBER_TEST_NET_TAPE=replay-strict:`):
a request no recording began with, a connect with no recording left, or a
connection writing past its recording's bytes is noted in
`<tape>.misses`, and any note fails the replay even when the product
swallows the failed request. A daemon session sends each stdin chunk once
the output reached what the recording had seen, or once every earlier
request has its final reply.

After a reviewed behaviour change, `KASSIBER_GOLDEN_WRITE=1` rewrites the
`expect` blocks from the current binary instead of comparing; it refuses to
run under `CI`. Diff the goldens and explain every change; tax numbers,
stored data and exports need the owner's sign-off.

## Tiers

| Tier | Command | Docker | Purpose |
| --- | --- | --- | --- |
| FAST | `./scripts/integration-harness.sh fast` | no | Replays recorded regtest tapes through the real sync adapter, import, journal, report, and XLSX export path with `KASSIBER_NO_EGRESS=1`. Includes a baseline watch-only tape and an edge-case tape (multi-address wallet, immature vs. mature coinbase, dust, RBF-replaced conflict pair, same-wallet self-spend, mempool-pending receipt). The same tapes are also served by a strict loopback Core RPC fake to the real CLI (`tests/integration/test_regtest_tape_cli.py`), so the replay runs on whichever runtime `KASSIBER_RUNTIME` selects. |
| SLOW | `./scripts/integration-harness.sh bitcoin-core` | yes, unless reusing a node | Starts or reuses the regtest Compose stack (Bitcoin Core, Elements, Bitcoin Fulcrum, plus local mempool/esplora-compatible loopback endpoints), creates real wallets and transactions (including coinbase maturity and a watched receive), drives the Core RPC sync/pricing/journal/report/export smoke, and compares a real Fulcrum/Electrum address-wallet sync against Core RPC for receipt, spend, incremental, and no-op sync parity. |
| CHAIN OBSERVERS | `./scripts/integration-harness.sh chain-observers [all\|bitcoin\|liquid]` | yes, unless reusing nodes | Extends the same disposable Compose stack with an independent Core/Elements truth oracle. It records bounded mode-0600 manifests for descriptor/address families, receive/change and gap ownership, mempool/confirmation/RBF/reorg/restart transitions, Bitcoin and Liquid UTXOs, policy/issued-asset values, and protocol endpoint heights. It also proves observer state commits and rollback/restart behavior inside a temporary SQLCipher book with no dependency sidecar store. |
| DEMO | `./scripts/integration-harness.sh demo-full` | yes, unless reusing a node | Builds the checked-in `full-accounting-v1` scenario: thirteen Kassiber wallets including 8–12-address Bitcoin wallets, legacy/nested-SegWit/bech32/taproot rotation targets, a Silent Payments wallet, a mining wallet, and descriptor-backed Liquid wallets synced from real `elementsd` transactions through the local Liquid Electrum endpoint; monthly EUR-denominated receipts, payments, payroll, rent, cloud, and other expenses converted through the bundled Kraken BTC/EUR rate for each cycle; a deterministic jittered clock with quiet and doubled activity months; era-sensitive fees, rare heavy-tail receipts, 2–3 outbound payments in busy regimes, five external counterparty clusters, threshold-driven working-capital/merchant/cold-reserve transfers, near-full Bitcoin rotations with an explicit bounded legacy reserve where required, a real Liquid treasury migration with a delayed-receipt catch-up sweep, three policy-selected UTXO consolidation windows, and assertions that deprecated wallets retain at most one fee-reserve UTXO; 2021–22 pool payouts plus one solo coinbase, scheduled batched/dust/PayJoin/RBF edge cases, closed and still-open collateralized loans, ownership-derived fan-out self-transfer matching, a mempool-pending receipt, local Bitcoin/Liquid Electrum and mempool-compatible backend rows, swap/peg bridge pairs, journals, reports, and transaction exports. The persistent `demo-up` variant also starts the Core Lightning and BTCPay overlays by default, seeding merchant Lightning, BTCPay connections, paid BTCPay invoice/payment-request examples, and reviewed commercial reconciliation unless explicitly disabled. |
| SILENT PAYMENTS | `./scripts/integration-harness.sh silent-payments` | yes | Starts the regtest Compose stack with the `silent-payments` profile, which builds/runs Sparrow Frigate against Bitcoin Core v30 and Fulcrum, waits until Frigate advertises `silent_payments: [0]` through `server.features`, then runs Kassiber's Silent Payments sync tests. Override the cold-start wait with `KASSIBER_REGTEST_FRIGATE_WAIT_SECONDS` if the local Frigate index is slow. |
| BOLTZ | `./scripts/integration-harness.sh boltz-liquid` | yes, upstream Boltz stack | Starts or reuses Boltz's official [`BoltzExchange/regtest`](https://github.com/BoltzExchange/regtest) Docker environment, probes the local Boltz API for Liquid-capable submarine, reverse, and BTC -> L-BTC chain-swap pairs, executes a Liquid on-chain payment plus an L-BTC -> BTC Lightning submarine swap, persists the paid invoice through Kassiber's native LND adapter boundary, records the Liquid policy-asset id and Elements regtest scope, and explicitly reviews the resulting strong provider-hash candidate while the plain Liquid payment stays unpaired. An outbound submarine lockup has no owned claim witness, so provider metadata plus a native invoice is intentionally not called exact. Optional `KASSIBER_BOLTZ_V2_EVIDENCE=/path/to/evidence.json` adds real Boltz wallet/client/provider v2 chain/reverse/refund evidence rows. Exact `provider_swap_id` assertions require a unique 1:1 provider key, canonical send/receive route txids, explicit chain/network scope and Liquid consensus asset id, plus integer-msat principals covering both complete rows; incomplete evidence must stay strong/manual. |
| BTCPAY | `./scripts/integration-harness.sh btcpay` | yes | Starts the standard Bitcoin regtest stack plus a BTCPay overlay (Postgres, NBXplorer, BTCPay Server) wired to the same disposable `bitcoind`, creates a first admin user, creates or reuses a test store, generates a BTC on-chain store wallet, creates and pays a realistic invoice mix from the regtest Core wallet, confirms it, syncs BTCPay wallet history into a temporary Kassiber book, syncs invoice provenance, and writes a local seed JSON containing the store id, disposable API key, invoice ids, scenarios, and payment txids. |
| BTCPAY-BARK | `./scripts/integration-harness.sh btcpay-bark` | yes | Brings up a regtest Ark server and BTCPay with the Bark plugin in its own Compose project, drives every wallet-history row kind, records the Greenfield answers to a scratch directory, and tears the stack down. Needs the plugin package in `$KASSIBER_BARK_PLUGIN_CACHE`. |
| LIGHTNING | `./scripts/integration-harness.sh lightning-business` | yes, Kassiber stack + CLN/LND overlay | Starts the existing regtest Compose stack plus `dev/regtest/compose.lightning.yml` with four pinned Core Lightning nodes and one LND backup merchant node. A seeded sim-ln-inspired business plan drives mainchain top-ups/withdrawals, merchant invoices, supplier payments, routed forwarding activity, LND backup receipts/outbound payments, an expired quote, and an intentionally failed oversized payment, opens private CLN merchant -> LND backup and LND backup -> CLN router channels, then verifies Kassiber through `wallets sync`, `ui.connections.node.snapshot` for both node implementations, `reports lightning-profitability`, and `export-lightning-profitability-csv`. |

The slow lane is opt-in with `KASSIBER_INTEGRATION=1`; normal unit gates do not
start Docker. To reuse an existing regtest node instead of Compose, set an
explicit Core URL and matching disposable RPC credentials:

```bash
export KASSIBER_REGTEST_CORE_URL=http://127.0.0.1:18443
export KASSIBER_REGTEST_RPC_USER=kassiber
export KASSIBER_REGTEST_RPC_PASSWORD=...
./scripts/integration-harness.sh bitcoin-core
# Or run only the Fulcrum/Electrum parity slice:
./scripts/integration-harness.sh bitcoin-electrum
```

### Runtime selection

Every lane starts the product through `tests/cli_runner.py`.
`KASSIBER_RUNTIME=rust` points those children at the binary
`KASSIBER_RUST_BIN` names (default `target/release/kassiber`) by exporting
`KASSIBER_TEST_CLI`; unset or `python` keeps `python -m kassiber`, which CI
uses. The harness's own scenario, seeding and probe helpers stay Python,
including the few that open the book in-process (demo rate refresh,
Lightning backdating, exchange fixture pricing).

```bash
cargo build -p kassiber --release
KASSIBER_RUNTIME=rust ./scripts/integration-harness.sh fast
```

Every lane also points `KASSIBER_OFFLINE_PREFERENCE_FILE`, the agent-access
and update files at `target/integration-harness/preferences/` unless they are
already set: the product otherwise reads the account's default state root
even with `--data-root`.

`KASSIBER_RUNTIME=shadow` compares the runtimes inside a live lane, where
each run creates new chain activity and two runs never see the same chain.
Python drives the lane; `tests/integration/runtime_shadow.py` copies the
book before every CLI step, runs the step on Python and then on Rust from
the copy against the same services, compares envelopes and every manifest
table, and appends one line with both timings to
`$KASSIBER_RUNTIME_SHADOW_DIR/steps.jsonl` (default under
`target/runtime-shadow/`). The lane continues on Python's answers. A second
run sees the first's side effects on live services, so a step such as the
first Core RPC sync legitimately differs in counters like
`core_wallet_created`. `sync-benchmark --runtimes python,rust` times both
runtimes on one chain, using the CLI's `journals process` as the journal
step, and compares their final books.

With `KASSIBER_RUST_BIN` set, `tests/integration/test_regtest_tape_cli.py`
and `tests/integration/test_regtest_exchange_cases.py` also compare the two
runtimes step by step from one path: envelopes and every table of the
store's schema manifest, with fresh identities and clocks masked, and
export artifacts checked for validity only because they follow the
[shared report design](report-design.md). The tape test runs everything
after sync on copies of one Python-synced book, so it compares identities
and their order strictly; the exchange lifecycle creates its rows in each
run, so steps whose order or digests follow random row identities are
compared identity-blind. Where Core RPC wallet sync is not native the
parity test requires Rust's refusal (`native_fallback_required`) to leave
the book unchanged; where it is, the two syncs and synced books must match.

## Pull-request CI

The required PR workflow is fail-fast without reducing the test inventory:

1. `Preflight` compiles Python, validates the test-shard manifest, runs the
   dependency/workflow/catalog/report drift contracts, and gates only the
   Python shards. Frontend, CLI smoke, and observer jobs start independently.
2. Pytest runs every `tests/**/test_*.py` module in exactly one lane. The
   `core-accounting`, `wallets-sync`, `daemon-cli`,
   `security-replication`, and `reports-contracts` lanes use two xdist workers
   with class/module scope. Isolated `serial-network`, `serial-daemon`,
   `serial-regressions`, and `serial-integration` jobs own
   socket-, listener-, broad-regression-, OS-process-, and opt-in
   integration-sensitive modules. The `Rust test binaries` job builds the
   parity suite's binaries once (`scripts/build-rust-test-binaries.sh`: the
   `kassiber` test-harness build, its example fixtures, and the release
   journal probes); every Python and shadow lane downloads them and runs
   with `KASSIBER_RUST_BIN` set, so no parity test refuses for a missing
   binary. `serial-daemon`, which holds hours of subprocess parity tests, and
   `core-accounting` run as several jobs, each with `--part K/N`: a test
   belongs to the part its node id's CRC-32 picks (`tests/conftest.py`), so
   the parts are disjoint and complete.
3. The CLI smoke lane checks the broad argparse help surface in-process and
   keeps only status/health/next-actions, command discovery, and daemon EOF as
   real subprocess probes. TypeScript, ESLint, and Vitest run together in an
   independent frontend job. The independent `Rust crates` job checks the
   crates' formatting and clippy lints (warnings fail) and runs their tests,
   then lints and tests the `kassiber` binary's `test-harness` build, which
   the [Rust shell](rust-shell.md#tests) supervisor-protocol tests need.
   Every job that compiles Rust, including every uv job (which builds the
   engine's binding), first installs the toolchain `rust-toolchain.toml` pins.
4. Specialized jobs can remain independent roots and join `ci/required`
   without blocking unrelated setup. Platform credential jobs retain their
   separate credential/packaging path filter, which includes `tax-engine/**`
   so the binding builds on macOS and Windows, and ordinary pull requests do
   not build release packages.
5. Each pytest/Vitest lane uploads a JUnit artifact even after failure. Python
   lanes print `--durations=50` output in their job summary. The stable
   `ci/required` aggregate is successful only when every required lane passes.
6. The Python lanes run under `KASSIBER_TEST_TAX_ENGINE=census`, so a lane
   fails when one of its modules reaches the tax engine but is missing from
   the [shadow overlay](#shadow-overlay). The `Tax shadow` job, after
   `Preflight`, reruns the overlay part of each lane that has one with
   `run-shadow <lane> --ci`, comparing production with the frozen oracle.

`scripts/python_test_shards.py` is the single file-to-lane mapping. Unknown new
test modules default to `core-accounting`, while `tests/test_ci_shards.py`
proves that every discovered module appears once and that sensitive modules do
not enter xdist. A local `pytest -n N` without `--dist` also keeps each test
class on one worker (`tests/conftest.py` defaults it to `loadscope`), since
some classes, like the CLI smoke suite, are ordered steps over one book. PR
pushes use a concurrency group and cancel superseded CI;
test failures are never broadly retried. Locked uv and pnpm stores are cached;
cache misses still install only from the checked-in lockfiles. Jobs that build
Rust restore a Cargo cache of the registry and the dependency builds (the
workspace's own crates rebuild after every checkout): the uv jobs share one
key that `Preflight` saves, the `Rust crates` job keeps its own, and only
pushes to `main` save, so pull requests restore `main`'s cache. Release
builds take no cache.

The local `./scripts/quality-gate.sh` favors deterministic serial execution,
but runs the same complete Python inventory once, under `census`, and then the
whole shadow overlay once with `run-shadow`, before the frontend checks.

For faster local shard runs, `run <shard> --ci --workers 16 --dist worksteal`
keeps a parallel lane on more workers without changing the CI policy.
`worksteal` lets idle workers take queued tests, which `load` and
`loadscope` never rebalance. `daemon-cli` keeps its classes together and
refuses `--dist`; serial lanes refuse both options. Measurements and the
slowest tests are in [test speed](test-speed.md).

## Dependency chain-observer oracle

Run both independent node scenarios before changing a dependency adapter:

```bash
./scripts/integration-harness.sh chain-observers
```

The optional selector narrows local debugging without changing the shared
Compose stack:

```bash
./scripts/integration-harness.sh chain-observers bitcoin
./scripts/integration-harness.sh chain-observers liquid
```

The lane creates a temporary output root and removes it together with the
per-worktree Compose volumes by default. A normal run also removes any stale
project/volumes left by an earlier keep-mode run before starting, so node truth
always begins on a clean chain. `KASSIBER_REGTEST_KEEP=1` preserves both for
debugging; `KASSIBER_REGTEST_REUSE_CORE=1` reuses already-running loopback
nodes and the existing port/credential environment. Generated
`bitcoin-truth.json` and `liquid-truth.json` files are private run artifacts,
not fixtures: Bitcoin Core and Elements RPC define expected txids, outpoints,
heights, confirmation/replacement relationships, ownership indices, UTXOs and
asset values. Fulcrum/Electrum and the local Esplora-compatible endpoints must
report the same tip, but never define truth themselves.

Bitcoin transitions are full scan, no-op, gap payment/discovery, unconfirmed
and confirmed receipt, unconfirmed spend, RBF replacement and confirmation,
block invalidation, mempool resurrection, reconsideration, process restart,
incremental refresh and final no-op. Wallet-form metadata covers BIP44/49/84/86,
fixed 2-of-2, receive/change, canonical multipath, multi-script logical xpub,
and representative Samourai Deposit/Badbank/Premix/Postmix/Ricochet sources.

Liquid transitions cover ranged confidential receive/change discovery, LBTC
receive/spend, a real issued asset receive/spend, confirmation, invalidation,
resurrection, reconsideration and restart. The manifest records the consensus
policy/issued asset ids, confidential unblinding success and the controlled
wrong-key failure expectation. Capability rows state whether the pinned local
Elements build exposes Taproot or executable ranged multisig rather than
pretending unsupported forms ran.

Every transition is also referenced from versioned observer JSON in the main
temporary SQLCipher database. The runner injects and rolls back a failed state
write, reopens the encrypted project, and rejects BDK/LWK-looking sidecar files.
For Bitcoin/all selections, the lane then creates a real Core descriptor wallet,
funds both low and gap-edge receive indices, and refreshes its public descriptor
through Fulcrum using BDK and through the local Esplora-compatible service using
Kassiber's explicit HTTP transport. It asserts native Electrum state and
per-branch coverage, the HTTP `http_route_policy` compatibility reason,
transaction/UTXO parity, SQLCipher-only persistence, process restart, and
byte-stable immediate no-op native state.
For Liquid/all selections, the lane creates a real Elements descriptor wallet
with private SLIP77 view material and public spending keys, then refreshes it
through local Electrum using LWK and the Esplora-compatible service using
Kassiber's explicit HTTP transport. It asserts opaque native `ForeignStore`
bytes only for Electrum, the HTTP compatibility reason, restart and immediate
no-op stability, confidential LBTC receive/spend and fee normalization,
issued-asset history, transport parity, confirmation, block invalidation and
unconfirmed resurrection.
Only loopback RPC, Electrum and HTTP targets are accepted. Routing metadata
pins pre-connect compatibility selection for this phase, forbids runtime
fallback, and records that `.onion` endpoints may not connect directly.
Separately, the transport oracle drives the pinned clients themselves: BDK crosses
plain Electrum, Esplora, insecure test TLS and SOCKS5h; LWK crosses plain
Electrum, Esplora, explicit insecure test TLS and an authentication-enforcing
Esplora reverse proxy. Focused routing and SSL-context tests cover custom-CA and
explicitly insecure Esplora rows through Kassiber's compatibility HTTP transport
because neither pinned dependency accepts a per-client trust root; the selected
TLS policy is exercised instead of being silently ignored. Direct SDK HTTP
probes establish the binding's capabilities; they do not authorize its use by
Kassiber. Production Esplora routing stays on the explicit compatibility
transport until the SDK exposes enforceable redirect and proxy controls.

Pull requests and main-branch pushes expose a required
`Chain observers (Linux Docker)` job in `.github/workflows/ci.yml`. It runs the
lane when observer, descriptor, sync, persistence, dependency-lock, or regtest
backend paths change; unrelated changes retain the same successful check name
without starting the Docker stack.

The Compose lane generates disposable RPC credentials per run unless you set
them explicitly, passes only the `rpcauth` hash to bitcoind, publishes RPC on
host loopback, and uses Bitcoin Core v30 by default. It also publishes Core's
ZMQ `sequence` feed inside the Compose network for Frigate. The lane uses a
per-worktree Compose project name so parallel runs do
not share containers or volumes. It uses regtest only, no mainnet funds, no user
wallet files, and no production descriptors. The Compose stack publishes only
loopback ports: Core RPC, Elements RPC, the optional Frigate Electrum endpoint,
and the four protocol endpoints used by the UI/backend health and graph paths:

- `core-regtest` -> Bitcoin Core RPC, assigned to the ordinary Bitcoin wallets
  that do not opt into the Fulcrum coverage slice
- Elements Core runs as `elementsd` on `elementsregtest` to provision a local
  Liquid daemon. The demo includes a descriptor-backed Liquid wallet with
  private blinding material stored inside the disposable test book; it receives
  and spends real elementsregtest LBTC, then syncs through the local Liquid
  Electrum service backed by `elementsd`.
- `bitcoin-electrum-regtest` -> the `fulcrum` container's Electrum TCP port
- `bitcoin-frigate-regtest` -> Sparrow Frigate's Electrum TCP port when
  `KASSIBER_REGTEST_COMPOSE_PROFILES=silent-payments` or the
  `silent-payments` lane is used; Frigate proxies ordinary Electrum calls to
  Fulcrum and handles Silent Payments discovery natively
- `bitcoin-mempool-regtest` -> local mempool/esplora-compatible HTTP API and
  the stored default backend, so transaction graph lookups and mempool-backed
  market-price probes use the local graph-capable endpoint instead of public
  infrastructure
- `liquid-electrum-regtest` -> local Liquid Electrum-compatible
  health/scripthash endpoint for the elementsregtest demo rail
- `liquid-mempool-regtest` -> local Liquid mempool/esplora-compatible HTTP
  API for graph lookups on deterministic LBTC demo txids

The HTTP explorer endpoints are host-loopback services and include regtest-only
CORS headers so a host browser can inspect them from a web explorer or local
tooling page. Use the manifest or `demo-up` output for the exact ports; by
default they are:

```text
Bitcoin explorer API: http://127.0.0.1:18544/api
Liquid explorer API:  http://127.0.0.1:18546/api
```

Set `KASSIBER_REGTEST_EXPLORER_CORS_ORIGIN` to a specific origin to narrow the
default `*`, or to an empty value to disable these CORS headers. The services
remain bound to host loopback and must stay regtest-only.

The Compose stack also maps `host.docker.internal` to the Docker host. A
containerized Kassiber process can use a host Ollama instance by seeding the
default local provider with:

```bash
KASSIBER_DEFAULT_AI_BASE_URL=http://host.docker.internal:11434/v1
```

This only affects first-time AI provider seeding; existing books should update
their `ollama` provider row to the host alias explicitly.

## BTCPay Regtest

The dedicated BTCPay proof lane pulls a full BTCPay Server/NBXplorer/Postgres
overlay in addition to the normal Bitcoin regtest stack:

```bash
./scripts/integration-harness.sh btcpay
```

The overlay is defined in `dev/regtest/compose.btcpay.yml`. It reuses the
managed regtest `bitcoind` and generated RPC credentials, runs NBXplorer against
that node, and publishes BTCPay on host loopback. By default the ports are
derived from the regtest base port:

```text
BTCPay Server: http://127.0.0.1:18549
BTCPay NBXplorer: http://127.0.0.1:18550
```

Override them with `KASSIBER_REGTEST_BTCPAY_PORT` and
`KASSIBER_REGTEST_BTCPAY_NBXPLORER_PORT`. The BTCPay image is pinned to the
published stable `btcpayserver/btcpayserver:2.3.9` tag because Docker Hub does
not publish a `latest` tag. Override images with `KASSIBER_REGTEST_BTCPAY_IMAGE`,
`KASSIBER_REGTEST_BTCPAY_NBXPLORER_IMAGE`, and
`KASSIBER_REGTEST_BTCPAY_POSTGRES_IMAGE`.

`dev/regtest/btcpay_permission_probe.py` checks Kassiber's key presets against
one BTCPay version (`up --btcpay 2.4.7`, `probe`, `down`) in its own Compose
project and ports: the server's permission tree, which wallet endpoints each
permission set can read, and what the browser authorize page grants. BTCPay
2.4 needs NBXplorer 2.6; the probe picks it.

`./scripts/integration-harness.sh btcpay-bark` runs the Bark (Ark) plugin
against a regtest Ark server (`dev/regtest/btcpay_bark_stack.py`,
`compose.bark.yml`): bitcoind, an Esplora electrs, captaind with its Core
Lightning, an external Core Lightning node, a counterparty `bark` wallet and
BTCPay 2.4.7 with the plugin mounted, on an internal Docker network with no
internet access. It drives every row kind the plugin's wallet history
produces (receives, board, Ark and Lightning sends, payouts, refresh,
offboards, on-chain sends, an emergency exit) and records the Greenfield
answers. It needs at least 40 GB of available memory and the plugin package
from the BTCPay plugin builder in
`$KASSIBER_BARK_PLUGIN_CACHE/<version>/BTCPayServer.Plugins.Bark.btcpay`
(nothing in the lane downloads it), and takes about 15 minutes.
`tests/fixtures/btcpay_bark/` holds the recordings for plugin 0.13.0, the
pre-0.10 plugin 0.9.6 and that store after upgrading; `tests/test_btcpay_bark.py`
replays them and checks that the booked balance equals the plugin's own.

The seed helper (`python -m dev.regtest.btcpay_seed`) uses the Greenfield API to
create a disposable admin user, store, BTC on-chain payment method, and scoped
API key. In the standalone `btcpay` lane it creates and pays a realistic
BTCPay-origin mix: a direct Greenfield invoice, a duplicate-order adjustment,
a point-of-sale sale, a two-transaction partial payment, a EUR-denominated
checkout invoice, a EUR-denominated payment-request invoice, and a crowdfund pledge. Each
invoice receives a fresh BTCPay on-chain address, is paid from the regtest Core
wallet, confirmed by a mined block, synced through BTCPay wallet history into a
temporary Kassiber book, and reconciled through invoice/payment provenance. The
standalone lane also creates a local commercial document keyed by the BTCPay
payment-request id, runs `btcpay provenance suggest`, reviews the combined
BTCPay-payment-to-wallet-transaction link as income, and checks that the
commercial subledger uses `btcpay_payment` pricing. The seed JSON records the
invoice ids, txids, scenarios, currencies, origin kinds, reviewed link id, and
applied pricing proof so failures can be replayed locally.

The same lane then runs the multi-store Greenfield scenario
(`python -m dev.regtest.btcpay_multistore`) in a second temporary book. It adds
a Pop-up store that shares the first store's wallet, a Café store with its own
hot wallet plus Lightning (an external node string) and LNURL, a real Point of
Sale sale and payment request created through BTCPay's public app routes, and a
Café sale refunded in EUR whose payout is paid on-chain. It issues a read-only
key for all stores and a wallet-history key scoped to the Café, then drives the
CLI: `btcpay key-url`, `btcpay inspect` for both keys, `btcpay setup
--recommended` (both shared stores map onto a watch-only descriptor wallet,
Lightning and LNURL share one Café payment ledger, and the read-only Café
wallet stays provenance-only), `btcpay setup --route …=wallet_source` for the
Café key, `wallets sync --all`, and a refund review against the outbound Café
transaction. It checks that the refund send books its miner fee separately and
that the synced Café store reports fresh data, and fails on any wrong
recommendation, missing payout txid, leaked key material, or unapplied review.
The persistent demo book starts the same BTCPay overlay by default:

```bash
./scripts/integration-harness.sh demo-up
```

That writes the BTCPay URL, store id, and disposable API key into
`demo-manifest.json` (mode `0600`) and configures the demo book with a
`btcpay-regtest` backend plus a `BTCPay Regtest Store` wallet using
`BTC-CHAIN` wallet-history sync. By default it also runs the paid invoice
exercise from the seed helper, so the long-lived demo book includes paid
direct invoices, a payment request, POS/crowdfund provenance, a multi-payment
invoice, duplicate commercial references, wallet-history import, a reviewed
commercial payment-request link, and visible transaction tags
(`btcpay`, `payment-request`, `commercial-income`) on the reconciled
membership-income transaction. Use the dedicated `btcpay` lane when the same
coverage should run in a temporary proof book instead of the persistent demo.
Disable the BTCPay overlay for a lighter persistent demo with:

```bash
KASSIBER_REGTEST_DEMO_BTCPAY=0 ./scripts/integration-harness.sh demo-up
```

Or keep the BTCPay store/UI in the persistent demo but skip paid invoice and
commercial-reconciliation seeding with:

```bash
KASSIBER_REGTEST_DEMO_BTCPAY_INVOICES=0 ./scripts/integration-harness.sh demo-up
```

Set `KASSIBER_REGTEST_KEEP=1` to keep the full Docker Compose project running
for debugging (containers, bound ports, and volumes); otherwise the stack is
removed on exit.
Fresh Compose runs use the scenario manifest's historical timestamp sequence,
starting in January 2019 and covering activity into spring 2026. Reused Core
nodes can only move forward from their existing regtest chain tip, so their
calendar dates may drift while preserving the same relative spacing and row
shape.

## Silent Payments Regtest

The `silent-payments` lane exercises the infrastructure boundary for
Sparrow Frigate, not a generic HTTP scanner. Frigate speaks Electrum protocol
extensions (`blockchain.silentpayments.*`) and advertises support through
`server.features`, so the harness starts it in a dedicated Compose profile and
probes that Electrum capability before running Kassiber's SP sync coverage. On
a fresh regtest volume the lane mines a disposable readiness block first, because
the genesis-only tip still looks like initial block download to Fulcrum and keeps
Frigate waiting for sync.

The full demo book also includes a `silent-payment` wallet. Because Kassiber's
current server-assisted SP adapter expects HTTP JSON while Frigate exposes
Electrum JSON-RPC, the demo wallet syncs through the existing `local-index`
mode: the builder creates a real regtest Taproot output, writes a private
scanner JSON file bound to the wallet's descriptor fingerprint, and stores that
path on the Frigate-marked backend. This keeps the accounting fixture honest
without pretending the Frigate Electrum adapter exists yet.

## Boltz Liquid Regtest

The `boltz-liquid` lane is a narrow bridge between Kassiber's demo accounting
path and Boltz's upstream Docker development setup. Boltz documents normal
submarine swaps as chain -> Lightning, reverse swaps as Lightning -> chain,
and chain swaps as chain -> chain, with "chain" including Bitcoin mainchain
and Liquid. Their regtest repository publishes a local API on
`http://127.0.0.1:9001`, WebSocket on `ws://127.0.0.1:9004`, Bitcoin Esplora
on `http://127.0.0.1:4002`, and Elements/Liquid Esplora on
`http://127.0.0.1:4003`.

Run it with an existing checkout:

```bash
git clone https://github.com/BoltzExchange/regtest ~/.cache/kassiber/boltz-regtest
./scripts/integration-harness.sh boltz-liquid
```

Or let the lane clone the upstream repo into the same cache path:

```bash
KASSIBER_BOLTZ_REGTEST_AUTO_CLONE=1 ./scripts/integration-harness.sh boltz-liquid
```

Set `KASSIBER_BOLTZ_REGTEST_REUSE=1` when the Boltz stack is already running,
or `KASSIBER_BOLTZ_REGTEST_KEEP=1` to leave it running after the test. The
lane uses `docker` directly when available and falls back to passwordless
`sudo docker`, matching the other regtest lanes. It uses Boltz's documented v2
API for the low-signing happy path that Kassiber needs to account for today:

- `POST /v2/swap/submarine` with `from=L-BTC`, `to=BTC`, a real LND invoice,
  and a refund pubkey.
- `elements-cli-sim-client sendtoaddress` to fund the returned Liquid lockup
  address, then local Elements mining.
- a separate `elements-cli-sim-client sendtoaddress` payment to prove ordinary
  Liquid outflows stay payments and are not matched as swaps.
- a temporary Kassiber book with Liquid identity rows carrying the observed
  txids, Elements regtest network, and policy-asset id; the paid invoice is
  normalized through the native LND adapter/import boundary. The shared hash is
  therefore a strong review candidate (native node + Boltz provider evidence),
  and the lane explicitly confirms it through `transfers bulk-pair`.

The lane still only live-executes the L-BTC -> BTC submarine path because
reverse, BTC -> L-BTC chain, and refund execution require client-side
claim/refund transaction construction. The default lane therefore does not claim
reverse, chain, or cooperative refund v2 execution coverage. Those cases become
covered only when the harness is given real wallet/client/provider evidence from
an executed swap.

If those v2 flows have been executed by an official Boltz client/SDK or another
wallet has the relevant facts, pass that evidence through
`KASSIBER_BOLTZ_V2_EVIDENCE` or `--v2-evidence` when running the `boltz-liquid`
lane. The evidence must contain real route identifiers and observed
amounts/timestamps, physical chain/network scope, and the consensus asset id for
every Liquid leg; the harness rejects obvious placeholder ids instead of
recreating the old metadata-only fixture path. If the user only has the chain
rows and no provider/client facts, Kassiber should still surface heuristic
swap-pair suggestions for review rather than claiming exact provider evidence.
Minimal shape:

```json
{
  "swaps": [
    {
      "provider": "boltz",
      "id": "provider-swap-id",
      "flow": "chain",
      "status": "completed",
      "version": "2",
      "taproot": true,
      "cooperative": true,
      "spend_path": "key",
      "out": {
        "txid": "real-send-txid-or-external-id",
        "occurred_at": "2026-07-02T11:00:00Z",
        "asset": "BTC",
        "chain": "bitcoin",
        "network": "regtest",
        "amount": "0.01000000",
        "fee": "0.00000500"
      },
      "in": {
        "txid": "real-receive-txid-or-external-id",
        "occurred_at": "2026-07-02T11:04:00Z",
        "asset": "LBTC",
        "asset_id": "real-32-byte-elements-policy-asset-id",
        "chain": "liquid",
        "network": "elementsregtest",
        "amount": "0.00990000"
      }
    }
  ]
}
```

Boltz's upstream Compose file binds bitcoind RPC to host port `18443`, the same
default used by Kassiber's own regtest lane. If that port is already occupied,
the harness writes a temporary Compose file with the host-only binding changed
to `19443 -> 18443`; internal Boltz services still use `bitcoind:18443`, and
the upstream checkout stays untouched. Set `KASSIBER_BOLTZ_BITCOIN_RPC_PORT=<port>`
to choose a different host binding.

## Lightning Business Regtest

The `lightning-business` lane is Kassiber's live Lightning merchant-node test.
It layers `dev/regtest/compose.lightning.yml` onto the existing Bitcoin regtest
compose file and uses Bitcoin Core regtest as the funding/mining source. The
CLN overlay defaults to the pinned `elementsproject/lightningd:v25.05` image;
set `KASSIBER_REGTEST_CLN_IMAGE` to test a different CLN build intentionally.
The LND backup node defaults to `lightninglabs/lnd:v0.18.4-beta`; set
`KASSIBER_REGTEST_LND_IMAGE` to test a different LND build intentionally.

Run:

```bash
./scripts/integration-harness.sh lightning-business
```

The lane creates these Docker-only actors:

- `cln_merchant` — the operational Core Lightning merchant node Kassiber syncs
- `cln_customer` — pays merchant invoices and routed supplier invoices
- `cln_supplier` — receives merchant expense payments
- `cln_router` — provides the extra hop for routed payments and fee rows
- `lnd_merchant_backup` — a live LND backup merchant node connected to
  `cln_merchant` and `cln_router` through funded private channels

Kassiber stores two Lightning backend/wallet rows: the operational
`cln_merchant` Core Lightning source and the `lnd_merchant_backup` LND source.
The CLN backend's `lightning_cli` points at
`dev/regtest/lightning-cli-merchant.sh`; customer, supplier, and router are
never created as Kassiber wallets or connections. The LND backend stores the
loopback REST URL plus the node's disposable read-only macaroon from the
regtest volume.

The bootstrap script is idempotent: it waits for CLN and LND, creates/reuses the
Bitcoin faucet wallet, funds CLN/LND wallets only below the threshold, opens the
private `merchant -- lnd_merchant_backup` and `lnd_merchant_backup -- router`
backup channels plus the public `customer -- merchant -- router -- supplier`
channels if absent, mines confirmations, and waits for normal/active channels. The scenario then generates a
seeded business plan at `$KASSIBER_LIGHTNING_BUSINESS_PLAN` (default:
`$KASSIBER_LIGHTNING_BUSINESS_HOME/business-plan.json`) and executes the
stable-label activity from that plan:

- customer-paid merchant invoices (`merchant-pos-sale-*`) with varied amounts
- merchant-paid supplier invoices routed through the router
- customer/router third-party payments that cross the merchant as forwards
- LND backup receipts and outbound LND payments so the backup-node dashboard
  has real invoice/payment activity
- one expired/unpaid merchant quote
- one intentionally oversized failed payment to exercise failed-payment rows
- Bitcoin Core mainchain actor wallets that send top-ups into the merchant CLN
  wallet and receive merchant CLN withdrawals, with regtest blocks mined between
  batches so the merchant node's L1 balance and bookkeeper rows are non-trivial

The plan borrows sim-ln's useful modeling knobs without making sim-ln a hard
test dependency: defined activity remains deterministic for assertions, while
`KASSIBER_REGTEST_LIGHTNING_SEED`,
`KASSIBER_REGTEST_LIGHTNING_CAPACITY_MULTIPLIER`, and
`KASSIBER_REGTEST_LIGHTNING_EXPECTED_PAYMENT_MSAT` control variation; the
channel-capacity input comes from
`KASSIBER_REGTEST_LIGHTNING_CHANNEL_CAPACITY_SAT`. The default multiplier is
intentionally modest so the route directions stay liquid on the default
5,000,000 sat channels.

Assertions happen in `tests/integration/lightning_business_regtest.py` and are
against Kassiber output/state, not only `lightning-cli`: daemon snapshot,
profitability report, CSV export, synced DB records, and the one-merchant-only
wallet/backend invariant. The lane also checks that the merchant snapshot has
on-chain balance evidence plus paid, expired, failed, outgoing-payment, channel,
and forwarding activity, while the LND snapshot has two private channels plus
paid invoice and completed payment evidence. Persisted Lightning records do not store raw RPC JSON;
channel lifecycle rows retain only the node-observed Bitcoin chain/network
scope required for safe L1 identity matching. AI-safe Lightning payloads omit
sensitive route, peer, preimage, payment-secret, bolt11, funding-outpoint, and
failure-source fields.

Set `KASSIBER_REGTEST_KEEP=1` to leave the full Docker Compose project running
for inspection (containers, bound ports, and volumes), or
`KASSIBER_REGTEST_LIGHTNING_REUSE=1` to reuse an already-running project. The
book lives under
`${KASSIBER_LIGHTNING_BUSINESS_HOME:-/tmp/kassiber-lightning-business-<project>}`.
The assertion book is rebuilt by default so stale DB rows cannot satisfy a
fresh run; set `KASSIBER_LIGHTNING_BUSINESS_REUSE_BOOK=1` only when debugging a
preserved book intentionally. The scenario state records the generated plan
hash, so changing seed, multiplier, expected-payment, or channel-capacity knobs
while reusing preserved Lightning state requires a fresh
`KASSIBER_LIGHTNING_BUSINESS_HOME` or manual cleanup of the preserved state and
volumes.

## Full Accounting Demo

`demo-full` is the replacement substrate for fake one-click accounting data. It
does not inject synthetic transaction rows into SQLite. Instead, it creates real
Bitcoin Core regtest wallets, broadcasts real transactions, syncs Kassiber from
the Core RPC backend, then verifies Kassiber behavior through the public CLI:

- address-wallet creation and Bitcoin Core watch-only sync, with every
  operational wallet watching several rotating addresses (fresh receive and
  change addresses per payment, funding spread across the address set, and
  greedy multi-UTXO coin selection) so the book looks like real wallet usage
- descriptor-backed Liquid wallet creation on `elementsregtest`, with LBTC
  receipts/spends and BTC<->LBTC bridge legs mined on the local `elementsd`
  chain before import through the same `wallets sync --all` path
- acquisition and disposal rows across Treasury, Cold Storage, Spending, and
  Merchant wallets, plus empty Bitcoin and Liquid rotation-target wallets that
  become active after security upgrades
- large single-source custody receipt into cold storage
- batched treasury payout to multiple external recipients in one transaction
- same-block merchant point-of-sale receipt burst with sat-level ragged
  amounts, followed by a many-input consolidation that imports as a fee-only
  wallet row, and an unsolicited dust deposit that lingers in the UTXO
  inventory
- an RBF fee-bumped payment: the conflicted original must be skipped by sync
  while only the confirmed replacement is booked
- solo-mining block rewards swept into treasury at two points in history
  (visibly smaller after regtest halvings), while the dedicated mining wallet
  stays intentionally tiny; immature coinbases must never import
- a customer payment that is still unconfirmed in the mempool when the book is
  synced, imported with an empty confirmed-at and a mempool UTXO state
- a deterministic historical stress lane: 84 cycles spaced 30 days apart, with
  batched inbound funding into operational wallets, rotating outbound payments,
  and regular fiat-expense disposals for payroll, rent, software, tax prep,
  contractors, and equipment; amounts and fees vary per cycle through a
  deterministic jitter (`stress.variation_bp`) so the ledger is volatile but
  reproducible; the demo still adds several hundred synced/imported wallet rows
  across seven years
- deterministic economic regimes (`stress.economic_regimes`) that scale
  inflows down and outflows up during downturns and vice-versa in booms, so
  operational balances genuinely rise **and** draw down (e.g. the treasury
  account falls through a 2020 shock and a 2022 bear market rather than
  climbing monotonically); regimes stay within each wallet's running balance
  so the tax engine's per-account balance gate never trips
- wallet key-rotation events for treasury, merchant, cold storage, and Liquid
  treasury, recognized from native ownership evidence after sync; the old source
  wallets are then marked deprecated so their history remains visible while
  refresh-all/background freshness skips them
- an ownership-derived Bitcoin fan-out transfer where one treasury transaction
  pays two owned operational wallets; before any manual `transaction_pairs`
  exist the demo runs `journals process` and `journals transfers list` to prove
  both MOVE legs are surfaced with `pairing_source=ownership_derived`
- Liquid/on-chain-style bridge events (`peg-in`, a Boltz-marked
  BTC -> L-BTC `chain-swap`, and `peg-out`) are backed by real Bitcoin Core
  and elementsregtest transactions so the generic-tax demo exercises taxable
  cross-asset swaps without synthetic transaction ids
- local backend rows for Bitcoin Core RPC, Bitcoin Electrum/Fulcrum-compatible
  TCP, Bitcoin mempool-compatible HTTP, Liquid Electrum-compatible TCP, and
  Liquid mempool-compatible HTTP; the demo deletes public/default backends and
  fails if any non-regtest backend remains
- automatic same-chain wallet-to-wallet movements, without preparing manual
  pairs to make the checks pass
- a signed CoinJoin-shaped PSBT with two owned participants and a separate
  foreign participant, equal owned outputs, and watched change. The foreign
  participant pays the network fee; the user's ledger must not charge it again
- a signed Payjoin-shaped PSBT with connected payer and merchant wallets;
  the receiving wallet's contributed input must be subtracted from its receipt.
  Collaborative rows remain included in accounting and exercise native matching
- Bitcoin-backed-loan marks for collateral lock/release and BTC principal
  receive/repay, linked under one loan id
- real historical BTC/EUR pricing from Kassiber's bundled Kraken daily cache,
  with LBTC rows priced through Kassiber's LBTC-to-BTC rate-pair alias; the
  docker preview also writes one current `mempool` latest-quote row from the
  local Bitcoin mempool-compatible endpoint so the overview's live BTC price
  is not a historical Kraken row. Set
  `KASSIBER_REGTEST_DEMO_LIVE_RATES=coinbase-exchange` or `coingecko` to
  override that provider, or `off` to keep only the historical cache.
- journal processing, summary reporting, PDF/CSV/XLSX report export, and
  CSV/XLSX transaction export
- a separate `Exchange Reconciliation` profile with a synthetic Strike export
  imported through the production importer and genuine Core withdrawal/deposit
  transactions. A deliberately missing purchase leaves an accounting case;
  importing the complete export and applying a portable reviewed custody plan
  for the deposit must restore report readiness without exclusions. The
  withdrawal needs no review: it books automatically as `exchange_txid_exact`
  because the export names its txid and exact amount. Independent quantities and
  FIFO basis distinguish the platform withdrawal charge from the foreign hot
  wallet's batch fee and the user's return fee. Repeated export import and native
  refresh must preserve the result. Prices are scoped to these transaction rows
- signed, paginated Coinbase API requests against a disposable loopback fixture,
  including a repeated import. This checks the production connection adapter's
  protocol behavior, not live Coinbase compatibility. Its temporary book and
  stopped service are not added as an unusable connection in the interactive demo
- `demo-cases.md` and `demo-cases.json` describe the successful assertions,
  evidence origins and exact native wallet deltas; exchange results and the
  review artifact are retained under the export directory's `exchange/` folder
- a generated oracle artifact at `generated-truth.json` in the demo export
  directory for the report/export build point (before the optional post-sync
  business tick). The demo records expected transaction row identities,
  reviewed transfer pairs, and active Bitcoin Core UTXOs/balances while it
  creates the chain activity, then verifies Kassiber's synced rows, report
  metrics, UTXO inventory, and transaction export contents against that
  generated truth. Exact checks are used for the Core-controlled row/UTXO
  surface; report file bytes remain intentionally unchecked because PDF/XLSX
  metadata and writer ordering are not stable enough to be the source of truth.

The scenario manifest lives at
`dev/regtest/scenarios/full_accounting.json`; the runner lives at
`tests/integration/regtest_demo.py`. The command prints a JSON summary with the
generated `data_root` and `export_dir`, for example:

```bash
./scripts/integration-harness.sh demo-full
```

## Developer Regtest Demo Environment

`demo-full` is a test lane: it builds a throwaway book and tears the node
down. For day-to-day development there is a persistent variant that replaces
browser fixtures with a real, synced book — the same model as
BTCPayServer's `docker-compose up dev` + launch-profile workflow:

```bash
./scripts/integration-harness.sh demo-up   # node + demo book, kept running
cd ui-tauri && pnpm dev:demo               # dev preview on that real book
```

Prerequisites on any machine: Docker (Desktop or engine), Python 3, `uv`, and
`pnpm`.
From a fresh clone, install the project/runtime dependencies once:

```bash
./scripts/bootstrap-dev-env.sh
pnpm --dir ui-tauri install --frozen-lockfile
```

After that, the two commands above take you to a browser preview backed by the
real Python daemon reading a multi-year regtest book. The harness checks for
core Python dependencies and fails with a setup hint instead of silently using
an unprepared system interpreter.

What `demo-up` does:

- starts (or reuses) the regtest Compose stack under the fixed Compose project
  `kassiber-regtest-demo`, separate from the per-worktree test projects, enables
  the `silent-payments` profile, and leaves Bitcoin Core, Elements, Fulcrum,
  Frigate, the Core Lightning overlay, and the local protocol API services
  running;
- builds the `full-accounting-v1` book once into
  `~/.kassiber/regtest-demo/data` (override with
  `KASSIBER_REGTEST_DEMO_HOME`) and reuses it on later runs while the
  scenario and generator are unchanged. The manifest fingerprint includes the
  selected scenario, integration helpers, local service configuration and
  harness, so fixing a generated case also rebuilds an existing demo book;
  set `KASSIBER_REGTEST_DEMO_REBUILD=1` to force a rebuild;
- seeds the Lightning business topology into that same demo book by default:
  `cln_merchant` appears as a `coreln` wallet/backend in
  `Regtest Demo / Full Accounting`, while `cln_customer`, `cln_supplier`, and
  `cln_router` remain Docker-only actors. Set
  `KASSIBER_REGTEST_DEMO_LIGHTNING=0` to opt out of the CLN overlay and
  Lightning seed when you need a lighter Bitcoin/Liquid-only demo;
- persists the generated regtest RPC credentials in
  `~/.kassiber/regtest-demo/demo-manifest.json` (mode 600, regtest-only
  throwaway secrets) so restarts keep matching the book's stored backend and
  refresh/sync from the GUI keeps working;
- keeps the demo Core wallets loaded (`--keep-core-wallets`) so incremental
  syncs from the app keep seeing new activity.

The embedded Lightning bootstrap funds its faucet from this demo's generated
external actor when mature funds are low. This avoids relying on regtest block
subsidies after the historical scenario has crossed many halvings. Standalone
Lightning runs mine their own faucet funds; an explicit
`KASSIBER_REGTEST_LIGHTNING_FUNDING_WALLET` selects a known regtest funding
wallet for an older chain. Failed funding aborts immediately.

Persistent demos also seed two explicitly synthetic source-of-funds cases:
a reviewed Strike CSV purchase through its exchange withdrawal into the observed
regtest wallet receipt, and a separate receipt whose origin evidence remains
missing. The attached CSV and `exports/source-funds-synthetic.pdf` are generated
test artifacts, not proof of a real exchange purchase. `source-funds-seed.json`
records the case IDs; restarts reuse them and preserve manual demo edits.

The `fulcrum` container is provisioned and exposed as the
`bitcoin-electrum-regtest` backend row. The full-accounting demo assigns the
active `treasury_2020`, `merchant_2022`, and `cold_2024` wallets to Fulcrum while
the remaining ordinary Bitcoin wallets stay on Core RPC. The slow Bitcoin lane
also includes a dedicated Fulcrum/Electrum parity slice that syncs the same real
address wallet
through Core RPC and Electrum, then compares the persisted transaction and UTXO
views after receipts, a spend, an incremental receipt, and a no-op sync. The
Liquid Electrum and Liquid mempool rows are local services
backed by `elementsd`; every Liquid wallet in the demo is descriptor-backed and
syncs real elementsregtest LBTC transactions. The preview stays repeatable
without inventing Liquid transaction ids or contacting public mainnet explorers.
The stored default backend is `bitcoin-mempool-regtest`: wallet configs still
pin their sync source (`core-regtest` or `bitcoin-electrum-regtest` for ordinary
Bitcoin, `bitcoin-frigate-regtest` for the Silent Payments wallet), while
graph-capable UI paths prefer the local
HTTP mempool/esplora endpoint.
Remaining backend-parity work is demo-wallet sync through Bitcoin explorer HTTP,
plus broader Liquid Electrum/explorer/Elements comparisons across the historical
fixture set.

`pnpm dev:demo` runs the Vite daemon bridge with
`KASSIBER_DEV_DATA_ROOT` pointed at the demo book; the desktop preview then
shows the regtest data mode instead of static fixtures. `pnpm dev:browser` is an
alias for the same regtest-backed browser preview. Focused fixture objects remain
in UI unit tests; there is no interactive mock transport.

The bridge treats `KASSIBER_DEV_DATA_ROOT` as the preview's launch book, and
Open Regtest Demo opens it in place rather than as an imported folder. Books
opened through the dev folder picker restart the daemon on that folder, and
leaving them (Back to setup, Reset Welcome state, cancelling an import) restarts
it on the launch book again. Only a bridge started without
`KASSIBER_DEV_DATA_ROOT` falls back to the implicit default root, which on a
developer machine is the real book, so point exploratory previews at a
disposable one.

### Making resync do something (`demo-tick`)

A freshly built book sits at the chain tip, so an in-app refresh imports
nothing. To simulate ongoing business so the incremental sync path has real
work:

```bash
./scripts/integration-harness.sh demo-tick        # one batch of fresh activity
./scripts/integration-harness.sh demo-tick 5      # five batches, five blocks
```

Each tick broadcasts a randomized batch of receipts (external → wallet),
payments (wallet → external), and self-transfers across the active
(non-deprecated) wallets, then mines a block so it confirms. Refresh in the
app (or `wallets sync --all`) and the new rows import. Activity is
random by default (that is the point); pass `--tick-seed` to
`tests.integration.regtest_demo --tick` for a reproducible batch. `demo-full`
itself always ends with one built-in tick + resync and fails if that resync
imports nothing — a standing guard that "refresh" is never a dead button.

### Large-book sync benchmark

The July 2026 performance slice used the checked-in full-accounting scenario on
the same local Docker stack before and after the changes. The resulting book
contained 998 transaction rows across 13 wallets after the built-in activity
tick and final indexer catch-up. Three active Bitcoin wallets used Fulcrum, the remaining ordinary Bitcoin
wallets used Core RPC, and Liquid wallets continued to use their Electrum
backend. Each reported resync figure is the median of repeated runs against the
same chain and SQLite book:

| Path | Before | After | Change |
| --- | ---: | ---: | ---: |
| CLI `wallets sync --all`, unchanged chain (5 runs) | 12.156 s | 2.461 s | 79.8% faster |
| Desktop freshness sync, unchanged chain (5 runs) | 2.966 s | 1.878 s | 36.7% faster |
| Desktop forced full replay, 594 Fulcrum transaction fetches / 642 records (3 runs) | — | 5.265 s | reference point |

The unchanged-chain runs imported zero rows, wrote zero false updates, and
fetched zero Fulcrum transaction bodies. The forced replay also wrote zero
false updates. End-to-end `demo-full` setup remains roughly 16 minutes on the
benchmark host because broadcasting and mining about 1,000 real operations over
the historical 50-block workload dominates that lane. Treat setup generation
and wallet resync as separate measurements when investigating regressions.

For one large wallet, `./scripts/integration-harness.sh sync-benchmark` runs
`scripts/benchmark-wallet-sync.py` against the lane's bitcoind and Fulcrum.
It builds one descriptor wallet from `--txs` payments over `--addresses`
receive addresses, plus a tenth as many spends. Then it times each step a user
waits for as its own child process:

- the first sync;
- `--noop-runs` unchanged resyncs;
- an `--increment` sync;
- a forced full replay;
- after each of these, the desktop's post-sync journal step
  (`refresh_journals_step` with automatic pairing).

The wallet uses the default route for an Electrum backend, BDK. Every backend
in the disposable book is a loopback regtest backend, child processes run under
the test socket guard, and the book is removed at the end. Each step emits a
JSON line with its seconds, peak RSS and the sync counters. The book events
also report how much of the stored `raw_json` is the per-row owned-script
attestation. `--profile-dir` writes a cProfile file per step and adds the
hottest functions to its line. `--smoke` checks the lane in about a minute.
Keep the stack between runs with `KASSIBER_REGTEST_KEEP=1`, then
`KASSIBER_REGTEST_REUSE_CORE=1` with the same `KASSIBER_REGTEST_RPC_PORT` and
password.

September 2026 reference, measured on the benchmark host without profiling.
The runs used 2,000 payments over 400 addresses (2,200 rows) and 5,000 over
1,000 (5,500 rows), each with a tenth as many spends. Cells give sync +
journal step, before and after BDK stopped re-emitting unchanged
transactions and the owned-script attestation was scoped to each graph:

| Step | 2,200 rows before | after | 5,500 rows before | after |
| --- | ---: | ---: | ---: | ---: |
| First sync | 14.3 s + 17.9 s | 10.6 s + 6.6 s | 83.1 s + 93.2 s | 59.9 s + 18.4 s |
| Unchanged resync (median of 3) | 18.1 s + 5.1 s | 4.4 s + 1.8 s | 99.2 s + 26.9 s | 9.6 s + 6.6 s |
| 25 new payments | 15.3 s + 18.9 s | 4.4 s + 6.4 s | 86.8 s + 92.9 s | 10.2 s + 18.6 s |
| Forced full replay | 18.4 s + 5.1 s | 11.7 s + 1.9 s | 100.6 s + 27.0 s | 61.9 s + 6.8 s |

Before, an unchanged resync cost more than the first sync: the BDK route
re-emitted every wallet transaction, and every row repeated the wallet's whole
owned-script set, so storage grew as rows × scripts:

| | 2,200 rows before | after | 5,500 rows before | after |
| --- | ---: | ---: | ---: | ---: |
| Stored `raw_json` | 65.6 MB | 2.4 MB | 401.6 MB | 5.9 MB |
| Database size | 84.8 MB | 22.0 MB | 444.4 MB | 50.4 MB |
| Peak memory, first journal step | 612 MB | 144 MB | 2,615 MB | 267 MB |

The first sync still grows faster than the row count, and an unchanged
journal step still runs every automatic-pairing pass.

Poke the node like BTCPayServer's `docker-bitcoin-cli.sh`:

```bash
./dev/regtest/bitcoin-cli.sh getblockchaininfo
./dev/regtest/bitcoin-cli.sh -generate 1
uv run --locked python -m kassiber --data-root ~/.kassiber/regtest-demo/data reports summary
```

Tear-down is explicit: `demo-down` stops the node but keeps the chain volume
and book (a later `demo-up` resumes both); `demo-down --purge` removes the
node, volume, and demo book.

Because the book is backdated with `setmocktime`, a resumed node mints new
blocks at wall-clock time — new activity lands "now", after the historical
span, which is exactly what a long-lived real book looks like.

Fixture transport remains useful for component tests, but it should not be
treated as an accounting or sync proof and is not exposed as an interactive data
mode.

## Custody oracle

Custody evidence commitments, decision, issue and relation ids, gap
fingerprints and uuid5 component ids are stored and later compared with
recomputed values, so their bytes may not drift. `tests/custody_hash_vectors.py`
generates seeded golden vectors for each of them into
`tests/fixtures/custody_hash_vectors/`, one JSON Lines file per family. Every
vector names the product function, the serialization it hashes (including the
`json.dumps` arguments) and the hashed text, so a port can reproduce it.
`tests/test_custody_hash_vectors.py` recomputes every vector and also checks
the recorded serialization against the hashed text.

```bash
uv run --locked python -m tests.custody_hash_vectors          # check
uv run --locked python -m tests.custody_hash_vectors --write  # regenerate
```

Regenerate only together with a migration for books that stored the old
values.

`KASSIBER_TEST_CUSTODY_SNAPSHOT_DIR`, set to an existing absolute
directory, records custody builds in-process and in Python children as one
JSON envelope each: the product-loaded raw v1 request, owned-index lookups,
stage results and decisions ([schema](custody-engine.md)). Add
`KASSIBER_TEST_CUSTODY_SNAPSHOT_VERIFY=1` to replay each raw request in a fresh
process under another `PYTHONHASHSEED` and fail on any difference. This needs
no database image and also works for SQLCipher-loaded requests. Product code
reads neither variable. Use disposable data only.

```bash
mkdir -p /tmp/custody-snapshots
KASSIBER_TEST_CUSTODY_SNAPSHOT_DIR=/tmp/custody-snapshots \
  KASSIBER_TEST_CUSTODY_SNAPSHOT_VERIFY=1 \
  uv run --locked python -m pytest tests/test_custody_*.py -q
uv run --locked python -m tests.custody_snapshot check /tmp/custody-snapshots
uv run --locked python -m tests.custody_snapshot compare DIR_A DIR_B
uv run --locked python -m tests.custody_snapshot check-rust /tmp/custody-snapshots --permuted
uv run --locked python -m tests.custody_snapshot refresh /tmp/custody-snapshots /tmp/custody-refreshed
```

`refresh` copies recordings with their stages recomputed by the current
Python reference, for recordings an older reference wrote; it never writes
the source directory.

## Guardrails

- `KASSIBER_TEST_NO_EGRESS=1` arms the socket guard only; `tests/conftest.py`
  sets it before collection for every non-integration run. It blocks Python
  DNS, TCP connects, and UDP sends to non-loopback hosts. A `sitecustomize` on
  `PYTHONPATH` carries it into spawned Python children and stops a child if the
  guard cannot start. A child that would not load it fails instead of running
  unguarded (`tests/egress_guard.py`): while the session is armed,
  `cli_runner.run`/`popen` and `runtime_parity.child_env` refuse a Python
  child whose environment drops the switch, whose `PYTHONPATH` does not
  resolve `sitecustomize` to `tests/_child_hooks`, or whose interpreter
  options skip site (`-I`, `-S`, `-E`), and every `python -m tests.<module>`
  reference exits 78 at import when the guard is not live in it. Build child
  environments by copying `os.environ` and use `guarded_pythonpath()` rather
  than replacing `PYTHONPATH`; a child that brings a stricter guard of its own
  names it with `own_egress_guard=` (`tests/rust_port/contracts.py`). The
  separate product override `KASSIBER_NO_EGRESS=1`
  blocks BDK/LWK observer construction, shared backend HTTP/SOCKS and Python
  Electrum connections, and chain-analysis acquisition, including loopback.
  It is not used suite-wide and is not an OS-wide network firewall.
- `tests/conftest.py` also points `KASSIBER_OFFLINE_PREFERENCE_FILE` at a
  disposable file for the whole run, so the user's real
  [offline mode](privacy-and-security.md#offline-mode) choice cannot make
  transport tests fail and no test can change it. Exploratory
  `kassiber offline on|off` runs change the machine-wide switch unless that
  variable points elsewhere.
- Tapes must include provenance (`backend_kind`, network, regtest anchor, and
  issue number) and fail closed: an adapter request absent from the tape raises
  `TapeMiss`, while unused recorded interactions fail the replay test.
- Export assertions are content-level. XLSX files are inspected for expected
  sheets and self-verification content rather than byte-compared.
- The full demo's generated truth is the oracle for controlled regtest facts:
  transaction row identities, transfer pair identities, Core wallet UTXOs, and
  Core wallet balances must match exactly. Broader report/export checks stay
  targeted where formatting, cached market data, or third-party writers make
  byte-for-byte comparison noisy.
- Docker infrastructure is contributor test tooling only. It must not add an
  app-facing shell/filesystem escape hatch or relax desktop daemon allowlists.
- The demo runner prints regtest addresses and txids, but never prints RPC
  passwords. CLI backend credentials are passed through file descriptors.

## Growth Path

The current checked-in slow lanes exercise Bitcoin Core RPC end to end (sync,
pricing, journal, report, export), a Core-vs-Fulcrum/Electrum parity slice, and
a full accounting demo on Bitcoin regtest. They also provision Elements Core,
Bitcoin Fulcrum, and local mempool/esplora-compatible endpoints, then sync
descriptor-backed elementsregtest/LBTC demo wallets through the local Liquid
Electrum-compatible service. Remaining parity targets are broader cross-backend
comparisons: Bitcoin Core vs Fulcrum vs explorer HTTP, and Liquid Electrum vs
explorer HTTP/`elementsd` views across the larger historical fixture set. Those
can be added without changing the contributor entrypoint.

Lightning now has an opt-in live merchant lane through
`./scripts/integration-harness.sh lightning-business`. Remaining parity work is
to add a fast recorded `lightning-cli` tape lane, broaden backend parity beyond
Core Lightning/LND as new adapters land, and eventually compare live
Lightning-derived accounting across multiple node implementations without
changing the contributor entrypoint.

## Local chain-analysis oracle

`./scripts/integration-harness.sh chain-analysis` runs the dedicated disposable
Core31 consensus/index oracle. It uses an ephemeral loopback RPC port, generated
credentials, its own container with `txindex` and `txospenderindex`, and temporary
Kassiber books. P2P networking is disabled; the node is limited to two CPUs and
1 GiB RAM. Cleanup removes only that container, including when a test fails or
the script is interrupted. It needs Docker and may pull `bitcoin/bitcoin:31.0`.

`tests.integration.test_live_chain_analysis` covers:

| Real Core fixture | Required result |
| --- | --- |
| Transfers through three intermediate wallets; only endpoints initially acquired | No invented path, explicit acquisition frontier, then exact create/spend path after acquiring the missing transaction; saved-case difference preserves the original incomplete evidence. |
| Another confirmed spend after saving a complete local trace | Current case comparison includes the new transaction; the saved snapshot remains unchanged. |
| Four independently funded legacy, nested SegWit, native SegWit and Taproot inputs | Empty PSBT amounts remain unknown; wallet-filled/signed PSBT fee and final vsize match Core; acquired transaction features match PSBT structure and signature categories after raw witness discard. |
| Two independent wallet signers with equal inputs and equal outputs | One signer cannot finalize; both can produce a mined transaction; the financial-flow model retains three possible partitions and grants no ownership authority. This exercises collaborative transaction structure, not a CoinJoin coordinator. |
| Original payment and receiver-signed Payjoin proposal | Added receiver contribution, fee delta, preserved output order and actual receiver signature commitments pass; reordered outputs fail; sender completion produces a Core-accepted mined transaction. The fixture constructs proposals locally and does not exercise HTTP negotiation. |
| Reviewed, encrypted block-range acquisition and actual Core reorg | Batches resume from the durable cursor, withhold the disconnected branch, recover the canonical range and keep accounting transactions empty. |
| Relative block lock and absolute height lock | Core rejects each transaction before its prerequisite height and accepts it afterward; structural locktime and RBF-signalling features agree. |

Every scenario also checks that acquisition creates no accounting transactions,
watch-only wallets, custody components or ownership provenance. Public observation
payloads retain categorical features while discarding raw witnesses, scriptSig,
derivations and credentials. The lane waits for confirmed outpoints and both
indexes at the exact mined tip rather than using fixed startup sleeps. Normal
quality-gate runs skip these live tests; the dedicated lane additionally sets
`KASSIBER_DISPOSABLE_CHAIN_ANALYSIS=1` to avoid running them against another test
stack. Unit tests cover official BIP174/370 vectors, BIP78 hostile proposals,
observer views, acquisition budgets, conditional entropy scenarios, labels,
case revisions, CLI/daemon and AI projection without network.

`tests/test_chain_analysis_consent.py` runs the actual assistant tool loop and
consent broker with synthetic provider turns and node responses. It checks
approval, denial, cancellation, timeout, once-only scope and stale plans while
forbidding even loopback DNS/socket attempts. Transport tests in
`tests/test_proxy.py`, `tests/test_ai_core.py`, `tests/test_lnd_adapter.py` and
`tests/test_update_check.py` separately exercise real urllib handler chains,
including vulnerable redirect controls, explicit proxy routing and TLS checks.

[Cashu-regtest](https://github.com/callebtc/cashu-regtest) is a useful reference
for later cross-rail fixtures: its
[startup checks](https://github.com/callebtc/cashu-regtest/blob/main/start.sh)
exercise channel readiness and settled payments, and its optional stacks cover
Arkade, Bark and Spark alongside LND, CLN and LDK. Reuse the evidence/readiness
pattern: bind a funding outpoint to confirmation, compare both payment endpoints
by payment hash, and check principal plus fees at each custody boundary. Starting
that entire environment is not part of this lane; its reset behavior and large
optional builds require a separately isolated project.

This Bitcoin-only lane does not validate exchange export reconciliation, Liquid
confidential amounts, live Lightning routing, Cashu proofs or Ark/Spark exits.
Existing accounting/Lightning/Elements lanes cover their stated adapters. LDK,
Ark and Spark still need Kassiber observation adapters and real protocol evidence
before a cross-rail edge can claim more than a reviewed custody relation. A
successful payment in an external demo alone does not establish that integration.

## Rust custody comparisons

R2 keeps Python as the production reference. `KASSIBER_TEST_CUSTODY_ENGINE=census`
or `shadow` compares the implemented Rust stages on every journal build,
including Python subprocesses. Shadow additionally audits the frozen SQL
composition; census keeps structural read-budget probes on one Python build.
Unported stages are reported as skipped; a recorded discrepancy fails the
session even if a test catches it. Ordinary Python CI shards and the quality
gate enable custody census. A required `custody-shadow` job and
`scripts/python_test_shards.py run-custody-shadow` additionally compare the
production pure flow against frozen SQL composition. The single-build
`test_disjoint_reviewed_routes_limit_observation_reads` budget stays in census
and is deselected only in shadow, which deliberately performs two builds.
A book holding a SQLite BLOB, or a REAL or TEXT transaction quantity, is
compared through the binding's loaded encoding, which runs the product's
loaded path; see [the custody engine](custody-engine.md#complete-finalized-projection-runtime-r2f).
Recorder lookup audits separately consult the full ownership index; recorded
snapshots and mismatch artifacts are created with mode 0600.
The CI matrix uses `run-custody-shadow <shard> --ci` and the same disjoint
overlay and distribution policy as tax shadow; ordered modules run serially.

Both CI lanes and the local quality gate build the test-only native loader in
release mode and set `KASSIBER_TEST_CUSTODY_LOADER_BIN` to `custody-load`. Every
successfully loaded database snapshot also passes its exact v1 byte comparison,
including pending caller writes. Pure stage tests with no connection have no
database to compare. The terminal summary counts native matches; only an
`equal` marker is appended to the disposable census records path plus `.loader`.
The loader hook does not add another Python journal build, so the observation
read-budget test remains enabled in census and retains its shadow exclusion.
Tests that deliberately alter computed Python state after loading, such as the
split-payout refusal regression, declare the exact stage and exception through
`injected_stage_failure`. Their snapshot still passes the loader byte gate;
the summary explicitly counts the injected build's skipped stage comparisons.
Recorder audit tests inject omitted ownership matches at the recorded-artifact
boundary, after comparing the canonical snapshot and production stages.
To reproduce locally, build with `cargo build --release --offline --locked -p
kassiber-journals --example custody-load --features test-harness`, set the hook
to the absolute binary path, then run the existing shard command.

Run a focused comparison with `KASSIBER_TEST_CUSTODY_ENGINE=shadow uv run --locked
python -m pytest tests/test_custody_generated_books.py tests/test_custody_shadow_harness.py -q`.
The seeded comparison covers at least 2,000 raw requests. The [custody engine
reference](custody-engine.md) owns the schema, stage inventory and remaining work.

For explicitly requested local recording, point
`KASSIBER_TEST_CUSTODY_SNAPSHOT_DIR` at an existing absolute disposable directory.
Each envelope's `input` is the exact product-loaded v1 request; no independent
recorder inventory is maintained. Add `KASSIBER_TEST_CUSTODY_SNAPSHOT_VERIFY=1`
to replay requests in fresh processes under another hash seed without a database
image. Recordings contain private accounting evidence and stay local; commit
only synthetic fixtures. Product code reads none of these test switches.

Every newly ported custody stage must supply killed entries in the
[custody mutation catalog](../../crates/custody/scripts/mutations.json). Run
`cargo run -p kassiber-mutation-check -- run custody` after changing ported
behavior or entries; `check` only validates applicability. Mutation runs build
only `kassiber-custody` in a disposable scratch tree.
`scripts/check-ledger-features.sh` also proves byte equality of the ledger with
and without custody's unified JSON features; the Rust CI lane and local gate
run it.

### Custody recordings

The local custody recordings (whole raw-fact requests recorded from test
runs; they stay on the machine that recorded them) are replayed by Rust from
frozen goldens. Each `*.replay.json` holds the exact requests one recording
sends to the engine, the recorded request and its permutation, and the
response Rust gave while the Python reference compared every stage with it
and found no difference. They were frozen once, at the last commit with the
Python reference, from a clean `check-rust --permuted` pass.

```bash
cargo run --release -p kassiber-custody --example custody-replay -- [--self-test] DIR
KASSIBER_CUSTODY_REPLAY_DIR=DIR ./scripts/quality-gate.sh
```

The replay requires every response byte for byte; `--self-test` first proves
a changed golden is reported. After a reviewed custody change,
`KASSIBER_GOLDEN_WRITE=1` rewrites the responses instead of comparing (it is
refused under `CI`); diff the goldens, explain every change, and keep them
only with the owner's sign-off when tax or stored results move.

## Generated test data

Fixtures a generator or recorder produces (Python-produced vectors, oracle
outputs, recorded server replies) are not in this tree. They live on the
orphan branch `t3code/test-data`, and `tests/test-data.lock` pins the commit
and the git blob id of every file this tree's tests read. Small fixtures
people write and read stay next to their tests.

- `scripts/fetch-test-data.sh` fetches the pinned commit from the same
  repository with git, writes each locked file at its path (the `.gitignore`
  block lists them all), and verifies every hash. CI runs it before any Python
  or Rust test; `scripts/quality-gate.sh` refuses to start without it.
- Without the data, pytest stops at once with "run scripts/fetch-test-data.sh",
  and a Rust test build names the missing file; nothing is skipped silently.
  `scripts/fetch-test-data.sh --check` verifies without fetching.
- To change data: run the generator (it writes at the same paths), then
  `scripts/fetch-test-data.sh --publish [new paths]` commits the files to the
  data branch locally and rewrites the lock and the `.gitignore` block. Push
  `t3code/test-data`, and commit the lock in the same PR as the generator or
  code change. A new data file needs its path on the `--publish` line once.
