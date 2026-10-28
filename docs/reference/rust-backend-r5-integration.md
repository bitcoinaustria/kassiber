# R5 integration audit

Branch: `t3code/rust-backend-r5`, based on custody tip `b61162fc`.
The verified code tip is `d96664ad`; the final commit records this audit and
clarifies a daemon registration comment. This integration adds no service
features. The partial-port and platform limitations in the owning slice status
documents still apply.

## Commit provenance

The branch contains 79 cherry-picks followed by six integration commits,
including this report. There are no merge commits. All 79 picked commit messages
match their originals in full, and their order matches the requested ranges.

| Input | Original range, excluding its lower bound | Picks |
| --- | --- | ---: |
| Storage and binary wiring | `6da3e459..348618ab` | 46 |
| Reviewed HTTPS client | `82f6b77b..1a0763b7` | 10 |
| Rates | `1409ffef..8cbc2d56` | 2 |
| Setup | `82f6b77b..31ccde83` | 5 |
| Transactions | `82f6b77b..f417d50a` | 6 |
| Wallets, including imports | `82f6b77b..5bde6914` | 10 |

The six cherry-picked input branch refs still resolve to the supplied tips.
During the final audit, the custody branch was advanced outside this integration
from `b61162fc` to `8a70ae74` (ref reflog: 2026-10-28 19:47:23 +0100). This stack
retains the explicitly requested `b61162fc` base; that additional custody commit
is outside the requested ranges. This integration changed no input branch,
popped or dropped no stash, and pushed or published nothing.

## Conflicts and resolutions

Commit IDs in this table identify the original commits. Paths use brace notation
where several files share a directory. Every conflicting commit is listed.

