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
| `crates/pytext` | CPython's `str.isspace`/`isalpha`/`strip()`/`lower()`/`upper()`, `repr(str)`, `int()`/`float()` parsing, ISO datetime parsing, and `json.loads` with the C scanner's messages, from Unicode 14.0.0 tables |
| `crates/envelope` | the machine envelope (`envelope.py`), `AppError`, `derive_kind` and `command_path`, `json_ready`, and the redaction rules of `redaction.py` |
| `crates/runtime` | the capability tables, `version_text()`, the `status` payload, and the store interface ([below](#store-interface)) |
| `crates/cli` | argparse's grammar for every parser (generated), its parsing and usage algorithm, the global flags, `emit`, `commands describe`, and the implemented leaves |
| `crates/daemon` | the JSONL protocol, request routing, the secret-store control channel, and registered service kinds |
| `crates/setup` | local workspace, profile policy, account bucket, context, saved-view, project catalog, book-network, offline-preference and backend-configuration services |
| `crates/kassiber` | the binary: arms the egress gate, then runs the CLI |
| `crates/rates` | exact fiat cache, manual/CSV rates, pricing lookup, consent-bound market providers and desktop rate projections |
| `crates/imports` | import-run listing, rollback plans and confirmed deletion |
| `crates/wallets` | local public descriptor parsing, derivation and preview |
| `crates/transactions` | local loan marks, managed attachment evidence, metadata edits, BIP329 labels, append-only history, and CLI/desktop transaction pages shared by both transports |
| `crates/secrets` | zeroized unlock, Python-compatible native credential lifecycle, encryption maintenance, dotenv credential migration and explicit initialization/inspection/verification |
| `crates/backup` | encrypted archive export, staging/install import and Python-compatible tar/manifest validation |
| `crates/reports` | read-only stored-journal summaries, Austrian forms, gains, live/historical holdings and journal listing, report readiness, and saved/filed snapshot listing |
| `crates/transfers` | stored effective pairs/payouts, gap history, historical receipts, local matching-rule configuration and advisory dismissals |

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

Transaction CLI pages preserve all filters,
four sorts, chronological ties, scoped cursor bytes, unpaged counts, integer msat
fields and Python's one-rounding BTC display. Custody flow filters read only
current stored projections; listing never processes journals or infers facts.
The separate Python reference correction and matching Rust continuation compare
transaction, metadata-record and edit-history bounds as exact integer
microseconds using the existing timestamp parser. Fractional bounds, including
one microsecond, select the correct rows before counting and pagination.
Malformed stored instants are excluded from bounded reads; timestamps and
cursor bytes are unchanged. The SQL helper preserves CPython's timezone and
terminal-Z parsing and safely reuses its per-connection registration.
Desktop `ui.transactions.list/search/extremes` use a separate typed projection
service over the same admitted book. Their signed display sorts, grouped counts,
queue status, scoped cursors and pair suppression follow Python, including exact
microsecond bounds. Stored MOVE/conversion relations and journal transfer rows
are read only while the full journal freshness markers permit them. Wallet
filters retain the desktop rule of selecting every matching ID or label. No
journal rebuild, ownership inference or provider lookup runs for these reads.
Document registration, transactionless managed file/URL evidence, and prior-report
archives share the admitted book service. Stored commercial matching preserves
Python's identifier signals, uniqueness refusals, principal-based confidence,
document enrichment, rejected-link retention and response ordering. Suggestions
never apply pricing or kind changes. Prior reports remain evidence without
ledger, lot, ownership, custody or tax authority.
`ui.transactions.dashboard/resolve/graph/review_context/ledger_template`,
`transactions export` in every format, journal/report processing, BTCPay provenance
sync and commercial review remain `not_implemented`. The acceptance record names the remaining
dependencies.

The [R5 local evidence boundary](rust-transactions-r5-status.md) records storage
invariants, parity evidence and remaining work.

Implemented leaves: `init`, `status`, `daemon`, `commands describe`, `transactions list`, all six
`rates` leaves (`pairs`, `set`, `latest`, `range`, `sync`, `rebuild`), all four
`loans` and six `attachments` commands, all four `documents` commands and stored
`btcpay provenance list/links/suggest`, 26 local `metadata` leaves (notes/tags,
inclusion, kind overrides, record get/list, stale edits, history list/activity
forward-edit reverts and four BIP329 leaves), `imports list`, `imports rollback`, wallet `create`, `list`, `get`, `update`, `delete`, `derive`, `import-csv` and `import-phoenix`,
the ten [report reads and CSV projections](#reports-tabular-csv-checkpoint-r5-fourth-continuation-partial), and every leaf of
`workspaces`, `profiles`, `accounts`, `context`, `views`, `projects`, `networks`,
`offline` and `backends`, plus all eight `secrets` leaves, `backup export` and `backup import`, including `--install` (35 setup leaves; 119 recognized leaves total). Each family registers through its own
CLI module. Every other leaf parses exactly as in Python and then fails
`not_implemented`
(`details.component` `cli`, `details.command` its dotted path), before any
project is read or created. Nothing falls back to Python in the product.
The development-only hybrid candidate may select Python before bootstrap.
Wallet lifecycle remains a partial service checkpoint with the explicit limitations below.

`commands describe` builds the catalog from the generated tree with the
registry's rules (capability, effect, `needs_database`, scope flags,
`secret_input`, `may_prompt`, arguments). It returns all 313 Python leaves,
including unported commands; a query matching nothing returns `unknown_command`.
Execution support remains separately recorded in `describe.rs::IMPLEMENTED`.
The full catalog and prefix queries match Python byte for byte. The generated
grammar drift check and `tests/rust_port/test_contracts.py` bind the Rust tree
and `tests/fixtures/rust_port/commands_describe.json` to `build_parser()` and
the Python registry annotations.

## Encryption service checkpoint (R5, partial)

This is the initial checkpoint; the [continuation below](#secrets-lifecycle-continuation-r5)
supersedes its remaining-secrets and daemon-kind statements.

`t3code/r5-secrets` starts at integration tip `0d4fbc28`. It implements three
of the requested twelve secrets/backup/maintenance/diagnostics CLI leaves.
**The service slice is incomplete.** None of its desktop daemon kinds is
enabled, and AI projections remain in Python.

`crates/secrets/src/service.rs` owns the command decisions; its family adapter
uses the real store without ordinary schema bootstrap. `init` creates an empty
SQLCipher container or calls R3's verified plaintext export, retaining the
rollback copy and both maintenance role reservations. The new-passphrase
minimum is eight Python characters. Only descriptors and a no-echo controlling
TTY supply passphrases; machine/non-interactive requests without a descriptor
fail `interaction_required`. Configured broker routing retains its existing
unported-operator refusal.

`init-resume` matches Python's artifact inspection, including its informational
hint; it does not automatically promote a candidate. `verify` uses the keyed
container API without the current-book schema gate or migrations, so it accepts
both empty containers and historical schemas. Repeated inspection preserves
database bytes. Neither path contacts a native credential store or a provider.

Empty-container creation now writes the Python-named `*.encrypted.sqlite3`
candidate privately, closes and freshly verifies it, flushes the candidate and
directory, then publishes with the existing no-overwrite rename. A zero-byte
live placeholder is removed only after verification; populated live files and
existing candidates are preserved. Fourteen observer boundaries cover candidate
creation/close, file and directory syncs, placeholder removal and publication.
Fifty killed-child cases cover absent/empty destinations and verification or
recreation through either runtime. Recovery of a complete candidate uses
Python's documented manual rename after verification; incomplete zero-byte
candidates require explicit removal before retry. These are process-interruption
checks, not a filesystem power-loss simulation.

The hybrid scenario module is `tests/test_rust_secrets_services.py`; it compares
success/refusal bytes, checks complete SQLCipher/rollback logical dumps and
checks compatible desktop/broker exclusion before file changes. The
fresh hybrid run passes all 62 cases, including the 50 killed-child cases.
Its per-command routing counts include successful and refused invocations:

| Command | Rust | Python fallback |
| --- | ---: | ---: |
| `secrets.init` | 14 | 0 |
| `secrets.init-resume` | 4 | 0 |
| `secrets.verify` | 5 | 0 |

The other nine requested leaves have no functional hybrid acceptance at this
checkpoint; a direct negative test checks their unported refusal.

The eight-entry mutation catalog runs with
`python crates/secrets/scripts/mutation_check.py`; the quality gate checks its
applicability. Each entry is killed by its named Rust test:

| Mutation | Killing test filter |
| --- | --- |
| `S-input-nul` | `fd_text_rules_and_bounds` |
| `S-input-limit` | `fd_text_rules_and_bounds` |
| `S-descriptor-close` | `descriptor_input_closes_and_bypasses_confirmation_in_machine_mode` |
| `S-new-key-characters` | `new_passphrase_minimum_counts_characters_and_preserves_spaces` |
| `S-empty-verification` | `corrupt_empty_candidate_never_replaces_the_live_placeholder` |
| `S-empty-file-is-not-encryption` | `empty_verification_never_recreates_a_missing_or_zero_byte_candidate` |
| `S-rekey-auth-before-invalidation` | `native_guard_callback_runs_only_after_authentication_and_stays_armed_on_ambiguity` |
| `S-rekey-new-key` | `encryption_and_rekey_preserve_identity_and_rollback_copy` |

The rekey entries cover the existing R3 storage primitive; they do not enable
the CLI rotation or native stale-credential lifecycle. This checkpoint adds
only internal crate dependencies and retains all registry versions/checksums.
Python product code is unchanged; no new Python reference bug was fixed.

Local Linux verification uses Cargo offline. Formatting and both workspace and
`kassiber --features test-harness` clippy checks pass with warnings denied.
The workspace test run excluding `kassiber-net` and `kassiber-rates` passes
853 tests; the harness test run passes 14. Full workspace acceptance remains
blocked by sandbox socket permissions: the unrestricted run reaches 30 passed /
16 failed in `kassiber-net`; excluding that crate reaches 16 passed / 11 failed
in `kassiber-rates`, with `Operation not permitted` at local socket creation.
The focused R3 encryption/rekey Python parity run passes 17 cases, and all
eight mutations are killed with their unmutated baselines passing. Inventory,
contract and shard validation pass. The harness and storage probe builds pass;
the harness is rebuilt after default workspace tests before Python parity.
The required `test_rust_shell_parity.py`, `test_rust_hybrid_candidate.py`,
`test_cli_runner.py` and `tests/rust_port` run passes 63 tests and 178 subtests.
The hybrid descriptor fixture uses a private Rust ownership namespace and
retains Python descriptor coverage through an unported encrypted backend read.

Remaining work is `secrets change-passphrase`, `remember-unlock`, `forget-unlock`,
`status`, `migrate-credentials`; `backup export/import`; `maintenance
purge-legacy-ledger`; and associated desktop daemon kinds. Diagnostics has
since been ported in the diagnostics service slice below. Unported operations
retain `not_implemented` before bootstrap.
Native credential writes/deletes and their disposable-bus tests, archive safety
mutations, age/.kassiber round trips in both directions, credential/restore
publication recovery, and native-platform/TTY acceptance remain unverified.
The complete slice disposition inventory is still open. The selected byte
comparisons above do not establish every descriptor/read-error or wrong-key
error envelope; complete refusal-byte parity also remains open.

## Secrets lifecycle continuation (R5)

`t3code/r5-secrets-2` starts at `96461ae3` and adds the remaining five secrets
CLI leaves: `change-passphrase`, `remember-unlock`, `forget-unlock`, `status`
and `migrate-credentials`. The associated existing desktop kinds are
`ui.secrets.change_passphrase` and `ui.secrets.forget_cli_unlock`; Python has
no separate desktop kinds for the other three commands. `ui.secrets.init`
remains unported from the initial checkpoint. AI projections stay in Python.

The shared secrets service retains descriptor/no-echo TTY input. CLI new keys
have Python's eight-character minimum; the desktop retains its distinct
twelve-character minimum. CLI forgetting requires a fresh authenticated open;
the desktop keeps Python's direct revocation path. Rotation uses R3's verified
rekey primitive and both-role maintenance exclusion, preserving a daemon's
existing desktop lease. Wrong keys cannot change credential policy. The
daemon releases a newly acquired lease when reopening after rotation fails;
an existing lease remains held, matching Python and the injected-failure test.
The authenticated callback publishes the independent operator generation and the
macOS desktop stale guard before rekey. Failed refresh disables CLI unlock,
quarantines undeletable legacy credentials and reports a warning without
undoing the completed database rotation.

Enrollment writes the Python v2 service/account digest after authentication,
cleans the prior and path-keyed CLI/shared items, then atomically binds policy
to the authenticated database and current file identity. Revocation verifies
native absence and publishes manual policy even when the native store is
unavailable. A pre-policy interrupted enrollment can leave an unreferenced
native item, as in Python; it grants no reusable unlock policy. Re-enrollment
can proceed. Disposable native testing must establish the platform cleanup
behavior before native acceptance is claimed.

Dotenv scanning preserves Python's aliases, quote/whitespace rules, line
numbers, duplicate-target refusal and secret-free finding projections. Only
the four credential fields move; username/password merge into `config_json`,
auth-header/token use their columns, missing backends remain skipped, and
each update commits before its source line can be removed. Complete logical
dumps compare against Python. The timestamped recovery copy remains plaintext.
Rust flushes staged snapshots and sanitized files before replacement and
directory sync. Compared with Python's in-place rewrite, interrupted Rust
publication leaves the original or complete sanitized file. A conflicting
same-name snapshot and observed concurrent source edits are refused instead of
overwritten. These conservative publication differences are not a claim of
filesystem power-loss coverage.

`tests/test_rust_secrets_lifecycle.py` adds CLI/daemon byte comparisons and
22 killed-child cases: seven credential-publication and four guard/rekey
boundaries, each resumed with either runtime. Together with the initial 50
empty-publication cases, the fresh hybrid pass is **91 passed / 29 gated**,
including 72 killed-child cases. Routing counts include successes and refusals:

| Command | Rust | Python fallback |
| --- | ---: | ---: |
| `secrets.change-passphrase` | 6 | 0 |
| `secrets.remember-unlock` | 3 | 0 |
| `secrets.forget-unlock` | 2 | 0 |
| `secrets.status` | 32 | 0 |
| `secrets.migrate-credentials` | 14 | 0 |
| `daemon` | 1 | 0 |

The daemon session compares five rotation requests (including input refusals)
and two revocations with Python. The same run also serves `init` 14 times,
`init-resume` four times and `verify` ten times in Rust, all without fallback.
Native availability is deliberately tested against an absent private bus;
these counts do not establish successful native enrollment.

The mutation catalog now has **18 killed entries**, all with passing unmutated
baselines. The initial eight remain listed above; ten additions are:

| Mutation | Killing test filter |
| --- | --- |
| `S-account-v2` | `credential_account_matches_the_python_v2_digest` |
| `S-enroll-auth-order` | `wrong_enrollment_key_never_writes_or_deletes_credentials` |
| `S-forget-auth-order` | `forget_requires_fresh_authentication_before_any_native_or_policy_effect` |
| `S-native-generation` | `rotation_authenticates_before_guards_and_refreshes_only_the_bound_item` |
| `S-refresh-disable` | `failed_rotation_refresh_disables_policy_and_keeps_the_successful_new_key` |
| `S-revoke-owned-legacy` | `failed_legacy_cleanup_rolls_back_new_enrollment_and_quarantines_revocation` |
| `S-credential-duplicates` | `duplicate_credential_targets_fail_before_database_or_dotenv_changes` |
| `S-credential-row-commit` | `every_credential_publication_boundary_retains_committed_values_and_is_repeatable` |
| `S-credential-field-selection` | `dotenv_aliases_quotes_line_numbers_and_secret_free_scan_match_python` |
| `S-credential-sanitize` | `credential_move_preserves_unknown_fields_and_skipped_dotenv_lines` |

Linux native tests are gated by `KASSIBER_TEST_DISPOSABLE_KEYRING=1`. The
fixture always starts a new `dbus-run-session` and `gnome-keyring-daemon` with
a temporary home/runtime directory; it never reuses the account's bus or
store. One interop scenario and 28 interruption cases cover item writes,
prior/legacy cleanup, policy publication, revocation, successful refresh and
refresh-failure cleanup through either runtime. This sandbox cannot bind the
private D-Bus socket (`Operation not permitted`), so all 29 are unexecuted.
Build the test probe with:

```sh
mise exec rust@1.99.1 uv@latest -- cargo build --offline --locked -p kassiber-secrets --features test-hooks,native-keyring --example kassiber-secrets-probe
```

Then run that module through the hybrid candidate with the native gate set.
The probe and deterministic token/clock sources require `test-hooks`; they
are not installed or enabled in production.

Verification: fmt, workspace clippy and binary harness clippy with warnings
denied, harness/probe builds, and 14 binary/harness tests pass. The workspace
pass excluding the two socket-dependent crates passes 867 tests, including
19 secrets tests and the concurrent-dotenv-edit regression. The required
full workspace run fails at `kassiber-net` (30 passed / 16 socket failures);
an unrestricted workspace run remains required. The required Python
shell/hybrid/runner/inventory pass is 63 tests and 178 subtests. Inventory,
contracts and shard validation pass.
The existing dependency versions/checksums are unchanged; secrets now directly
uses the locked `getrandom` for portable OS randomness, reuses `tempfile` for
publication, and the binary reuses `zeroize` for transport copies.

Remaining acceptance: the 29 disposable native cases, native macOS/Windows
item/ACL/signing and interruption runs (including Windows compound-target
displacement), interactive TTY behavior, and broader secrets disposition and
complete refusal/error-byte coverage. Backup, maintenance and desktop
encryption initialization were later work at this checkpoint; diagnostics
has since been ported below.
Python product code is unchanged; no new Python reference bug was confirmed.

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

This is a partial wallets/file-import slice. The remaining 33 `wallets` leaves
answer `not_implemented`: kinds, reveal-descriptor, identify,
file analysis/templates/document preview/import, JSON/CSV and every dedicated
importer, Samourai, attachment/enrichment routes, and all sync commands.
Descriptor parsing and local derivation are covered below. Native file parsing,
normalization, integer-msat/timestamp conversion, deduplication and batch creation
also remain. `wallets sync` and the provider-specific sync commands belong to
the later network slice. Hybrid setup/import calls still run in Python and are
not evidence of native importer execution. The exact unported leaves are:

```text
wallets kinds / reveal-descriptor
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

At the descriptor checkpoint, lifecycle and its 41 config vectors were still
unported. The lifecycle checkpoint below now implements their shared service.
The broader malformed-descriptor error/preflight audit remains open.

## Wallet lifecycle checkpoint (R5)

The real store now serves CLI `wallets create`, `wallets list`, `wallets get`,
`wallets update`, and `wallets delete` through `crates/wallets`. These are
Python's command names (`get`, with no invented `show` alias). The daemon adds
`ui.wallets.create`, `ui.wallets.update`, and `ui.wallets.delete`; preview stays
local. No AI projection, file importer, sync command, schema or egress path is
added. The CLI's new dependencies are existing workspace packages and the
already locked JSON package; there are no new third-party versions.

The 41 frozen configuration vectors now pass natively. Raw authored config is
stored as Python-compatible sorted JSON. Ordinary output uses the explicit
safe-field projection and private-material presence flags. Spending keys are
refused without echoing their values, including a change-only configuration
and unsafe legacy material encountered on update/list/get. Liquid view keys
remain permitted sensitive material. Book scope and observed network history
remain separate from authored policy and never grant ownership or tax facts.

Update uses one savepoint for the wallet row, durable policy retirement and
replacement, observer/output-inventory cleanup, freshness changes and journal
invalidation. Retired epochs retain their old authored private material and
coverage. Chain/network aliases, script ordering, gap limit and ownership scan
depth preserve policy identity where Python does. Profile/account/wallet
selection, history contradictions, missing clear fields and cascade deletion
retain their guards. Failure injection proves rollback leaves no partial writes.

Desktop mutations require the exact destructive confirmation and current
wallet label where applicable. Plaintext acknowledgements are scope-specific;
encrypted mutations verify the passphrase afresh for each request, including
requests following a successful unlock or mutation. SQLCipher's default
stderr authentication diagnostics are disabled; typed redacted errors remain
on the protocol. No disk logging is introduced.

Two independently committed Python fixes accompany this checkpoint. CLI
safe-channel options now consume descriptor/change-descriptor stdin and FDs,
including exclusive stdin selection. Change-only private spending material is
preflighted before create/update can store it. Each regression was observed
failing before the fix.

Verification passed: formatting; workspace and harness clippy with
`-D warnings`; 500 workspace Rust tests; 14 binary/harness tests; the harness
build; the required shell/hybrid/runner/inventory/contracts suite (62 tests,
178 subtests); all accepted vectors (124 plans, 43 material inputs, 41 configs,
3 normalizations, 165 previews, 32 branch selections); and all 35 mutations.
The complete workspace tests needed loopback access for four `kassiber-net`
socket fixtures. Plaintext CLI/daemon and encrypted daemon records compare as
wire bytes; the CLI lifecycle sequence also compares complete SQLite dumps.

The six hybrid modules (`test_wallet_descriptors`, `test_wallet_setup`,
`test_wallet_config_multi_script`, `test_import_batches`,
`test_review_regressions`, `test_cli_smoke`) passed 381 tests and 42 subtests.
An immutable harness copy avoided concurrent Cargo builds replacing it. A
small test-only wrapper retained the binary/log settings for the default-home
test that deliberately clears `KASSIBER_*`. The mandatory hybrid routing test
also proves all five lifecycle commands select Rust. The routing log for the
six-module run plus one complete five-command lifecycle scenario records:

| Command | Rust | Python fallback |
| --- | ---: | ---: |
| `wallets.create` | 215 | 0 |
| `wallets.list` | 3 | 0 |
| `wallets.get` | 9 | 0 |
| `wallets.update` | 8 | 0 |
| `wallets.delete` | 1 | 0 |
| `wallets.derive` | 4 | 0 |

Across that larger run there were 426 Rust-served calls and 1,044 explicit
Python fallbacks for setup and other unported commands. These totals cover
more scenarios than the earlier 268-fallback descriptor checkpoint. Lifecycle
fallbacks are zero. File ingestion/sync and AI coverage remain Python.

The catalog has 35 killed mutations: the 12 descriptor entries above and the
23 lifecycle entries below. Every row names its killing native test; the
catalog contains the exact replacement and rule.

| Mutation | Killing native test |
| --- | --- |
| `W-config-redaction` | `python_wallet_config_vectors` |
| `W-config-scan-depth` | `python_wallet_config_vectors` |
| `W-config-storage` | `lifecycle_selection_storage_and_redaction` |
| `W-list-selection` | `lifecycle_selection_storage_and_redaction` |
| `W-epoch-identity` | `policy_rotation_retains_material_and_is_atomic` |
| `W-epoch-retired-material` | `policy_rotation_retains_material_and_is_atomic` |
| `W-epoch-gap-identity` | `scan_depth_gap_aliases_and_script_order_do_not_rotate_policy` |
| `W-epoch-atomic` | `policy_rotation_retains_material_and_is_atomic` |
| `W-sync-invalidation` | `config_changes_reset_freshness_but_deprecation_and_scan_depth_preserve_checkpoint` |
| `W-observer-invalidation` | `alias_rewrites_preserve_observer_state_but_policy_changes_discard_it` |
| `W-history-guard` | `history_and_book_binding_reject_relabeling_without_side_effects` |
| `W-observation-guard` | `contradictory_observation_history_never_becomes_configured_fact` |
| `W-book-binding` | `history_and_book_binding_reject_relabeling_without_side_effects` |
| `W-delete-guard` | `delete_guard_cascade_and_invalidation_are_profile_scoped` |
| `W-clear-guard` | `conflicting_labels_clear_fields_accounts_and_failed_invalidation_roll_back` |
| `W-auth-fresh` | `wallet_mutations_require_fresh_encrypted_auth_and_exact_plaintext_ack` |
| `W-auth-plaintext` | `wallet_mutations_require_fresh_encrypted_auth_and_exact_plaintext_ack` |
| `W-config-address-shape` | `malformed_address_containers_fail_closed` |
| `W-change-only-private` | `change_only_private_and_unsafe_legacy_reads_fail_closed` |
| `W-private-legacy-read` | `change_only_private_and_unsafe_legacy_reads_fail_closed` |
| `W-list-gap-default` | `descriptor_list_default_gap_matches_reference` |
| `W-account-selection` | `conflicting_labels_clear_fields_accounts_and_failed_invalidation_roll_back` |
| `W-id-selection` | `resolver_prefers_id_refuses_ambiguity_and_scopes_profiles` |

At this checkpoint, nonempty birthdays, configured Silent Payments and
`ui.wallets.list` remained disabled. The follow-up below ports those services
and adds successful-create wire comparison and broader config refusals.

## Wallet lifecycle remainder checkpoint (R5)

Wallet create/update now normalize birthdays with CPython 3.11's ISO rules,
including Python's trailing-`Z` rewrite and its accepted clock/offset whitespace.
The local Silent Payments validator stores compact BIP392 watch-only scan
material exactly as Python does. Private scan keys are permitted; private
spend keys, including nested expressions and inactive ordinary wallet key
fields, are refused before persistence. Scan birthdays, network families,
full-history and server-assisted warning acknowledgements, and an explicit
backend retain their separate guards. Saving configuration starts no scanner
or backend probe.

The daemon now serves `ui.wallets.list` in Rust. Its safe snapshot includes
active-book selection, backend/default routing metadata, transaction counts
excluding excluded rows, journal input-version freshness, Silent Payments
presence, BTCPay routing metadata and allowlisted Samourai fields. Backend
kinds come from already loaded configuration; the read adds no network path.
AI projections remain Python and gain no access to these wallet materials.

The frozen oracle now contains 193 config cases and seven desktop snapshots,
in addition to the unchanged 124 descriptor plans, 43 material inputs,
3 normalizations, 165 previews and 32 branch selections. Native lifecycle
tests pin persisted scan material, redacted output, birthday normalization
and failure atomicity. A test-harness-only identity/clock override permits
successful-create wire-byte and full SQLite-dump comparisons; production
builds ignore those environment variables. CLI refusal and daemon-list records
also compare as wire bytes. Lifecycle dispositions name their native
replacements, with mixed sync/observer obligations explicitly retained.

Two isolated Python reference fixes accompany this checkpoint, each with a
regression observed failing before the fix. The wallet-config wrapper now
converts embit miniscript errors into secret-free typed validation errors.
Silent Payments validation now preflights inactive ordinary descriptor,
change-descriptor and xpub fields before they can store spending keys.

Verification passed: fmt, workspace and harness clippy with `-D warnings`,
502 workspace Rust tests, 14 binary/harness tests, the harness build, all
accepted vectors and all 51 mutations. The final snapshot adjustment also
passed all 26 wallet tests and the three snapshot mutations. Workspace socket
fixtures needed loopback access. The reference SP suite passed 27 tests and
9 subtests; the wallet-config reference suite passed 21 tests and 3 subtests.
The exact requested shell/hybrid/runner/inventory/contracts command passed
66 tests and 245 subtests against the final harness. Its binary hash remained
unchanged throughout the run. The encrypted FD scenario shares one disposable
ownership namespace across the test launcher's Python fallback and native
test build; production ownership is unchanged.

The seven hybrid modules (`test_wallet_descriptors`, `test_wallet_setup`,
`test_wallet_config_multi_script`, `test_import_batches`,
`test_review_regressions`, `test_cli_smoke`, `test_silent_payments`) passed
409 tests and 54 subtests with loopback fixture access. An immutable harness
and test-only wrapper preserve candidate/log settings when a test deliberately
clears `KASSIBER_*`. That fresh run plus one complete SP lifecycle scenario
records 426 Rust calls and 1,044 explicit Python fallbacks:

| Command | Rust | Python fallback |
| --- | ---: | ---: |
| `wallets.create` | 215 | 0 |
| `wallets.list` | 3 | 0 |
| `wallets.get` | 9 | 0 |
| `wallets.update` | 8 | 0 |
| `wallets.delete` | 1 | 0 |
| `wallets.derive` | 4 | 0 |
| `imports.list` | 4 | 0 |
| `imports.rollback` | 4 | 0 |
| `wallets.import-csv` | 0 | 78 |
| `wallets.import-phoenix` | 0 | 1 |

The daemon wire scenario separately compares two `ui.wallets.list` responses
served natively with Python's exact bytes. The hybrid CLI log does not count
individual daemon requests.

The wallet catalog has 51 mutations: the 35 inherited entries above plus these
16 entries. Each entry names its killing test in the checked-in catalog.

| Mutation | Killing native test |
| --- | --- |
| `W-birthday-normalization` | `python_wallet_config_vectors` |
| `W-time-Z-rewrite` | `python_wallet_config_vectors` |
| `W-time-offset-space` | `python_wallet_config_vectors` |
| `W-sp-private-spend` | `python_wallet_config_vectors` |
| `W-sp-network` | `python_wallet_config_vectors` |
| `W-sp-start-required` | `python_wallet_config_vectors` |
| `W-sp-full-ack` | `python_wallet_config_vectors` |
| `W-sp-server-ack` | `python_wallet_config_vectors` |
| `W-sp-storage` | `python_wallet_config_vectors` |
| `W-snapshot-excluded` | `python_wallet_snapshot_vectors` |
| `W-snapshot-freshness` | `python_wallet_snapshot_vectors` |
| `W-snapshot-redaction` | `python_wallet_snapshot_vectors` |
| `W-public-path-hardened` | `python_wallet_config_vectors` |
| `W-sp-inactive-descriptor` | `python_wallet_config_vectors` |
| `W-sp-inactive-xpub` | `python_wallet_config_vectors` |
| `W-sp-full-birthday` | `python_wallet_config_vectors` |

At this historical checkpoint, the **wallets/import slice was partial**.
`wallets import-csv` and `wallets import-phoenix` remained `not_implemented`,
along with the other file
importers and sync commands. Native generic CSV parsing, decimal/msat and fee
conversion, dedupe/enrichment, import-run creation and journal invalidation
had not landed. The existing import-run list/rollback services stayed native,
but hybrid `test_cli_smoke` ingestion still fell back to Python: the requested
Rust import-step acceptance was **not met**. The file-ingestion checkpoints
below supersede that limitation.

The malformed-descriptor audit is broader but not exhaustive. In particular,
Python accepts a nonterminal wildcard suffix such as `/0/*/1` that Rust still
refuses; direct desktop preview also retains Python parser exception gaps
outside the lifecycle wrapper. Those remain parity obligations, as do sync,
observer-owned index construction and technical-coverage snapshots mentioned
in the dispositions. Do not interpret the native lifecycle tests as acceptance
of those unported services.

## Wallet CSV ingestion checkpoint (R5)

`wallets import-csv` now uses the network-free `crates/wallets::file_import`
service on the admitted book connection. The CSV reader follows Python's
default non-strict dialect: UTF-8, literal BOM headings, quoted commas and
embedded CR/LF, duplicate headings, null short cells and its field-size limit.
Amounts, fees, exact pricing and fingerprints reuse the existing decimal and
JSON compatibility crates. ISO timestamps reuse the wallet timestamp parser.
File enrichment cannot replace a current observer commitment; the custody
crate supplies that predicate. No third-party dependency or egress is added.

Deduplication, enrichment, journal invalidation, import-run creation and the
links to newly inserted rows share one savepoint. Reimports that insert no
rows create no run. Existing `imports rollback` can remove these runs without
deleting rows a later import only enriched. AI projections stay in Python.
Harness-only identity/clock inputs permit exact CLI bytes and full book-dump
comparison without normalizing persisted quantities or evidence.

Initial verification: all five native importer tests and the frozen Python
reader/normalization vectors passed. Thirteen new importer mutations were killed
by `csv_dialect_preserves_python_rows`, `python_file_import_vectors`,
`csv_import_dedupe_batch_membership_and_journal_invalidation`, and
`csv_import_failures_are_atomic_and_empty_runs_preserve_readiness`.
The CLI smoke module passed 95 tests: `wallets.import-csv: rust=66, python=0`;
`wallets.import-phoenix: rust=0, python=1`. The broader hybrid run and Phoenix follow in the next checkpoint.

## Wallet file-ingestion checkpoint (R5)

Both `wallets import-csv` and `wallets import-phoenix` now use the same local
Rust service. Phoenix preserves signed integer msat, zero-amount type rules,
sat mining fees plus msat service fees, fiat value/rate arithmetic, sanitized
raw export rows, payment-hash evidence and idempotent notes/tags. Imported
notes never overwrite authored notes. Metadata-only enrichment invalidates the
journal once; unchanged repeats create no import run or invalidation.

Generic CSV enrichment now also follows Python's pricing rank and raw-source
ownership rules for imported descriptions, kinds, counterparties and privacy
boundaries. Authored metadata stays protected. Existing hash-source and
funding-output evidence can be enriched without replacing their identities.
Array/scalar raw JSON keeps its original text; object raw JSON removes the
reserved observer-provenance marker. A closed observer commitment still bars
file-supplied payload replacement, independently of these metadata decisions.

The frozen importer oracle contains 145 cases (78 generic and 67 Phoenix),
including quoting, line endings, BOM, invalid UTF-8/UTF-16, field limits,
decimal special values/refusals, half-msat rounding, timestamps and hashes.
Native tests cover persistence, atomicity, exact dedupe, import-run membership,
readiness, source ownership and metadata idempotency. SQLite and SQLCipher
CLI comparisons check bytes and every persisted field before and after rollback.
The opt-in hybrid import oracle replays Python on an in-memory copy before
each Rust import and compares complete typed book dumps without filtering
accounting fields. Clock/identity inputs exist only in the test-harness build.

Verification passed: workspace and package fmt, workspace and harness clippy
with `-D warnings`, 510 workspace Rust tests, 14 binary/harness tests, the
offline harness build, both wallet vector generators, inventory and contract
checks, and all 83 mutation entries. The exact requested
`test_rust_shell_parity.py` / `test_rust_hybrid_candidate.py` /
`test_cli_runner.py` / `tests/rust_port` command passed 68 tests and 275
subtests against the same final harness. The seven hybrid modules named above ran
in three independent pytest invocations: 409 tests and 54 subtests passed.
An immutable final harness and a test-only wrapper retained routing/oracle
settings even when scenarios scrubbed `KASSIBER_*`. Every CSV/Phoenix import
compared its complete typed book dump with Python. There were 499 Rust calls
and 962 expected Python fallbacks for other services; relevant command counts:

| Command | Rust | Python fallback |
| --- | ---: | ---: |
| `wallets.create` | 214 | 0 |
| `wallets.list` | 2 | 0 |
| `wallets.get` | 8 | 0 |
| `wallets.update` | 7 | 0 |
| `wallets.delete` | 0 | 0 |
| `wallets.derive` | 4 | 0 |
| `imports.list` | 4 | 0 |
| `imports.rollback` | 4 | 0 |
| `wallets.import-csv` | 78 | 0 |
| `wallets.import-phoenix` | 1 | 0 |

The CSV total is 66 calls in `test_cli_smoke` plus 12 in the other hybrid
modules. This closes the 78-call acceptance gap recorded above. These modules
do not invoke wallet deletion; the separate wire/state suite covers it.
Socket fixtures needed loopback access; all books and ownership roots were
disposable. No Python reference fix was needed for these importers.

All 83 wallet mutation entries were killed: the inherited 51 above plus these
32 file-import entries. Their named killers are:

| Mutation | Named killer |
| --- | --- |
| `W-csv-quote` | `csv_dialect_preserves_python_rows` |
| `W-csv-short-row` | `python_file_import_vectors` |
| `W-csv-field-limit` | `python_file_import_vectors` |
| `W-import-msat` | `python_file_import_vectors` |
| `W-import-round-half` | `python_file_import_vectors` |
| `W-import-fee` | `python_file_import_vectors` |
| `W-import-price` | `python_file_import_vectors` |
| `W-import-dedupe` | `csv_import_dedupe_batch_membership_and_journal_invalidation` |
| `W-import-journal` | `csv_import_dedupe_batch_membership_and_journal_invalidation` |
| `W-import-batch-membership` | `csv_import_dedupe_batch_membership_and_journal_invalidation` |
| `W-import-empty-batch` | `csv_import_dedupe_batch_membership_and_journal_invalidation` |
| `W-import-atomic` | `csv_import_failures_are_atomic_and_empty_runs_preserve_readiness` |
| `W-import-observer-commitment` | `csv_enrichment_preserves_closed_observer_evidence` |
| `W-phoenix-unit` | `python_file_import_vectors` |
| `W-phoenix-header` | `python_file_import_vectors` |
| `W-phoenix-sign` | `python_file_import_vectors` |
| `W-phoenix-zero-type` | `python_file_import_vectors` |
| `W-phoenix-mining-fee` | `python_file_import_vectors` |
| `W-phoenix-rate` | `python_file_import_vectors` |
| `W-phoenix-payment-hash` | `python_file_import_vectors` |
| `W-phoenix-note-authority` | `phoenix_metadata_is_idempotent_preserves_authored_notes_and_invalidates_once` |
| `W-phoenix-tag-idempotency` | `phoenix_metadata_is_idempotent_preserves_authored_notes_and_invalidates_once` |
| `W-phoenix-metadata-invalidation` | `phoenix_metadata_is_idempotent_preserves_authored_notes_and_invalidates_once` |
| `W-import-exchange-rank` | `csv_exchange_enrichment_respects_metadata_authorship_and_price_rank` |
| `W-import-metadata-authority` | `csv_exchange_enrichment_respects_metadata_authorship_and_price_rank` |
| `W-import-raw-text` | `python_file_import_vectors` |
| `W-import-privacy-flags` | `python_file_import_vectors` |
| `W-import-privacy-authorship` | `csv_evidence_enrichment_preserves_authored_boundaries_and_promotes_exact_links` |
| `W-import-privacy-source` | `csv_evidence_enrichment_preserves_authored_boundaries_and_promotes_exact_links` |
| `W-import-payment-promotion` | `csv_evidence_enrichment_preserves_authored_boundaries_and_promotes_exact_links` |
| `W-import-funding-vout` | `csv_evidence_enrichment_preserves_authored_boundaries_and_promotes_exact_links` |
| `W-import-kind-source` | `csv_evidence_enrichment_preserves_authored_boundaries_and_promotes_exact_links` |

Other file importers and all sync commands remain `not_implemented`. AI
projections remain in Python. No Python production code or reference bug fix
is part of this run. The optional nonterminal-wildcard and malformed-descriptor
audit remains open, including Python's `/0/*/1` acceptance and direct desktop
preview exception gaps described above.

## Backup archive checkpoint (R5, partial)

This initial checkpoint is superseded for installation and recovery by the
[continuation below](#backup-install-and-recovery-continuation-r5).

Branch t3code/r5-backup starts at secrets tip a051e1ef. It adds
backup export and staging-only backup import. **The requested backup slice
is incomplete.** Import with --install returns not_implemented before
decryption or target creation. All four ui.backup desktop kinds remain
unported; no restore publication or interruption recovery is claimed.
Maintenance, diagnostics and AI projections remain out of this checkpoint.

The backup crate owns archive format and validation. Its CLI family uses the
real-store local service seam. Export authenticates through the existing
fd/remembered/TTY chain and required-schema path, then uses the store's shared
bounded online-copy primitive and SQLCipher configuration. Private sibling
temporary output is replaced only after encryption and file fsync, as Python
does. Staging import leaves its private decrypted workspace to the caller;
errors clean it. Neither command contacts a provider or invokes Python.

Passphrase mode reuses the already locked Rust age crate. Python actually
prefers in-process pyrage in this mode; it has no supported safe binary
passphrase invocation. The historical age_backend value "pyrage" identifies
the compatible in-process format; Rust does not import pyrage. Recipient mode
retains Python's age-then-rage PATH search, repeated --recipient and
--decrypt --identity arguments, file streams, inherited environment/stderr
and child exit refusal. A local scripted child exercises discovery, exact
arguments and reciprocal recipient-format interoperability. Native age/rage
and Windows discovery remain acceptance gates.

There are no registry version/checksum changes: the lockfile adds only the
internal crate's existing dependencies. The age and tar license rows cover
both callers; std provides neither format. Development scrypt/Salsa20 are
optimized so age's elapsed-time calibration uses comparable primitives to
optimized pyrage; work limits remain age defaults.

Reference behavior, explicitly preserved:

- Plaintext export is refused with plaintext_database, Python's exact
  message/hint and exit 1. There is no plaintext export round trip.
- Schema-1 manifests record attachment **counts**, without content hashes.
  Import checks the existing schema/database path, declared sidecars and
  regular-file count. It does not authenticate the inner DB, check attachment
  contents or change credentials. CLI staging accepts plaintext payloads that
  Python accepts. A possible future hash-based format is in TODO.
- Non-inline provider references remain metadata and restore as unavailable
  warnings. A lingering inline key on an OS-backed reference blocks export
  without disclosing its value.

The normal tar reader inspects all logical members before payload extraction:
absolute paths, every parent component, backslashes, NUL, normalized duplicates,
links, devices, FIFOs, unknown types and files over 4 GiB retain Python's
refusal code/reason. Unix surrogateescape bytes and GNU/PAX extended paths are
preserved. Permissions retain Python's stripped member mode.
Live refusal comparisons use exact stdout/stderr/exit bytes. Manifest success
comparison excludes only its creation clock; round trips compare complete
typed/all-object logical dumps with no masked DB values, plus sidecar bytes.

### Backup verification and remaining work

Linux verification used Rust 1.99.1, locked uv/Python 3.11,
`MATURIN_NO_INSTALL_RUST=1`, offline Cargo and disposable book, home and
ownership roots. `uv sync --locked --python 3.11` succeeded. The archive oracle
ran through the hybrid candidate: 26 cases passed together and the recipient
child case passed separately. Only explicit reference calls used Python.

| Command | Rust-served calls | Python fallbacks |
| --- | ---: | ---: |
| `backup export` | 4 | 0 |
| `backup import` | 26 | 0 |

These counts include refusals and the closed installation path; they do not
establish installation or desktop-kind acceptance.

| Payload / direction | Result |
| --- | --- |
| SQLCipher, Rust export -> Python staging import | Pass; complete normalized book dump and sidecars match |
| SQLCipher, Python export -> Rust staging import | Pass; complete normalized book dump and sidecars match |
| Plaintext export, either runtime | Refused with identical stdout/stderr/exit bytes (`plaintext_database`, exit 1) |
| Plaintext payload accepted by Python -> Rust staging import | Pass; complete normalized dump matches |
| Recipient encryption, both directions | Pass with the scripted age-compatible child; native age/rage not run |

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | Pass |
| Workspace clippy, all targets, `-D warnings` | Pass |
| Binary clippy with `test-harness`, all targets, `-D warnings` | Pass |
| `cargo test --workspace --locked` | Sandbox denied socket creation: `kassiber-net` 30 passed / 16 failed; separate rates run 16 passed / 11 failed for the same restriction |
| Workspace tests excluding `kassiber-net` and `kassiber-rates` | Pass |
| Backup crate tests / binary harness tests | 6 / 14 passed |
| Harness build, storage probe and secrets probe | Pass |
| Required shell-parity, hybrid-candidate, CLI-runner and `tests/rust_port` suite | 63 passed, 178 subtests passed |
| Backup hybrid oracle | 27 passed across the two runs above |
| Secrets services/lifecycle plus Python backup/daemon/secrets regressions, isolated ownership namespace | 157 passed, 29 native cases skipped, 2 subtests passed |
| Mutation run | All 19 killed; five unmutated filters passed |
| Inventory, shard manifest and contract checks | Pass |

The socket failures need an unrestricted orchestrator run. Native credential
cases remain gated; they are not established by injected test credentials.
Existing in-process Python ownership tests require a disposable
`_owner_lock_root` injection here because the account namespace is read-only.
The full repository quality gate and release packaging were not run.

Run the mutation catalog with python crates/backup/scripts/mutation_check.py;
the quality gate checks applicability. All 19 entries are killed with five
passing unmutated test filters:

| Mutation | Killing Rust test filter |
| --- | --- |
| B-path-absolute | path_types_sizes_and_normalized_duplicates_match_python |
| B-path-backslash | path_types_sizes_and_normalized_duplicates_match_python |
| B-path-nul | path_types_sizes_and_normalized_duplicates_match_python |
| B-path-traversal | path_types_sizes_and_normalized_duplicates_match_python |
| B-path-top | path_types_sizes_and_normalized_duplicates_match_python |
| B-type-symlink | path_types_sizes_and_normalized_duplicates_match_python |
| B-type-hardlink | path_types_sizes_and_normalized_duplicates_match_python |
| B-type-device | path_types_sizes_and_normalized_duplicates_match_python |
| B-type-fifo | path_types_sizes_and_normalized_duplicates_match_python |
| B-size-bound | path_types_sizes_and_normalized_duplicates_match_python |
| B-path-duplicate | path_types_sizes_and_normalized_duplicates_match_python |
| B-manifest-schema | manifest_paths_schema_counts_and_declared_sidecars_match_python |
| B-manifest-db-path | manifest_paths_schema_counts_and_declared_sidecars_match_python |
| B-manifest-count-type | manifest_paths_schema_counts_and_declared_sidecars_match_python |
| B-manifest-count | manifest_paths_schema_counts_and_declared_sidecars_match_python |
| B-manifest-sidecars | manifest_paths_schema_counts_and_declared_sidecars_match_python |
| B-plaintext-refusal | export_plaintext_refusal_keeps_the_python_message_hint_and_destination |
| B-secret-conflict | provider_refs_never_materialize_external_secrets_or_hide_inline_conflicts |
| B-install-gate | unverified_installation_never_decrypts_or_creates_the_target |

Remaining: same-filesystem installation, retained recovery and rollback,
all four desktop kinds (bound/expiring preview, staging and target
fingerprints, confirmation, close-under-lease, credential invalidation and
locked outcomes), and shared cross-runtime restore recovery. No restore
interruption cases ran here. Python's current installer has no durable
resume journal; a reference fix needs a separately committed, failing-first
regression before product changes. No Python reference bug is claimed fixed.
Python product code is unchanged.

Also open: compressed tar containers accepted by Python's auto-detecting
tarfile reader, export-side hardlink metadata/count parity, exhaustive malformed
tar/JSON and filesystem-error byte parity,
concurrent-source/failed-output publication interruptions, native recipient
binaries, Windows/macOS, interactive TTY acceptance and remaining backup
disposition ports. These limits prevent full slice acceptance.

## Backup install and recovery continuation (R5)

`t3code/r5-backup-2` starts at `2c14fdc0`. `backup import --install` now
reserves both maintenance roles before decryption and retains those reservations
across every rename, including an absent destination database. Staging-only
import does not install or reserve the target; any pending startup recovery
still runs first under its own maintenance exclusion. Success/refusal
envelopes preserve Python's field order; the success comparison maps only the
random retained-copy path. Plaintext export remains refused, plaintext import
payloads retain Python's acceptance, and attachment counts still have no hashes.
AI projections stay in Python.

### Python restore reference correction

The backup continuation first fixes a reference bug independently of the Rust
port. Killing the old installer immediately after moving the live database
made `open_db` initialize an empty book on restart. The failing-first regression
is `test_restart_after_live_database_move_recovers_original_before_open` in
`tests/test_backup_recovery.py`. A shared-format restore intent now precedes
live moves. Pending intents roll back; committed intents finish cleanup and
retain the rollback copy. Recovery runs under both owner roles before layout,
unlock-policy reads or database creation. Invalid intent metadata fails closed.
The same module injects I/O errors at every observed install boundary.

The independent local commit is `64378ed2`; it includes the Python regression
and updated error/environment contracts. Its regression failed against the old
installer before the fix. Rust implementation is kept in a following commit.

### Shared publication and recovery

`crates/store/src/restore.rs` and `kassiber/backup/install.py` share version 1
of `.restore-pending.json`. The bounded 64 KiB, owner-checked intent binds the
canonical data root, a `.restore-install-*` prepared tree, its paired
`pre-restore-*` recovery tree, ordered allowlisted original/incoming entries,
and a pending/committed phase. Symlinked destinations/artifacts, invalid names,
wrong roots, unsafe journal permissions and malformed intent are refused.

Preparation copies the validated staging tree onto the destination filesystem
and flushes files and directories. Pending intent is atomically published and
flushed before moving the original database, SQLite journals, attachments,
settings, backend config and exports into the recovery tree. Incoming database
and sidecars are renamed into place and both parents are flushed at each move;
old SQLite journals and exports never accompany the restored database. Only
publication of the committed intent establishes completion. The transaction
provides recoverable publication across several entries, not one atomic rename
of the whole container.

Startup takes both owner roles before recovery, ahead of settings validation,
layout writes, unlock-policy reads and database creation. The binary resolves
again afterwards against the recovered sidecars. A pending intent restores the
original entries or removes incoming entries with no original counterpart;
a committed intent only finishes cleanup and retains the old recovery tree.
Low-level book/layout APIs refuse a pending intent. Repeating recovery after a
second kill preserves the same result. I/O failures before commit roll back;
failures after commit report the completed install when cleanup can finish.
An unfinished rollback keeps the intent and artifacts for explicit recovery.

### Interruption matrix

`tests/test_rust_backup_install.py` uses real SIGKILL children, both plaintext
and SQLCipher books, empty and populated targets, and complete DB/sidecar byte
comparisons. The frozen observer inventory is checked in both runtimes. CLI
scenarios additionally compare complete logical dumps, retained copies and
wire bytes. Import probes are explicit `test-hooks` examples and are never
packaged. All **1,348 interruption scenarios** below pass; the install module
passes all 1,370 cases and the combined archive/install oracle passes 1,397.

| Cases | Dimensions | Result |
| --- | --- | --- |
| 832 install interruptions | 68 populated + 36 empty checkpoints × 2 codecs × 4 starter/restart combinations | Pass |
| 416 second interruptions during rollback | 44 populated + 8 empty checkpoints × 2 codecs × 4 runtime combinations | Pass |
| 64 import interruptions before publication | 4 phases × 2 target layouts × 2 codecs × 4 runtime combinations | Pass |
| 32 ordinary CLI restarts | 4 publication points × 2 codecs × 4 runtime combinations | Pass |
| 4 recovery owner refusals | 2 roles × 2 codecs; both CLI refusal envelopes compared | Pass |

These are process-interruption checks, not filesystem power-loss simulations.
Kills before intent can leave private, unreferenced temporary workspaces, as in
Python; startup never scans or deletes arbitrary system-temp paths.

### Continuation verification and remaining work

Linux verification uses Rust 1.99.1, locked uv/Python 3.11,
`MATURIN_NO_INSTALL_RUST=1`, offline Cargo and disposable book, home and
ownership roots. `uv sync --locked --python 3.11` succeeds. The reference
backup/recovery/desktop regression run passes 26 cases with an isolated owner
namespace. The secrets services/lifecycle hybrid run passes 91 cases, with
29 native-keyring cases gated by the sandbox's denied private D-Bus socket.

| Check | Result |
| --- | --- |
| fmt and both workspace/harness clippy checks with `-D warnings` | Pass |
| `cargo test --workspace --locked` | Socket permission failures in `kassiber-net`: 30 passed / 16 failed |
| Separate rates run | 16 passed / 11 socket permission failures |
| Workspace excluding the network/rates crates | 880 passed; final affected backup/store pass: 63 passed |
| Binary harness tests / harness and test-probe builds | 14 passed / builds pass |
| Install/owner/ordinary-restart CLI recheck after the final rebuild | 50 passed |
| Required shell-parity, hybrid-candidate, CLI-runner and `tests/rust_port` suite | 63 passed, 178 subtests passed |
| Mutation catalog | All 47 killed; 11 unmutated filters pass |
| Inventory, shard manifest and contracts | Pass |

The full workspace socket tests need an unrestricted orchestrator run. The
repository-wide quality gate and release packaging were not run. The fresh
combined backup oracle records the following routing counts, including success
and refusal invocations; explicit Python reference calls are not fallbacks.

| Command | Rust-served calls | Python fallbacks |
| --- | ---: | ---: |
| `backup export` | 4 | 0 |
| `backup import` (including install) | 36 | 0 |
| `status` (restart/owner admission) | 20 | 0 |

The four desktop backup kinds have no native acceptance at this boundary.

The checked-in mutation table is `crates/backup/scripts/mutations.json`, run by
`python crates/backup/scripts/mutation_check.py`. It replaces the old
`B-install-gate` with real install/recovery coverage. The complete current table
and named killers follow below.

| Mutation | Killing Rust test filter |
| --- | --- |
| `B-path-absolute` | `path_types_sizes_and_normalized_duplicates_match_python` |
| `B-path-backslash` | `path_types_sizes_and_normalized_duplicates_match_python` |
| `B-path-nul` | `path_types_sizes_and_normalized_duplicates_match_python` |
| `B-path-traversal` | `path_types_sizes_and_normalized_duplicates_match_python` |
| `B-path-top` | `path_types_sizes_and_normalized_duplicates_match_python` |
| `B-type-symlink` | `path_types_sizes_and_normalized_duplicates_match_python` |
| `B-type-hardlink` | `path_types_sizes_and_normalized_duplicates_match_python` |
| `B-type-device` | `path_types_sizes_and_normalized_duplicates_match_python` |
| `B-type-fifo` | `path_types_sizes_and_normalized_duplicates_match_python` |
| `B-size-bound` | `path_types_sizes_and_normalized_duplicates_match_python` |
| `B-path-duplicate` | `path_types_sizes_and_normalized_duplicates_match_python` |
| `B-manifest-schema` | `manifest_paths_schema_counts_and_declared_sidecars_match_python` |
| `B-manifest-db-path` | `manifest_paths_schema_counts_and_declared_sidecars_match_python` |
| `B-manifest-count-type` | `manifest_paths_schema_counts_and_declared_sidecars_match_python` |
| `B-manifest-count` | `manifest_paths_schema_counts_and_declared_sidecars_match_python` |
| `B-manifest-sidecars` | `manifest_paths_schema_counts_and_declared_sidecars_match_python` |
| `B-plaintext-refusal` | `export_plaintext_refusal_keeps_the_python_message_hint_and_destination` |
| `B-secret-conflict` | `provider_refs_never_materialize_external_secrets_or_hide_inline_conflicts` |
| `B-install-owner-order` | `installation_requires_both_owner_roles_before_decryption_and_preserves_live_files` |
| `R-broker-role` | `installation_requires_both_owner_roles_before_decryption_and_preserves_live_files` |
| `R-desktop-role` | `installation_requires_both_owner_roles_before_decryption_and_preserves_live_files` |
| `R-intent-before-moves` | `durable_install_boundaries_recover_original_or_complete_and_retain_copy` |
| `R-initial-phase` | `durable_install_boundaries_recover_original_or_complete_and_retain_copy` |
| `R-commit-phase` | `durable_install_boundaries_recover_original_or_complete_and_retain_copy` |
| `R-save-original` | `durable_install_boundaries_recover_original_or_complete_and_retain_copy` |
| `R-publish-incoming` | `durable_install_boundaries_recover_original_or_complete_and_retain_copy` |
| `R-no-old-journals` | `durable_install_boundaries_recover_original_or_complete_and_retain_copy` |
| `R-pending-rollback` | `durable_install_boundaries_recover_original_or_complete_and_retain_copy` |
| `R-rollback-move` | `rollback_is_repeatable_at_every_recovery_boundary` |
| `R-recovery-copy-retention` | `durable_install_boundaries_recover_original_or_complete_and_retain_copy` |
| `R-path-binding` | `invalid_intents_and_target_links_fail_closed` |
| `R-prepared-path` | `invalid_intents_and_target_links_fail_closed` |
| `R-name-allowlist` | `invalid_intents_and_target_links_fail_closed` |
| `R-intent-size` | `invalid_intents_and_target_links_fail_closed` |
| `R-target-symlink` | `invalid_intents_and_target_links_fail_closed` |
| `R-absent-target` | `rollback_is_repeatable_at_every_recovery_boundary` |
| `R-sync-journal-file` | `durable_install_boundaries_recover_original_or_complete_and_retain_copy` |
| `R-sync-journal-directory` | `durable_install_boundaries_recover_original_or_complete_and_retain_copy` |
| `R-sync-copy-file` | `durable_install_boundaries_recover_original_or_complete_and_retain_copy` |
| `R-open-admission` | `invalid_intents_and_target_links_fail_closed` |
| `R-recover-before-settings` | `startup_recovers_before_reading_partial_sidecar_configuration` |
| `R-sync-move-source-directory` | `durable_install_boundaries_recover_original_or_complete_and_retain_copy` |
| `R-sync-move-target-directory` | `durable_install_boundaries_recover_original_or_complete_and_retain_copy` |
| `R-layout-admission` | `startup_recovers_before_reading_partial_sidecar_configuration` |
| `B-stage-owner` | `staging_only_import_never_requires_exclusive_ownership` |
| `R-rollback-failure-code` | `failed_rollback_preserves_intent_and_reports_recovery_location` |
| `R-rollback-location-none` | `failed_rollback_preserves_intent_and_reports_recovery_location` |

Remaining: all four desktop kinds (`ui.backup.export`, `ui.backup.preview`,
`ui.backup.apply`, `ui.backup.cancel`) remain unported. Their bound/expiring
preview, staging/target fingerprints, confirmation, close-under-lease,
credential invalidation and locked outcomes still need native acceptance.
CLI install retains Python's known debt around credential invalidation and
proof of encrypted inner payload, tracked separately in TODO; it does not
silently adopt the stricter desktop contract.

Compressed tar handling, export hardlink metadata/count parity, exhaustive
malformed-input and filesystem-error byte parity, failed-output/source-change
publication, native age/rage, platform/TTY checks and the remaining backup
dispositions are open. No external dependency or lockfile change is introduced
by this continuation. The complete backup service slice is not accepted.

## Desktop backup continuation (R5)

`t3code/r5-backup-3` starts at `0d775706`. All four desktop kinds are native:
`ui.backup.export`, `ui.backup.preview`, `ui.backup.apply` and
`ui.backup.cancel`. They share `crates/backup/src/desktop.rs`; the daemon's
family adapter only supplies the live book, lifecycle and ownership seams.
There is no Python dispatch or new egress. AI projections remain in Python.
Python product code, including the independently reviewed `64378ed2` restore
installer, is unchanged.

The checkpoint supersedes the desktop omissions in the preceding two backup
checkpoints. It preserves the reference's desktop contract:

- Export validates an absolute `.kassiber` destination outside the active
  container, refuses plaintext before requesting an outer passphrase, and
  maps destination I/O failures to `backup_export_failed`. Python's PAX disk
  metadata and 10 KiB tar record padding now preserve the reported encrypted
  artifact size. Unix surrogateescape names, Unicode and long paths survive.
- Preview clears the previous staging first, requires both passphrases,
  authenticates the encrypted inner database without migration or the active
  operator binding, and checks integrity. SQLCipher authentication diagnostics
  are suppressed only for this incoming validation, restoring the prior log
  level afterwards. Invalid archives/credentials/integrity use the same
  redacted refusal; missing files and required passphrases retain their codes.
- Summaries contain sorted workspace/book labels (100 of each), the full book
  count, attachment file count, existing replaced entries, unavailable-secret
  flag and the 600-second lifetime. They contain no wallet/configuration
  projection, attachment hashes or passphrases. A preview retains one private
  validated snapshot and no password.
- Capabilities use 32 OS-random bytes. Expiry cleans decrypted staging while
  idle as well as on requests; lock and shutdown clear it. Cancel works while
  locked, clears only the matching capability and otherwise succeeds as a
  no-op. These are synchronous handlers: Python emits no backup progress
  records and cancel does not interrupt an in-flight export/import/apply.
- Apply requires exact `RESTORE` confirmation, the current capability, and
  unchanged target and staging commitments. The target fingerprint includes
  canonical container/project, connection identity, database and recursive
  restore-sidecar metadata, connection changes and SQLite data version. The
  staging digest commits relative names and bytes and refuses symbolic links.
- Apply retains the desktop lease, excludes the broker role, rechecks the
  target immediately before checkpoint/close and refuses busy WAL readers.
  It installs the validated snapshot through the shared restore transaction,
  retains the rollback copy, invalidates native credential guards and disables
  remembered unlock without probing a credential store. It reloads local
  runtime configuration/project metadata and leaves the restored book locked.
  Credential-setting failures warn on a completed restore. Failures after
  close carry `details.locked: true`; pre-close failures retain the connection.
  Owner refusals retain the preview; consumed previews clean up on either
  outcome. Rust has no ported background backup/analysis workers to retire.

`tests/test_rust_backup_desktop.py` compares raw response JSONL, including
field order, separators, request IDs and the absence of extra progress records.
It injects only OS-random preview/credential tokens and the private install
suffix through compile-time test hooks; it normalizes no response fields.
Both runs use the same database inode, moving it back from retained recovery
between runs rather than allowing hard-link aliases. Tests compare complete
logical incoming books and retained exports, and assert native routing through
the hybrid candidate. The deterministic hooks are absent in production.

### Desktop verification

The fresh hybrid desktop oracle passes **11 cases** with byte-identical JSONL
on the same fixtures. Its measured routing includes successful and refused
requests; every daemon session is served by Rust:

| Kind | Rust | Python fallback |
| --- | ---: | ---: |
| `ui.backup.export` | 13 | 0 |
| `ui.backup.preview` | 20 | 0 |
| `ui.backup.apply` | 14 | 0 |
| `ui.backup.cancel` | 4 | 0 |

Formatting and both workspace/all-targets and `kassiber --features test-harness`
clippy checks pass with warnings denied. The harness and both restore probes
build offline. The Python desktop/restore reference regressions pass 19 cases.
The archive hybrid oracle passes 27 cases, routing four exports and 26 imports
through Rust with zero Python fallbacks. The required shell parity, hybrid
candidate, CLI runner and `tests/rust_port` suite passes 63 tests and 178 subtests;
inventory, contracts and shard validation pass.

The workspace excluding `kassiber-net` and `kassiber-rates` passes **892 tests**;
the harness tests pass 14. Full workspace acceptance remains blocked by sandbox
loopback permissions: `kassiber-net` reaches 30 passed / 16 failed and the
separate rates run reaches 16 passed / 11 failed, all failures at socket creation
with `Operation not permitted`. Workspace verification uses a separate build
directory so it cannot replace the harness during the byte oracles.
The fresh install/recovery oracle passes **1,370 cases**, including the retained
1,348 killed-process interruption scenarios across Python/Rust starters and
restarters. No Python reference bug was found or fixed in this continuation.

The mutation catalog now has **107 entries**: the retained 47 archive/install/
recovery rules, 58 desktop rules and two export-format rules. The initial run
exposed a missing assertion: removing the early target check still failed at
close with the same error code. Its named test now checks the full refusal and
that ownership was never acquired. A fresh complete run passes all 22 named
unmutated baselines and kills **all 107 mutations**.

The new entries and their named native killers follow. The catalog contains
the exact source replacements and rule descriptions; run
`python crates/backup/scripts/mutation_check.py` to repeat the scratch-tree
check. Every named baseline must pass before mutation.

| Mutation | Killing Rust test filter |
| --- | --- |
| `D-cancel-token` | `cancel_capabilities_expiry_and_cleanup_match_reference` |
| `D-cancel-locked` | `cancel_capabilities_expiry_and_cleanup_match_reference` |
| `D-cancel-cleanup` | `cancel_capabilities_expiry_and_cleanup_match_reference` |
| `D-expiry` | `cancel_capabilities_expiry_and_cleanup_match_reference` |
| `D-target-locked` | `export_paths_plaintext_and_secret_refusals_match_reference` |
| `D-export-suffix` | `export_paths_plaintext_and_secret_refusals_match_reference` |
| `D-export-contained` | `export_paths_plaintext_and_secret_refusals_match_reference` |
| `D-export-plaintext` | `export_paths_plaintext_and_secret_refusals_match_reference` |
| `D-preview-replace` | `preview_validates_encryption_integrity_summaries_counts_and_redaction` |
| `D-preview-path` | `preview_validates_encryption_integrity_summaries_counts_and_redaction` |
| `D-preview-outer` | `preview_secret_path_and_validation_error_contracts_match_reference` |
| `D-preview-inner` | `preview_secret_path_and_validation_error_contracts_match_reference` |
| `D-preview-encrypted` | `preview_validates_encryption_integrity_summaries_counts_and_redaction` |
| `D-preview-integrity` | `preview_validates_encryption_integrity_summaries_counts_and_redaction` |
| `D-preview-workspaces` | `preview_validates_encryption_integrity_summaries_counts_and_redaction` |
| `D-preview-books` | `preview_validates_encryption_integrity_summaries_counts_and_redaction` |
| `D-preview-book-count` | `preview_validates_encryption_integrity_summaries_counts_and_redaction` |
| `D-preview-redacted-target` | `preview_validates_encryption_integrity_summaries_counts_and_redaction` |
| `D-preview-unavailable` | `preview_validates_encryption_integrity_summaries_counts_and_redaction` |
| `D-preview-lifetime` | `preview_validates_encryption_integrity_summaries_counts_and_redaction` |
| `D-preview-error-mask` | `preview_secret_path_and_validation_error_contracts_match_reference` |
| `D-preview-token-random` | `preview_validates_encryption_integrity_summaries_counts_and_redaction` |
| `D-fingerprint-project` | `fingerprint_pins_connection_project_database_sidecars_and_sqlite_generations` |
| `D-fingerprint-connection` | `fingerprint_pins_connection_project_database_sidecars_and_sqlite_generations` |
| `D-fingerprint-database` | `fingerprint_pins_connection_project_database_sidecars_and_sqlite_generations` |
| `D-fingerprint-sidecars` | `fingerprint_pins_connection_project_database_sidecars_and_sqlite_generations` |
| `D-fingerprint-changes` | `fingerprint_pins_connection_project_database_sidecars_and_sqlite_generations` |
| `D-fingerprint-version` | `fingerprint_pins_connection_project_database_sidecars_and_sqlite_generations` |
| `D-fingerprint-stat` | `fingerprint_pins_connection_project_database_sidecars_and_sqlite_generations` |
| `D-digest-links` | `apply_confirmation_tokens_target_and_stage_guards_match_reference` |
| `D-digest-paths` | `apply_confirmation_tokens_target_and_stage_guards_match_reference` |
| `D-digest-content` | `apply_confirmation_tokens_target_and_stage_guards_match_reference` |
| `D-apply-confirm` | `apply_confirmation_tokens_target_and_stage_guards_match_reference` |
| `D-apply-token` | `apply_confirmation_tokens_target_and_stage_guards_match_reference` |
| `D-apply-target` | `apply_confirmation_tokens_target_and_stage_guards_match_reference` |
| `D-apply-staging` | `apply_confirmation_tokens_target_and_stage_guards_match_reference` |
| `D-apply-owner` | `apply_ownership_close_recheck_failures_and_locked_outcomes_match_reference` |
| `D-apply-maintenance` | `apply_ownership_close_recheck_failures_and_locked_outcomes_match_reference` |
| `D-apply-consumption` | `apply_ownership_close_recheck_failures_and_locked_outcomes_match_reference` |
| `D-apply-close` | `apply_ownership_close_recheck_failures_and_locked_outcomes_match_reference` |
| `D-apply-install` | `apply_ownership_close_recheck_failures_and_locked_outcomes_match_reference` |
| `D-apply-credentials` | `apply_ownership_close_recheck_failures_and_locked_outcomes_match_reference` |
| `D-apply-warning` | `apply_ownership_close_recheck_failures_and_locked_outcomes_match_reference` |
| `D-apply-locked-error` | `apply_ownership_close_recheck_failures_and_locked_outcomes_match_reference` |
| `D-apply-release` | `apply_ownership_close_recheck_failures_and_locked_outcomes_match_reference` |
| `D-apply-cleanup` | `apply_ownership_close_recheck_failures_and_locked_outcomes_match_reference` |
| `D-close-recheck` | `apply_ownership_close_recheck_failures_and_locked_outcomes_match_reference` |
| `D-close-readers` | `close_refuses_busy_wal_readers_before_retiring_connection` |
| `D-credentials-generations` | `restore_credentials_revoke_remembered_unlock_without_native_access` |
| `D-credentials-disable` | `restore_credentials_revoke_remembered_unlock_without_native_access` |
| `D-incoming-quiet` | `incoming_authentication_is_quiet_and_ignores_active_operator_binding` |
| `D-incoming-binding` | `incoming_authentication_is_quiet_and_ignores_active_operator_binding` |
| `D-preview-preserved-errors` | `preview_secret_path_and_validation_error_contracts_match_reference` |
| `D-preview-replaces` | `preview_validates_encryption_integrity_summaries_counts_and_redaction` |
| `D-preview-count-only` | `preview_validates_encryption_integrity_summaries_counts_and_redaction` |
| `D-preview-expiry-field` | `preview_validates_encryption_integrity_summaries_counts_and_redaction` |
| `D-apply-restored` | `apply_ownership_close_recheck_failures_and_locked_outcomes_match_reference` |
| `D-apply-locked` | `apply_ownership_close_recheck_failures_and_locked_outcomes_match_reference` |
| `E-record-padding` | `disk_members_preserve_pax_names_mtimes_and_python_record_padding` |
| `E-pax-mtime` | `disk_members_preserve_pax_names_mtimes_and_python_record_padding` |

Remaining: compressed tar import, hardlink export/count parity, broader
malformed-input and filesystem-error byte parity, source-change/output failure
publication matrices, native age/rage and Windows/macOS/TTY acceptance, and the
remaining backup test dispositions. The complete backup service slice and
repository-wide quality/release gates are not claimed by this checkpoint.

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
`ui.transactions.history`, `ui.transactions.history.revert`,
`ui.metadata.bip329.preview/import/export`, `ui.transactions.metadata.update`,
and `ui.transactions.list/search/extremes`. Document kinds are
`ui.documents.list/create/attach/import_report`; stored commercial kinds are
`ui.btcpay.provenance.list/links/suggest` and `ui.transactions.commercial_context`.
The context read retains invoice/payment/document provenance, payout-batch exact
amounts and fiat-currency barriers, and stored sync freshness. It performs no
provider lookup. These additions do not port AI projections.
BIP329 daemon exports use the resolved
managed reports directory; URLs in labels remain literal local evidence.
Metadata writes append Python-compatible edit events and fields atomically with
journal-input invalidation; no-op edits append nothing. Reverts append forward edits and
preserve the active-custody exclusion guard. Grouped saves validate note/tag,
pricing, review and tax fields together, preserving omitted/null distinctions,
32-digit decimal arithmetic and authored price scale. One changed save appends
one ordered event; timestamp-only pricing changes are no-ops. The separate
Python reference correction rejects NaN and infinity before any metadata writes.
The three BIP329 kinds share the CLI
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

The chain-client checkpoint below now serves five of those egress kinds.
`ui.backends.lightning.test` remains deferred with the node-snapshot adapters.

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

## Reports read checkpoint (R5, partial)

`crates/reports` reads the journal tables that Python still produces. The
CLI and desktop adapters delegate through `Book::reports` on the admitted
real-store connection. A read savepoint covers scope resolution, admission
and projection; report reads neither rebuild journals nor change snapshots.
There is no Python subprocess, connector, credential or acquisition path in
this service. No external dependency or version is added. AI projections stay
in Python.

Implemented CLI leaves are `reports summary`, `capital-gains`, `balance-sheet`,
`portfolio-summary`, and `filed-snapshots list`. The desktop kinds are
`ui.reports.summary`, `ui.reports.balance_sheet` and
`ui.reports.portfolio_summary`; their readiness advertisement follows Python
order. Summary includes Python's dedicated table/plain presentation. The
snapshot-list envelope retains the explicit `reports.filed-snapshots.list`
kind, rather than the generic parent-derived kind.

Live reports retain network review, tax-country support, authored component
integrity, persisted quantity blockers and journal freshness, in that order.
Freshness requires a processing timestamp, the active transaction count and
the current input version. Missing quantity state fails closed with the
reference code/hint; public quantity issues retain order and the twenty-row
bound. The component loader reuses the R2 validator and normalized wallet
configuration facts. Report admission does not read saved evidence payloads.
Quarantines remain disclosed by summary exactly as Python; the port does not
invent a broader quarantine barrier.

Gains preserve disposal/income and taxability filters, exclude `neu_swap`,
and order split rows by occurrence, creation and entry ID. Explanation
references bind database, book and processed input version. Holdings preserve
integer msat, legacy stored basis, exact-price preference, transaction fallback
ordering and cached-quote priority (timestamp, manual, fetched time, source).
Decimal arithmetic uses precision 32 and HALF_EVEN before Python-compatible
float serialization. Summary retains both float accumulation stages and
stable null/empty account ordering. An absent optional market quote produces
the reference market value while retaining stored basis. Snapshot lists expose
saved/filed captured summaries with stale live journals and remain read-only.

`tests/test_rust_reports.py` compares exit status, stdout and stderr bytes for
each live CLI read on generic FIFO, generic HIFO and Austrian
`MOVING_AVERAGE_AT` books, at 64 and 3,000 transactions. Books include old/new
acquisitions, income, multi-lot sales and fractional satoshis. It also compares
cached rates, stale inputs, scoped/unknown wallets, snapshot listing,
table/plain output and native daemon success/validation records. The module
runs in the serial-daemon CI lane with the harness binary. The frozen Python
oracle adds 49 adversarial stored-row cases, checked by eight named Rust
tests, including component/revision/term replay and deterministic split rows.
Every vector verifies the service made no database writes.

The final ten-scenario report oracle passed. Its hybrid log records the
following calls, including refused reads; every ported read ran in Rust:

| Command | Rust | Python fallback |
| --- | ---: | ---: |
| `reports.summary` | 23 | 0 |
| `reports.capital-gains` | 11 | 0 |
| `reports.balance-sheet` | 11 | 0 |
| `reports.portfolio-summary` | 11 | 0 |
| `reports.filed-snapshots.list` | 1 | 0 |

Two `filed-snapshots create` fixture calls deliberately ran in Python. The
native daemon comparison covers eight summary requests (success and refused
arguments), one balance-sheet request and one portfolio-summary request.

| Live report JSON bytes | Generic FIFO | Generic HIFO | Austrian moving average AT |
| --- | --- | --- | --- |
| Summary | exact, 64 / 3,000 transactions | exact, 64 / 3,000 | exact, 64 / 3,000 |
| Capital gains | exact, 64 / 3,000 | exact, 64 / 3,000 | exact, 64 / 3,000 |
| Balance sheet | exact, 64 / 3,000 | exact, 64 / 3,000 | exact, 64 / 3,000 |
| Wallet holdings | exact, 64 / 3,000 | exact, 64 / 3,000 | exact, 64 / 3,000 |

The table/plain comparisons use the generic FIFO book. Saved/filed list bytes
also match after a metadata edit makes the live journals stale. The additional
CLI smoke run serves summary 6, gains 12, balance sheet 3 and portfolio 4 times
in Rust, with no fallback for those reads; unported report/export commands in
that run continue through Python.

The mutation catalog and runner are
[`crates/reports/scripts/mutations.json`](../../crates/reports/scripts/mutations.json)
and `crates/reports/scripts/mutation_check.py`. Each entry records its exact
rule, edit and named killer in `crates/reports/tests/vectors.rs` or
`crates/reports/src/sql.rs`:

| Rules | Entries | Named test |
| --- | ---: | --- |
| Admission, freshness, issue order/bounds | 11 | `readiness_vectors` |
| Gains selection, split order, quantity and reference binding | 8 | `capital_gains_vectors` |
| Holdings, Decimal precision/rounding and price/cache selection | 20 | `holdings_vectors` |
| Scope, flows, two-stage rollups and transfer projections | 25 | `summary_vectors` |
| Summary presentation precision and grouping | 3 | `summary_text_vectors` |
| Desktop float and exact-msat totals | 2 | `desktop_vectors` |
| Saved/filed period/creation/ID order | 1 | `snapshots_vectors` |
| Active component facts, allocations, terms, anchors and revisions | 6 | `components_vectors` |
| Optional-cache fallback and invalid-value refusal | 2 | `optional_quote_errors` |

All 78 mutations were killed in the final scratch-tree run, after nine named
baseline selections passed. The explicit HALF_EVEN tie is exposed through
market-value/basis cancellation rather than hidden below float precision.

Verification used Rust 1.99.1, locked Python 3.11 and Cargo offline: fmt,
workspace and harness clippy with `-D warnings`, all 941 workspace tests, the
harness build, nine native report tests, Python vector regeneration, and
inventory/contract/shard checks passed. The latter focused suite passed 29
tests and 47 subtests. Workspace transport fixtures passed outside the socket
sandbox on owned loopback listeners. `uv sync --locked --python 3.11` installed
the reference environment; subsequent locked runs used `UV_NO_SYNC=1` to keep
the sandbox from rebuilding the binding through an unavailable tool cache.

The required shell/hybrid/runner/port suite finished with 62 tests and 178
subtests passing, and one sandbox failure: the Python `secrets init` descriptor
fallback tried to create `/home/tg/.kassiber/run` on the read-only filesystem.
The additional hybrid CLI smoke/blocker/contract/shard run passed 84 tests and
51 subtests, with one unrelated AI-provider failure (`internal_error` instead
of `ai_remote_ack_required`); its cause was not established then. The
[third continuation](#reports-ai-provider-failure-diagnosis-r5-third-continuation)
diagnoses and retests both sandbox failures. These runs do not establish the
full quality gate or push readiness.

This first boundary did not establish complete reports slice acceptance.
The continuation below adds the Austrian, tax-summary and journal-entry
reads. Python remained the reference; no Python product bug or product
change was needed for the first checkpoint.

## Reports Austrian and journal read checkpoint (R5 continuation, partial)

`t3code/r5-reports-2` starts at `571190d9` and adds `reports tax-summary`,
`austrian-e1kv`, `austrian-tax-summary`, `journal-entries`, and the desktop
kind `ui.reports.tax_summary`. Nine report CLI leaves and four desktop report
kinds now run natively (80 CLI leaves overall). AI projections and journal
production remain Python. The existing read savepoint and readiness barriers
cover all new reads. Austrian profile/EUR validation follows network/country
admission and precedes component, quantity and freshness checks, as in Python.
No journal, snapshot, amendment, schema or dependency is changed.

Austrian output maps the stored semantic categories to Kennzahlen 172, 174,
176 and 801. It retains unsupported section placeholders and the separate
Alt/Neu/income sections, positive disposal losses for KZ 176, per-detail
HALF_UP integer cents, signed income amounts, fragment counts and integer
section totals. Stored-code mismatches, quarantines and report assumptions
remain visible. Basis-carry swaps, fee rows and reviewed non-taxable rows
stay outside the form. Split fragments retain occurrence/creation/ID order.
The terminal overview splits Neu gains and losses before summing their
rounded proceeds, basis and form amounts. Both command aliases produce the
same annual payload and presentation.

Generic tax summaries read the saved summary and subtract non-reportable
journal contributions. Austrian summaries group stored journal rows by the
Vienna tax year and effective income kind. They retain the reference Decimal
precision, float conversion boundaries, mixed-asset null quantities,
annual/grand totals and projected swap-fee msat totals. Vienna dates use a
frozen TZif transition table and its post-2037 EU rule, including historical
offsets; runtime reads do not consult the host timezone database. The metadata
and calendar oracle regenerate with `crates/reports/scripts/gen_metadata.py`.

Journal listing reads every stored row without the internal 200-row page
limit. It preserves signed integer msat, optional Austrian fields and reverse
occurrence/creation/ID ordering. Desktop tax filtering preserves sorted unique
nonnegative available years before filtering, Python integer conversion
(including booleans, Unicode integer strings and truncated floats), and
reference validation errors.

The new leaves support JSON/table/plain. Their explicit `--format csv` mode
returns `not_implemented`; artifact exports remain outside this read slice.
The oracle pins this refusal separately from byte comparisons. Snapshot
listing remains readable with stale journals and does not alter saved/filed
history.

The final `tests/test_rust_reports.py` run passes all twelve scenarios and
records these calls, including refused reads:

| Command | Rust | Python fallback |
| --- | ---: | ---: |
| `reports.summary` | 23 | 0 |
| `reports.capital-gains` | 11 | 0 |
| `reports.balance-sheet` | 11 | 0 |
| `reports.portfolio-summary` | 11 | 0 |
| `reports.tax-summary` | 11 | 0 |
| `reports.journal-entries` | 11 | 0 |
| `reports.austrian-e1kv` | 14 | 0 |
| `reports.austrian-tax-summary` | 14 | 0 |
| `reports.filed-snapshots.list` | 1 | 0 |

Two snapshot-creation fixture calls run in Python. Native daemon sessions on
all three book types compare 27 tax-summary, 24 summary, three balance-sheet
and three portfolio-summary responses, with zero fallback for these kinds.
They cover tax year strings, floats, booleans, unknown arguments and invalid
types. CLI table/plain comparisons use the Austrian book, including both
Austrian aliases. The snapshot list comparison uses saved/filed captures
after a metadata edit makes live journals stale.

| JSON bytes, 64 / 3,000 transactions | Generic FIFO | Generic HIFO | Austrian moving average AT |
| --- | --- | --- | --- |
| Summary | exact | exact | exact |
| Capital gains | exact | exact | exact |
| Balance sheet | exact | exact | exact |
| Portfolio summary | exact | exact | exact |
| Tax summary | exact | exact | exact |
| Journal entries | exact | exact | exact |
| Austrian E1kv | exact profile refusal | exact profile refusal | exact |
| Austrian tax summary | exact profile refusal | exact profile refusal | exact |

The frozen stored-row oracle now has 145 cases, including every Austrian
category, per-row cent ties, mixed assets, year boundaries, excluded rows,
swap fees, full journal listing, nullable fields and disclosure absence as
well as presence. All cases assert that the read made no database writes.
The 174-entry mutation catalog retains the previous 78 and adds these 96:

| New rules | Entries | Named killer |
| --- | ---: | --- |
| Austrian admission, mapping, cents, sections, order and disclosure | 50 | `austrian_vectors` |
| Tax grouping, exclusions, ordering and fee/year/grand totals | 26 | `tax_summary_vectors` |
| Journal scope, order, signed msat and optional fields | 5 | `journal_entry_vectors` |
| Vienna historical offsets, future rule and local dates | 4 | `vienna_calendar_vectors` |
| Desktop sorted unique available years and filtering | 4 | `desktop_tax_vectors` |
| Terminal Alt/Neu gain/loss splits and HALF_EVEN quantity ties | 4 | `austrian_text_vectors` |
| Desktop float/boolean year conversion | 2 | `tax_summary_daemon_arguments` |
| HALF_UP tax-total msat ties | 1 | `tax_summary_total_rounding` |

All 174 entries have named kill evidence across the catalog run and focused
reruns. The final 172-entry scratch run killed 171; it copied the oracle
before the multi-year desktop fixture finished regenerating, so year sorting
survived in that copy. A targeted run against the final oracle kills year
sorting and both subsequently added terminal quantity-rounding mutations.
Seventeen named baseline selections pass unmutated. This is aggregate kill
evidence, not a claim that one final 174-entry invocation passed.

Verification uses Rust 1.99.1, Cargo offline and locked Python 3.11. Formatting,
workspace and harness clippy with `-D warnings`, all 949 workspace tests,
the harness build and all sixteen native report tests pass. Loopback transport
fixtures required permission to bind outside the socket sandbox. Inventory,
contracts, shard validation and both oracle generators pass; the focused
inventory/contract suite passes 29 tests and 47 subtests.

The required shell/hybrid/runner/port suite passes 62 tests and 178 subtests,
with the same sandbox failure as the first checkpoint: Python `secrets init`
tries to create `/home/tg/.kassiber/run` on the read-only filesystem. The broader
hybrid CLI/blocker/tax-year suite passes 117 tests and four subtests with the
previous AI-provider failure (`internal_error` instead of
`ai_remote_ack_required`); its cause was not established then. Eight selected
report regressions pass. A ninth combines reads and exports and fails at the
deliberately excluded Austrian CSV path. The
[third continuation](#reports-ai-provider-failure-diagnosis-r5-third-continuation)
diagnoses and retests both sandbox failures; Austrian CSV remains excluded.
This is not a full quality-gate or push-readiness claim.

This remains **partial acceptance**: requested steps 1–2 and their tax-summary
desktop kind are implemented. Historical balances/holdings, individual
snapshot reads and amendment-review state, and their desktop callers remain
for the next checkpoint. The historical core reconstructs compensated journal
quantities and allocates residual pooled basis; it is not the current holdings
table read. `get_filed_report_snapshot` is a Python core API without a direct
CLI leaf or desktop kind in this checkout; this change invents no new wire
contract for it.

Other unported CLI reads remain `not_implemented`: `reports balance-history`,
`exit-tax`, `explain-capital-gain`, `privacy-hygiene`, `privacy-mirror`,
`psbt-privacy`, `cross-check`, `lightning-profitability`,
`commercial-subledger` and `source-funds`. So do
`reports filed-snapshots create`, every `reports export-*` leaf, report
mutations, reads requiring custody/tax recomputation, and network-dependent reads. In particular,
`ui.reports.balance_history`, desktop capital gains/explanations, exit-tax,
privacy/Lightning/cross-check, handoff and export kinds are not registered.
No Python product bug or Python product change was found in this continuation.

## Reports historical read checkpoint (R5 third continuation, partial)

`t3code/r5-reports-3`, based on `89f978ac`, adds `reports balance-history` and
`ui.reports.balance_history`. Ten report CLI leaves and five desktop report
kinds now use Rust. The current portfolio CLI and desktop contracts expose no
as-of argument; the historical core-only portfolio helper gains no new wire
contract. AI projections and all journal production remain Python.

History reconstructs holdings from stored journal rows in occurrence,
creation and id order. Acquisition rows already count earned coins, and
transfer-out quantities already include transfer fees, so income and
transfer-fee recognition do not double-count quantities. Basis excludes income,
adds nonnegative rows' fiat value and subtracts negative rows' stored basis.
Wallet/account quantities stay scoped while basis follows the profile-wide
residual asset pool. Missing optional quotes affect market valuation, never
replace stored basis or bypass report readiness. Numeric transaction-price
fallback and exact cached-price preference follow the historical Python path.

UTC hour/day/Monday-week/calendar-month buckets include pre-period holdings
and the complete final bucket. Cache selection retains the explicit end cap,
timestamp/manual/fetched/source precedence and exclusive next-bucket cutoffs.
Desktop limits coerce like Python, cap at 500 and retain the last rows after
projection, with original filters and the full pre-truncation count. Reads
share the existing savepoint and network/component/quantity/freshness barriers;
they neither acquire prices nor rebuild anything. CSV remains `not_implemented`
with exports. The internal wallet resolver is reused with no new external
dependency, schema or runtime requirement.

All twelve report process scenarios pass. The fresh routing log contains 154
ported CLI reads, including refused reads, with zero Python fallback:

| Command | Rust | Python fallback |
| --- | ---: | ---: |
| `reports.summary` | 23 | 0 |
| `reports.capital-gains` | 11 | 0 |
| `reports.balance-sheet` | 11 | 0 |
| `reports.portfolio-summary` | 11 | 0 |
| `reports.tax-summary` | 11 | 0 |
| `reports.journal-entries` | 11 | 0 |
| `reports.austrian-e1kv` | 14 | 0 |
| `reports.austrian-tax-summary` | 14 | 0 |
| `reports.balance-history` | 47 | 0 |
| `reports.filed-snapshots.list` | 1 | 0 |

Two snapshot-creation fixture calls deliberately use Python. Nine native daemon
sessions compare 150 `ui.reports.balance_history`, 27 tax-summary, 24 summary,
three balance-sheet and three portfolio-summary responses, with zero fallback
for those kinds. Historical sessions cover all three book types at both sizes,
including defaults, all intervals, scoped holdings, limits and refused inputs.

| JSON bytes, 64 / 3,000 transactions | Generic FIFO | Generic HIFO | Austrian moving average AT |
| --- | --- | --- | --- |
| Summary | exact | exact | exact |
| Capital gains | exact | exact | exact |
| Balance sheet | exact | exact | exact |
| Portfolio summary | exact | exact | exact |
| Tax summary | exact | exact | exact |
| Journal entries | exact | exact | exact |
| Austrian E1kv | exact profile refusal | exact profile refusal | exact |
| Austrian tax summary | exact profile refusal | exact profile refusal | exact |
| Historical balances/holdings (CLI and desktop) | exact | exact | exact |

CLI table/plain bytes also match on the Austrian book. The snapshot listing
still matches after a metadata edit makes live journals stale. Both output
streams and exit status are compared; daemon response lines are compared as
raw bytes, excluding their intentionally different supported-kind inventories.

The broader hybrid run additionally serves summary 6, capital gains 13,
balance sheet 4, portfolio summary 7, tax summary 3, journal entries 7 and
history 5 times in Rust. It records no fallback for a ported report command;
unported explanation/export fixtures deliberately continue in Python.

The frozen Python oracle now contains 316 cases, including 171 historical
and desktop-history cases. They cover compensated quantities, negative/zero
holdings, precision-32 cancellation and HALF_EVEN ties, scoped residual basis,
profile isolation and resolver precedence, chronological/creation/id ordering,
cache and transaction quote selection, invalid legacy quantities, UTC and
calendar boundaries, empty/reversed ranges, readiness and desktop projection.
Every vector asserts that the read made no database writes. The catalog adds
75 rules to the existing 174, with these named tests:

| New rules | Entries | Named killer |
| --- | ---: | --- |
| Historical admission | 1 | `history_readiness_vectors` |
| Range admission, interval floors/steps, boundaries and empty results | 19 | `history_period_vectors` |
| Quantity compensation, basis accumulation, precision, ordering and msat conversion | 18 | `history_quantity_vectors` |
| Scoped quantities, profile-wide pools and wallet/account resolution | 12 | `history_scope_vectors` |
| Stored/cache price selection, ordering, exact values and cutoffs | 16 | `history_quote_vectors` |
| Desktop tail selection, total count and truncation | 3 | `desktop_history_vectors` |
| Desktop coercion, defaults, bounds and unknown-key order | 6 | `history_daemon_arguments` |

Existing readiness mutations also name `history_readiness_vectors`, retaining
their original named killers. The manifest records each exact edit and rule.

The full 249-entry audit passed 24 baseline selections and killed 245 entries,
including all 174 inherited rules. It exposed two fixture gaps: an hour floor
could become a day floor before any holdings existed, and reversing already
descending unknown keys looked like sorting. The final fixtures add an
intraday range with prior holdings and keys in an unsorted order. An equivalent
cache SQL-order mutation is replaced by reversing the final timestamp sort;
a nonterminating backward December mutant is replaced by a terminating
two-year step. No production or Python reference change was needed for these
audit corrections. Reusing the prior scratch target initially exposed cached
mutant artifacts in the baseline; that run is discarded, and the report/daemon
artifacts in the disposable target are cleared before the final rerun.

The final historical-only scratch run passes all seven baseline selections
and kills all 75 mutations, including the corrected hour-floor, December,
cache-order and unknown-key cases. Together with the 174 inherited kills,
all 249 entries have named kill evidence. This is aggregate evidence across
the full audit and the successful final 75-entry run, not a claim that one
final 249-entry invocation passed.

Verification uses Rust 1.99.1, Cargo offline and locked Python 3.11. Formatting,
workspace and test-harness clippy with `-D warnings`, all 956 workspace tests,
all 14 binary/harness integration tests, the harness build, and both generated
oracle/metadata checks pass. The inventory/contract/shard suite passes 50 tests
and 61 subtests. The required shell/hybrid/CLI-runner/port suite passes 63 tests
and 178 subtests. The broader hybrid CLI-smoke/blocker/contract/year-handoff
suite, including both selected historical-balance regressions, passes 129
tests and four subtests. Owned-loopback and Python account-home owner-lock
fixtures ran with the filesystem/socket access their contracts require;
all books and data roots were disposable. There is no full quality-gate or
push-readiness claim. After the audit fixture corrections, all six historical
vector groups, the daemon-argument baseline and all twelve report process
scenarios were rechecked successfully.

Individual saved/filed lookup and amendment-review reads are **deferred by
scope correction**. This base is phase 4, not phase 5. The upgrade-impact model
lives on separate, unstacked phase-5 branches, and the existing core APIs have
no CLI or desktop read caller at this base. Process-boundary acceptance cannot
verify an API with no caller. Port them when a caller needs them and the stack
is restacked onto phase 5; this change leaves saved/filed snapshots, custody
impacts and their resolutions append-only.

Exit-tax; explanations and desktop capital gains; privacy/PSBT, Lightning,
cross-check and commercial/source-funds reads; exports; report mutations;
recomputing and network reads remain `not_implemented`. No AI projection is
ported. The complete reports slice remains open.

## Reports AI-provider failure diagnosis (R5 third continuation)

The earlier broader hybrid failure is
`tests/test_cli_smoke.py::CliSmokeTest::test_01b_ai_providers_roundtrip`,
at its first `chat hello --provider smoke-remote` acknowledgement assertion
(line 686 at `89f978ac`). Python alone reproduces the returned error:
`internal_error`, `[Errno 30] Read-only file system: '/home/tg/.kassiber/run'`.
Chat starts its Python daemon, then `_resolve_default_model` requests the
provider list. That request lazily opens the daemon's book and takes ownership
before provider acknowledgement. The ownership guard resolves the account
home through `pwd`, deliberately ignoring `HOME` and XDG overrides, then
creates its stable owner-lock namespace. This sandbox permits
reads there but not directory creation. A disposable book or temporary `HOME`
therefore does not remove the restriction.

Both the Python-only and hybrid executions of that exact smoke test pass with
the owner-lock filesystem access available. The fresh hybrid routing log records
all nine calls in Python: provider list/create/get/update/set-default/
clear-default/delete, `chat`, and `ai.models`. Rust serves none of them; chat's
child daemon is explicitly Python as well. Both acknowledgement refusals pass
before any provider request. This is an environmental filesystem restriction,
not an AI-routing, report-port or harness defect; no Python product fix is
needed. The earlier `secrets init` descriptor-fallback failure has the same
owner-lock directory restriction.

## Reports tabular CSV checkpoint (R5 fourth continuation, partial)

`t3code/r5-reports-4`, based on `6d0d7c88`, enables `--format csv` for all ten
ported report reads, including the Austrian detail and summary projections.
This is a partial checkpoint within requested step 1. Full CSV artifacts,
saved/filed mutations and their daemon kinds are still unported; this is
**not complete reports slice acceptance**.

The CLI retains first-record column order, minimal quoting, doubled quotes,
CRLF records, Unicode cells, blank null/missing cells, Python boolean spelling,
ordered ASCII-escaped nested JSON and eight-place float formatting, including
HALF_EVEN ties and negative zero. Empty lists retain `format_unsupported`.
Austrian E1kv selects detail rows; Austrian tax summary selects summary rows.
All existing readiness and profile refusals remain in place.

Explicit report CSV `--output` publication uses the shared atomic storage
writer: a private temporary file is synced and atomically replaces the final
artifact, then the directory is synced. A prepublication failure preserves the
old artifact and cleans the temporary file. Publication leaves export-parent
permissions unchanged. Explicit final symlinks retain Python's destination
semantics, including dangling links, and never create missing target parents.
An existing destination must still permit writing. Other CLI output paths
retain their existing behavior. No new third-party package, schema or runtime
requirement is introduced; CLI publication tests reuse the pinned `tempfile`.

These tabular read projections do not register saved snapshots, just as in
Python. Only the accounting-artifact exporters register completed exports;
only explicit user action may establish a filed marker. The process fixtures
seed saved and filed rows and prove that CSV reads and publication preserve
them and the phase-4 impact/capture/dependency tables. They do not establish
parity for new stored snapshot rows, content hashes or sealed-summary writes.

The process oracle compares stdout, stderr, exit status and explicit output
files at both 64 and 3,000 transactions. Journal descriptions include Unicode,
commas, quotes and embedded CR/LF. Refused stale/empty reads preserve existing
artifacts. File naming follows the explicit `--output` path; generated
artifact and bundle names remain outside this checkpoint.

All 19 report scenarios pass. The fresh log records 280 ported report reads,
including 126 CSV calls, with zero Python fallback. Counts exclude pytest's
`current` directory symlinks so each invocation is counted once:

| Command | Rust, all formats | Rust, CSV | Python fallback |
| --- | ---: | ---: | ---: |
| `reports.summary` | 36 | 13 | 0 |
| `reports.capital-gains` | 25 | 14 | 0 |
| `reports.balance-sheet` | 25 | 14 | 0 |
| `reports.portfolio-summary` | 25 | 14 | 0 |
| `reports.tax-summary` | 25 | 14 | 0 |
| `reports.journal-entries` | 25 | 14 | 0 |
| `reports.austrian-e1kv` | 23 | 9 | 0 |
| `reports.austrian-tax-summary` | 23 | 9 | 0 |
| `reports.balance-history` | 60 | 13 | 0 |
| `reports.filed-snapshots.list` | 13 | 12 | 0 |

Two unported `reports.filed-snapshots.create` fixture calls deliberately use
Python. Nine native daemon sessions compare 150 balance-history, 27 tax-summary,
24 summary, three balance-sheet and three portfolio-summary result lines as
raw bytes, with zero fallback. The intentionally different ready-handshake
kind inventories are excluded. No new daemon kind is enabled here.

| CSV bytes, 64 / 3,000 transactions | Generic FIFO | Generic HIFO | Austrian moving average AT |
| --- | --- | --- | --- |
| Summary | exact | exact | exact |
| Capital gains | exact | exact | exact |
| Balance sheet | exact | exact | exact |
| Portfolio summary | exact | exact | exact |
| Tax summary | exact | exact | exact |
| Journal entries | exact | exact | exact |
| Austrian E1kv detail | exact profile refusal | exact profile refusal | exact |
| Austrian tax summary | exact profile refusal | exact profile refusal | exact |
| Balance history | exact | exact | exact |
| Saved/filed snapshot listing | exact | exact | exact |
| Newly stored snapshot rows | unported | unported | unported |

The frozen oracle contains 320 cases, adding four Austrian CSV projections.
Native renderer tests cover formatting, special-character quoting, refusal
and publication boundaries. The mutation manifest contains 278 entries: the
249 inherited rules and these 29 new entries, all killed by named tests:

| New rules | Entries | Named killer |
| --- | ---: | --- |
| Austrian detail/summary projection | 2 | `report_csv_projection_vectors` |
| CLI CSV request forwarding | 1 | `report_csv_arguments_select_payload_without_terminal_text` |
| CSV columns, cells, formatting and empty-list refusal | 19 | `report_csv_format_contract` |
| Atomic CLI output routing | 1 | `report_csv_file_publication_replaces_complete_artifacts` |
| Explicit output symlink destination | 1 | `report_csv_publication_follows_explicit_output_symlinks` |
| Publication bytes, replacement, failure and parent permissions | 5 | `export_publication_is_atomic_and_preserves_parent_permissions` |

The final new-entry audit passes six baseline selections and kills all 29
mutations in a fresh disposable target. An initial nonfinite mutation was
equivalent; replacing it with a wrong literal makes the intended rule
observable. The 249 inherited entries retain the preceding checkpoint's kill
evidence and were not rerun in this continuation. There is no claim that one
fresh invocation killed all 278 entries.

Verification uses Rust 1.99.1, offline/locked Cargo and locked Python 3.11.
Workspace formatting, workspace and test-harness clippy with `-D warnings`,
all 962 workspace tests, all 14 binary/harness integration tests, the harness
build and the generated oracle check pass. After the final helper review,
the store suite passes all 46 tests and both clippy selections pass again.
The inventory/contract/shard checks pass 28 tests and 47 subtests; all 278
mutation entries apply. The broader hybrid CLI-smoke/blocker/contract/year-handoff
selection passes 127 tests and four subtests. The required
shell/hybrid/CLI-runner/port suite passes 63 tests and 178 subtests. A truncated
generated inventory was restored and compared with the base: only the two
intended test additions differ. The previous slice's CSV-refusal assertion is
removed; the seven new CSV cases verify acceptance, refusal and preservation.
Interrupted reruns are excluded from these totals; the final harness was rebuilt
under `/tmp` before the successful process suites. Owned-loopback and Python
account-home owner-lock fixtures use the access their contracts require; all
books and data roots are disposable. There is no full quality-gate or
push-readiness claim.

Remaining requested work is full `reports export-csv`, the Austrian E1kv CSV
bundle, and report-owned `transactions export` CSV (including draft and range
semantics); export-completion saved registration and explicit
`reports filed-snapshots create`; hashes through `crates/pyjson`, sealed
summaries, dependency capture/copy, phase-4 impacts and all mutation refusals;
and the corresponding daemon exports, including capital-gains CSV. All remain
`not_implemented`. No phase-5 upgrade-impact semantics are added.

XLSX/PDF, exit-tax, explanations, privacy/Lightning/cross-check/commercial
reads and network reads remain `not_implemented`. AI projections and journal
production stay Python. No Python reference bug was found or fixed.

## Not yet ported

The diagnostics slice now implements explicit public reports and native debug
provenance (below). There is no cached update banner, no RAM log
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

### Chain clients and backend probes (R5 partial)

`crates/chain` exposes immutable `Backend` snapshots, `HttpClient::get_json` /
`get_text`, `CoreClient::call`, and `ElectrumClient::connect` / `call` /
`batch_call`. Construction performs local configuration/trust reads only.
Every request requires the matching `kassiber-net::Consent` and an overall
`Instant` deadline. Core and Electrum accept observation methods only; spending,
signing and broadcasting are refused. Bitcoin and Liquid share the same
Esplora/Electrum transports. Wallet scans, observer persistence and AI
projections are not part of this checkpoint.

The native daemon advertises `ui.backends.http.test`, `.electrum.test`,
`.bitcoinrpc.test`, `.btcpay.test` and `.detect_core`. `ProbeConsent` binds the
exact kind, form arguments and book identity. The real-store adapter mints it
only after the explicit daemon request passes unlock/expected-scope checks.
Saved backend reads and startup never grant it. Each HTTP/BTCPay check is one
logical probe; Electrum reuses one version handshake; Core takes one snapshot
with wallet/filter/prune checks. Core detection considers local configuration
only and refuses non-loopback candidates. Credential references contain no
password or cookie contents. A reference bug allowing nested config URL
changes to retain saved Core credentials has a separate regression/fix commit.

There are no backend-test or detect-core CLI leaves in this Python reference's
command catalog. The CLI `daemon` entry serves these kinds; backend CRUD and
BTCPay inspection/setup CLI commands retain their existing dispositions.

`tests/test_rust_chain_candidate.py` compares five probe kinds byte for byte
with Python on the same owned loopback fakes, checks native hybrid routing,
and compares the three direct client APIs using the test-only
`crates/chain/examples/client_fixture.rs` driver. Build that driver after Cargo
workspace tests, alongside the test-harness binary, before running the module:

```sh
mise exec rust@1.99.1 -- cargo build -p kassiber --features test-harness --offline --locked
mise exec rust@1.99.1 -- cargo build -p kassiber-chain --example client_fixture --features test-harness --offline --locked
KASSIBER_RUST_BIN=$PWD/target/debug/kassiber uv run --locked python -m pytest -q tests/test_rust_chain_candidate.py
```

Twenty native tests cover refusal, wire shapes, ordered batching, duplicate
IDs, errors, bounded retry, large Core history, prune uncertainty and credential
scope. Twenty checked-in mutations pin protocol, retry, selection and grant
rules in `crates/chain/scripts/mutations.json`.
The fixture and independent attempt observer require the chain crate's explicit
`test-harness` feature. Ordinary workspace tests keep the production binary's
guard refusal intact; the mutation launcher enables and arms the observer.

```sh
KASSIBER_TEST_NO_EGRESS=1 mise exec rust@1.99.1 -- cargo test -p kassiber-chain --features test-harness --offline --locked
mise exec rust@1.99.1 uv@latest -- uv run --locked python crates/chain/scripts/mutation_check.py
```

The chain candidate module's four hybrid daemon sessions have zero Python
fallbacks. The per-kind counts below include offline and input-validation refusals;
the direct fixture driver is tested separately from product dispatch.

| Native kind | Rust requests | Python fallbacks |
| --- | ---: | ---: |
| `ui.backends.http.test` | 4 | 0 |
| `ui.backends.electrum.test` | 3 | 0 |
| `ui.backends.bitcoinrpc.test` | 4 | 0 |
| `ui.backends.btcpay.test` | 3 | 0 |
| `ui.backends.detect_core` | 3 | 0 |

The matching direct fixtures compare successful results and wire shapes, HTTP
401, a typed Core HTTP 500 RPC failure and an Electrum RPC failure. The offline
session includes a locally configured Core candidate and asserts neither HTTP
nor Electrum fake receives a request.

This is a partial boundary. Lightning probing remains `not_implemented` rather
than substituting a smaller LND/CLN health request for Python's full snapshot.
Complete differential failure/TLS/proxy coverage and platform/regtest runs
remain before declaring full chain-client parity. Transport failures retain the
reviewed net client's fixed diagnostics; socket/OpenSSL-specific Python prose
is not claimed byte-identical. Probe HTTP bodies have the reviewed bounded
buffer limit and sample their first 4096 bytes for output. No peer body is logged.
Unlike Python's prefix-only read, the current native HTTP probe buffers the
bounded response before sampling; large-body/streaming probe parity remains
part of the response-limit follow-up.

Linux verification at this checkpoint passed with Rust 1.99.1 and the locked
Python 3.11 environment: fmt; workspace and test-harness clippy with warnings
as errors; locked offline workspace tests; the net `test-observer` suite;
the guarded harness/example builds; and all 20 chain mutations (14 named
unmutated baselines). The requested shell/hybrid/CLI-runner/rust-port Python
modules, plus rates/chain candidate modules and the Core probe regression
module, passed together: 96 tests and 178 subtests. Live regtest was not run;
the existing Core/Fulcrum integration scenarios still exercise Python wallet
sync and do not establish native Rust sync parity.

## Read-path performance

Rust read commands retain their existing output and admission contracts while
avoiding fresh-schema reconstruction and redundant compatibility DDL on current
books. Report admission omits review-only inventory serialization, reuses neutral
wallet/asset evidence within its snapshot, streams holdings quote candidates,
and avoids unused capital-gains presentation in summary. No network, schema,
dependency or accounting-policy changes are introduced. The
[read-path report](rust-read-performance.md) records release measurements,
startup spans, verification and remaining costs.

The [second performance round](rust-read-performance-round2.md) adds streaming
admission/history/summary projections, reusable row conversion, exact integer
amount scaling and narrow transaction-page selection. It records phase spans
and scratch index measurements; schema proposals remain unimplemented.
## Diagnostics service slice (R5)

`crates/diagnostics` owns `diagnostics collect`, optional `--save`, and the
explicit global `--diagnostics-out PATH|auto` error export. The CLI has one
family adapter; the reference has no diagnostics daemon kind or separate AI
projection. Error collection uses the admitted book before it closes. Failures
before admission report unavailable state. A report-write failure is best effort:
it emits Python's redacted stderr notice and preserves the original error.
No automatic log, network request, provider probe, descriptor dispatch or new
AI authority is introduced.

The ordered report schema, argument classes, sensitive-key tokens, fourteen
ordered text rules, recursive detail shapes, aggregate SQL summaries and
syntactic backend locality follow `kassiber/diagnostics.py`. The implementation
reuses the already locked regex engine because CPython lookarounds and Unicode
case/word/space/digit matching are not available in the standard library.
The same existing `nix` dependency supplies public OS identity on Unix; no new
third-party package is introduced.

Native provenance is an owner-approved exception to byte parity. Historical
schema keys `environment.python_version` and `python_implementation` contain
the actual compiler identity and `Rust`, respectively; `sqlite_version` is the
native linked SQLite version. Platform values describe the running host.
The report's `stack` is an empty array on failure: a meaningful bounded public
native trace is unavailable, so Python frames are never fabricated and native
Debug/Display backtraces are never serialized. The CLI envelope always retains
`error.debug` in its original position: it is null without `--debug`, including
for unhandled failures, and an empty string with `--debug`. No native frame
paths, usernames, home directories, build-machine paths or stack locals escape.
An empty stack/debug has zero frames and bytes, below Python's 20-frame report
cap and traceback-size cap.

The report oracle normalizes only `environment.python_version`,
`environment.python_implementation`, `environment.sqlite_version`, and `stack`.
A separate unhandled diagnostics-export envelope oracle normalizes only
`error.debug` when it is a string. Scope regressions prove other values and
layout remain observable. Clocks are injected on both sides in tests, rather
than normalized. The timestamp hook is compiled only into test-harness builds.
All other keys, ordering, nesting, types, bytes, errors and saved-file modes
remain the reference's. Python product code is unchanged.

Python currently uses `Path.write_text` directly, with ordinary umask-derived
permissions, rather than an atomic replacement or an explicit private mode.
Rust deliberately preserves this behavior, including existing-inode/mode
retention. The report filename also follows Python's timestamp sanitizer:
ordinary timestamps produce `kassiber-diagnostics--timestamp-.json`, so a
later automatic export overwrites that filename. These reference limitations
are recorded, not silently fixed in the port.

The checked-in oracle corpus has 29 text cases, 135 sensitive-key cases,
140 recursive detail cases, four argument-class cases, a planted aggregate
state fixture and 146 backend locality vectors. The mutation catalog covers
all fourteen text rules, every sensitive key, native report/debug frame
refusal, debug presence, detail shapes and effective review selection.
See [slice acceptance](rust-diagnostics-r5-status.md) for verification,
served-by evidence, reference repros and platform limits.

## Transfers and review classification (R5)

This classification uses Python at `65c74e84`. The requested
`core/transfers.py` is named `core/custody_review_terms.py` in this base;
`kassiber/transfers.py` contains shared observation helpers. AI projections
remain in Python. A stored membership is reviewed custody, not ownership
inferred from an observation or a matching suggestion.

(a) means stored reads, (b) authoring and its validation, and (c) current-state
computation or network dependencies. A command's name or registry read effect
does not imply that it is a stored read. None of these paths requires network
access directly; journal and custody dependencies are listed below.

| CLI command | Group | Related desktop kind | Dependency / behavior |
| --- | --- | --- | --- |
| `transfers list` | a | `ui.transfers.list` | Stored effective memberships, economic terms and observation context |
| `transfers pair` | b | `ui.transfers.pair` | Authored component, native residual validation, active conflicts, append-only terms |
| `transfers unpair` | b | `ui.transfers.unpair` | Supersede and re-slice connected pair group; invalidate journals |
| `transfers update` | b | `ui.transfers.update` | Append component revision and terms; invalidate journals |
| `transfers gaps list` | c | `ui.custody.gaps.list` | Gap candidate computation and custody state |
| `transfers gaps review` | c | `ui.custody.gaps.review_context` | Gap candidate computation and review context |
| `transfers gaps history` | a | `ui.custody.gaps.history` | Bounded redacted stored history and impact counts |
| `transfers gaps plan` | b | `ui.custody.review.plan` | Current candidate fingerprints, authored component plan, filed-report impacts |
| `transfers gaps apply` | b | `ui.custody.review.apply` | Recheck candidates/version; append reviews, relations, components, impacts; invalidate |
| `transfers components list` | c | `ui.transfers.components.list` | Materialize validation, native support, evidence and effective custody state |
| `transfers components show` | c | `ui.transfers.components.get` | Same component materialization; privacy-safe desktop projection |
| `transfers components plan` | b | `ui.transfers.components.plan` | Component planner, scope/anchor/allocation/conservation validation |
| `transfers components apply` | b | `ui.transfers.components.apply` | Version-bound planner; append aggregate/revision; activation and invalidation |
| `transfers payouts list` | a | `ui.transfers.payouts.list` | Stored effective memberships and payout terms |
| `transfers payouts create` | b | `ui.transfers.payouts.create` | Authored conversion component, source residual validation and invalidation |
| `transfers payouts delete` | b | `ui.transfers.payouts.delete` | Supersede authored payout and invalidate |
| `transfers suggest` | c | `ui.transfers.suggest` | Matcher plus ownership/custody projections; not a stored list |
| `transfers bulk-pair` | c | `ui.transfers.bulk_pair` | Matcher, conflict guards and authored pair service |
| `transfers dismiss` | b | `ui.transfers.dismiss` | Scoped observations; dismissal upsert only; no ownership or journal mutation |
| `transfers rules list` | a | `ui.transfers.rules.list` | Stored rules, validated fail-closed projection |
| `transfers rules create` | b | `ui.transfers.rules.create` | Predicate validation, recommended policy and Python JSON serialization |
| `transfers rules delete` | b | `ui.transfers.rules.delete` | Scoped rule deletion |
| `transfers rules enable` | b | `ui.transfers.rules.set_enabled` | Validate stored predicate/kind/policy before enabling |
| `transfers rules disable` | b | `ui.transfers.rules.set_enabled` | Disable without granting matching authority |
| `transfers rules apply` | c | `ui.transfers.rules.apply` | Matcher, rule selection and authored pair service |
| `review cases` | c | `ui.review.cases` | Canonical custody journal rebuild, version-bound cursor |
| `review plan` | c | `ui.review.plan` | Isolated snapshot, canonical journal rebuild and economic effects |
| `review apply` | c | `ui.review.apply` | Atomic rebuild/revalidation and durable verification receipt |
| `review receipt` | a | `ui.review.receipt` | Stored historical receipt; no current-readiness claim |

Additional related desktop kinds have no leaf in these two CLI families:

| Desktop kind | Group | Dependency |
| --- | --- | --- |
| `ui.transfers.review_context` | c | Matcher, ownership and journal projections |
| `ui.review.worklist` | c | Aggregated current review cases |
| `ui.review.request_input` | c | Verification against current canonical cases |
| `ui.review.badges` | c | Freshness and review projections |
| `ui.journals.transfers.list` | c | Journal transfer projection |
| `ui.custody.coverage.snapshot` | c | Custody coverage composition |
| `ui.custody.lineage.snapshot` | c | Custody lineage composition |

These stay `not_implemented` until their dependencies are ported. Group (c) is not
advertised as supported. Matching remains in Python; porting it requires a
separate mutation catalog and an oracle against both the Python matcher and
its naive all-pairs reference.

The preceding checkpoint implements all five group (a) commands and five of the
fourteen group (b) commands through `crates/transfers`: **9/25 transfers and
1/4 review CLI leaves**, with **9 desktop kinds**. The complete classification
is **(a) 5, (b) 14, (c) 10**. Pair/payout selection uses
persisted effective memberships, never the compatibility pair tables or a new
ownership inference. Split allocations take precedence over review source
amounts. Gap history exposes bounded redacted review facts. Historical receipt
verification never establishes current readiness. Rules are configuration:
invalid stored rules are projected disabled with Python's validation reason.
The deterministic Python vectors and native service tests compare ordered JSON
bytes, while `tests/test_rust_transfer_services.py` compares real CLI/daemon
bytes and witnesses Rust routing for each implemented leaf.

The authoring checkpoint adds rule create/delete/enable/disable and dismissal,
with desktop rule create/delete/set-enabled and dismissal. Predicate validation
and default policy selection are shared domain rules; they do not select or
match observations. Stored predicates use `crates/pyjson` sorted-key Python JSON.
Asset normalization uses the locked reference's Unicode 14 uppercase mappings,
including expansions and characters assigned only after that Unicode release.
An invalid stored rule cannot be enabled. Dismissals resolve profile-scoped
transaction IDs before external IDs and refuse ambiguous external references.
Repeated dismissals retain identity/creation time and coalesce absent reasons.
Positive expiry days preserve Python conversion and datetime overflow limits;
explicit CLI zero never expires, while desktop zero retains Python's 90-day
default. Each write owns a savepoint and preserves a caller's transaction.

The deterministic authoring oracle compares **41 sequential operations: 11
successful writes and 30 refusals**. After every operation it compares the
complete rules, dismissals and profile rows as Python JSON bytes, including
stored JSON strings, UUIDs and timestamps injected through a native test-only
source. Every other application table must remain unchanged. Production UUIDs
and time have no environment overrides. Neither rule configuration nor advisory
dismissals invalidates journals or establishes observation, ownership, custody,
basis or provenance facts.

As in the existing shell contract, unexpected Python exceptions include
interpreter-specific traceback text which Rust does not fabricate. The daemon
oracle normalizes only that `error.debug` field for internal errors, plus
production UUID/time fields; error codes/messages and stored bytes remain pinned.

The pair revision checkpoint on `t3code/r5-transfers-2`, based at `c9bd660b`,
adds **pair, unpair and update** plus `ui.transfers.pair`,
`ui.transfers.unpair` and `ui.transfers.update`. The supported total is now
**13/29 CLI commands (12/25 transfers, 1/4 review), with 12 desktop kinds**.
Supported commands and kinds execute no Python. Group (c) and AI projections
remain unchanged.

`pair_authoring.rs` owns scoped resolution, validation, fee treatment, term
reuse, revision/no-op/delete behavior and atomic invalidation. `pair_spec.rs`
constructs deterministic UUIDv5 groups, source hashes and residual allocations;
`component_store.rs` persists append-only aggregates. It reuses
`crates/custody/src/components/review.rs`: `validate_activation` calls the
existing component conservation, anchor, route, lineage and raw-active overlap
checks, and `activation_evidence` reuses canonical raw evidence and commitment
hashing. `validation_projection` restores Python field order using generated
field lists; this presentation step makes no custody decisions. No second
component interpreter was added. Draft validation precedes
commitment capture; journal computation still selects authored-active rows.
Raw-active conflicts block activation even when stored memberships are absent.

The fixed-source oracle compares **115 operations: 67 successes and 48
refusals**, across 20 scenarios. After each call it compares normalized complete
rows for profiles, aggregates, children, memberships, author-bound evidence,
commitments and chain-index invalidation. Every other application table must
stay unchanged. UUIDs, times, source hashes, JSON strings, counts, revision and
supersession links match Python byte for byte. Refusals cover ambiguity, scope,
policy, network contradictions, excluded/canonical anchors, empty slices,
active overlap, existing drafts/lineages and input drift. Sequences also cover
connected group reslicing, unchanged updates, idempotent unpair, repeated
payload retargeting, explicit portions, fee inclusion, refunds and unique or
ambiguous native remainders. Native failure injection checks standalone late
invalidation rollback and preservation of an outer caller transaction. Existing
store immutability triggers are exercised on the authored rows.

| Command | Oracle successes | Oracle refusals | Focused hybrid Rust calls | Affected-module Rust calls | Python fallbacks |
| --- | ---: | ---: | ---: | ---: | ---: |
| `transfers pair` | 27 | 36 | 2 | 6 | 0 |
| `transfers update` | 22 | 10 | 2 | 0 | 0 |
| `transfers unpair` | 18 | 2 | 2 | 1 | 0 |

The direct native daemon sequence additionally exercises pair **4**, update
**4**, and unpair **2** calls, comparing Python responses after normalization of
generated UUIDs/times and interpreter debug text. A native adapter test pins
alias priority, falsy defaults, coercion, required IDs, gui authorship and the
omitted-versus-cleared notes distinction.

Two Python atomicity bugs were fixed in separate commits, with regressions
shown failing before each fix. `26e64be8` keeps draft creation and activation
inside one savepoint: an excluded or noncanonical anchor previously left a
committed draft after refusal. `2091516c` includes final pair journal
invalidation in the authoring savepoint: injected invalidation failures
previously left creation, update or deletion committed. All five parameterized
regression cases compare the complete reference database dump after rollback.
The successful authoring row format and invalidation counts are unchanged.

This remains a partial service slice. **Five group (b) commands** still answer
`not_implemented`: payout create, component plan/apply and gap plan/apply.
They need the same complete-row parity, revision/hash commitments, refusal
coverage, report impacts and mutation acceptance. All ten group (c) commands
and the additional computation-dependent desktop kinds remain unported.
Matching still requires its later Python/naive-all-pairs oracle and mutation
catalog. AI projections stay in Python.

### Earlier rule/dismissal verification

Verification used Rust 1.99.1 and the locked Python 3.11 environment, Cargo
`--offline`, and disposable books. Workspace and package formatting, workspace
and `-p kassiber --features test-harness` clippy with `-D warnings`, and the
harness build passed. `cargo test --offline --workspace --locked` passed **943
tests**. The five added real-process transfer oracles passed, and all 44
mutations were killed. The mutation applicability check also passed.
The exact requested shell/parity/hybrid-candidate/CLI-runner/`tests/rust_port`
gate passed **63 tests and 178 subtests**, including inventory and contracts.

The affected hybrid modules were `test_swap_matching_cli`, `test_swap_rules`,
`test_transfer_matching`, `test_custody_component_surfaces`,
`test_review_workflow`, `test_review_cli_ai` and `test_review_regressions`:
**386 tests and 35 subtests passed; one test failed**.
`ReviewRegressionTest.test_diagnostics_out_auto_writes_public_error_report`
expects a diagnostics artifact, but `--diagnostics-out` already returned
`not_implemented` in `65c74e84` (`crates/cli/src/run.rs`). This inherited global
flag dependency is outside the transfer checkpoint; the broader hybrid suite
is not reported green. A disposable launcher restored only the hybrid binary
and routing-log settings for tests that intentionally scrub `KASSIBER_*` from
child environments. It did not change command routing or product behavior.

Earlier attempts collided with a workspace build overwriting the harness
binary, and a direct hybrid invocation lost its configuration in a scrubbed
environment. The final runs used the rebuilt harness without concurrent Cargo
builds and the disposable launcher. Loopback-dependent workspace tests passed
outside the restricted socket sandbox. No Python reference bug was found in
that earlier checkpoint.

### Earlier stored/rule/dismissal routing evidence

The two focused hybrid witnesses in `tests/test_rust_transfer_services.py`
assert the following counts. All thirteen calls were served by Rust; none
fell back to Python. The affected-module hybrid run independently recorded
20 Rust calls to this subset, again with zero fallbacks. Its tests do not
call payout listing or gap history; the focused witnesses cover both.

| Command | Focused Rust calls | Affected-module Rust calls | Python fallbacks |
| --- | ---: | ---: | ---: |
| `transfers list` | 1 | 5 | 0 |
| `transfers payouts list` | 1 | 0 | 0 |
| `transfers gaps history` | 1 | 0 | 0 |
| `transfers rules list` | 2 | 2 | 0 |
| `review receipt` | 1 | 1 | 0 |
| `transfers rules create` | 2 | 7 | 0 |
| `transfers rules delete` | 1 | 1 | 0 |
| `transfers rules enable` | 1 | 2 | 0 |
| `transfers rules disable` | 1 | 1 | 0 |
| `transfers dismiss` | 2 | 1 | 0 |

Native daemon oracle sessions exercise every one of the nine supported kinds,
including validation failures and large-integer projections. They invoke the
Rust binary directly, which has no Python fallback or AI projection dispatch.

### Transfer/review mutation catalog

The catalog is `crates/transfers/scripts/mutations.json`; its scratch-tree runner
is `crates/transfers/scripts/mutation_check.py`. Every entry below names a test
that must pass unmutated and fail after its rule is changed. The earlier
checkpoint passed all ten named unmutated selections and killed **44/44
mutations**.

| Mutation | Named killing test |
| --- | --- |
| `stored-membership` | `stored_reads_match_python_bytes_without_touching_rows` |
| `stored-allocated-source` | `stored_allocations_override_reviewed_amounts` |
| `history-bounds` | `stored_scope_limits_and_receipt_selectors_fail_closed` |
| `history-newest-page` | `stored_reads_match_python_bytes_without_touching_rows` |
| `rule-unknown-fields` | `malformed_stored_rules_are_disabled_with_python_reasons` |
| `rule-finite-nonnegative` | `malformed_stored_rules_are_disabled_with_python_reasons` |
| `rule-confidence-values` | `malformed_stored_rules_are_disabled_with_python_reasons` |
| `rule-invalid-disabled` | `malformed_stored_rules_are_disabled_with_python_reasons` |
| `rule-kind` | `stored_reads_match_python_bytes_without_touching_rows` |
| `rule-policy` | `stored_reads_match_python_bytes_without_touching_rows` |
| `rule-string-type` | `malformed_stored_rules_are_disabled_with_python_reasons` |
| `rule-confidence-type` | `malformed_stored_rules_are_disabled_with_python_reasons` |
| `payout-malformed-fiat` | `malformed_stored_payout_value_fails_closed` |
| `authored-kind` | `authoring_sequences_match_python_rows_and_errors_byte_for_byte` |
| `authored-policy` | `authoring_sequences_match_python_rows_and_errors_byte_for_byte` |
| `rule-sorted-python-json` | `authoring_sequences_match_python_rows_and_errors_byte_for_byte` |
| `rule-create-disabled` | `authoring_sequences_match_python_rows_and_errors_byte_for_byte` |
| `rule-enable-invalid` | `authoring_sequences_match_python_rows_and_errors_byte_for_byte` |
| `rule-scoped-mutation` | `rule_scope_dismissal_reference_priority_and_rollback_are_preserved` |
| `policy-same-asset` | `profile_policy_defaults_do_not_author_ownership_or_journal_freshness` |
| `policy-country` | `profile_policy_defaults_do_not_author_ownership_or_journal_freshness` |
| `policy-rail-preference` | `profile_policy_defaults_do_not_author_ownership_or_journal_freshness` |
| `policy-python-uppercase` | `profile_policy_defaults_do_not_author_ownership_or_journal_freshness` |
| `dismiss-reference-ambiguity` | `rule_scope_dismissal_reference_priority_and_rollback_are_preserved` |
| `dismiss-id-priority` | `rule_scope_dismissal_reference_priority_and_rollback_are_preserved` |
| `dismiss-reason-coalesce` | `authoring_sequences_match_python_rows_and_errors_byte_for_byte` |
| `dismiss-identity` | `authoring_sequences_match_python_rows_and_errors_byte_for_byte` |
| `dismiss-expiry` | `authoring_sequences_match_python_rows_and_errors_byte_for_byte` |
| `rule-no-journal-invalidation` | `authoring_sequences_match_python_rows_and_errors_byte_for_byte` |
| `dismiss-no-journal-invalidation` | `authoring_sequences_match_python_rows_and_errors_byte_for_byte` |
| `fee-bool-is-not-number` | `authoring_sequences_match_python_rows_and_errors_byte_for_byte` |
| `dismiss-created-time` | `authoring_sequences_match_python_rows_and_errors_byte_for_byte` |
| `dismiss-days-range` | `authoring_sequences_match_python_rows_and_errors_byte_for_byte` |
| `dismiss-days-c-int` | `authoring_sequences_match_python_rows_and_errors_byte_for_byte` |
| `authoring-atomic-rollback` | `rule_scope_dismissal_reference_priority_and_rollback_are_preserved` |
| `rule-predicate-object` | `authoring_sequences_match_python_rows_and_errors_byte_for_byte` |
| `rule-integer-overflow` | `authoring_sequences_match_python_rows_and_errors_byte_for_byte` |
| `history-snapshot-object` | `historical_snapshot_quantities_follow_python_integer_coercion` |
| `history-integer-coercion` | `historical_snapshot_quantities_follow_python_integer_coercion` |
| `receipt-selector-exclusive` | `stored_scope_limits_and_receipt_selectors_fail_closed` |
| `receipt-profile-scope` | `stored_scope_limits_and_receipt_selectors_fail_closed` |
| `dismiss-profile-scope` | `rule_scope_dismissal_reference_priority_and_rollback_are_preserved` |
| `history-reopened-status` | `stored_reads_match_python_bytes_without_touching_rows` |
| `desktop-expiry-integer` | `desktop_dismissal_expiry_uses_python_integer_coercion` |

### Pair revision mutation additions

The continuation adds 87 entries to the existing 44, including the shared
activation facade, immutable-trigger retention, desktop adaptation and both
Python reference fixes. The final full run passed all **19 named unmutated
selections** and killed **131/131 mutations**. Reused custody
conservation/anchor/commitment rules
also retain their existing 318-entry custody catalog; its applicability check
passes. The transfer runner names the following killing tests:

| Mutation | Named killing test |
| --- | --- |
| `pair-kind` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-policy` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-source` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-distinct-transactions` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-profile-resolution` | `pair_writes_refuse_other_review_terms_and_profile_references` |
| `pair-external-ambiguity` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-direction` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-network-policy` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-network-contradictions` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-network-boundaries` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-same-asset-taxable` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-cross-asset-policy` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-same-asset-out-amount` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-positive-amount` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-amount-scale` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-amount-rounding` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-amount-exceeds-source` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-duplicate` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-nonpair-conflict` | `pair_writes_refuse_other_review_terms_and_profile_references` |
| `pair-leg-reuse` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-swap-fee` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-included-fee` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-split-fee` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-update-kind` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-update-policy` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-update-notes` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-update-noop` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-update-fee-recompute` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-group-collection` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-unpair-history` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-unpair-reslice` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-unpair-memberships` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-service-atomic` | `pair_revisions_keep_immutability_triggers_and_outer_transaction_ownership`, `pair_final_invalidation_refusals_are_atomic` |
| `pair-stable-namespace` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-row-hash` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-order` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-principal` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-available` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-conversion-value` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-positive-slices` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-reviewed-source` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-group-full-residual` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-residual-fee` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-refund-residual` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-native-residual` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-retarget` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-optional-text` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-invalidation-version` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-invalidation-cache` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-invalidation-ownership-cache` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-activation-gate` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-revision-counter` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-supersession` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-membership-insert` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-evidence-capture` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-evidence-count` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-commitment-id` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-child-count` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-raw-active-conflicts` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-draft-validation` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-retain-trg_custody_authored_evidence_immutable` | `pair_revisions_keep_immutability_triggers_and_outer_transaction_ownership` |
| `pair-retain-trg_custody_component_allocation_revision_delete_immutable` | `pair_revisions_keep_immutability_triggers_and_outer_transaction_ownership` |
| `pair-retain-trg_custody_component_allocation_revision_immutable` | `pair_revisions_keep_immutability_triggers_and_outer_transaction_ownership` |
| `pair-retain-trg_custody_component_evidence_revision_delete_immutable` | `pair_revisions_keep_immutability_triggers_and_outer_transaction_ownership` |
| `pair-retain-trg_custody_component_evidence_revision_immutable` | `pair_revisions_keep_immutability_triggers_and_outer_transaction_ownership` |
| `pair-retain-trg_custody_component_leg_revision_delete_immutable` | `pair_revisions_keep_immutability_triggers_and_outer_transaction_ownership` |
| `pair-retain-trg_custody_component_leg_revision_immutable` | `pair_revisions_keep_immutability_triggers_and_outer_transaction_ownership` |
| `pair-retain-trg_custody_component_revision_delete_immutable` | `pair_revisions_keep_immutability_triggers_and_outer_transaction_ownership` |
| `pair-retain-trg_custody_component_revision_immutable` | `pair_revisions_keep_immutability_triggers_and_outer_transaction_ownership` |
| `pair-retain-trg_custody_component_terms_delete_immutable` | `pair_revisions_keep_immutability_triggers_and_outer_transaction_ownership` |
| `pair-retain-trg_custody_component_terms_immutable` | `pair_revisions_keep_immutability_triggers_and_outer_transaction_ownership` |
| `pair-draft-exists` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-lineage-exists` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-native-residual-ambiguity` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `pair-asset-id-case` | `pair_sequences_match_python_complete_database_rows_and_refusals` |
| `desktop-pair-source` | `desktop_pair_aliases_defaults_and_absent_notes_match_python` |
| `desktop-pair-aliases` | `desktop_pair_aliases_defaults_and_absent_notes_match_python` |
| `desktop-pair-defaults` | `desktop_pair_aliases_defaults_and_absent_notes_match_python` |
| `desktop-pair-required-id` | `desktop_pair_aliases_defaults_and_absent_notes_match_python` |
| `desktop-pair-notes-unset` | `desktop_pair_aliases_defaults_and_absent_notes_match_python` |
| `desktop-pair-note-alias` | `desktop_pair_aliases_defaults_and_absent_notes_match_python` |
| `reference-native-review-atomic` | `tests/test_custody_authored_migration.py::test_native_review_activation_failure_leaves_no_committed_draft` |
| `reference-pair-create-invalidation` | `tests/test_custody_authored_migration.py::test_pair_invalidation_failure_keeps_complete_database_unchanged[create]` |
| `reference-pair-update-invalidation` | `tests/test_custody_authored_migration.py::test_pair_invalidation_failure_keeps_complete_database_unchanged[update]` |
| `reference-pair-unpair-invalidation` | `tests/test_custody_authored_migration.py::test_pair_invalidation_failure_keeps_complete_database_unchanged[unpair]` |
| `desktop-pair-capability-order` | `desktop_pair_aliases_defaults_and_absent_notes_match_python` |
| `pair-validation-wire-order` | `pair_sequences_match_python_complete_database_rows_and_refusals` |

### Pair revision verification

Final verification used Rust 1.99.1, locked Python 3.11, Cargo `--offline`, and
disposable books. Workspace formatting, workspace clippy and the harness-feature
clippy with `-D warnings` passed. `cargo test --offline --workspace --locked`
passed **948 tests**. The final four pair tests also passed against the expanded
115-operation oracle, and the harness build passed. The seven real-process
transfer tests plus the 29-test reference regression module passed **36 tests**.
The exact requested shell/parity/hybrid-candidate/CLI-runner/`tests/rust_port`
gate passed **63 tests and 178 subtests**, including inventory and contracts.
The refreshed error-code snapshot changes source locations only.

All **131 mutations** were killed in a fresh isolated build target after all
**19 named baseline selections** passed. Initial mutation survivors exposed
missing oracle scenarios for unsplit separate swap fees, independent active
memberships and partial group residuals; all were added before the final full
run. The common activation regression also invokes the native helper directly
so the outer pair-service savepoint cannot hide its mutation. Transfer and
shared custody applicability checks passed (131 and 318 entries respectively).

The seven affected hybrid modules passed **386 tests and 35 subtests**, with
one inherited failure:
`ReviewRegressionTest.test_diagnostics_out_auto_writes_public_error_report`.
The global `--diagnostics-out` flag still returns `not_implemented` as it did
at the base checkpoint. This broader run is not reported green. Its 27 calls
to the supported transfer/review subset were all served by Rust, including
pair 6 and unpair 1; the focused witness covers update 2 as well. No supported
command fell back to Python. Group (c) calls intentionally used Python in the
hybrid candidate. Direct native daemon sessions have no Python dispatch.

Sandbox setup hit a quota error under `/tmp`, so disposable roots moved to the
workspace and then memory-backed temporary storage. Early checks also exposed
an unreviewed direct hash dependency, test-tree source-scan pollution, daemon
capability order, a direct Python CLI construction in a test and stale contract
line locations; each was resolved before the final gates. Final Cargo builds
were completed before process tests; mutation builds used a separate fresh
target to avoid reusing mutated artifacts. No push or PR was performed.


### Payout deletion checkpoint

`transfers payouts delete` and `ui.transfers.payouts.delete` now supersede the
stored payout component, preserve immutable terms/evidence, rebuild effective
memberships and perform Python's two journal invalidations. Repeats return the
original deletion time without changing rows. The service owns a savepoint and
preserves an outer caller transaction, including on late invalidation failure.
Desktop input preserves Python truthiness, identifier string conversion and GUI
audit attribution. AI projections remain Python. This brings the supported
subset to **14/29 CLI leaves and 13 desktop kinds**.

| Parity boundary | Evidence |
| --- | --- |
| Service outputs and normalized database rows | `payout_deletions_match_python_complete_database_rows_and_refusals`: six scenarios, 24 operations; every unrelated application table unchanged |
| CLI bytes and hybrid dispatch | `test_payout_delete_cli_daemon_and_hybrid_bytes`: three calls, all Rust, zero Python fallbacks |
| Daemon bytes | Same test: six requests including repeats, missing/false/numeric IDs and untrusted source |
| Rollback and caller transaction | `payout_deletion_rolls_back_late_failure_and_preserves_outer_transaction` |
| Desktop validation and source | `desktop_payout_delete_requires_id_and_overrides_source` |

The oracle generator is `crates/transfers/scripts/export_payout_delete.py`.
Payout creation is Python-only setup. Its SQL replay pins the exact starting
trigger counters and proves the replayed application rows equal Python before
deletion. Every changed table is compared completely after each operation;
all unrelated tables must stay unchanged in the native runner. No output
UUIDs, timestamps, hashes or array orders are masked. Real-process byte checks
use a stored deletion timestamp; first deletion timestamps and persisted bytes
are checked by the fixed-source native oracle.

The Python savepoint atomicity fix (`ba651df7`, cherry-picked as `be959dd6`)
now makes late payout invalidation rollback match Rust. The deletion oracle's
`late-failure` scenario rejects the second input-version increment and compares
every application table with its pre-operation state after caller rollback.
The old payout-delete bug repro is retired; this path is failure-path parity.

Payout creation, component plan/apply and gap plan/apply remain unported.
The ten computation-dependent commands and the additional desktop kinds listed
above also remain unported. Automatic transfer pairing during journal processing
belongs to R4; no matcher or journal behavior changes here.

The payout deletion catalog adds these named kills:

| Mutation | Named killing test |
| --- | --- |
| `payout-delete-atomic` | `payout_deletion_rolls_back_late_failure_and_preserves_outer_transaction` |
| `payout-delete-id` | `payout_deletions_match_python_complete_database_rows_and_refusals` |
| `payout-delete-idempotent` | `payout_deletions_match_python_complete_database_rows_and_refusals` |
| `payout-delete-state` | `payout_deletions_match_python_complete_database_rows_and_refusals` |
| `payout-delete-reason` | `payout_deletions_match_python_complete_database_rows_and_refusals` |
| `payout-delete-invalidations` | `payout_deletions_match_python_complete_database_rows_and_refusals` |
| `payout-delete-membership-clear` | `payout_deletions_match_python_complete_database_rows_and_refusals` |
| `payout-delete-membership-effective` | `payout_deletions_match_python_complete_database_rows_and_refusals` |
| `payout-delete-return-time` | `payout_deletions_match_python_complete_database_rows_and_refusals` |
| `payout-delete-final-invalidation` | `payout_deletions_match_python_complete_database_rows_and_refusals` |
| `desktop-payout-delete-source` | `desktop_payout_delete_requires_id_and_overrides_source` |
| `desktop-payout-delete-id-string` | `desktop_payout_delete_requires_id_and_overrides_source` |

Verification used Rust 1.99.1, locked Python 3.11, offline Cargo and disposable
books. Workspace fmt, workspace clippy and harness-feature clippy with
`-D warnings` passed. The workspace passed **951 tests**; the final native
transfer/daemon selection passed **29 tests** against the expanded fixture.
Oracle regeneration was byte-identical. The transfer mutation catalog now has
**143 entries**: the full 141-entry run killed all entries after 21 named baseline
selections passed; a fresh-target 14-entry follow-up rechecked payout deletion
and the adjusted desktop mutations and killed both new desktop entries.
The initial follow-up reused a target containing an old daemon test binary and
selected no new test; it was discarded and rerun on a fresh target. All **143/143**
entries are killed by named tests, and catalog applicability passes.

The final real-process transfer/shell/hybrid-candidate/CLI-runner and
`tests/rust_port` gate passed **71 tests and 178 subtests**. The regenerated
inventory and final preflight shard passed **57 tests**. The new hybrid witness
records `transfers.payouts.delete`: **3 Rust calls, 0 Python fallbacks**;
native daemon sessions contain no Python dispatch. The remaining setup and
computation commands retain the hybrid's existing fallback policy.

Initial restricted runs could not acquire the product's ownership locks or bind
workspace test loopback listeners. The successful runs used disposable books
with the required sandbox permissions. One process-gate attempt began before
the harness rebuild completed and hit the non-harness no-egress refusal; it was
discarded, and the complete successful gate ran after the build finished.
Mutation and verification build trees were removed after recording these results.
No push or PR was performed.


### Payout creation checkpoint

The local `t3code/r5-transfers-4` continuation starts at `3bd2370e` and adds
`transfers payouts create` and `ui.transfers.payouts.create`. The supported
subset is now **15/29 CLI leaves and 14 desktop kinds**. The domain service
reuses pair boundary resolution, protocol scope, exact quantity conversion,
native residual handling and the shared custody activation stage. Component
persistence takes the evidence term kind from the plan so payout evidence
commits to `direct_swap_payout`. Automatic pairing stays with R4; AI projections
remain Python.

| Parity boundary | Evidence |
| --- | --- |
| Ordered service output and complete normalized database rows | `payout_creations_match_python_complete_database_rows_and_refusals`: 38 scenarios, 98 operations (36 successes, 62 refusals) |
| CLI error bytes, desktop outputs and hybrid routing | `test_payout_create_cli_daemon_and_hybrid`: four Rust calls, zero Python fallbacks; three CLI byte comparisons and four daemon requests |
| Atomic failure and caller transaction ownership | `payout_creation_rolls_back_late_failure_and_preserves_outer_transaction` |
| Desktop aliases, coercion and trusted provenance | `desktop_payout_creation_normalizes_aliases_and_authority` |

`crates/transfers/scripts/export_payout_create.py` generates the synthetic
fixed-source oracle. It compares every changed row after each operation and
asserts all unrelated application tables stay unchanged. No service output
UUID, timestamp, evidence hash or serialized value is normalized. Real daemon
comparisons remove only generated UUIDs and timestamps; fixed-source vectors
pin those values separately. Scenarios cover same-asset taxable payouts,
profile policy defaults, fee-inclusive/separate-fee sources, split and exact
native residuals, idempotent deletion/recreation, Unicode asset and decimal
normalization, non-finite values, integer overflow, timestamp normalization,
optional text validation and original-versus-normalized return fields.

The fixed Python reference now rolls back late payout-create invalidation.
The creation oracle's `late-failure` scenario rejects the second input-version
increment, checks every application table is unchanged, and supplies the same
error and complete normalized rows to the Rust parity runner. The old
payout-create bug repro is retired. Rust additionally checks standalone and
outer-transaction failure, preserving the caller's unrelated pending writes.

| New mutation | Named killing test |
| --- | --- |
| `payout-create-kind`, `payout-create-policy`, `payout-create-asset`, `payout-create-fee`, `payout-create-hash`, `payout-create-residual`, `payout-create-invalidation`, `payout-create-raw-return`, `payout-create-physical`, `payout-create-nan`, `payout-create-range` | `payout_creations_match_python_complete_database_rows_and_refusals` |
| `payout-create-atomic` | `payout_creation_rolls_back_late_failure_and_preserves_outer_transaction` |
| `desktop-payout-create-source` | `desktop_payout_creation_normalizes_aliases_and_authority` |

Component plan/apply and gap plan/apply are still `not_implemented`. Their
version-bound fingerprints, complete revision parity, report impacts and
mutation gates remain outstanding; this checkpoint does not complete the
requested authoring continuation.

Verification used Rust 1.99.1, locked Python 3.11 and offline Cargo. Workspace
fmt, workspace and harness-feature clippy with `-D warnings` passed; all **954
workspace tests** passed. The regenerated inventory and preflight shard passed
**57 tests**. Oracle regeneration was byte-identical. The catalog has **156
applicable entries**; this continuation's **13 new mutations** and the adjusted
desktop registration-order mutation were all built and killed by four named
baseline selections on a fresh target. Earlier catalog kills remain recorded
above. One initial run exposed a stale registration-order assertion, and one
ineffective mutation changed an equivalent pre-normalization value; both were
corrected before the successful final run. No survivor is counted as a kill.

The process gate caught payout create/delete advertisement order in the daemon
handshake. Registration now follows Python's create-before-delete order; all
five handshake tests passed on recheck (including two subtests), and the
adjusted registration mutation was rebuilt and killed on another fresh target.
Final harness-feature clippy and native daemon tests passed after this fix.

The final complete shell rerun passed **41 tests and 172 subtests**. Together
with the other **31 passing process/inventory tests** from the wider gate,
this covers all 72 selected tests; the wider gate's five pre-fix failures were
exactly the corrected handshake tests. The nine transfer process tests all
passed, including payout-create's **4 Rust / 0 Python** hybrid witness.
Mutation targets and the verification build tree were removed after recording
results. No push or PR was performed.

### Savepoint reference-fix parity checkpoint

The local `t3code/r5-transfers-5` continuation starts at `b3721292` and
cherry-picks the Python atomicity fix `ba651df7` as `be959dd6`. The cherry-pick
had one documentation conflict; its unrelated dependency-origin context is
excluded from this branch's database compatibility notes.

The two payout rollback repro scripts are retired. Both fixed-source exporters
now inject a refusal of the final, second journal-input-version increment,
assert that Python leaves every application table unchanged after caller
rollback, and export the exact failure response and complete normalized rows
for Rust. Trigger refusal text now survives the transfer service error adapter;
it does not expose SQL statements or connection details, and the shell still
applies shared output redaction. Other SQLite errors retain the existing store
mapping. This is a failure-path parity checkpoint, not an added command family.

| Parity boundary | Result |
| --- | --- |
| Payout creation | 39 scenarios / 99 operations; 36 successes, 63 refusals, including late rollback |
| Payout deletion | 7 scenarios / 28 operations, including late rollback followed by successful retry |
| Complete service rows, responses and caller-transaction checks | All 8 `pairs.rs` tests pass |
| Reference atomicity regression | 29 tests pass |
| Existing transfer CLI, daemon and hybrid tests | 9 tests pass; 26 Rust calls, zero Python fallbacks |
| Fixture regeneration | Both payout fixtures regenerate byte-identically |
| Mutation checks | `payout-create-atomic`, `payout-delete-atomic`, and new `trigger-refusal-message` built and killed by named tests; all 157 entries apply |
| Required checks | Workspace fmt, workspace and harness clippy with `-D warnings`, 954 workspace tests, regenerated test inventory and 57-test preflight pass |

The new trigger-message mutation is killed by both
`payout_creations_match_python_complete_database_rows_and_refusals` and
`payout_deletions_match_python_complete_database_rows_and_refusals`.

Verification used Rust 1.99.1, locked Python 3.11 and offline Cargo. Disposable
mutation trees were removed immediately after their summaries. Restricted
workspace tests initially failed to bind owned loopback listeners; the complete
suite passed with the required sandbox permissions. Reference daemon checks
also needed ownership-lock permissions. A simultaneous workspace rebuild
replaced the harness executable during one process run; the successful final
process gate used an isolated copy. Persistent trigger injection at the process
boundary is correctly refused by structural schema admission, so late-write
injection remains in the service oracle rather than weakening admission.

Component plan/apply, gap plan/apply and their daemon kinds remain unported.
The supported command subset is still 15/29 CLI leaves and 14 desktop kinds.
Automatic pairing remains with R4. This checkpoint does not complete the
requested component/gap authoring continuation. No push or PR was performed.

A separate reference behavior issue surfaced in one generated daemon parity
run: operational amount redaction can replace part of a valid component UUID.
For example, `00000000-cad1-4000-8000-000000000000` becomes
`00000000-amount#<runtime-salted-hash>-4000-8000-000000000000` because `cad1`
is interpreted as a CAD amount. Independent generated UUIDs can therefore
cause a stochastic transcript mismatch even after UUID normalization. The
final rerun passed, but that does not resolve the flake. Reproduce without any
book or network using
`.venv/bin/python crates/transfers/scripts/repro_component_id_redaction.py`.
Python is left unchanged; the repro records this issue for separate review.

### Component plan/apply checkpoint

The local `t3code/r5-transfers-6` continuation starts at `01e113a8` and adds
`transfers components plan`, `transfers components apply`,
`ui.transfers.components.plan` and `ui.transfers.components.apply`. The
supported subset is now **17/29 CLI leaves and 16 desktop kinds**. All five
review actions are served: create (single or bulk, active or draft), revise,
undo, activate and supersede. Gap plan/apply and their desktop kinds remain
`not_implemented`; automatic pairing stays with R4 and AI projections stay
Python.

`component_review.rs` owns the version-bound planner: argument rules, reference
resolution (transaction id before external id, wallet id before label), rail,
exposure and unit defaults, `amount_btc` conversion with CPython decimal
signals, deterministic UUIDv5 bulk, revision and row ids, planned untracked
wallets and the bulk caps. `component_rows.rs` ports Python's authored
component contract: header, leg, allocation and economic-term normalization,
anchor checks, immutable creation and revision rows, activation with
author-bound evidence, supersession and membership reconciliation. Every
apply rebuilds its plan inside one savepoint after the input-version check.
Interpretation is not duplicated: `components::review` gains
`plan_validation` (conservation and raw anchors for unsaved rows),
`planned_batch_routes` (route continuity across active and planned quantity
rows) and `materialize_component` (Python's `get_component`, with
cross-component routes only when another active quantity component exists).
Planned wallet ids use a generated CPython 3.11 / Unicode 14 `casefold` table
in `kassiber-pytext`; component timestamps use Python's microsecond
`isoformat`, wallet timestamps stay second-precision.

| Parity boundary | Evidence |
| --- | --- |
| Ordered outputs and complete normalized rows | `component_reviews_match_python_complete_database_rows_and_refusals`: 24 scenarios, 253 operations (125 successes, 128 refusals); every unrelated application table unchanged |
| Late failure and caller transaction | `component_apply_rolls_back_late_failure_and_preserves_outer_transaction` |
| CLI bytes, normalized rows and hybrid routing | `test_component_plan_apply_cli_bytes_rows_and_hybrid`: 21 CLI calls (13 plan, 8 apply), all served by Rust, zero Python fallbacks |
| Daemon transcript and rows | `test_native_daemon_component_review_matches_python`: 11 requests |
| Desktop cap, alias, version and authorship rules | `desktop_component_review_caps_aliases_versions_and_authority` |

`crates/transfers/scripts/export_components.py` generates the fixed-source
oracle. Scenarios cover the full lifecycle, drafts, revisions (notes, legs,
allocations, evidence, conversion metadata), batches, raw-active conflicts and
competing active revisions, untracked placeholders (created, reused, missing
treasury account, Unicode case-folding collisions), wallet references,
quantity and `amount_btc` edge values, ~70 validation refusals, requested ids
and lineages, local location references, conversion and Liquid components,
profile scope, desktop authorship, stale revisions, revising drafts, pair
components with migrated economic terms, untracked route continuity across a
batch, recovered same-transaction native support and a late invalidation
failure. No output UUID, timestamp, hash or
serialized value is normalized in the native oracle. Process tests normalize
only wall-clock times; component ids are deterministic.

| New mutation | Named killing test |
| --- | --- |
| `component-bulk-id`, `component-revision-id`, `component-row-id`, `component-planned-wallet-casefold`, `component-batch-conflict`, `component-plan-replacing-lineage`, `component-quantity-range`, `component-input-version`, `component-already-applied`, `component-wallet-invalidation`, `component-supersedes-link`, `component-supersede-reconcile`, `component-default-type`, `component-undo-reason`, `component-hidden-location`, `component-json-constant`, `component-lightning-rail`, `component-economic-term-legs`, `component-effective-superseded`, `component-native-support`, `component-batch-route-filter`, `component-valuation-groups`, `component-planned-wallet-anchor` | `component_reviews_match_python_complete_database_rows_and_refusals` |
| `component-apply-atomic` | `component_apply_rolls_back_late_failure_and_preserves_outer_transaction` (and the oracle) |
| `desktop-component-cap`, `desktop-component-source`, `desktop-component-version` | `desktop_component_review_caps_aliases_versions_and_authority` |

Two reference behaviors are recorded with repros and left unchanged:

- A create plan can succeed while its own apply refuses it. Planned
  untracked-wallet ids case-fold labels, but label lookup uses SQLite's
  ASCII-only `lower()`. After `Straße` exists, `STRASSE` plans against the
  existing row and apply reports that `STRASSE` already exists:
  `.venv/bin/python crates/transfers/scripts/repro_planned_wallet_casefold.py`.
- Create plans echo caller-supplied `location_ref` values on planned legs to
  the renderer/AI projection, while materialized components and revision plans
  drop them: `.venv/bin/python crates/transfers/scripts/repro_batch_plan_location_ref.py`.

Rust matches both behaviors byte for byte.

Verification used Rust 1.99.1, locked Python 3.11, offline Cargo and disposable
books. Workspace fmt, workspace and harness-feature clippy with `-D warnings`
passed; all **958 workspace tests** passed. Oracle regeneration is
byte-identical. The catalog has **184 applicable entries**; the 27 new entries
were built and killed on scratch copies of the tree (24 in the first run; one
equivalent route-guard mutation was replaced by `component-native-support`,
and two survivors gained the `competing-active` and leading-zero cases before
their reruns killed them). The real-process transfer, shell, hybrid-candidate,
CLI-runner and `tests/rust_port` gate passed **74 tests and 178 subtests**; the
regenerated inventory and the preflight shard passed **57 tests**. Mutation
scratch trees were removed after their summaries. No push or PR was performed.

### Gap search dependency checkpoint

The continuation on `t3code/r5-transfers-7` imports only the pure gap-search
module from R2 checkpoint `23c059dc` (`t3code/r2d-gaps`), alongside its
synthetic CPython vectors. Journal composition, automatic pairing and product
routing are unchanged by this prerequisite. The public `gaps::compute` facade
accepts a raw worklist and explicit ignored populations; the SQL-worklist bounds
facade uses the same admission normalizer as search.

Eight named Rust tests cover 173 searches, 30 computations, 396 fingerprint
pairs, seven review-revision cases, 31 display cases and 173 worklist-bound
cases, plus admission-before-ignore and the adversarial timing witness.
`crates/transfers/scripts/export_gap_search.py --check` replays the fixed inputs
against this checkout's Python. All 109 applicable R2 search mutations and four
new admission/worklist mutations were killed in disposable scratch trees;
the build trees were removed after their summaries. This prerequisite alone
adds no served CLI commands or daemon kinds.

### Gap plan/apply checkpoint

The local `t3code/r5-transfers-7` continuation starts at `c3d18194` and adds
`transfers gaps plan`, `transfers gaps apply`, `ui.custody.review.plan` and
`ui.custody.review.apply`. The supported subset is **19/29 CLI leaves
(18/25 transfers, 1/4 review) and 18 desktop kinds**. Component plan/apply
was already complete at the preceding boundary; its existing oracle and
process checks remain part of this continuation's regression gate.

`gap_candidates.rs` mirrors Python's ordered small-book worklist and bounded
large-book SQL selection, current journal freshness and stored ignored
populations, plus active authored and journal claims when stored inputs are
unavailable. It reuses the pure R2 search from the prerequisite above.
`gap_review.rs` owns create, dismiss, reopen, revise and residual classification,
with exact fingerprints, deterministic component/child UUIDs, canonical FIFO
allocations, separate fees and unresolved suspense, version checks and a
candidate recheck inside apply's savepoint. `gap_review_store.rs` owns evidence
and correction-state checks, correction commitments, append-only review and
relation records, and saved/filed-report impacts. Shared component validation,
evidence, activation and supersession remain in the existing custody and
transfer services. Exact aggregate arithmetic uses `BigInt` throughout.

| Parity boundary | Named evidence | Result |
| --- | --- | --- |
| Ordered outputs and complete normalized database rows | `gap_reviews_match_python_complete_database_rows_and_refusals`, `gap_worklists_match_python_without_changing_database_rows` | 29 scenarios, 116 operations: 87 successes, 29 refusals |
| Late failure and caller transaction | `gap_apply_rolls_back_late_failure_and_preserves_outer_transaction` | Complete rollback without committing the caller's transaction |
| Loaded candidate and desktop version ordering | `gap_plan_rechecks_loaded_candidate_before_authoring`, `gap_desktop_version_validation_follows_candidate_resolution` | Stale evidence and malformed versions refused in Python order |
| CLI bytes, normalized rows, hybrid dispatch | `test_gap_cli_bytes_rows_and_hybrid` | 12 calls: 6 plan, 6 apply; all Rust, zero Python fallbacks |
| Daemon bytes, normalized rows, hybrid dispatch | `test_gap_daemon_bytes_rows_and_validation` | 12 Rust sessions; 108 gap requests plus 12 shutdowns; zero Python fallbacks |
| Desktop authority, Unicode bounds and dispatch | `desktop_gap_checks_shape_authority_and_dispatch` | Both kinds enforce GUI authorship and Python validation rules |

`crates/transfers/scripts/export_gaps.py --check` regenerates the fixed-source
oracle without writing. It asserts every unrelated application table is
unchanged and every refusal rolls back completely. The vectors cover the
full correction lifecycle, all residual classifications, fee-included sources,
N:M FIFO allocation, aggregate quantities above `i64`, excess returns,
saved/filed impacts and downstream years, late review/impact failure,
current/malformed/stale journal inputs, stale evidence and conflicting history.
Two 50,003-row books pin structured and ordinary SQL worklists; separate cases
pin timestamp overflow, malformed digit quantities and Unicode year prefixes.
Candidate years retain the saved-report calendar-year validation even when
the book has no saved or filed snapshots.
The native oracle normalizes no identities, timestamps, hashes or serialized
values. Process comparisons map only each new random review identity and its
derived relation identities, plus timestamps in the current test clock window;
historical timestamps remain exact.

| New mutation entries | Named killing test |
| --- | --- |
| `gap-input-version`, `gap-component-id`, `gap-child-id`, `gap-source-movement`, `gap-fifo`, `gap-residual-role`, `gap-residual-custody`, `gap-revision-fingerprint`, `gap-residual-fingerprint`, `gap-dismiss-invalidation`, `gap-post-version`, `gap-evidence-check`, `gap-correction-state`, `gap-correction-commitments`, `gap-review-history`, `gap-review-relations`, `gap-report-summary`, `gap-report-overlap`, `gap-report-persistence`, `gap-current-journal`, `gap-stored-ignored`, `gap-error-detail` | `gap_reviews_match_python_complete_database_rows_and_refusals` |
| `gap-large-worklist` | `gap_worklists_match_python_without_changing_database_rows` |
| `gap-report-year-range` | `gap_reviews_match_python_complete_database_rows_and_refusals` |
| `gap-apply-atomic` | `gap_apply_rolls_back_late_failure_and_preserves_outer_transaction` |
| `gap-candidate-recheck` | `gap_plan_rechecks_loaded_candidate_before_authoring` |
| `desktop-gap-version` | `gap_desktop_version_validation_follows_candidate_resolution` |
| `desktop-gap-unknown`, `desktop-gap-source`, `desktop-gap-reason`, `desktop-gap-classification` | `desktop_gap_checks_shape_authority_and_dispatch` |

Verification used Rust 1.99.1, locked Python 3.11, offline Cargo and disposable
books. Workspace fmt, workspace and harness-feature clippy with `-D warnings`,
and all **972 workspace tests** passed. Both oracle replay checks pass;
authoring fixture regeneration is byte-identical.
The transfer catalog has **215 applicable entries**; all 31 new authoring and
desktop entries were built and killed (30 in the first serial scratch run,
then the final calendar-year guard). Together with the prerequisite, this run
adds **144 killed mutations**. Both catalogs validate, including 431 custody
entries. The process/contract gate passed **76 tests and 178 subtests**;
the required inventory update and preflight shard passed **57 tests**.
Mutation and verification build trees were removed after their summaries.

Ten CLI leaves remain `not_implemented`: gaps list/review, components list/show,
suggest, bulk-pair, rules apply, and review cases/plan/apply. Their associated
desktop computation/projection kinds and AI projections remain Python.
Automatic journal pairing belongs to R4 and is unchanged by this continuation.
The UUID-redaction fix is reserved for integration.

## R4 automatic pairing library checkpoint

R4-14 supplies `kassiber_transfers::pairing` for suggestions, rule application,
bulk exact pairing and the four-pass journal automatic-pairing sequence.
[Its contract and parity report](rust-pairing.md) describe the R4-13/R4-15
transaction boundary, generated and fixture oracles, performance and mutations.
No additional CLI leaf or daemon kind is registered; route totals above stay
unchanged. Python desktop/AI projections continue reading the same authored rows.
