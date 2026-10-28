# Rust shell

The Rust backend's binary shell, built in stage R1 of
[plan 21](../plan/21-rust-backend.md): the `kassiber` binary, its command
line, the machine envelope, and the daemon's JSONL protocol. The Python
backend stays the reference; this page records how the Rust shell matches
it, what it does not do yet, and where it deliberately differs. The binary now uses
the real R3 store for supported books; the production build contains no fake store.

## Crates

| Crate | Role |
| --- | --- |
| `crates/pytext` | CPython's `str.isspace`/`strip()`, `repr(str)`, `int()`/`float()` parsing, and `json.loads` with the C scanner's messages, from Unicode 14.0.0 tables |
| `crates/envelope` | the machine envelope (`envelope.py`), `AppError`, `derive_kind` and `command_path`, `json_ready`, and the redaction rules of `redaction.py` |
| `crates/runtime` | the capability tables, `version_text()`, the `status` payload, and the store interface ([below](#store-interface)) |
| `crates/cli` | argparse's grammar for every parser (generated), its parsing and usage algorithm, the global flags, `emit`, `commands describe`, and the implemented leaves |
| `crates/daemon` | the JSONL protocol, request routing, the secret-store control channel, and registered service kinds |
| `crates/setup` | local workspace, profile policy, account bucket, context, saved-view, project catalog, book-network and offline-preference services |
| `crates/kassiber` | the binary: arms the egress gate, then runs the CLI |
| `crates/rates` | exact fiat cache, manual/CSV rates, pricing lookup, consent-bound market providers and desktop rate projections |

## Command line

`crates/cli/scripts/gen_parser_spec.py` walks `build_parser()` and writes
every parser's actions, option strings (in argparse's prefix-matching order),
mutually exclusive groups, help sections, and defaults to
`crates/cli/src/spec_data.rs`. The Rust engine follows CPython 3.11's
`_parse_known_args`, `consume_optional`, `consume_positionals`, nargs
patterns, subparser recursion, `HelpFormatter`, and `textwrap`, so every
leaf parses, fails, and prints usage and help as in Python. Run the script
with `--check` after changing the Python grammar; the parity test runs it
too.

The global flags and their order of effects follow `cli/main.py`:

- `--machine` anywhere on the command line turns an argparse error into a
  `usage_error` envelope on stdout with `details.usage`, exit code 2.
  Otherwise argparse prints usage and `kassiber: error: ...` to stderr, exit 2.
  Only the exact token counts (`--machine=1` or `--mach` parse as the flag but
  do not switch the error format).
- `--machine` implies `--non-interactive`; with `--format` other than `json`
  it fails `invalid_flag_combination`, reported on stderr, exit 1.
- `--output` receives success and error envelopes; usage errors always go to
  stdout.
- Output open and parent-directory failures retain Python's `OSError` text,
  including the failing normalized path quoted with Python `repr`. Stream
  write failures have no filename, as in Python.
- Exit codes: 0 success, 1 application error, 2 usage error, 78 when the test
  egress guard is armed on a build without its observer.
- `--operator-broker-server` alone answers `not_implemented: ...` on stderr,
  exit 1. `--migrate-default-state-root` alone performs the rename-only cutover,
  returning 0 without output on success and a typed error on stderr on failure.

Implemented leaves: `init`, `status`, `daemon`, `commands describe`, all six
`rates` leaves (`pairs`, `set`, `latest`, `range`, `sync`, `rebuild`), and every
leaf of `workspaces`, `profiles`, `accounts`, `context`, `views`, `projects`, `networks`, and `offline` (26 setup
leaves; 36 total). Each family registers through its own CLI module. Every
other leaf parses exactly as in Python and then fails `not_implemented`
(`details.component` `cli`, `details.command` its dotted path), before any
project is read or created. Nothing falls back to Python in the product.
The development-only hybrid candidate may select Python before bootstrap.

`commands describe` builds the catalog from the generated tree with the
registry's rules (capability, effect, `needs_database`, scope flags,
`secret_input`, `may_prompt`, arguments). It returns all 313 Python leaves,
including unported commands; a query matching nothing returns `unknown_command`.
Execution support remains separately recorded in `describe.rs::IMPLEMENTED`.
The full catalog and prefix queries match Python byte for byte. The generated
grammar drift check and `tests/rust_port/test_contracts.py` bind the Rust tree
and `tests/fixtures/rust_port/commands_describe.json` to `build_parser()` and
the Python registry annotations.

