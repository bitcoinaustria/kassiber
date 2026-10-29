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

## Round 2: R5b stopped at the setup test conflict

**This round is incomplete.** Branch `t3code/rust-backend-r5b` starts at the
requested custody tip `a4e017a3`. Its last replayed code commit is
`c9432a16eb780a9d3786445152ce25495dc1cc14`; a following documentation-only
commit records this stop. The completed first-round acceptance above does not
apply to this newer custody base or establish acceptance of this partial stack.

### Provenance and remaining picks

The branch has 87 completed cherry-picks, in order, with every original commit
message preserved in full and no merge commits:

| Input | Required picks | Completed | Remaining |
| --- | ---: | ---: | ---: |
| Base `b61162fc..65c74e84` | 86 | 86 | 0 |
| Setup `31ccde83..14e5a23a` | 7 | 1 | 6 |
| Transactions `f417d50a..d6d9d4f4` | 10 | 0 | 10 |
| Wallets `5bde6914..2fe3541a` | 8 | 0 | 8 |
| Chain clients `82f6b77b..5e7e6489` | 3 new | 0 | 3 |
| Secrets `0d4fbc28..a051e1ef` | 2 | 0 | 2 |
| Backup `a051e1ef..6271064d` | 5 | 0 | 5 |
| Reports `0d4fbc28..349bf2ca` | 5 | 0 | 5 |
| Total | 126 | 87 | 39 |

The one completed setup increment is `9c74e890`, the separately committed
Python daemon fix. No skip was executed before the stop. The chain-client
range was audited before replay: **96 of its 99 commits are already ancestors
of `65c74e84`**, so they must be skipped when that range is reached. Exactly
`git rev-list --reverse 82f6b77b..0d4fbc28` identifies those 96 commits. The
only new chain picks, in order, are `3108ac4e`, `3c0e5b7a`, and `5e7e6489`.
Their full IDs and the complete contained-commit list are in the local manifest.

All base Python fixes are present once through the ordered replay. The later
timestamp-bound, wallet preflight, Core-credential, and backup-recovery Python
fixes are still pending with their slices; this partial branch does not claim
to contain them. No pending fix was applied separately or duplicated.

### Conflicts resolved before the stop

Every completed conflicting commit is listed below. `inventory` means only
the generated count table in `docs/reference/rust-port.md`; it was regenerated
with `python -m tests.rust_port.classify_tests update`, together with
`tests/rust_port/dispositions.json`. Surrounding prose survived. No generated
counts or dispositions were hand-merged.

| Original commit | Conflicting paths | Resolution |
| --- | --- | --- |
| `5331f395` | inventory; `tests/rust_port/reviews.py` | Unioned the separate custody-channel/safety and storage review dictionary entries; regenerated inventory. |
| `e2734062` | inventory | Regenerated. |
| `fe4ace19` | inventory | Regenerated. |
| `4b633e22` | inventory | Regenerated. |
| `c9181145` | inventory | Regenerated. |
| `2a8f1454` | inventory | Regenerated. |
| `9dfa92dd` | inventory | Regenerated. |
| `f6f55b15` | inventory | Regenerated. |
| `b75b7419` | inventory | Regenerated. |
| `3924ef01` | inventory | Regenerated. |
| `ad9579b1` | inventory | Regenerated. |
| `e6ba90a6` | inventory | Regenerated. |
| `3ed80fbb` | inventory | Regenerated. |
| `459bab11` | inventory | Regenerated. |
| `a04e1e1a` | inventory | Regenerated. |
| `9c74e890` | inventory | Regenerated. |

Contracts were regenerated during the base replay with
`python -m tests.rust_port.contracts update`; the partial-tip check passes.
One transient resolver attempt truncated working copies of the inventory
document and review dictionary. Both were reconstructed from the three index
stages before completing the first conflicted pick; no malformed trial commit
or lost surrounding content is in the branch.

### Required stop: `64d8a78d`