| Original commit | Conflicting files | Resolution |
| --- | --- | --- |
| `aee8679b` | `docs/reference/rust-port.md` | Regenerated inventory counts; retained the surrounding prose. |
| `82f6b77b` | `docs/reference/rust-port.md` | Regenerated inventory counts; retained the surrounding prose. |
| `2d74a968` | `TODO.md` | Kept the newer R0 completion, replay-bound and native-spike tasks, and added the incoming HTTPS acceptance task. Superseded pending inventory/harness/spike wording follows the newer completion status. |
| `8cbc2d56` | `crates/net/tests/reviewed-dependencies.txt`; `docs/reference/{rust-port-egress.md,rust-port.md}` | Retained the reviewed client's feature restrictions and the rates/DEFLATE entries. Preserved policy/cost prose and the complete rates section. Regenerated inventory counts. |
| `24846270` | `TODO.md`; `crates/cli/src/{describe.rs,run.rs}`; `crates/daemon/src/lib.rs`; `crates/kassiber/Cargo.toml`; `crates/kassiber/src/real_store.rs`; `crates/kassiber/tests/binary.rs`; `crates/runtime/src/store.rs`; `docs/reference/{rust-port.md,rust-shell.md}`; `tests/test_rust_shell_parity.py` | Unioned setup/rates registrations, dependencies, book methods and dispatch. Kept setup's operation/namespace tuple and rates dispatch. Preserved both prose sections. Tests distinguish the complete command catalog from executable commands/kinds rather than keeping obsolete fixed counts. Regenerated inventory. |
| `669e408c` | `crates/cli/src/describe.rs`; `docs/reference/{rust-port.md,rust-shell.md}` | Added projects alongside rates in IMPLEMENTED; retained both service descriptions and regenerated counts. |
| `dfebf7c1` | `Cargo.lock`; `THIRD_PARTY_LICENSES.md`; `crates/kassiber/src/real_store.rs`; `docs/reference/{rust-port.md,rust-shell.md}` | Kept the integration lock side and resolved additions offline. Unioned license rows and combined purposes, including fancy-regex/tempfile and transport/DEFLATE/age/tar. Kept both the rates provider and retained-key fields. Preserved rates/network prose and regenerated counts. |
| `31ccde83` | `THIRD_PARTY_LICENSES.md`; `crates/cli/src/describe.rs`; `docs/reference/{rust-port.md,rust-shell.md}` | Preserved toml_edit/DEFLATE rows and tempfile's expanded purpose; added offline registrations beside existing families. Preserved rates/offline prose and regenerated counts. |
| `7b7e720d` | `docs/reference/rust-port.md` | Regenerated inventory counts. |
| `a0766f34` | `Cargo.lock`; `THIRD_PARTY_LICENSES.md`; `TODO.md`; `crates/cli/src/{describe.rs,lib.rs,run.rs}`; `crates/kassiber/Cargo.toml`; `crates/kassiber/src/real_store.rs`; `crates/kassiber/tests/binary.rs`; `crates/runtime/src/store.rs`; `docs/plan/21-rust-backend.md`; `docs/reference/{rust-port.md,rust-shell.md}`; `tests/test_rust_shell_parity.py` | Unioned registrations, dependencies and book methods. The shared book dispatcher retains setup's tuple and invokes attachment/loan services before generic setup dispatch. Preserved the full-catalog assertions and all setup/rates/transaction prose. Unioned license purposes, resolved the lock offline and regenerated counts. |
| `185c3707` | `crates/cli/src/run.rs`; `crates/kassiber/src/real_store.rs`; `crates/runtime/src/store.rs`; `docs/reference/{rust-port.md,rust-shell.md}`; `tests/rust_port/dispositions.json`; `tests/test_rust_shell_parity.py` | Registered metadata through the existing operation tuple; retained all prior book methods and full-catalog assertions. Preserved every service section; regenerated dispositions/counts. |
| `df73a011` | `docs/reference/rust-port.md` | Regenerated inventory counts for the incoming Python reference regression. |
| `f417d50a` | `docs/reference/{rust-port.md,rust-shell.md}`; `tests/test_rust_shell_parity.py` | Retained setup/rates prose and the expanded metadata/history description; kept full-catalog assertions and regenerated counts. |
| `e2dfbaf4` | `Cargo.lock`; `TODO.md`; `crates/cli/src/{describe.rs,lib.rs,run.rs}`; `crates/daemon/{Cargo.toml,src/lib.rs}`; `crates/kassiber/src/real_store.rs`; `crates/kassiber/tests/binary.rs`; `crates/runtime/{Cargo.toml,src/store.rs}`; `docs/plan/21-rust-backend.md`; `docs/reference/{rust-port.md,rust-shell.md}`; `tests/rust_port/dispositions.json`; `tests/test_rust_shell_parity.py` | Retained the combined setup dispatcher and added imports' separate `with_book` admission path, including rollback `.plan` kinds. Unioned methods, registrations, dependencies and parity coverage. Preserved both sides' prose; resolved the lock offline and regenerated inventory. |
| `a21e1d6e` | `docs/reference/rust-port.md` | Regenerated inventory counts. |
| `5bde6914` | `Cargo.lock`; `crates/daemon/src/lib.rs`; `crates/kassiber/src/real_store.rs`; `crates/kassiber/tests/binary.rs`; `crates/runtime/src/store.rs`; `docs/reference/rust-port.md`; `tests/rust_port/dispositions.json`; `tests/test_rust_shell_parity.py` | Added wallet methods and descriptor-preview registration alongside every prior family. Kept full-catalog assertions. The unported-command witness uses `transactions.list` because `rates.pairs` is now implemented. Resolved the lock offline and regenerated inventory/contracts. |

Root workspace membership merged automatically and was checked against the union
of the inputs. License rows retain the union, with shared rows describing both
uses. Generated inventory/disposition conflicts were resolved by
`python -m tests.rust_port.classify_tests update`; contract fixtures were
regenerated with `python -m tests.rust_port.contracts update`. Generated data was
not hand-merged. No doc was resolved wholesale with `--ours` or `--theirs`.

An initial setup-resolution script mishandled multiple conflict blocks. That
range was aborted and replayed correctly with rerere disabled; the malformed
trial commits are absent from the integration branch. No unresolved
same-function conflict required choosing one slice's behavior over another.

## Integration commits and rates adaptations

Each additional commit has an imperative subject explaining its purpose and the
`Co-Authored-By: Codex <noreply@openai.com>` trailer.