## Daemon

`kassiber [--data-root PATH] daemon` bootstraps the selected project, then
writes `daemon.ready` with `version` and the kinds it implements, without a
`request_id`. Requests and records follow [the daemon reference](daemon.md):

- one JSON object per line, read as Python's text-mode `readline` reads stdin
  (UTF-8, a line cap of 1,000,000 characters, lines split at `\n` only; on
  Windows `\r\n` and `\r` also end a line, as CPython's universal newlines
  do there); a longer line is drained and answered `request_too_large`;
- malformed JSON (`invalid_json` with the C scanner's message), non-object
  requests, and missing kinds answer as Python does, `request_id: null` where
  Python cannot read one;
- an unclassified kind answers `unsupported_kind`; a classified kind the port
  lacks answers `not_implemented`, before `expected_scope` is examined;
- error messages, hints, and details are redacted with Python's rules;
- `expected_scope` refuses a request meant for another book with
  `stale_context`;
- `status` opens a plaintext book on first use and answers `auth_required`
  for a locked one; `daemon.unlock`, `daemon.lock`, and `daemon.shutdown`
  behave as in Python, taking project ownership before opening and releasing
  it after closing;
- end of input or `daemon.shutdown` closes the book and releases ownership.

The secret-store control channel (`supervisor.ai_secret_store.request` and
`.response`, 15-second wait, other lines set aside in order) is in place for
the AI kinds that will use it.

Redaction uses generated CPython 3.11 / Unicode 14.0.0 word, space, digit,
and SRE case-equivalence tables. Case folding applies to literals and ranges,
never word-boundary assertions. Golden and live subprocess comparisons cover
every secret keyword with generated case-special and boundary-special inputs,
including characters assigned after Unicode 14.0.0. Regex resource failures
withhold the whole string.

The first setup boundary additionally serves `ui.profiles.create`, `.rename`,
`.update`, `.switch`, and `ui.saved_views.list`, `.create`, `.delete`. Their
family modules dispatch through `Book::service` into `kassiber-setup`; they use
the existing lazy unlock, ownership, error redaction and `expected_scope` guards.
The ready handshake advertises this subset in Python's order. Context switching
has no separate daemon kind: the desktop uses `ui.profiles.switch`.

The network continuation also serves all six `ui.networks.*` kinds: binding,
inventory, plan, bind, partition_plan and partition_export. Their services use
the active desktop scope and preserve the same stale-context guards.

Workspace desktop mutations, `ui.profiles.reset_data`, and `ui.profiles.snapshot`
(including its AI read projection) remain unported. This is a partial service
boundary, not complete setup or AI parity. Project desktop kinds, backend
configuration/reveal/probes remain `not_implemented`. Offline mode registers
`ui.network.offline` and `.set` through a local service seam; neither opens or
unlocks the book.
No setup operation opens a network connection, probes a configured provider,
or grants network consent.

The deferred egress kinds are `ui.backends.bitcoinrpc.test`,
`ui.backends.btcpay.test`, `ui.backends.electrum.test`, `ui.backends.http.test`,
`ui.backends.lightning.test`, and `ui.backends.detect_core`.

### Setup parity and remaining work

The shared resolver follows the Python reference at `82f6b77b`: exact ID first,
then SQLite's case-insensitive label lookup, rejecting ambiguous folded labels.
It lives only in `crates/setup/src/references.rs`, including the shell's scope
guard lookup, so the later phase-5 label-resolution change has one owner.
Creating a workspace clears the current profile. Selecting another workspace
clears an incompatible profile, and a failed selection rolls back atomically.

Profiles preserve Python's generic/Austrian policy normalization, explicit
methods, country-controlled global pool scope, default Treasury bucket, and
policy-triggered journal invalidation. An incidental label edit preserves the
stored method and journal input version. Saved views retain scoped CRUD,
opaque filter JSON, Python serialization, duplicate errors and ordering.
All writes and context changes share a transaction; failed operations roll back.

Verification includes 25 Rust tests: all nine `test_saved_views.py` service
cases; method, preservation, region and desktop-payload cases from
`test_profile_policy_label.py`; scope/account regressions; and 71 frozen Python
operations comparing responses, errors, identity consumption and every value
of the affected tables. Identity and time are injected before execution, never
masked afterwards. `crates/setup/scripts/export_vectors.py --check` detects
reference drift. Six checked-in setup mutations are killed by named tests.
The process oracle in `test_rust_shell_parity.py` compares deterministic CLI
bytes and daemon JSONL records, including stale-scope refusal, on real books.
The hybrid routing test exercises every ported CLI leaf and retains an unported
Python call on the same book.

The profile display-label tests and the broader desktop/AI setup snapshots stay
with their unported projections. Full daemon smoke sessions mix other services
and still select Python in the hybrid candidate. An explicit Rust daemon session
exercises all seven registered setup kinds, including successful creations.
The full Python shards remain an orchestrator gate: the sandbox cannot write
Python's account-home ownership runtime or open its loopback fixtures. Focused
setup CLI parity and the native Rust tests do not establish completion of the
then-remaining 21 setup leaves or R3's storage/platform matrix.

### Setup continuation: catalog and projects

The four `projects` commands now use `kassiber-setup::projects`, through
`BookStore::local_service`, without bootstrapping or opening a book. Listing on
a new install creates nothing. Creation preserves catalog order, ID
normalization, explicit paths, duplicate and overwrite refusal, selection and
`--no-select`. Inspection refreshes encryption metadata, including legacy
database filenames and empty files. Project flags retain Python's early
dispatch behavior. This adds only an internal dependency on `kassiber-paths`,
so catalog paths and project IDs keep their existing owner.

Encrypted creation consumes and clears its passphrase descriptor through the
existing secrets crate, enforces Python's 12-character minimum, and writes the
empty SQLCipher container through storage. Book schema/identity initialization
waits for an explicit open. A selected encrypted creation records the
second-resolution UTC project timestamp; `--no-select` preserves last-opened
metadata. Catalogs contain only public project metadata.

Eleven new Rust project tests cover catalog selection, metadata, refusal and
independent encrypted books. Two process tests compare Python and the hybrid
candidate on the same disposable paths: exact stdout/stderr/error bytes and
the complete catalog after each operation, plus descriptor-based encrypted
creation and Python reopening. All eleven setup mutations are killed, including
the five added project/catalog selection mutations; each entry names its tests.

The continuation's local gates pass: fmt, workspace and harness clippy with
`-D warnings`, 485 workspace tests (`--exclude kassiber-net --locked`), harness
build, frozen setup vectors, inventory and contract checks, and all eleven
mutations. Cargo uses the pinned toolchain and `--offline`. The locked Python
3.11 environment was synced first; subsequent tests reuse that environment
with `UV_NO_SYNC=1` because the sandbox cannot populate uv's shared cache.
The hybrid-configured family modules pass 187 tests (plus two subtests); this
includes Python service tests and does not establish Rust execution for
unported families.

The two project process oracles record 18 calls in `KASSIBER_HYBRID_LOG`:

| Command | Rust | Python fallback |
| --- | ---: | ---: |
| `projects.create` | 7 | 0 |
| `projects.list` | 6 | 0 |
| `projects.select` | 2 | 0 |
| `projects.show` | 3 | 0 |

A separate hybrid metadata witness records three `commands.describe` calls,
all served by Rust. The full catalog (313 entries, 644,633 bytes), `wallets`
(39 entries, 96,640 bytes), and `wallets sync` (one entry, 2,492 bytes) are
byte-identical to Python. Native tests compare every catalog prefix, and the
hybrid routing regression requires all 18 implemented setup leaves plus the
unported wallet catalog query to select Rust.

This continuation stops at the catalog/project boundary. Seventeen CLI leaves
remain: networks (5), offline (3), and backends (9). Their desktop kinds,
workspace mutations, profile reset/snapshot/display projections and AI
projections remain unported; network probes remain refused. Project desktop
switching/lifecycle also remains a separate runtime boundary.

The next continuation fixes **legacy metadata permissions** admission in
`kassiber-paths::read_private_json`: owned `0600` and `0644` settings/catalogs
are accepted; group/world-writable files and foreign ownership remain refused.
Native regression coverage exercises all four modes (`0600`, `0644`, `0664`,
`0666`) through both loaders and foreign ownership through the admission policy,
without requiring privileged `chown`. Reads preserve bytes and modes, and the
storage writer's existing private-file test pins new catalog writes to `0600`.

The catalog/project continuation's combined Python shell/hybrid/runner/port suite reported 55 passed,
59 subtests passed and one sandbox failure. The independent hybrid descriptor
test's Python `secrets init` fallback still
tries the account-home ownership runtime (`/home/tg/.kassiber/run`) despite a
disposable `HOME`; the sandbox refuses that write. Its failing result is an
orchestrator verification item, distinct from the legacy metadata refusal.

### Setup continuation: book networks

All five `networks` leaves and six desktop network kinds now share
`kassiber-setup::networks`. Inventory preserves absent and conflicting evidence,
including canonical source columns and importer-specific exceptions. Plans
commit to the complete history, authored declarations and regtest instance;
binding recomputes that preview and invalidates journals atomically. Reads
preserve caller-owned transactions and live cursors.

Partition plans check authored relationship closure, foreign keys and scoped
legacy-ledger blockers. Exports keep authored IDs, public watch policy and
verified attachments, exclude backend credentials and historical whole-book
reports, create a fresh database identity and reset processing state. Encrypted
source books are keyed before copying rows. Staging is private and publication
uses a hard link that refuses an existing destination. The existing secret
wrapper retains the unlocked key only while the book is open. Passphrase
encryption uses `age`; recipient mode retains Python's `age`/`rage` executable
requirement. The standard library and existing workspace crates do not supply
age encryption or tar archives, so this boundary adds `age` and `tar` and moves
the existing `tempfile` dependency into runtime use. See the license inventory.

Twelve native network tests port the book-network, partition and read-snapshot
service contracts, including 136 frozen responses in eight scenarios and 69
evidence vectors. `export_network_vectors.py --check` checks both those vectors
and the authored table scope queries against Python. Import/sync integration
cases in `test_book_network_boundaries.py` remain with their unported services;
the shared guards and cache matching rules have native coverage. AI projections
remain Python. No Python reference bug or product-code change was required.

The real-book process oracle compares deterministic CLI bytes, binds through
Rust, checks Python's subsequent inventory, and restores a Rust encrypted export
with Python. Its hybrid witness records:

| Command | Rust | Python fallback |
| --- | ---: | ---: |
| `networks.inventory` | 2 | 0 |
| `networks.plan` | 2 | 0 |
| `networks.bind` | 2 | 0 |
| `networks.split-plan` | 1 | 0 |
| `networks.split` | 2 | 0 |

The daemon process comparison includes all six network kinds, with reads/plans
and deterministic refused mutations. Native tests and the CLI process oracle
cover successful binding/export. Ten new mutations cover declarations, stale
history, cache instance selection, journal invalidation, partition closure,
legacy rows, raw-payload non-authority, attachment hashes, private watch material
and identity types. Together with the prior eleven, all 21 mutations are killed
by their catalog's named tests.

Verification passes fmt, workspace and harness clippy with `-D warnings`,
498 workspace tests excluding `kassiber-net`, the harness build, frozen network
vectors/scopes, mutation-catalog validation, and inventory/contract checks.
The hybrid-configured network modules pass 34 tests and two subtests; those
modules call Python services directly, so native execution is established by
the separate process oracle above. The requested combined shell/hybrid/runner/
port suite reports 56 passed, 59 subtests passed and one sandbox failure:
Python's unported `secrets init` fallback attempts the account-home ownership
runtime `/home/tg/.kassiber/run` despite disposable `HOME`. That single test
requires orchestrator verification; no product code was changed to bypass it.

### Setup continuation: offline preference

All three `offline` CLI leaves and both `ui.network.offline` desktop kinds
share `kassiber-setup::offline`. The CLI retains Python's layout bootstrap,
without opening the book; desktop operations work while an encrypted book
remains locked. `expected_scope` still uses the daemon's existing guard.
The preference path is machine-wide, including the existing environment
override, and independent of the selected book. The process egress override
is reported separately from the user's persisted switch.

Missing preferences read online; malformed, oversized, unreadable, symlinked
or special files fail closed. The paths crate supplies the bounded,
nonblocking regular-file reader. This preference policy remains distinct
from settings/catalog metadata admission. Storage supplies private atomic
publication, preserving Python's explicit replacement of a final symlink
without following it. New files are `0600` and the parent is `0700` where
supported. The existing `tempfile` package moves into the store's runtime
dependencies for this private publication primitive; no new package or
installation requirement is added.

Seven native service tests and a FIFO reader regression cover preference
serialization, schema and strict boolean validation, environment precedence,
path overrides and file admission. Three new policy mutations are killed by
named tests, bringing the setup catalog to 24. The real-process oracle
compares exact CLI bytes for writes and malformed reads, then compares both
daemon kinds while an encrypted database remains unopened and unchanged.
Its hybrid witness records:

| Command | Rust | Python fallback |
| --- | ---: | ---: |
| `offline.status` | 10 | 0 |
| `offline.on` | 1 | 0 |
| `offline.off` | 2 | 0 |

The final combined shell/hybrid/runner/port suite reports 57 passed and 59
subtests passed, with the same single sandbox failure in the Python
`secrets init` descriptor fallback (`/home/tg/.kassiber/run`). The
hybrid-configured network and offline modules pass 77 tests and two subtests;
their Python service calls remain reference checks, with native execution
established by the process witnesses. Fmt, both clippy configurations with
warnings denied, all 506 workspace tests excluding `kassiber-net`, the harness
build, frozen vectors, inventory/contract checks and the full 24-entry mutation
run pass.

This run adds the following mutations to the existing eleven; every entry
is killed. The complete executable catalog is
[`crates/setup/scripts/mutations.json`](../../crates/setup/scripts/mutations.json).

| Mutation | Named killer test |
| --- | --- |
| `network-unknown-declaration` | `network_unknown_declaration_cannot_override_conflicts_or_other_book_ids` |
| `network-history-preview` | `network_bind_recomputes_history_and_invalidates_journals_atomically` |
| `network-instance-cache` | `network_regtest_cache_requires_exact_instance_and_never_relabels_wallet` |
| `network-journal-invalidation` | `network_bind_recomputes_history_and_invalidates_journals_atomically` |
| `partition-closure` | `partition_closure_and_legacy_rows_block_before_files` |
| `partition-legacy` | `partition_closure_and_legacy_rows_block_before_files` |
| `partition-raw-authority` | `partition_closure_and_legacy_rows_block_before_files` |
| `partition-attachment-hash` | `partition_export_preserves_ids_encryption_attachments_and_source` |
| `partition-private-watch-material` | `partition_public_wallet_material_drops_credentials_and_private_keys` |
| `partition-identity-types` | `partition_identity_types_match_python_without_coercing_numbers` |
| `offline-malformed-policy` | `offline_malformed_or_oversized_preferences_fail_closed` |
| `offline-independent-switch` | `offline_environment_override_does_not_replace_the_users_switch` |
| `offline-strict-boolean` | `offline_desktop_validation_is_strict_and_never_writes_on_error` |

Nine setup CLI leaves remain: backend configuration/list/kinds/reveal and
default selection, plus their desktop kinds, workspace desktop mutations,
profile reset/snapshot/display projections and AI projections. Network probes
remain refused. The shared network gate remains unavailable until a later
transport slice supplies the persisted preference reader and consent-bound
HTTP client; this setup port adds no egress path.

## Store interface

`crates/runtime/src/store.rs` is the contract the shell needs from the
storage crates, in the shapes Python's runtime, `db`, `projects`, and
`operator.project` modules give `cli/main.py` and `daemon.py`:

- `migrate_default_state_root()`: `migrate_hidden_home_state_root_if_needed`,
  which the shell calls where `_maybe_migrate_default_state_root` does
  (without `--data-root` or `--env-file`, before broker routing and
  bootstrap);
- `bootstrap(selection)`: `bootstrap_runtime(args, needs_db=False)`, returning
  the paths and the runtime configuration loaded from the env file;
- `effective_unlock_mode(selection)` for broker routing;
- `database_is_encrypted(data_root)`, `canonical_project(data_root)`, and
  `acquire_ownership(project, owner_kind, generation)` returning a
  `ProjectOwner` whose `release` can be retried;
- `open_book(data_root, Unlock::Cli { passphrase_fd, allow_prompt,
  expected_database_identity, initialize_version })` with the CLI's unlock chain and
  `interaction_required`, or `Unlock::Daemon { passphrase,
  require_existing_schema, expected_project_identity }` failing
  `passphrase_required` or `unlock_failed`;
  both run the post-unlock migration check and merge the book's backends into
  the caller's runtime configuration, which `status` reports (`env_file`,
  `default_backend`);
