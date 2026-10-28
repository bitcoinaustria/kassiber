# Rust shell

The Rust backend's binary shell, built in stage R1 of
[plan 21](../plan/21-rust-backend.md): the `kassiber` binary, its command
line, the machine envelope, and the daemon's JSONL protocol. The Python
backend stays the reference; this page records how the Rust shell matches
it, what it does not do yet, and where it deliberately differs. The binary now uses
the real R3 store for supported books; the production build contains no fake store.

The [combined R5 integration audit](rust-backend-r5-integration.md) records the
stacked commit provenance, conflict resolutions, dependency pins and final
verification, including per-command Rust routing counts.

## Crates

| Crate | Role |
| --- | --- |
| `crates/pytext` | CPython's `str.isspace`/`strip()`, `repr(str)`, `int()`/`float()` parsing, ISO datetime parsing, and `json.loads` with the C scanner's messages, from Unicode 14.0.0 tables |
| `crates/envelope` | the machine envelope (`envelope.py`), `AppError`, `derive_kind` and `command_path`, `json_ready`, and the redaction rules of `redaction.py` |
| `crates/runtime` | the capability tables, `version_text()`, the `status` payload, and the store interface ([below](#store-interface)) |
| `crates/cli` | argparse's grammar for every parser (generated), its parsing and usage algorithm, the global flags, `emit`, `commands describe`, and the implemented leaves |
| `crates/daemon` | the JSONL protocol, request routing, the secret-store control channel, and registered service kinds |
| `crates/setup` | local workspace, profile policy, account bucket, context, saved-view, project catalog, book-network, offline-preference and backend-configuration services |
| `crates/kassiber` | the binary: arms the egress gate, then runs the CLI |
| `crates/rates` | exact fiat cache, manual/CSV rates, pricing lookup, consent-bound market providers and desktop rate projections |
| `crates/imports` | import-run listing, rollback plans and confirmed deletion |
| `crates/wallets` | local public descriptor parsing, derivation and preview |
| `crates/transactions` | local loan marks, managed attachment evidence, metadata edits, BIP329 labels and append-only history shared by both transports |

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

BIP329 preview/import and wallet-scoped stored export use the currently ported local
address/inventory/Core-receive index. Descriptor-backed matching and complex
input graphs fail `not_implemented`; standard string fields containing lone
surrogates also refuse explicitly. Unknown JSON fields, including surrogate
keys/values, are preserved. Global stored and synthesized exports do not need
that index. This is a partial BIP329 acceptance boundary.

The [R5 local evidence boundary](rust-transactions-r5-status.md) records storage
invariants, parity evidence and remaining work.

Implemented leaves: `init`, `status`, `daemon`, `commands describe`, all six
`rates` leaves (`pairs`, `set`, `latest`, `range`, `sync`, `rebuild`), all four
`loans` and six `attachments` commands, 26 local `metadata` leaves (notes/tags,
inclusion, kind overrides, record get/list, stale edits, history list/activity
forward-edit reverts and four BIP329 leaves), `imports list`, `imports rollback`, `wallets derive`,
and every leaf of
`workspaces`, `profiles`, `accounts`, `context`, `views`, `projects`, `networks`,
`offline` and `backends` (35 setup leaves; 84 total). Each family registers through its own
CLI module. Every other leaf parses exactly as in Python and then fails
`not_implemented`
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

## Import-run checkpoint (R5)

`crates/imports` ports `core/import_batches.py` run listing, rollback planning
and confirmed deletion. At that checkpoint, `crates/wallets` supplied the
profile-scoped wallet resolver and shared journal invalidation. The later
descriptor checkpoint below adds local derivation.
The CLI and daemon each register the imports family through their own module,
using the same domain service on the real store's admitted connection. No new
third-party dependency, transport or network constructor is added. Existing AI
allowlists and projections are unchanged; neither kind grants generic dispatch
or access to wallet configuration or material.

The CLI preserves implicit planning without `--confirm`, the distinct
`imports.rollback.plan` envelope and `--dry-run` precedence. The daemon kinds
`ui.imports.list` and `ui.imports.rollback` preserve active-book scope checks,
`confirm='DELETE'` and `applied`. Like Python, a successfully applied daemon
rollback writes its final response and closes the daemon; planning and errors
leave it running. Python-only internal-error traceback text is not fabricated
by Rust; expected application errors and successful records compare as bytes.

Only rows created by the selected run are deleted. Enriched pre-existing rows
and other runs remain. Collateral counts follow the schema's CASCADE graph,
including transitive paths; repeated paths count each row once and SET NULL
references are excluded. IDs are staged in an in-memory SQLite temp table,
without a parameter per transaction. Rollback and journal invalidation share a
savepoint, including recovery from a failed cascade/invalidation. Empty runs
do not change journal inputs. The owning profile's input version advances after
nonempty deletion, preserving report-readiness barriers.

`tests/test_import_batches.py` runs through the hybrid candidate; its three
`RollbackScaleTests` cases have native replacements in
`crates/imports/src/tests.rs`, including a 33,000-row case. Real-book CLI/daemon
comparisons in `tests/test_rust_shell_parity.py` compare wire bytes and complete
post-rollback SQLite/SQLCipher states. `crates/imports/scripts/mutations.json`
uses R2's scratch-tree checker and names a killing test for every selection,
collateral and invalidation mutation.

Checkpoint verification passed: 456 workspace Rust tests (excluding
`kassiber-net`), 14 binary/harness tests, both fmt/clippy gates, the harness
build, and the inventory/contract checks. The mandatory shell/hybrid/runner
suite passed 56 tests and 53 subtests. The broader eight-module wallet/import
hybrid run passed 202 tests and 26 subtests; its fresh routing log records
`imports.list: rust=4, python=0` and `imports.rollback: rust=4, python=0`.
Wallet/importer calls in that run used Python, so those results do not establish
native wallet or file-parser parity.

The checkpoint's mutation evidence (all killed):

| Mutation entries | Named Rust test |
| --- | --- |
| `I-batch-membership`, `I-journal-version` | `rollback_large_run_uses_staged_ids_and_preserves_enriched_rows` |
| `I-profile-scope`, `I-atomic-failure` | `rollback_is_profile_bound_and_failures_are_atomic` |
| `I-cascade-only`, `I-cascade-union`, `I-cascade-transitive`, `I-bookkeeping` | `collateral_counts_once_and_follows_transitive_cascades` |
| `I-empty-invalidation` | `empty_run_does_not_invalidate_and_lists_actual_remaining_rows` |
| `I-remaining-count` | `list_is_ordered_scoped_and_filters_wallet_without_exposing_config` |
| `I-limit-type` | `python_list_limits_preserve_zero_types_and_integer_bounds` |
| `W-resolver-scope`, `W-resolver-id-first` | `resolver_prefers_id_refuses_ambiguity_and_scopes_profiles` |
| `I-json-recursion` | `column_map_decode_errors_are_null_but_recursion_errors_fail_closed` |

This is a partial wallets/file-import slice. The remaining 38 `wallets` leaves
answer `not_implemented`: CRUD/list/get, kinds, reveal-descriptor, identify,
file analysis/templates/document preview/import, JSON/CSV and every dedicated
importer, Samourai, attachment/enrichment routes, and all sync commands.
Descriptor parsing and local derivation are covered below. Native file parsing,
normalization, integer-msat/timestamp conversion, deduplication and batch creation
also remain. `wallets sync` and the provider-specific sync commands belong to
the later network slice. Hybrid setup/import calls still run in Python and are
not evidence of native importer execution. The exact unported leaves are:

```text
wallets list / create / kinds / get / update / delete / reveal-descriptor
wallets import-json / import-csv / import-btcpay / import-phoenix / import-wasabi
wallets import-river / import-bull / import-bullbitcoin / import-coinfinity
wallets import-21bitcoin / import-pocket / import-pocketbitcoin / import-strike
wallets import-ledger-live / import-cointracking / import-blockpit
wallets import-binance-supplemental / import-ledger / analyze-file / ledger-template
wallets preview-document / import-document / import-samourai
wallets attach-btcpay / attach-bullbitcoin-wallet / identify
wallets sync-kraken / sync-coinbase / sync-binance / sync-btcpay / sync
```

## Wallet descriptor checkpoint (R5)

`wallets derive` and `ui.wallets.preview_descriptor` now run in Rust. The
network-free service in `crates/wallets` uses `bitcoin`, `miniscript`, and
`elements-miniscript` for public descriptor parsing, script construction,
BIP32 derivation and confidential addresses. CLI derivation reads existing
wallet rows without changing them. The preview uses no wallet configuration
write, provider lookup or network capability. AI projections remain in Python.

The checked-in synthetic Python oracle contains 124 descriptor plans, 43
wallet-material inputs, 3 normalizations, 165 previews and 32 branch selections.
It covers pkh, wpkh, sh(wpkh), tr including script trees, wsh multi/sortedmulti,
sh(wsh(sortedmulti)) and sh(multi); fixed suffixes, explicit change pairs,
receive/change multipath, key origins, fingerprints, checksums, gaps and
network refusals. Liquid vectors cover mainnet, testnet and Elements regtest,
SLIP77 WIF/hex view keys, scripts and both confidential/unconfidential addresses.
Each ranged plan samples indices 0–4 on both branches. Material intake covers
Core JSON, Sparrow text, BSMS, xpub/ypub/zpub/tpub/upub/vpub, and the six
corresponding private versions with secret-free refusals.

Generate or check the oracle with
`uv run --locked python crates/wallets/scripts/generate_vectors.py --check`.
`python_descriptor_vectors`, `python_material_vectors`,
`python_normalization_and_branch_vectors` and `python_preview_vectors` compare
the native computation with it. A separate native service test pins profile
scope, read-only behavior and range refusals. CLI and daemon subprocess tests
compare wire bytes, including ordered target/preview fields, on disposable
books with the no-egress guard armed.

Checkpoint verification passed: workspace and binary formatting, workspace
clippy and `-p kassiber --features test-harness` clippy with `-D warnings`,
479 workspace Rust tests, 14 binary/harness tests and the final harness build.
The mandatory shell/hybrid/runner/inventory suite passed 58 tests and 162
subtests; the focused Python reference/regression suite passed 189 tests and
85 subtests. The hybrid descriptor/setup/CLI scenarios passed 141 tests and
32 subtests. Their routing log records `wallets.derive: rust=2, python=0`
and `daemon: rust=1, python=0`; that daemon served six preview requests checked
against the vectors. Across that run, setup and unported operations remained
explicit fallbacks (48 Rust-served calls, 268 Python-served calls, including
46 Python wallet creations). Four network crate socket-fixture tests could not
bind under the sandbox; the complete workspace run passed with loopback access.

The vectors uncovered three Python reference bugs, each fixed separately with
a regression: fixed non-ranged xpub suffixes were skipped; Liquid Taproot used
Bitcoin tweak/leaf/branch tags and discarded script trees; and confidential
Taproot addresses used Blech32 instead of Blech32m. Liquid address decoding
now also preserves the witness version. The key-path script and address were
independently checked with LWK.

All entries in `crates/wallets/scripts/mutations.json` were killed using R2's
scratch-tree checker:

| Mutation entries | Named Rust test |
| --- | --- |
| `W-private-spend`, `W-network-family`, `W-gap-limit`, `W-change-synthesis`, `W-fixed-suffix`, `W-fingerprint-gap`, `W-liquid-network`, `W-origin-metadata` | `python_descriptor_vectors` |
| `W-slip132-private` | `python_material_vectors` |
| `W-checksum-normalization`, `W-qualified-branch` | `python_normalization_and_branch_vectors` |
| `W-preview-count` | `python_preview_vectors` |

This checkpoint does **not** complete the wallet service slice. Create,
list/get, update/delete, stored public policies/private-presence flags, policy
epochs, history/network guards, freshness invalidation and desktop mutation
reauthentication remain unported. The 41 wallet-configuration vectors freeze
that layout but have no Rust implementation yet. Descriptor error parity
outside the frozen refusal cases also needs a broader malformed-input audit;
the parser-error adapter and preflight are not a complete embit replacement.
Importers, sync, network discovery and journals remain disabled. The full
wallet group's subprocess and `rust-test` dispositions therefore remain open.

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

R5 also registers the four `ui.loans.*` kinds and the six
`ui.attachments.*` kinds (including managed evidence copy and local open-target
resolution), plus `ui.activity.stale`, `ui.activity.history`,
`ui.transactions.history`, `ui.transactions.history.revert`, and
`ui.metadata.bip329.preview/import/export`. BIP329 daemon exports use the resolved
managed reports directory; URLs in labels remain literal local evidence.
Metadata writes append Python-compatible edit events and fields atomically with
journal-input invalidation; no-op edits append nothing. Reverts append forward edits and
preserve the active-custody exclusion guard. The grouped metadata update remains
unregistered until its complete pricing/review contract is ported. The three BIP329 kinds share the CLI
service and its explicit unsupported-index barriers. AI projections stay Python.
Per-family modules call the same domain services as the CLI;
unlock, ownership and expected-book checks still precede execution. URL
attachments never trigger a fetch. Unregistered kinds still fail closed.

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

Workspace create/rename/delete, `ui.profiles.reset_data`, desktop
`ui.profiles.snapshot` and owner-only `backends.reveal_token` are also registered.
Destructive actions and reveal require a fresh local-auth round trip after
book unlock. This is a partial service boundary, not complete setup or AI parity.
The seven backend configuration kinds are registered; AI projections,
project desktop kinds and backend probes remain unported. Offline mode registers
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

The first boundary left profile display labels and desktop/AI setup snapshots
unported; the desktop continuation below ports the local display projection.
Full daemon smoke sessions mix other services
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

The backend continuation below completes the setup CLI leaves. Workspace
desktop mutations, profile reset/snapshot/display projections and AI
projections remain unported. Network probes remain refused. The shared network
gate remains unavailable until a later
transport slice supplies the persisted preference reader and consent-bound
HTTP client; this setup port adds no egress path.

### Backend configuration continuation

All nine `backends` CLI leaves and seven desktop configuration kinds now use
the shared setup service: `ui.backends.options`, `public_defaults`,
`settings.list`, `create`, `update`, `delete`, and `set_default`. The CLI uses
Python's exact names, including `get`, `delete`, and `reveal-token`. This brings
setup to 35 CLI leaves and 22 desktop kinds. The AI projection
`ui.backends.list` stays in Python. All six probe/detection kinds listed above
remain `not_implemented`. The desktop continuation below adds creation with a
Core `credential_ref` by re-resolving current local candidates, as Python does.

Credentials retain Python's columns and normalized `config_json` inside the
encrypted `backends` table. Existing secrets and JSON crates provide the
descriptor inputs and codecs; no package or runtime requirement is added.
Normal projections use the Python allowlist, secret-presence flags and URL/
proxy redaction. Unparsed locations follow the refined Python bare-host fullmatch:
ASCII hostname/IPv4 or bracketed hex/colon/dot text, optionally followed by
a numeric port of one to five digits. Other unparsed locations are redacted.
Only explicit `reveal-token`, after book unlock, returns raw
credentials. Bootstrap promotion never persists ephemeral process overrides;
default replacement remains on-chain and prefers user sync backends. Wallet
and BTCPay provenance references block deletion. Updates invalidate the named
backend's derived output inventory; deletion detaches account routes and their
freshness state. No service constructs a transport client or probes a backend.

Nine native tests include 94 frozen Python responses and affected-table
states. The byte-comparison process oracle covers CLI fd/stdin/inline secret
input, all configuration desktop kinds, status after default edits and refused
probes. It also compares malformed URL/proxy edits and subsequent reads. Its
hybrid witness contains 15 calls, all served by Rust:

| Command | Rust | Python fallback |
| --- | ---: | ---: |
| `backends.list` | 2 | 0 |
| `backends.kinds` | 1 | 0 |
| `backends.get` | 3 | 0 |
| `backends.create` | 1 | 0 |
| `backends.update` | 4 | 0 |
| `backends.delete` | 1 | 0 |
| `backends.set-default` | 1 | 0 |
| `backends.clear-default` | 1 | 0 |
| `backends.reveal-token` | 1 | 0 |

The broader hybrid backend regressions pass 32 tests; the CLI smoke selection
passes two tests, and desktop backend smoke passes eleven tests plus eight
subtests using the harness's disposable owner-lock root. Eight new mutations
extend the catalog to 32, all killed:

| Mutation | Named killer test |
| --- | --- |
| `backend-default-chain` | `backend_default_replacement_stays_on_chain_and_prefers_user_sync_backends` |
| `backend-default-sync-kind` | `backend_default_replacement_stays_on_chain_and_prefers_user_sync_backends` |
| `backend-default-canonical` | `backend_environment_overrides_are_ephemeral_and_cannot_be_deleted_or_defaulted` |
| `backend-wallet-references` | `backend_wallet_and_btcpay_provenance_references_block_deletion` |
| `backend-output-allowlist` | `backend_secrets_are_stored_exactly_and_allowlisted_output_never_discloses_them` |
| `backend-url-redaction` | `backend_secrets_are_stored_exactly_and_allowlisted_output_never_discloses_them` |
| `backend-malformed-url-redaction` | `malformed_backend_locations_never_echo_embedded_credentials` |
| `backend-deletion-tombstone` | `backend_deleted_presets_stay_deleted_after_restart_and_seed` |

The process oracle found a Python reference bug: successful desktop backend
create/update/delete returned the daemon's shutdown flag. A separate commit
changes all three to keep the daemon running, with a failing-before regression
that checks each edit and the following read.
The privacy review found a second Python bug: malformed URL/proxy authorities
were returned unchanged, and invalid ports could raise exceptions containing
input text. Its separate reference fix and failing-before regression replace
malformed locations with `<redacted>`. Rust mirrors it with native/process
regressions and a killed mutation. Both implementations preserve stored values.

Fmt, workspace and harness clippy with warnings denied, the harness build,
frozen backend vectors and mutation execution pass. The full workspace test
run is blocked by four existing `kassiber-net` loopback socket tests with
`EPERM`; all other crates pass (515 tests).
The combined shell/hybrid/runner/port suite reports 58 passed and 59 subtests
passed, with one sandbox failure in Python's unported `secrets init` descriptor
fallback when its production owner namespace tries to write
`/home/tg/.kassiber/run`. Inventory and contract checks pass separately.

### Setup desktop continuation

The local setup boundary now serves workspace create/rename/delete, profile
reset and desktop snapshot, and owner-only `backends.reveal_token`. Setup has
27 desktop kinds plus this owner daemon kind; its 35 CLI leaves are unchanged.
Workspace rename requires an exact ID; deletion targets the current workspace.
Destructive confirmations require the exact current workspace/profile name.
Reset keeps the profile, accounts, wallets, backends and other books, preserves
shared rates unless explicitly cleared, and invalidates the journal. Retained
scoped or unscoped legacy-ledger rows block destructive actions. Managed
attachment cleanup runs after the database commit and stays under the resolved
attachment root. Frozen vectors compare responses and affected stored rows,
including protected custody purges and rate-cache preservation.

The snapshot reports actual stored tax methods (including Austrian FIFO),
active selection, account/wallet counts, jurisdiction/currency summaries and
relative timestamps. Custody's CPython ISO parser now lives in `kassiber-pytext`
and serves both projections without changing custody's UTC policy. The
deterministic UUID and clock sources used by process tests are injected before
execution and available only in harness builds.

Sensitive daemon actions take desktop ownership and require a fresh passphrase
verification with the existing backoff, even after unlock. Plaintext books
require exact `DELETE LOCAL DATA` or `COPY LOCAL SECRET` acknowledgements.
The owner-only reveal kind retains Python's target precedence and error
redaction; desktop/AI allowlists are unchanged. Wrong-passphrase verification
also suppresses SQLCipher's native stderr diagnostics, matching Python's
`quiet_unlock_errors` and leaving the typed auth failure on the protocol.

Backend creation accepts Core `credential_ref` values by re-reading current
local candidates from `~/.bitcoin`. Python does this too: references are opaque
candidate hashes, not a session credential cache. Identity includes username
and excludes password, so password rotation preserves a valid reference but a
changed candidate identity invalidates it. Local-only selection, network alias
precedence, cookie policy and deduplication match the reference. Creation binds
the resolved endpoint/network and removes conflicting auth fields. These are
explicit local reads; no setup service probes or connects to the endpoint.
Core detection and all six deferred backend probe/detection kinds remain
`not_implemented`, pending the later HTTPS/transport slice.

The bare-host refinement was cherry-picked as its own Python reference commit;
Rust mirrors it. No additional Python reference bug was found in this run.
Only internal crate dependencies were added (daemon to setup, custody to
pytext); there is no new third-party package or installation requirement.

Three hybrid daemon sessions compare plaintext acknowledgements, encrypted
reauthorization and current Core references byte for byte. Their successful
witness records 36 requests with zero Python fallbacks:

| Kind | Rust | Python fallback |
| --- | ---: | ---: |
| `ui.workspace.create` | 1 | 0 |
| `ui.workspace.rename` | 3 | 0 |
| `ui.workspace.delete` | 4 | 0 |
| `ui.profiles.reset_data` | 7 | 0 |
| `ui.profiles.snapshot` | 4 | 0 |
| `backends.reveal_token` | 12 | 0 |
| `ui.backends.create` (Core references) | 4 | 0 |
| `daemon.unlock` | 1 | 0 |

Fmt, workspace and harness clippy with `-D warnings`, the harness build, all
four frozen setup vector checks, and 527 workspace tests excluding
`kassiber-net` pass. The exact workspace test command reaches four existing
`kassiber-net` loopback binds that fail with sandbox `EPERM`. The broader hybrid
service modules pass 67 tests and two subtests; many call Python directly, so
the process witnesses above establish native execution. All 57 setup mutations
are killed across the full run and targeted recheck of a corrected owner mutant;
both setup and custody mutation catalogs validate. Cargo uses the pinned
toolchain offline throughout.

Inventory and contract checks pass (10 tests), after regenerating only the
Python source-position changes in the error/environment snapshots. The exact
requested shell/hybrid/runner/port/backend suite reports 61 passed and 59
subtests passed, with one sandbox failure: the unported Python `secrets init`
descriptor fallback tries to write `/home/tg/.kassiber/run`, which is read-only.
The combined witness reconfirms all 55 project/network/offline/backend CLI
calls in the earlier per-command tables were served by Rust with zero
fallbacks. Full workspace socket tests, that descriptor fallback, and broader
mixed-service daemon acceptance remain orchestrator checks.

The continuation adds 25 mutation entries to the prior 32. Each entry in
[`mutations.json`](../../crates/setup/scripts/mutations.json) names its killer:

| Mutation IDs | Named killer test |
| --- | --- |
| `backend-bare-host-visible`, `backend-bare-host-fullmatch`, `backend-bare-host-port-length` | `bare_backend_location_fullmatch_preserves_only_the_python_allowlist` |
| `workspace-desktop-exact-id`, `workspace-delete-current` | `frozen_python_desktop_responses_and_database_states` |
| `sensitive-confirmation`, `sensitive-current-name`, `sensitive-rate-boolean`, `reveal-target-precedence` | `sensitive_selection_requires_exact_current_names_and_boolean_rate_scope` |
| `destructive-legacy-barrier` | `archived_ledger_blocks_only_its_profile_and_unscoped_rows_block_all` |
| `reset-profile-boundary`, `reset-attachment-confinement` | `profile_reset_keeps_other_books_and_confines_attachment_cleanup` |
| `reset-shared-rates-opt-in`, `profile-snapshot-active-selection` | `frozen_python_desktop_responses_and_database_states` |
| `profile-display-stored-method` | `profile_display_uses_the_stored_method_for_each_country` |
| `core-reference-exact`, `core-reference-identity`, `core-local-candidates` | `core_references_reselect_current_local_credentials_without_password_in_identity` |
| `core-network-alias-precedence` | `core_candidate_alias_precedence_cookie_dedup_and_ipv6_match_python` |
| `core-reference-auth-replacement`, `core-reference-endpoint` | `core_reference_merge_replaces_endpoint_and_conflicting_auth` |
| `sensitive-auth-before-service`, `sensitive-plaintext-ack` | `plaintext_sensitive_actions_require_the_exact_typed_acknowledgement` |
| `sensitive-passphrase-verification`, `sensitive-desktop-owner` | `sensitive_actions_require_reauthorization_after_unlock_and_hold_the_owner` |

AI projections stay in Python, including the AI-scoped profile snapshot and
`ui.backends.list`. Project desktop lifecycle and remaining R3 runtime/storage
dispositions are separate work. Import/sync integration still owns network
admission; these local setup operations grant no egress consent.

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