| Commit | Adaptation |
| --- | --- |
| `375a986f` | Restored function-local `py_str`/`truthy` imports in `RealBook::rates`, which the shared setup resolver no longer imports globally. |
| `d1116f6d` | Routed rates' pure URL parsing through the reviewed client's `HttpUrl` re-export and removed rates' direct `url` dependency/lock edge. Kept the reviewed client's IDNA pin and dependency confinement. Explicitly reviewed only the already-used `flate2/rust_backend` and net test-support features. |
| `902b2472` | Preserved the combined daemon registration union in Python order and formatted stacked adapters. Explicitly reviewed the existing local slice dependencies, the exact setup partition age child call, and pure IPv6 literal parsing. Socket/import/feature bypass checks remain active, with regression assertions. |
| `9893bc10` | Carried existing custody-base ISO-week and timezone-adjacent separator/fraction fixes into metadata's copied parser after the newer custody corpus exposed drift. No new timestamp behavior beyond the integration base was invented. |
| `d96664ad` | Regenerated metadata vectors using the checked-in Python 3.11 generator: 40,665 timestamp cases, including 34,989 additions; all 5,676 prior input/outcome pairs and all 2,009 legacy float vectors are unchanged. Updated the owning status document. |
| Final report commit | Records this audit and clarifies the lifecycle-versus-family registration comment. |

The reviewed client's public request/grant API required no further rates call
adaptations. Final registration audits find exactly the input union: 71 CLI
commands, 40 product daemon kinds in Python order, and four harness-only kinds.
The complete 313-entry command metadata catalog remains separate from execution
support.

## Lockfile audit

For each lock conflict, the current integration side was retained and
`cargo metadata --offline --format-version 1` supplied missing dependencies.
Final locked builds/checks leave the lockfile unchanged.

All 337 registry package identities match an input tip exactly by name, version,
source and checksum. No new package version was introduced. Where origins use
multiple versions, the required versions coexist instead of being upgraded.

| Origin | Registry identities retained |
| --- | ---: |
| Custody `b61162fc` | 44/44 |
| Storage `348618ab` | 178/178 |
| Reviewed client `1a0763b7` | 207/207 |
| Rates `8cbc2d56` | 212/233 |
| Setup `31ccde83` | 287/287 |
| Transactions `f417d50a` | 178/178 |
| Wallets `5bde6914` | 205/205 |

The 21 omitted rates-origin identities belong to its pre-review URL/ICU stack.
The reviewed client's `idna_adapter` stays at `1.0.0`; rates does not reintroduce
`1.2.2` or its ICU stack. Setup's independent displaydoc/tinystr/zerovec pins
match setup's origin, rather than the obsolete rates URL graph.

Final `Cargo.lock` SHA-256:
`e97a9c42b114972385e2c992770fddc5553b8132f09764c399c9c20f054c4099`.
The machine-readable package/checksum provenance and original-to-picked commit
mapping are retained in the local verification artifacts described below.

## Verification

Verification used `mise exec rust@1.99.1 uv@latest --`, Rust 1.99.1,
`MATURIN_NO_INSTALL_RUST=1`, and the environment created by
`uv sync --locked --python 3.11 --offline` (CPython 3.11.17). Cargo ran offline;
dependency-consuming checks also used `--locked`. Books, fixtures and mutation
trees used disposable roots. HOME was never redirected to a tooling install.

| Required native check | Result |
| --- | --- |
| `cargo fmt --all --check` | Passed |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Passed |
| `cargo clippy -p kassiber --all-targets --features test-harness --locked --offline -- -D warnings` | Passed |
| `cargo test --workspace --locked --offline` | 932 passed; zero failed/ignored |
| `cargo test -p kassiber-net --features test-observer --locked --offline` | 62 passed; zero failed/ignored |
| `cargo build -p kassiber --features test-harness --locked --offline` | Passed after the final parser/corpus changes |

Every checked-in mutation catalog completed with its unmutated baseline passing
and every entry killed. Separate scratch targets prevent mutant artifacts from
entering other acceptance runs.

| Catalog under `crates/<area>/scripts/mutation_check.py` | Killed |
| --- | ---: |
| custody | 318/318 |
| setup | 24/24 |
| rates | 9/9 |
| transactions | 30/30 |
| wallets | 12/12 |
| imports | 14/14 |
| Total | 407/407 |

The required Python command ran with
`KASSIBER_RUST_BIN=$PWD/target/debug/kassiber` after the final harness build:

```sh
uv run --locked --offline python -m pytest -q \
  tests/test_rust_shell_parity.py tests/test_rust_hybrid_candidate.py \
  tests/test_cli_runner.py tests/rust_port tests/test_ci_shards.py
```