- `verify_passphrase(data_root, scope, passphrase)` under the auth backoff;
- `mark_project_opened(project_id, data_root, select)`;
- `Book`: `is_initialized`, `context` (empty strings when no workspace or
  profile is selected, as `current_context_snapshot` returns them),
  `status_facts`, `resolve_scope`, `rollback`, and a retryable `close`.

The binary implements this trait in `crates/kassiber/src/real_store.rs` over
`kassiber-paths`, `kassiber-store`, `kassiber-ownership` and `kassiber-secrets`.
`runtime` owns transport contracts and status projections; `paths` is the sole
implementation of root resolution. There is no second path resolver.

Explicit command bootstrap ensures the default project (including legacy flat
project migration) and publishes the durable runtime layout. `init` creates the
Python schema and persists bootstrap backends and `app_version`; ordinary
`status` creates an absent schema as Python does, then reports `not_initialized`
until init or a legacy workspace establishes initialization. Neither action
contacts a configured backend. The default hidden-home cutover precedes broker
routing when no explicit data root/env file bypasses it.
Legacy project copying refuses an existing held local owner reservation and
nonempty source WAL/rollback journals. Full concurrent-copy acceptance remains
part of the open R3 durability matrix.

`Book::open_migrating` authenticates file/database bindings before the ordered
compatibility pass, then checks structural and semantic admission. Books that
need authored pair/payout consolidation or componentless candidate matching
return `unsupported_schema_layout`, with a Python-build migration hint. No
custody interpretation is guessed. Existing journals never change mode through
this API; an unsafe mode is refused. New books conservatively use DELETE/FULL.
The admission-only `Book::open` remains a separate library API.