The next setup commit, `64d8a78d` ("Port backend configuration through shared
setup services"), conflicts in these six paths:

| Path | Conflict and disposition |
| --- | --- |
| `Cargo.lock` | Local-package dependency additions overlap; no resolution retained. |
| `crates/kassiber/src/real_store.rs` | Separate `rates` and `current_default_backend` method additions overlap at the impl boundary; no resolution retained. |
| `crates/runtime/src/store.rs` | Separate default trait-method additions overlap; no resolution retained. |
| `crates/kassiber/tests/binary.rs` | Both sides change the same test function differently; required stop. |
| `docs/reference/rust-port.md` | Generated count conflict; no resolution retained. |
| `tests/rust_port/dispositions.json` | Generated disposition conflict; no resolution retained. |

The exact function is
`describes_all_commands_and_keeps_unported_execution_closed`. The completed
base tests these unported witnesses:

```rust
[("wallets", "list"), ("backends", "list"), ("transactions", "list")]
```

The incoming setup commit changes the same loop to:

```rust
[("wallets", "list"), ("rates", "pairs")]
```

Both edits affect the same test function. The user's rule, "Same function
changed two different ways: stop and report rather than choose," therefore
stops this replay. The CLI/daemon registration-union rule does not explicitly
authorize choosing between these test witness sets. No witness set was chosen.
The full conflict, staged diff, status, and original incoming patch were saved
before aborting only this pending cherry-pick. The 87 completed picks remain;
there is no cherry-pick in progress. Resume with `64d8a78d` only after the
orchestrator resolves this policy decision. Then replay the remaining inputs
in the original order and skip the 96 contained chain commits.

### Partial-tip dependency and ref audit

`Cargo.lock` is byte-identical to `65c74e84`. All **337/337 registry package
identities**, including exact version, source and checksum, match the prior
base and occur in the supplied input lockfiles. No package was upgraded.
`cargo metadata --offline --locked --format-version 1` succeeds and leaves
the lockfile unchanged. No lock conflict resolution from the aborted setup
pick is retained.

SHA-256: `e97a9c42b114972385e2c992770fddc5553b8132f09764c399c9c20f054c4099`.

This run changed no input branch or stash. During replay, an external actor
advanced `t3code/rust-backend-r2` from `a4e017a3` to `cc18b710` (its reflog
records `branch: Reset to cc18b710`). This integration retains the explicitly
requested `a4e017a3` base and does not include that later advance. Other saved
`t3code/rust-backend-*` input refs and the stash ref are unchanged. Nothing was
pushed or published.

### Partial-tip verification and limits

Tooling used `mise exec rust@1.99.1 uv@latest --`,
`MATURIN_NO_INSTALL_RUST=1`, locked CPython 3.11.17, and offline Cargo.
`uv sync --locked --python 3.11 --offline` passed using the existing shared
cache with elevated execution. Temporary-cache attempts lacked `lwk` or the
`maturin==1.15.0` build requirement; they are not acceptance evidence.
HOME was never redirected to a tooling-install directory. Generated catalog
reads used disposable roots and a no-socket guard.

| Check | Partial-tip result |
| --- | --- |
| `cargo fmt --all --check` | Passed. |
| `cargo metadata --offline --locked --format-version 1` | Passed; lock unchanged. |
| Inventory check | Passed through locked uv using the shared cache. |
| Contracts check | Passed through locked uv using the shared cache. |
| Ordered replay, original messages, linear history | Passed for all 87 completed picks. |
| Workspace clippy and `kassiber` test-harness clippy | Not run: integration stopped. |
| Workspace tests and net test-observer suite | Not run: integration stopped. |
| Every `crates/*/scripts/` mutation catalog | Not run: integration stopped; no kills claimed. |
| Test-harness build and required Python suite | Not run: integration stopped. |
| Per-slice hybrid parity modules | Not run: integration stopped. |

No successful hybrid routing log was produced in this round. Setup, rates,
transactions/documents, wallets/imports, chain clients, secrets, backup and
reports therefore have **no measured served-by totals**, and zero Python
fallbacks are **not established**. Earlier-round or slice-branch totals above
must not be reused as R5b acceptance. Loopback acceptance and native credential
platform checks were not attempted, so this run makes no sandbox or platform
acceptance claim for them. The full census and custody-shadow shards remain
with the orchestrator, after the stack is completed.

Evidence is in `/tmp/kassiber-r5b-verification/`: `inputs.json`, `replay.json`,
`replay.log`, `origin-locks.json`, `partial-audit.json`, `refs-before.txt`,
`stopped-conflicts.diff`, `stopped-staged.diff`, `stopped-status.txt`,
`stopped-original.patch`, `partial-fmt.log`, `partial-metadata.json`, and
`partial-{inventory,contracts}-shared-cache.log`. These artifacts describe the
partial code tip, not a completed R5b integration.

## Round 2 continuation: stopped at the shared datetime replacement

The orchestrator authorized resolution of the first stop: derive test witnesses
from `IMPLEMENTED`, union separate method additions, and continue stopping when
the same function body changes differently. Under that decision, replay resumed
through setup commit `4674a1b5`. The last replayed code tip is
`581b1a8875d473dd1d9fb8840e5e202f8489baaf`. **The integration remains incomplete:**
`14e5a23a` introduces a different same-function conflict and was not picked.

### Completed work and test-witness integration

There are now **92 completed picks**: all 86 base commits and six of the seven
setup increments. Every original message still matches in full, and the history
is linear. Of the originally requested 126 picks, 34 remain: setup 1,
transactions 10, wallets 8, chain 3, secrets 2, backup 5, and reports 5.
No skip has yet been executed. The previously identified 96 contained chain
commits still need to be skipped when that range is reached.

Integration commit `91112304` ("Derive unported witnesses from the catalog so
slice integrations preserve the refusal gate") replaces the hard-coded list in
`describes_all_commands_and_keeps_unported_execution_closed`. It iterates over
every catalog leaf absent from `IMPLEMENTED`, derives required arguments and
required mutually exclusive choices from the parser specification, and retains
the existing catalog assertions, exit/refusal assertions and no-bootstrap check.
This runs the full set rather than a sampled subset. Formatting of this addition
was included in the following setup conflict resolution.

The focused native check passed:

```sh
mise exec rust@1.99.1 uv@latest -- cargo test -p kassiber --test binary \
  --locked --offline describes_all_commands_and_keeps_unported_execution_closed
```

It selected one test and passed with zero failures/ignores in 1.25 seconds.
The partial stack has 80 implemented leaves out of the 313-leaf catalog, so
the test exercises all 233 unported leaves. The dynamic witnesses remain valid
as later slices extend `IMPLEMENTED`.

### Additional conflicts and resolutions

This table supplements the first-stop table above; its resolution of
`64d8a78d` supersedes that commit's earlier pending disposition.

| Original commit | Conflicting paths | Resolution |
| --- | --- | --- |
| `64d8a78d` | `Cargo.lock`; `crates/kassiber/src/real_store.rs`; `crates/runtime/src/store.rs`; `crates/kassiber/tests/binary.rs`; `docs/reference/rust-port.md`; `tests/rust_port/dispositions.json` | Unioned separate `rates` and `current_default_backend` methods and retained both fields; kept catalog-derived test witnesses under the orchestrator decision; kept the current lock side and supplied the CLI/secrets and setup/JSON dependency edges through offline metadata; regenerated inventory. |
| `a69ad9fa` | `docs/reference/rust-port.md` | Regenerated counts and dispositions. |
| `19e2f7ef` | `docs/reference/rust-shell.md` | Combined the full existing service summary with backend configuration: 35 setup leaves and 80 implemented leaves overall. Both sides' service content survives. |
| `14e5a23a` (not picked) | `Cargo.lock`; `crates/custody/src/components/time.rs`; `crates/daemon/src/lib.rs`; `docs/reference/rust-port.md`; `tests/rust_port/dispositions.json`; `tests/rust_port/reviews.py` | Saved all conflict/staged/original evidence and aborted only this pending pick. No resolution from it is retained. |

### Required second stop

The conflicting function is `crates/custody/src/components/time.rs::parsed`.
The integrated custody implementation retains its expanded local parser,
including trailing-`Z` rewriting and CPython's separator/fraction/offset
admission. Incoming `14e5a23a` removes the local parser helpers and replaces
the body after the empty/null guard with:

```rust
kassiber_pytext::iso_datetime(pystr(v).py_strip()).map(|dt| dt.utc_us())
```

The incoming shared parser was extracted against an older version of the
custody parser. Git reports both helper-removal and `parsed`-body conflicts.
This is a same-function body change, not separate method additions or example
command lists, so the orchestrator's remaining stop rule applies. Neither the
local implementation nor the shared replacement was chosen as the resolution.
Proving the expanded shared parser preserves every custody boundary needs an
explicit integration decision before this commit can proceed.

Resume with `14e5a23a`, then the remaining inputs in their original order.
Custody advance `a4e017a3..cc18b710` is exactly one commit, `cc18b710` (HTLC
production and reservations). It was neither rebased nor cherry-picked here:
the user's prerequisite of finishing and verifying the remaining slice picks
has not been reached. The branch still has the requested `a4e017a3` base.

### Updated partial verification

The lockfile retains all 337 registry package identities from the previous base
with exact versions, sources and checksums; no identity outside the supplied
origin locks appears, and no package was upgraded. The only lockfile differences
from `65c74e84` are the CLI's local dependency on `kassiber-secrets` and setup's
dependency on the already locked `serde_json`. Locked offline metadata succeeds
without changing it.

SHA-256: `6cf2e3f4a4804ccc13381bbac16093d217fa33de263cbe167407f80a44349b3e`.

Formatting and the full catalog-witness test pass. The initial inventory and
contract checks correctly detected the newly picked Python regression and
shifted source contracts; both were regenerated with their checked-in generators
and checked again. These generated changes are recorded with this continuation
audit, separately from the original slice messages.

Workspace/harness clippy, workspace/net tests, mutation catalogs, the harness
build, the required Python suite, and per-slice hybrid acceptance were not run:
replay stopped at the explicitly retained same-function boundary. No per-slice
served-by totals or zero-fallback claim is established in this continuation.
No new unresolved sandbox failure was encountered; shared cache/Git metadata
operations used approved elevation and native tests used disposable books.
The full census and custody-shadow shards remain for the orchestrator after
the stack is completed. Nothing was pushed, no input ref was changed by this
run, and no stash was popped or dropped.

Additional evidence in `/tmp/kassiber-r5b-verification/`:
`resolutions.json`, `resumed-partial-audit.json`, `catalog-witness-test.log`,
`resume-metadata.json`, and `stop-14e5a23a/{conflicts.diff,staged.diff,status.txt,original.patch}`.