It passed **81 tests and 225 subtests**. No main-target Cargo builds overlapped
this final run. Additional hybrid checks used a byte-for-byte copy of that final
harness at `/tmp/kassiber-r5-verified-bin/kassiber`.

| Hybrid/reference modules or scenario | Result |
| --- | --- |
| Setup: `test_saved_views`, `test_profile_policy_label`, `test_projects`, `test_book_network`, `test_book_network_boundaries`, `test_book_network_read_snapshot`, `test_book_network_migration`, `test_rust_hybrid_candidate` | 116 passed, 2 subtests |
| Rates: `test_rust_rates_candidate`, `test_rates_incremental`, `test_rates_kraken_csv`, `test_rates_csv_validation` | 53 passed |
| Transactions: `test_rust_transaction_services`, `test_rust_metadata_services`, `test_transaction_edit_history`, `test_loans`, `test_attachment_open_scope` | 39 passed, 2 subtests |
| Wallets/imports: `test_wallet_descriptors`, `test_wallet_setup`, `test_wallet_config_multi_script`, `test_import_batches` | 92 passed, 26 subtests |
| Full `test_cli_smoke` module through the hybrid candidate | 95 passed |
| Existing project/catalog/encrypted-creation, network/partition and offline process witnesses from `test_rust_shell_parity` | 4 passed |
| Existing all-setup-leaves hybrid scenario, with its routing log exported before cleanup | Passed; 23 routing records |
| Existing metadata mutation/cursor and four attachment regressions from `test_review_regressions` | 6 passed |
| Existing `test_daemon_transaction_attachments_round_trip`, explicitly routed to Rust | 1 passed |
| `test_egress_policy` | 43 passed |

Python service calls made in process remain reference checks. Native execution is
established by the routing witnesses, direct byte comparisons and native tests.
The required shell suite includes descriptor-preview byte comparisons with six
native preview cases; direct daemon requests are not counted as CLI invocations.

Inventory, contract and CI shard checks passed. `tests/rust_port` contributes ten
passing tests to the required suite. Checked-in setup, network-scope, wallet,
metadata, MIME and lowercase fixture generators all pass `--check` at the tip.

### Sandbox limits and excluded attempts

The restricted sandbox cannot open the loopback socket fixtures, write uv's
shared cache, mutate this worktree's Git metadata outside its writable root, or
write Python's fixed account-home ownership runtime. Required affected commands
were rerun with approved elevated execution and passed on disposable books.
Network tests used fixture servers, with no real provider acceptance claim.

Initial restricted workspace/net/rates-mutation attempts failed at those socket
boundaries; those failures are not acceptance evidence. The first metadata
fixture check exposed the real corpus drift addressed above. An earlier CLI
hybrid run (94 passed, one failed) and setup process run (three passed, one
failed) overlapped a harness rebuild and encountered a temporarily absent binary.
Their routing logs are excluded. Clean final reruns passed all 95 and four
tests respectively, using the stable copied harness.

No requested integration check remains unrun or failing. The full census and
custody-shadow shards remain with the orchestrator as requested. This audit does
not claim the complete repository quality gate, native macOS/Windows acceptance,
or the deferred service/platform work in the input status documents.

## Served-by audit

Only successful acceptance logs are counted. Across setup, rates, transactions,
wallet/import modules, full CLI smoke, process witnesses and supplemental
regressions, the aggregate log has 1,011 calls: 547 Rust and 464 Python.
The Python calls belong to unported commands. Every one of the 67 ported slice
commands below was observed, with **zero Python fallbacks**.

| Slice | Ported CLI commands observed | Rust calls | Python fallbacks |
| --- | ---: | ---: | ---: |
| Setup | 26/26 | 278 | 0 |
| Rates | 6/6 | 33 | 0 |
| Transactions | 32/32 | 126 | 0 |
| Imports | 2/2 | 8 | 0 |
| Wallets | 1/1 | 4 | 0 |

Other implemented-shell counts are `init` 89, `status` two,
`commands.describe` one, and `daemon` six, all Rust with zero Python fallbacks.
The six logged daemon invocations represent explicitly selected Rust sessions;
mixed default hybrid daemon sessions still follow the documented routing rule.
Individual daemon requests compared directly by the process oracles add evidence
without adding hybrid-log records.