CLI fd input is bounded, consumed once, closed and zeroized by `UnlockSession`.
Its remaining chain is bound unattended remembered unlock, then a no-echo TTY
prompt when allowed; noninteractive manual unlock returns `interaction_required`.
The binary enables `native-keyring`; manual/brokered/unbound enrollments do not
contact it. The daemon accepts the desktop's `auth_response.passphrase_secret`,
checks it on a throwaway read-only connection under shared persisted auth
backoff, then opens/migrates under its ownership lease. Lock/shutdown close the
connection before releasing ownership. A first daemon-created inode is bound
while retaining its original path reservations. The provider secret-store
reverse control channel remains available to its existing protocol consumers;
it is not a generic database-passphrase or filesystem dispatch API.

## Tests

| What | Command |
| --- | --- |
| Crate tests, including the CPython and Python golden vectors | `cargo test --workspace` |
| The binary under the desktop supervisor's routing rules | `cargo test -p kassiber --features test-harness` |
| Python against the binary, same argv and daemon input | `cargo build -p kassiber --features test-harness`, then `KASSIBER_RUST_BIN=target/debug/kassiber uv run --locked python -m pytest tests/test_rust_shell_parity.py` |

The `test-harness` feature, never in a release build, adds the egress attempt
observer, the daemon's protocol test kinds (`test.stream`, `test.emit_event`,
`test.late`, `test.secret_store`), and an in-memory store selected by
`KASSIBER_RUST_TEST_FAKE_STORE` (a JSON object: `data_root`, `encrypted`,
`passphrase`, `initialized`, `project_id`, `brokered`, `events_file`, ...).
The Python suite arms `KASSIBER_TEST_NO_EGRESS`, so it needs that build.
Absent the fake-store variable, even a harness build uses the real store.
The fake remains for `crates/kassiber/tests/supervisor_protocol.rs`,
`crates/daemon/tests/protocol.rs` and `crates/cli/tests/run.rs`: deterministic
scope/state, ownership trace and close/release failure injection while testing
streaming, routing, cancellation, stale scope and reverse-control behavior.
Python protocol parity and the new real-book tests use the real adapter.
Harness-only namespace and migration-clock/identity inputs are explicit test
injections, compiled out of production. They do not redirect production locks
or authorize native credential access.
The required CI `serial-daemon` shard and local quality gate build this harness
and supply its absolute path as `KASSIBER_RUST_BIN` before the Python pass.
Under `KASSIBER_CI=1`, a missing binary setting fails collection instead of
silently skipping subprocess comparisons.
The Python reference starts through `tests.cli_runner` with `reference=True`;
`KASSIBER_TEST_CLI` cannot redirect it to the candidate. Stable CLI stdout,
stderr, saved JSON output, and daemon JSONL records compare as raw bytes,
including indentation, separators, key order, and line endings. These scenarios
normalize no nondeterministic fields. The complete catalog compares directly
with Python; `daemon.ready` compares against Python projected onto the
implemented kind subset.
Child wrappers inject indentation, key-order, and separator changes to prove
the oracle rejects them, including usage and caught output errors.

Golden data and the scripts that write it, all run with the locked Python
3.11:

| Data | Script |
| --- | --- |
| `crates/pytext/src/unicode_tables.rs` | `crates/pytext/scripts/gen_unicode_tables.py` |
| `crates/pytext/tests/data/vectors.txt` | `crates/pytext/scripts/gen_vectors.py` |
| `crates/envelope/tests/data/golden.txt` | `crates/envelope/scripts/gen_golden.py` |
| `crates/cli/src/spec_data.rs` | `crates/cli/scripts/gen_parser_spec.py` |
| `crates/cli/tests/data/argparse_cases.txt` | `crates/cli/scripts/gen_cli_fixtures.py` |
| `crates/cli/tests/data/render_cases.txt` | `crates/cli/scripts/gen_render_cases.py` |
| `crates/runtime/tests/data/capabilities.json` | `crates/runtime/scripts/gen_capabilities.py` |

## Rates service checkpoint (R5, partial)

`crates/rates` owns the shared service independently of either adapter.
`cached_at_or_before` is the pricing/journal lookup API; `latest`, `range`
and `pairs` are local reads. `Sample`/`upsert` retain exact Decimal strings
alongside legacy REAL columns, using `pydecimal` and `pyjson`. Low-level writes
participate in the caller's transaction; `command` supplies the CLI transaction
boundary, including rebuild rollback. `ensure_bundled_seed` imports embedded
Kraken daily/hourly data only when that pair/granularity has no seed. Cache
keys remain `(pair,timestamp,source)` and checked-minute keys also include the
source. This Python base has no rate TTL or interpolation.