### Setup

| Command | Rust | Python |
| --- | ---: | ---: |
| `offline.status` | 10 | 0 |
| `offline.on` | 1 | 0 |
| `offline.off` | 2 | 0 |
| `projects.list` | 7 | 0 |
| `projects.show` | 4 | 0 |
| `projects.create` | 8 | 0 |
| `projects.select` | 3 | 0 |
| `workspaces.list` | 1 | 0 |
| `workspaces.create` | 95 | 0 |
| `profiles.list` | 2 | 0 |
| `profiles.create` | 116 | 0 |
| `profiles.get` | 5 | 0 |
| `profiles.set` | 2 | 0 |
| `accounts.list` | 2 | 0 |
| `accounts.create` | 4 | 0 |
| `context.show` | 1 | 0 |
| `context.current` | 1 | 0 |
| `context.set` | 1 | 0 |
| `views.list` | 2 | 0 |
| `views.create` | 1 | 0 |
| `views.delete` | 1 | 0 |
| `networks.inventory` | 2 | 0 |
| `networks.plan` | 2 | 0 |
| `networks.bind` | 2 | 0 |
| `networks.split-plan` | 1 | 0 |
| `networks.split` | 2 | 0 |

### Rates

| Command | Rust | Python |
| --- | ---: | ---: |
| `rates.pairs` | 2 | 0 |
| `rates.set` | 13 | 0 |
| `rates.latest` | 2 | 0 |
| `rates.range` | 2 | 0 |
| `rates.sync` | 13 | 0 |
| `rates.rebuild` | 1 | 0 |

### Transactions

| Command | Rust | Python |
| --- | ---: | ---: |
| `loans.mark` | 7 | 0 |
| `loans.unmark` | 2 | 0 |
| `loans.link` | 2 | 0 |
| `loans.list` | 2 | 0 |
| `attachments.add` | 9 | 0 |
| `attachments.list` | 3 | 0 |
| `attachments.rename` | 4 | 0 |
| `attachments.remove` | 3 | 0 |
| `attachments.verify` | 2 | 0 |
| `attachments.gc` | 3 | 0 |
| `metadata.tags.list` | 1 | 0 |
| `metadata.tags.create` | 27 | 0 |
| `metadata.tags.add` | 3 | 0 |
| `metadata.tags.remove` | 2 | 0 |
| `metadata.notes.set` | 7 | 0 |
| `metadata.notes.clear` | 1 | 0 |
| `metadata.exclude` | 2 | 0 |
| `metadata.include` | 2 | 0 |
| `metadata.records.get` | 2 | 0 |
| `metadata.records.note.set` | 5 | 0 |
| `metadata.records.note.clear` | 2 | 0 |
| `metadata.records.tag.add` | 2 | 0 |
| `metadata.records.tag.remove` | 1 | 0 |
| `metadata.records.excluded.set` | 4 | 0 |
| `metadata.records.excluded.clear` | 1 | 0 |
| `metadata.records.kind.set` | 3 | 0 |
| `metadata.records.kind.clear` | 3 | 0 |
| `metadata.records.history.stale` | 1 | 0 |
| `metadata.records.list` | 8 | 0 |
| `metadata.records.history.list` | 4 | 0 |
| `metadata.records.history.activity` | 3 | 0 |
| `metadata.records.history.revert` | 5 | 0 |

### Imports

| Command | Rust | Python |
| --- | ---: | ---: |
| `imports.list` | 4 | 0 |
| `imports.rollback` | 4 | 0 |

### Wallets

| Command | Rust | Python |
| --- | ---: | ---: |
| `wallets.derive` | 4 | 0 |

## Local evidence and remaining work

Raw logs are in `/tmp/kassiber-r5-verification/`. Key artifacts are
`workspace-tests-tip.log`, `net-observer-loopback.log`,
`mutations-*-final.log` / the successful per-area mutation logs,
`python-required-tip.log`, `python-hybrid-cli-tip.log`,
`hybrid-served-by-final.json`, `lock-provenance.json`,
`picked-commits.json`, and `registrations.json`.

There is no unresolved integration conflict or stop condition. The branch is
local-only and ready for the orchestrator's full census and custody-shadow
verification. Existing partial-port and platform gates are preserved.