Latest selection orders by timestamp, manual preference, fetched time, then
source. Pricing lookup has the same first three terms and intentionally no
source or id tie-break in this base. The insertion tie is independently pinned
against Python. Range ordering, confirmed-time transaction windows, sparse
checked minutes, 300-candle boundaries, finest CSV interval and LBTC-to-BTC
fallback have native tests and the mutation catalog below.

The family modules in `crates/cli` and `crates/daemon` delegate through
`Book::rates`. The daemon advertises `ui.rates.summary`, `ui.rates.coverage`,
`ui.rates.kraken_csv.import` and `ui.rates.latest`. Reads and local imports
do not fetch. Latest requires the active book's market-rate opt-in before
minting an exact-action grant. The [egress reference](rust-port-egress.md#rates-service-paths-r5)
lists each provider path and its no-traffic regression. AI projections remain
in Python.

This checkpoint is **not complete slice acceptance**:

- `ui.rates.rebuild` returns `not_implemented` before mutation or egress and is
  not advertised. Its repricing/journal composition depends on unported services.
- Native tests cover the cache/providers/CSV core, but the entire `rust-test`
  intent inventory is not yet ported. In-process Python daemon tests do not
  prove Rust execution. In particular the journal rollback, desktop rebuild
  and full transaction-pricing preservation scenarios remain open.
- HTTP `Retry-After` currently accepts integer seconds, not HTTP dates. Generic
  upstream error wording still differs: Rust withholds peer response text.
  Nonfinite Decimal inputs fail closed. Full provider error parity remains open.
- Local ZIP ingestion supports stored/DEFLATE ZIP/ZIP64, UTF-8 filenames and a
  1 GiB uncompressed member limit. Python accepts additional compression/name
  encodings. Hostile archive and full-history resource-limit acceptance remains
  open. Bundled data are embedded, but the returned resource display path still
  names the build checkout; installed-binary path parity remains open.
- Desktop context resolution currently expects saved IDs; Python also accepts
  label references. Coverage rejects mismatched workspace/profile IDs and
  latest refuses them before egress. Automatic scheduling is unported: only
  the scoped enabled-refresh service API exists.

`tests/test_rust_rates_candidate.py` explicitly records Rust service for all six
CLI leaves and compares deterministic native daemon summary/coverage/validation
records with Python as raw bytes. The existing CSV subprocess cases and rate
CLI smoke scenarios also run through the hybrid candidate. Direct Python mock
tests remain reference checks until their native replacements are complete.
The native daemon scenario also proves that a failed second CSV member rolls
back the first member's pending writes before the next request.

Checkpoint verification on Linux used the locked Python 3.11 environment and
Rust 1.99.1, with Cargo offline. Workspace tests, both workspace and harness
clippy (`-D warnings`), fmt, the harness build, inventory/contract/shard checks
and all nine mutations passed. The rates modules plus sequential CLI smoke
class passed 107 tests; five targeted rates regressions also passed. The
required shell/hybrid/runner/port suite passed 53 tests and 46 subtests. Native
rates tests passed 27 tests. Loopback tests needed execution outside the socket
sandbox and passed there; no real provider was used. This does not establish
the full quality gate or the deferred slice gates above.

The successful hybrid acceptance log recorded these Rust-served calls (zero
rates fallbacks): `rates.pairs` 2, `rates.set` 18, `rates.latest` 3,
`rates.range` 3, `rates.sync` 13 and `rates.rebuild` 1. A separate candidate
scenario asserts native execution for each leaf; daemon readiness advertises
only the four supported rates kinds.
The final candidate module passes three scenarios, including pending-import
rollback. Native service methods reject grants for a different method, and
enabled background Coinbase/mempool sync cannot warm an idle cache.

The R2-style checker at `crates/rates/scripts/mutation_check.py` uses disposable
scratch trees. All nine entries were killed by their named tests:

| Mutation | Killer in `crates/rates/src/tests.rs` |
| --- | --- |
| `latest-manual-priority` | `latest_manual_then_fetched_then_source_ties_match_base` |
| `lookup-no-future` | `lookup_excludes_future_and_preserves_insertion_tie` |
| `manual-positive` | `csv_missing_path_errors_and_manual_validation` |
| `csv-finest-interval` | `csv_selection_chooses_finest_and_first_duplicate` |
| `coinbase-midnight-window` | `windows_respect_midnight_max_candles_and_future` |
| `cached-minute-selection` | `coinbase_transaction_need_no_traffic_before_consent_and_incremental_cache` |
| `checked-minute-selection` | `coinbase_empty_window_is_checked_without_refetch` |
| `background-no-idle-warming` | `action_grants_cannot_relabel_fetches_or_warm_background_cache` |
| `lbtc-fallback` | `normalized_provider_fallback_and_asset_aliases` |

Python reference fix: Kraken's OHLC validator used truthiness and accepted
negative prices. `tests/test_rates_csv_validation.py` failed for each of the
four columns before the separate fix and passes after it. Rust pins the same
positive-price rule.

## Not yet ported

`--diagnostics-out` fails `not_implemented` instead of writing its report;
`--debug` adds no traceback; there is no cached update banner, no RAM log
ring or request log, and no operator broker or broker child. A bound brokered
project's commands fail `not_implemented`.

## Reviewed differences

Each is a proposal for review under plan 21's parity rule:

- Legacy project copying refuses an existing held local owner reservation or
  nonempty WAL/rollback journal; Python's copy path lacks these checks. Close
  and checkpoint the source before retrying Rust. This guard does not establish
  full concurrent-copy acceptance.
- `commands describe` resolves no project and creates no layout; Python
  bootstraps (creating the default project) before describing. Only the
  `--project`/`--data-root` conflict is still refused.
- Where Python hangs or crashes, the Rust daemon answers: a request line that
  is not UTF-8 gets `invalid_json` (Python's reader thread dies); JSON nested
  beyond 1,000 levels or an int over 4,300 digits gets `invalid_json` with
  Python's exception text (Python's loop crashes); end of input while
  draining an oversized line, or while the control channel waits, stops the
  daemon (Python polls forever); a second control-channel wait in one request
  examines only new lines (Python spins on its own deferred lines until the
  timeout).
- A `daemon.unlock` passphrase holding lone surrogates is denied
  (`local_auth_denied`); Python fails encoding it.
- Command-line arguments that are not UTF-8 are refused with exit 2; Python
  carries them as surrogate escapes.
- Without `COLUMNS`, usage and help wrap at 80 columns even on a terminal;
  Python asks the terminal for its width. Pipes, which agents and the desktop
  use, wrap at 80 in both.
- Named-user `--output ~user/...`, and current-user forms without a home
  environment variable, fail `internal_error` with `Could not determine home
  directory.` before filesystem changes. R1 does not consult passwd/NSS or
  directory services; refusing these forms prevents writes to literal tilde
  directories. `~` and `~/...` use `HOME` on Unix; Windows ignores `HOME` and
  uses `USERPROFILE`, then `HOMEDRIVE` + `HOMEPATH`, and accepts `~\...` too.
- A panic in a daemon handler answers `internal_error` with the panic message
  and no `debug` text.
