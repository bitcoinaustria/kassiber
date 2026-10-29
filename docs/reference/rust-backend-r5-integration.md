# R5 integration audit

The completed R5b rebuild and its remaining diagnostics parity failure are
recorded in [the final round-2 audit](#round-2-completed-audit-r5b). Earlier
sections retain the previous audit and stopped checkpoints.

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

## Round 2 completed audit: R5b

This section supersedes the two stopped R5b checkpoints above. Branch
`t3code/rust-backend-r5b` starts at `a4e017a3` and includes the requested
custody increment through `cc18b710`. The verified tip is `8f0fd86e`. The native harness was built at `05a582a7`;
later commits change only Python refusal witnesses and one mutation entry.
Native sources and dependency pins are unchanged;
the final documentation commit records this audit. There is no unresolved
integration conflict. This rebuild introduces no service feature.

### Ordered provenance and contained commits

All **127 original commits** were cherry-picked in the requested order, with
their complete messages unchanged. The history has no merge commits. The
base's 86 non-custody commits were replayed on `a4e017a3`, then each slice
increment, then the single custody advance. **96 contained chain commits were
skipped**, including 85 already replayed from the base and 11 inherited custody
ancestors. No Python fix was applied separately to compensate for a skip.

| Input | Exact range, lower bound excluded | Picked | Skipped |
| --- | --- | ---: | ---: |
| base | `b61162fc..65c74e84` | 86 | 0 |
| setup | `31ccde83..14e5a23a` | 7 | 0 |
| transactions | `f417d50a..d6d9d4f4` | 10 | 0 |
| wallets | `5bde6914..2fe3541a` | 8 | 0 |
| chain | `82f6b77b..5e7e6489` | 3 | 96 |
| secrets | `0d4fbc28..a051e1ef` | 2 | 0 |
| backup | `a051e1ef..6271064d` | 5 | 0 |
| reports | `0d4fbc28..349bf2ca` | 5 | 0 |
| custody advance | `a4e017a3..cc18b710` | 1 | 0 |
| Total | | 127 | 96 |

The new chain picks are exactly `3108ac4e`, `3c0e5b7a` and `5e7e6489`.
After the slice/native checkpoint, **cherry-picked** `cc18b710` as `e44d0d0b`;
the branch was not rebased. That commit supplies HTLC production and
reservations. Its custody code applied cleanly; only the lock, license table
and generated inventory conflicted.

The following is the complete skipped chain list, in its original order
(`git rev-list --reverse 82f6b77b..0d4fbc28`):

```text
5cd77225 0cd014ee f583ccd1 7d28cddc fc7aae47 16b5b57d e4184742 32ed9eca
c7018146 98fc8528 b61162fc 741fceea 19268e9a 482c59bc 6df0385b 8bd0c086
556b8027 5f4c6047 b65da7ba a1d45b1d b280bbda 1b7fae97 12ae62bb 0dbf105f
069ee11e f48dac1d ebd461c9 16fce1c2 99b49865 def10ba6 b3e399bb 4d0e35a4
d2603f56 552c696f 48bdc3f1 664fad05 d601b0bd c9941947 507e8fe5 7084757d
6ef4e094 17d35a14 d9384377 2dab8f67 aaef1672 163e9554 685c135e 6291fc88
d430eeb6 281b82a0 6b6d323d 4931fde0 5331f395 fa9107b4 be09872d e2734062
401caac0 f4e9513b 720219e1 7317432c 048cf379 653b6e71 418b8e94 5eb7dbf6
6cd69a98 ea5cf87b c327493d 9a5c0808 fe4ace19 4b633e22 c9181145 3f117ab6
2a8f1454 9dfa92dd f6f55b15 b75b7419 ebb5d2dc 3924ef01 ad9579b1 e6ba90a6
3ed80fbb 68948812 439c37a9 33eb7482 459bab11 87516f35 033aa56b b9581e0e
5d68d764 a04e1e1a 375a986f d1116f6d 902b2472 9893bc10 d96664ad 0d4fbc28
```

The 12 incremental Python production-fix commits are present exactly once,
and each Python-only stable patch ID equals its originating commit:

- setup: `9c74e890`, `a69ad9fa`, `5f4bf929`.
- transactions: `d3ec0f94`, `4f3cb12d`, `4fc40b2f`.
- wallets: `a9bfc1b1`, `129868ae`, `9d4fc3a2`, `79de6256`.
- chain: `3108ac4e`.
- backup: `64378ed2`.

This includes the instant-bound timestamp correction, wallet preflight fixes,
Core credential normalization and durable backup recovery. Base fixes remain
in their original replay positions. The ordered mapping is `replay.json`;
`final-audit.json` records message, pin, registration and Python patch checks.

No input branch or stash was modified by this integration; no stash was popped
or dropped, and nothing was pushed or published. Other workers advanced
`t3code/rust-backend-r2` from `a4e017a3` through `cc18b710` to `16234030`,
`t3code/r5-reports-5` from `349bf2ca` to `3b20eecc`, and
`t3code/r5-sync-4` from `72c18e53` to `f9ff09d0`. Their reflogs identify those
external updates. The later tips are outside the exact requested inputs and
were not incorporated. The stash ref is unchanged.

### Complete round-2 conflict table

Every one of the 47 conflicting original commits is listed. `inventory` means
the generated count table in `docs/reference/rust-port.md`; `dispositions`
means `tests/rust_port/dispositions.json`. Those files and generated contracts
were regenerated using their checked-in update commands, then checked. Frozen
vectors were checked with their generators and were not hand-merged. All
surrounding prose was preserved. Lock conflicts kept the integrated side before
`cargo metadata --offline` supplied missing edges; no versions were upgraded.

| Original commit | Conflicting paths | Resolution |
| --- | --- | --- |
| `5331f395` | inventory; `tests/rust_port/reviews.py` | Union separate custody-channel/safety and storage review entries; regenerate inventory. |
| `e2734062` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `fe4ace19` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `4b633e22` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `c9181145` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `2a8f1454` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `9dfa92dd` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `f6f55b15` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `b75b7419` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `3924ef01` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `ad9579b1` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `e6ba90a6` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `3ed80fbb` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `459bab11` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `a04e1e1a` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `9c74e890` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `64d8a78d` | `Cargo.lock`; `crates/kassiber/src/real_store.rs`; `crates/kassiber/tests/binary.rs`; `crates/runtime/src/store.rs`; inventory; dispositions | Union separate rates/current_default_backend methods; retain catalog-derived witnesses under orchestrator decision; retain ours lock then offline metadata; regenerate inventory. |
| `a69ad9fa` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `19e2f7ef` | `docs/reference/rust-shell.md` | Combine the existing full service summary with backends; 35 setup leaves, 80 implemented total. |
| `14e5a23a` | `Cargo.lock`; `crates/custody/src/components/time.rs`; `crates/daemon/src/lib.rs`; inventory; dispositions; `tests/rust_port/reviews.py` | Keep custody parsed and its local parser bodies unchanged; retain new pytext iso_datetime for setup-only callers. Only custody file changed by incoming commit was time.rs. Union daemon registrations, review overrides; resolve lock offline; regenerate inventory. |
| `a6359897` | `crates/cli/src/run.rs`; `crates/daemon/src/lib.rs`; `crates/daemon/src/metadata.rs`; inventory; `docs/reference/rust-shell.md`; `docs/reference/rust-transactions-r5-status.md`; `tests/test_rust_shell_parity.py` | Combine tuple/preparation API with BIP329 emit metadata; union daemon kinds in Python order without duplicate lifecycle registrations; preserve full-catalog byte equality over obsolete subset projections; combine all prose including timestamp alignment and BIP329 limitations; regenerate inventory. |
| `d3ec0f94` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `17a69a38` | `crates/daemon/src/lib.rs`; `crates/daemon/src/metadata.rs`; inventory; `docs/reference/rust-shell.md` | Union grouped metadata registration in family KINDS and Python order; retain lifecycle-only constants; combine all pytext capabilities including setup ISO parsing; regenerate inventory. |
| `e66eafae` | `crates/cli/src/describe.rs`; `crates/cli/src/lib.rs`; `crates/cli/src/run.rs`; `crates/kassiber/src/real_store.rs`; `crates/runtime/src/store.rs`; inventory; `docs/reference/rust-shell.md`; `tests/test_rust_shell_parity.py` | Union CLI modules and IMPLEMENTED, separate Book methods; add listing dispatch to existing operation/preparation API; retain full-catalog byte checks; preserve every service and listing prose section; regenerate inventory. |
| `4f3cb12d` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `d219bed5` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `4fc40b2f` | inventory; `tests/rust_port/reviews.py` | Union separate desktop-validation and custody/setup review dictionary entries; regenerate inventory. |
| `d5296a02` | `crates/daemon/src/lib.rs`; `crates/kassiber/src/real_store.rs`; `crates/runtime/src/store.rs`; inventory; `docs/reference/rust-shell.md` | Union separate desktop methods and daemon module/registration with existing service families; combine new desktop limits/prose and retain complete implemented catalog summary; regenerate inventory. |
| `d6d9d4f4` | `crates/cli/src/run.rs`; `crates/daemon/src/lib.rs`; `crates/kassiber/src/real_store.rs`; `crates/runtime/src/store.rs`; inventory; `docs/reference/rust-shell.md`; `tests/test_rust_shell_parity.py` | Union document/commercial modules, methods, CLI and daemon registrations using existing operation tuple; retain full-catalog parity; deduplicate daemon witnesses; preserve all prose and regenerate inventory. |
| `a9bfc1b1` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `f4769452` | `Cargo.lock`; `THIRD_PARTY_LICENSES.md`; `crates/cli/src/run.rs`; `crates/kassiber/tests/binary.rs`; `crates/runtime/src/store.rs`; inventory; `docs/reference/rust-shell.md`; dispositions; `tests/test_rust_shell_parity.py` | Retain setup credential preparation and full-catalog assertions; derive daemon witnesses from registered supported kinds; union distinct wallet Book method and license purposes; preserve complete service summary plus wallet limits; keep integration lock and resolve offline; regenerate inventory. |
| `9d4fc3a2` | inventory | Regenerate inventory counts and dispositions; retain surrounding prose. |
| `ee1551dc` | `crates/kassiber/src/real_store.rs`; `crates/kassiber/src/real_store/backends.rs`; `crates/runtime/src/store.rs`; inventory; dispositions; `tests/test_rust_shell_parity.py` | Union wallet snapshot fields and methods with existing services. Carry the exact added Config::kinds method to the setup backend_runtime module where Config moved; retain the setup re-export. Preserve registration-derived witnesses and regenerate inventory. |
| `f4b4b341` | `THIRD_PARTY_LICENSES.md`; inventory; dispositions; `tests/test_rust_shell_parity.py` | Retain full-catalog assertions over obsolete subset; combine all license explanations including wallet import numeric normalization; regenerate inventory. |
| `2fe3541a` | `crates/kassiber/tests/binary.rs`; inventory; `docs/reference/rust-shell.md`; dispositions; `tests/test_rust_shell_parity.py` | Retain catalog-derived counts and all unported witnesses; preserve added wallet catalog assertions; retain full-catalog Python byte equality; combine service prose and import limitations; regenerate inventory. |
| `5e7e6489` | `TODO.md`; `crates/daemon/src/backends.rs`; `crates/daemon/src/services.rs`; `crates/kassiber/src/real_store.rs`; `crates/kassiber/src/real_store/backends.rs`; inventory; dispositions; `tests/rust_port/reviews.py` | Union backend configuration/probe registration and handler routing, retain workspace routes. Place explicit probe dispatch before broad local-backend dispatch; reuse existing Config and carry exact added resolve method to its moved setup module. Preserve TODO completions and new remaining limitations, union distinct review entries, regenerate inventory/contracts. |
| `96461ae3` | `TODO.md`; `crates/cli/src/describe.rs`; inventory; `docs/reference/rust-shell.md`; dispositions; `tests/rust_port/reviews.py` | Union IMPLEMENTED and separate review entries; preserve both complete TODO sections and crate/service descriptions with corrected integrated total; regenerate inventory. |
| `a051e1ef` | `crates/daemon/src/lib.rs`; inventory; `docs/reference/rust-shell.md`; `tests/rust_port/reviews.py` | Union secrets and reset daemon registration, distinct review entries and complete crate/service prose; regenerate inventory. |
| `2c14fdc0` | `Cargo.lock`; `crates/cli/src/lib.rs`; `crates/kassiber/Cargo.toml`; `docs/plan/21-rust-backend.md`; inventory; `docs/reference/rust-shell.md`; dispositions; `tests/rust_port/reviews.py` | Union CLI modules, binary dependencies and disjoint review entries; preserve all wallet/backup plan sections and service descriptions; keep integration lock, resolve offline, regenerate inventory. |
| `64378ed2` | inventory; `tests/fixtures/rust_port/environment.json`; `tests/rust_port/reviews.py` | Union distinct review entries; regenerate environment contracts and inventory from the combined Python fixes rather than merge generated line references. |
| `0d775706` | inventory; `docs/reference/rust-shell.md`; `tests/rust_port/reviews.py` | Preserve all integrated service prose while advancing backup installation boundary; union distinct review entries and regenerate inventory. |
| `6271064d` | `crates/daemon/src/lib.rs`; inventory | Union backup module/kinds and document/transaction handlers, retain backup-session expiration before these dispatches and existing profile reset; regenerate inventory. |
| `571190d9` | `crates/cli/src/describe.rs`; `crates/cli/src/lib.rs`; `crates/cli/src/services.rs`; `crates/daemon/src/lib.rs`; `crates/kassiber/Cargo.toml`; `crates/kassiber/src/real_store.rs`; `crates/runtime/src/store.rs`; `docs/plan/21-rust-backend.md`; inventory; `docs/reference/rust-shell.md`; `scripts/python_test_shards.py`; dispositions; `tests/rust_port/reviews.py` | Union report modules, command/daemon registries, distinct Book methods and dependencies; preserve backup expiry and all earlier handlers; union disjoint CI/review entries and full plan/service prose; regenerate inventory. |
| `89f978ac` | `docs/reference/rust-shell.md` | Preserve all crate/service descriptions while advancing report summary to Austrian forms and nine report reads. |
| `6d0d7c88` | `THIRD_PARTY_LICENSES.md`; `docs/reference/rust-shell.md` | Preserve all numeric-normalization and historical-resolver license explanations and complete service prose, advancing to ten report reads. |
| `349bf2ca` | `Cargo.lock`; `THIRD_PARTY_LICENSES.md`; `crates/cli/Cargo.toml`; inventory; `docs/reference/rust-shell.md` | Union CLI dependencies and license purposes; retain complete service summary plus CSV checkpoint; keep integration lock and resolve offline, regenerate inventory. |
| `cc18b710` | `Cargo.lock`; `THIRD_PARTY_LICENSES.md`; inventory | Cherry-pick the single authorized custody HTLC/reservation increment after the slice/native checkpoint. Union bitcoin_hashes license row with all existing rows, keep integration lock and resolve local dependency edges offline; regenerate inventory/contracts. No custody code conflict. |

Root workspace members merged automatically and equal the input union. License
rows retain the union and all originating purposes. CLI `IMPLEMENTED` is exactly
119 unique leaves; daemon registration is exactly 88 product kinds in Python
wire order, followed by four harness kinds. Full catalog byte comparisons remain
in place; execution support is separate from the 313-leaf metadata catalog.

### Integration decisions and review corrections

- `91112304` derives refusal witnesses from every catalog leaf absent from
  `IMPLEMENTED`, including required arguments and mutually exclusive choices.
  The original catalog/refusal/no-bootstrap assertions remain. At the final tip
  this covers all 194 unported leaves. Later ported/unported test examples use
  the same rule, or derive daemon examples from advertised kinds.
- `62485dab` preserves custody's `time.rs::parsed` body and its trailing-`Z`
  rewrite/CPython offset semantics. Setup's separate `iso_datetime` remains at
  setup's own callers. `14e5a23a` touched no other custody source call site.
  The full 45,477-case CPython timestamp/fuzz corpus passed unchanged, setup and
  custody tests passed (568 at that checkpoint), and the then-current custody
  catalog killed 889/889 entries. No parser unification is claimed.
- Separate `Book`/`RealBook` methods and fields were unioned. The combined CLI
  retains setup's operation/namespace/preparation contract and dispatches every
  service family, including chain probes before broad backend configuration.
- Setup had moved backend `Config` to `backend_runtime.rs`. Wallet `kinds` and
  chain `resolve` were carried there. Chain's original field-only environment
  overlay is retained as `apply_probe_fields`; setup's source/name-annotating
  helper is a different operation and was not substituted.
- `e0f58310` fixes combined wiring, preserves daemon wire order, removes the
  duplicate backend module, formats stacked adapters and derives stale test
  examples from registries. It reviews the existing entropy/backup/test-hook
  dependencies, exact age and test-child launches, and pure IP parsing in the
  net guard. Connector/feature bypass regression assertions remain active.
  A custody mutation description was reworded to avoid the scanner's accidental
  `hyper` substring match; its mutation and killer are unchanged.
- The test-only hybrid fallback launcher shares the existing disposable native
  owner namespace with Python. `05a582a7` removes an unreachable duplicate of
  that setup. `e9ab3807` adds the required manual census sample for the expanded
  shell module; `89586485` reviews the already-pinned pure custody hash crate.
- `68ef0498` derives the two transaction/metadata refusal tests from a deterministic argument-free database-read
  catalog subset absent from `IMPLEMENTED`, after the parity run exposed their
  obsolete `reports.summary` examples. Both original refusal/no-bootstrap
  assertions remain.
- `8f0fd86e` retargets the equivalent backup logging mutant to enable ERROR
  logging at the incoming guard. Shared keying already disables logging, so
  removing the backup-only guard cannot affect the integrated behavior. The
  ID, rule and existing stderr killer remain; its fresh baseline passes and
  the new mutant is killed. No product code changes.
- `8b0aaedd` and `4739bd3c` retain the earlier stop audits. Each extra integration
  commit uses an imperative why-message and the required Codex coauthor trailer.

### Lockfile verification

Every one of the **337 registry identities** matches an originating lock by name,
version, source and checksum. No novel identity or upgraded version appears.
Every source lock identity is retained, including all 51 at `cc18b710`; local
workspace edges are resolved offline. Locked checks/builds leave the lockfile
unchanged.

| Origin tip | Registry identities retained |
| --- | ---: |
| `a4e017a3` | 44/44 |
| `65c74e84` | 337/337 |
| `14e5a23a` | 287/287 |
| `d6d9d4f4` | 178/178 |
| `2fe3541a` | 205/205 |
| `5e7e6489` | 337/337 |
| `a051e1ef` | 337/337 |
| `6271064d` | 337/337 |
| `349bf2ca` | 337/337 |
| `cc18b710` | 51/51 |

Final `Cargo.lock` SHA-256:
`ec6b1a871ad0ab34b02d6b3d6445d4fd38dcf3b5751e11a15dbe0606ee149bc1`.

### Verification at the final code tip

Tooling was `mise exec rust@1.99.1 uv@latest --`, Rust 1.99.1,
`MATURIN_NO_INSTALL_RUST=1`, and the locked CPython 3.11.17 environment created
by `uv sync --locked --python 3.11 --offline`. Cargo dependency operations used
`--offline --locked`. Every book, native keyring and mutation scratch tree used
a disposable root. HOME was never pointed at a tooling installation. Shared
uv-cache and Git metadata operations used approved elevation.

| Required native check | Result |
| --- | --- |
| `cargo fmt --all --check` | Passed |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Passed |
| `cargo clippy -p kassiber --all-targets --features test-harness --locked --offline -- -D warnings` | Passed |
| `cargo test --workspace --locked --offline` | 1,176 passed; 0 failed; 0 ignored |
| `cargo test -p kassiber-net --features test-observer --locked --offline` | 62 passed; 0 failed; 0 ignored |
| `cargo build -p kassiber --features test-harness --locked --offline` | Passed |

The names above map to the exact requested format, workspace/all-target clippy,
`kassiber`/all-target/test-harness clippy, locked workspace tests, locked
`kassiber-net --features test-observer` tests, and locked `kassiber
--features test-harness` build. Storage/store/secrets/backup probes and the chain
client fixture were also built offline/locked with their required test hooks.
No main-target Cargo build overlapped the final required Python suite.
Per-slice acceptance used a byte-for-byte copy of that final harness binary.

All ten catalogs under `crates/*/scripts/` ran through their checked-in mutation
checkers, with all named baselines passing before mutation. Partitioning retained
all catalog entries; scratch trees and build targets were independent. Every
current entry was killed. The backup quiet-auth entry uses the passing
supplemental check after retargeting its equivalent mutant; the original
106 kills and all original baseline checks remain valid. The original failed
partition report is retained alongside that supplemental proof:

| Catalog | Killed |
| --- | ---: |
| custody | 1035/1035 |
| setup | 57/57 |
| rates | 9/9 |
| transactions | 226/226 |
| wallets | 83/83 |
| imports | 14/14 |
| chain | 20/20 |
| secrets | 18/18 |
| backup | 107/107 |
| reports | 278/278 |
| Total | 1,847/1,847 |

The exact required Python command ran after the final harness build with
`KASSIBER_RUST_BIN=$PWD/target/debug/kassiber` and `KASSIBER_HYBRID_LOG`:

```sh
mise exec rust@1.99.1 uv@latest -- uv run --locked --offline python -m pytest -q \
  tests/test_rust_shell_parity.py tests/test_rust_hybrid_candidate.py \
  tests/test_cli_runner.py tests/rust_port tests/test_ci_shards.py
```

Result: **92 passed, 331 subtests passed in 617.32s (0:10:17)**.

All 27 frozen-generator/inventory/contract checks pass. These include every
custody timestamp, component, interpretation, ownership, consolidation, channel,
HTLC/native and safety corpus; wallets' descriptor/import vectors; reports'
full generator (including historical cases); transactions' timestamp/desktop
vectors; and setup's four vector generators. Default check mode or `--check`
was used. The channel generator has no check flag, so its in-memory output was
compared with the frozen file without invoking its unconditional write entry
point. Inventory and contracts were regenerated after integration and checked
again at the tip.

### Per-slice hybrid parity and served-by totals

The following modules ran against the copied harness. The test-only wrapper
preserves each module's local routing log while collecting command/served-by
pairs centrally, even for tests that scrub the environment. The three shell
helpers also mirror their local logs; 37 duplicate mirror records were removed
from the coverage stream after checking that each repeated adjacent block is
identical (networks 9, backends 15, offline 13). Raw records and the deduction
manifest are retained, so no invocation is counted twice. CSV/Phoenix calls
also enable `KASSIBER_HYBRID_IMPORT_ORACLE=1` for typed whole-book comparisons.
Daemon instrumentation records only kind names from native parity exchanges;
interactive backup sessions use
the module's existing native request counter. Payloads and secrets are not
retained. Explicit Python reference-oracle subprocesses are excluded.

| Slice suite | Modules | Result |
| --- | --- | --- |
| setup | `test_rust_setup_desktop.py`, `test_backend_configuration.py`, `test_book_network.py`, `test_saved_views.py`, `test_projects.py` | 78 passed in 28.14s |
| rates | `test_rust_rates_candidate.py`, `test_rates_csv_validation.py`, `test_rates_incremental.py`, `test_rates_kraken_csv.py` | 53 passed in 53.72s |
| transactions | `test_rust_transaction_services.py`, `test_rust_metadata_services.py`, `test_rust_bip329_services.py`, `test_rust_grouped_metadata_services.py`, `test_rust_transaction_listing.py`, `test_rust_transaction_desktop.py`, `test_rust_document_services.py`, `test_timestamp_filters.py`, `test_transaction_desktop_validation.py`, `test_transaction_edit_history.py` | 53 passed, 97 subtests passed in 518.46s (0:08:38) |
| wallets | `test_wallet_descriptors.py`, `test_wallet_setup.py`, `test_wallet_config_multi_script.py`, `test_import_batches.py`, `test_review_regressions.py`, `test_cli_smoke.py`, `test_silent_payments.py`, `test_rust_hybrid_candidate.py` | 1 failed, 414 passed, 54 subtests passed in 786.56s (0:13:06) |
| chain | `test_rust_chain_candidate.py`, `test_daemon_core_probe.py` | 30 passed in 12.23s |
| secrets | `test_rust_secrets_services.py`, `test_rust_secrets_lifecycle.py` | 120 passed in 218.13s (0:03:38) |
| backup | `test_rust_backup_services.py`, `test_rust_backup_install.py`, `test_rust_backup_desktop.py`, `test_backup_recovery.py` | 1411 passed in 644.56s (0:10:44) |
| reports | `test_rust_reports.py` | 19 passed in 534.94s (0:08:54) |

CLI counts below aggregate the required suite and final slice runs, including
the wallet diagnostics failure, grouped by the owning command family rather than the test module
that happened to call it. Supplementary coverage reran 11 existing parity
tests (11 passed and 23 subtests passed) and compared ten additional native
daemon requests with Python on the existing disposable transaction fixture.
Direct native CLI calls are identified by the actual executable and its pure
implementation probe, then recorded with the same command/served-by schema.
All 115 ported service CLI leaves were observed in
Rust with **zero Python service fallbacks**. Four shell leaves are reported
separately. All 88 product daemon kinds were observed in native sessions.

| Owner | Rust CLI calls | Python service fallbacks | Rust daemon requests |
| --- | ---: | ---: | ---: |
| shell | 431 | See routing note | 175 |
| setup | 732 | 0 | 69 |
| rates | 53 | 0 | 14 |
| transactions | 493 | 0 | 462 |
| wallets | 462 | 0 | 185 |
| imports | 8 | 0 | 2 |
| chain | 0 | 0 | 17 |
| secrets | 163 | 0 | 7 |
| backup | 40 | 0 | 51 |
| reports | 353 | 0 | 226 |

The hybrid harness intentionally defaults a whole `daemon` session to Python
unless `KASSIBER_HYBRID_RUST_DAEMON=1`, because a session may mix unported kinds.
Routing tests exercise that policy. Native-session kind counts above are
separate from daemon-process CLI counts. Unported commands such as journal
processing still use explicit reference fallbacks and do not enter the
zero-service-fallback claim. Usage-error routing counts remain in `served-by-final.json`.

Per-command counts make the coverage claim inspectable:

| CLI command | Rust calls | Python fallbacks |
| --- | ---: | ---: |
| `commands.describe` | 10 | 0 |
| `daemon` | 153 | 1 (intentional session policy) |
| `init` | 242 | 0 |
| `status` | 26 | 0 |
| `accounts.create` | 6 | 0 |
| `accounts.list` | 4 | 0 |
| `backends.clear-default` | 4 | 0 |
| `backends.create` | 25 | 0 |
| `backends.delete` | 9 | 0 |
| `backends.get` | 20 | 0 |
| `backends.kinds` | 2 | 0 |
| `backends.list` | 12 | 0 |
| `backends.reveal-token` | 2 | 0 |
| `backends.set-default` | 5 | 0 |
| `backends.update` | 14 | 0 |
| `context.current` | 3 | 0 |
| `context.set` | 28 | 0 |
| `context.show` | 4 | 0 |
| `networks.bind` | 4 | 0 |
| `networks.inventory` | 4 | 0 |
| `networks.plan` | 4 | 0 |
| `networks.split` | 4 | 0 |
| `networks.split-plan` | 2 | 0 |
| `offline.off` | 4 | 0 |
| `offline.on` | 2 | 0 |
| `offline.status` | 20 | 0 |
| `profiles.create` | 258 | 0 |
| `profiles.get` | 8 | 0 |
| `profiles.list` | 3 | 0 |
| `profiles.set` | 15 | 0 |
| `projects.create` | 8 | 0 |
| `projects.list` | 7 | 0 |
| `projects.select` | 3 | 0 |
| `projects.show` | 4 | 0 |
| `views.create` | 2 | 0 |
| `views.delete` | 3 | 0 |
| `views.list` | 4 | 0 |
| `workspaces.create` | 233 | 0 |
| `workspaces.list` | 2 | 0 |
| `rates.latest` | 3 | 0 |
| `rates.pairs` | 2 | 0 |
| `rates.range` | 5 | 0 |
| `rates.rebuild` | 1 | 0 |
| `rates.set` | 28 | 0 |
| `rates.sync` | 14 | 0 |
| `attachments.add` | 11 | 0 |
| `attachments.gc` | 3 | 0 |
| `attachments.list` | 4 | 0 |
| `attachments.remove` | 3 | 0 |
| `attachments.rename` | 4 | 0 |
| `attachments.verify` | 3 | 0 |
| `btcpay.provenance.links` | 1 | 0 |
| `btcpay.provenance.list` | 1 | 0 |
| `btcpay.provenance.suggest` | 1 | 0 |
| `documents.attach` | 5 | 0 |
| `documents.create` | 5 | 0 |
| `documents.import-report` | 2 | 0 |
| `documents.list` | 1 | 0 |
| `loans.link` | 2 | 0 |
| `loans.list` | 2 | 0 |
| `loans.mark` | 7 | 0 |
| `loans.unmark` | 2 | 0 |
| `metadata.bip329.export` | 12 | 0 |
| `metadata.bip329.import` | 17 | 0 |
| `metadata.bip329.list` | 10 | 0 |
| `metadata.bip329.preview` | 10 | 0 |
| `metadata.exclude` | 4 | 0 |
| `metadata.include` | 3 | 0 |
| `metadata.notes.clear` | 2 | 0 |
| `metadata.notes.set` | 10 | 0 |
| `metadata.records.excluded.clear` | 2 | 0 |
| `metadata.records.excluded.set` | 5 | 0 |
| `metadata.records.get` | 6 | 0 |
| `metadata.records.history.activity` | 15 | 0 |
| `metadata.records.history.list` | 5 | 0 |
| `metadata.records.history.revert` | 5 | 0 |
| `metadata.records.history.stale` | 3 | 0 |
| `metadata.records.kind.clear` | 4 | 0 |
| `metadata.records.kind.set` | 5 | 0 |
| `metadata.records.list` | 27 | 0 |
| `metadata.records.note.clear` | 3 | 0 |
| `metadata.records.note.set` | 8 | 0 |
| `metadata.records.tag.add` | 3 | 0 |
| `metadata.records.tag.remove` | 2 | 0 |
| `metadata.tags.add` | 7 | 0 |
| `metadata.tags.create` | 37 | 0 |
| `metadata.tags.list` | 3 | 0 |
| `metadata.tags.remove` | 3 | 0 |
| `transactions.list` | 225 | 0 |
| `wallets.create` | 301 | 0 |
| `wallets.delete` | 2 | 0 |
| `wallets.derive` | 4 | 0 |
| `wallets.get` | 10 | 0 |
| `wallets.import-csv` | 131 | 0 |
| `wallets.import-phoenix` | 1 | 0 |
| `wallets.list` | 4 | 0 |
| `wallets.update` | 9 | 0 |
| `imports.list` | 4 | 0 |
| `imports.rollback` | 4 | 0 |
| `secrets.change-passphrase` | 7 | 0 |
| `secrets.forget-unlock` | 12 | 0 |
| `secrets.init` | 15 | 0 |
| `secrets.init-resume` | 4 | 0 |
| `secrets.migrate-credentials` | 14 | 0 |
| `secrets.remember-unlock` | 9 | 0 |
| `secrets.status` | 64 | 0 |
| `secrets.verify` | 38 | 0 |
| `backup.export` | 4 | 0 |
| `backup.import` | 36 | 0 |
| `reports.austrian-e1kv` | 36 | 0 |
| `reports.austrian-tax-summary` | 24 | 0 |
| `reports.balance-history` | 65 | 0 |
| `reports.balance-sheet` | 29 | 0 |
| `reports.capital-gains` | 42 | 0 |
| `reports.filed-snapshots.list` | 13 | 0 |
| `reports.journal-entries` | 38 | 0 |
| `reports.portfolio-summary` | 34 | 0 |
| `reports.summary` | 43 | 0 |
| `reports.tax-summary` | 29 | 0 |

| Daemon kind | Rust requests | Python kind fallbacks |
| --- | ---: | ---: |
| `daemon.lock` | 2 | 0 |
| `daemon.shutdown` | 159 | 0 |
| `daemon.unlock` | 13 | 0 |
| `status` | 1 | 0 |
| `backends.reveal_token` | 12 | 0 |
| `ui.backends.create` | 4 | 0 |
| `ui.backends.delete` | 1 | 0 |
| `ui.backends.options` | 1 | 0 |
| `ui.backends.public_defaults` | 1 | 0 |
| `ui.backends.set_default` | 1 | 0 |
| `ui.backends.settings.list` | 1 | 0 |
| `ui.backends.update` | 1 | 0 |
| `ui.network.offline` | 2 | 0 |
| `ui.network.offline.set` | 6 | 0 |
| `ui.networks.bind` | 1 | 0 |
| `ui.networks.binding` | 1 | 0 |
| `ui.networks.inventory` | 1 | 0 |
| `ui.networks.partition_export` | 1 | 0 |
| `ui.networks.partition_plan` | 1 | 0 |
| `ui.networks.plan` | 1 | 0 |
| `ui.profiles.create` | 1 | 0 |
| `ui.profiles.rename` | 1 | 0 |
| `ui.profiles.reset_data` | 7 | 0 |
| `ui.profiles.snapshot` | 4 | 0 |
| `ui.profiles.switch` | 2 | 0 |
| `ui.profiles.update` | 3 | 0 |
| `ui.saved_views.create` | 2 | 0 |
| `ui.saved_views.delete` | 2 | 0 |
| `ui.saved_views.list` | 3 | 0 |
| `ui.workspace.create` | 1 | 0 |
| `ui.workspace.delete` | 4 | 0 |
| `ui.workspace.rename` | 3 | 0 |
| `ui.rates.coverage` | 5 | 0 |
| `ui.rates.kraken_csv.import` | 4 | 0 |
| `ui.rates.latest` | 3 | 0 |
| `ui.rates.summary` | 2 | 0 |
| `ui.activity.history` | 17 | 0 |
| `ui.activity.stale` | 3 | 0 |
| `ui.attachments.add` | 1 | 0 |
| `ui.attachments.copy` | 1 | 0 |
| `ui.attachments.list` | 5 | 0 |
| `ui.attachments.open` | 1 | 0 |
| `ui.attachments.remove` | 1 | 0 |
| `ui.attachments.rename` | 1 | 0 |
| `ui.btcpay.provenance.links` | 2 | 0 |
| `ui.btcpay.provenance.list` | 2 | 0 |
| `ui.btcpay.provenance.suggest` | 1 | 0 |
| `ui.documents.attach` | 8 | 0 |
| `ui.documents.create` | 13 | 0 |
| `ui.documents.import_report` | 5 | 0 |
| `ui.documents.list` | 8 | 0 |
| `ui.loans.link` | 1 | 0 |
| `ui.loans.list` | 1 | 0 |
| `ui.loans.mark` | 2 | 0 |
| `ui.loans.unmark` | 1 | 0 |
| `ui.metadata.bip329.export` | 4 | 0 |
| `ui.metadata.bip329.import` | 2 | 0 |
| `ui.metadata.bip329.preview` | 3 | 0 |
| `ui.transactions.commercial_context` | 6 | 0 |
| `ui.transactions.extremes` | 13 | 0 |
| `ui.transactions.history` | 17 | 0 |
| `ui.transactions.history.revert` | 6 | 0 |
| `ui.transactions.list` | 158 | 0 |
| `ui.transactions.metadata.update` | 109 | 0 |
| `ui.transactions.search` | 70 | 0 |
| `ui.wallets.create` | 2 | 0 |
| `ui.wallets.delete` | 6 | 0 |
| `ui.wallets.list` | 2 | 0 |
| `ui.wallets.preview_descriptor` | 171 | 0 |
| `ui.wallets.update` | 4 | 0 |
| `ui.imports.list` | 1 | 0 |
| `ui.imports.rollback` | 1 | 0 |
| `ui.backends.bitcoinrpc.test` | 4 | 0 |
| `ui.backends.btcpay.test` | 3 | 0 |
| `ui.backends.detect_core` | 3 | 0 |
| `ui.backends.electrum.test` | 3 | 0 |
| `ui.backends.http.test` | 4 | 0 |
| `ui.secrets.change_passphrase` | 5 | 0 |
| `ui.secrets.forget_cli_unlock` | 2 | 0 |
| `ui.backup.apply` | 14 | 0 |
| `ui.backup.cancel` | 4 | 0 |
| `ui.backup.export` | 13 | 0 |
| `ui.backup.preview` | 20 | 0 |
| `ui.reports.balance_history` | 150 | 0 |
| `ui.reports.balance_sheet` | 4 | 0 |
| `ui.reports.portfolio_summary` | 4 | 0 |
| `ui.reports.summary` | 32 | 0 |
| `ui.reports.tax_summary` | 36 | 0 |

### Corrected attempts, platform limits and handoff

The first required Python run after slice integration found two integration
bookkeeping failures: setup's old probe refusal examples and the missing
manual top-30 census sample for the enlarged shell module. They were corrected,
focused checks passed, and the complete required suite above was rerun.
The first final native/vector/mutation attempt exhausted temporary-storage
quota: SQLite reported `SQLITE_IOERR_WRITE`, and mutation workers reported
`Disk quota exceeded`. Those outcomes were not counted as kills or passes.
Only this integration's disposable mutation targets were removed; scratch trees
and caches were relocated, concurrency was bounded, and every affected check
was rerun. No product behavior was changed to accommodate those failures.

The wallet suite has **one known parity failure**: `test_diagnostics_out_auto_writes_public_error_report` expects a diagnostics file from `rates range --diagnostics-out`. The native dispatcher deliberately returns `not_implemented` for this global flag; `rust-shell.md` already records that boundary. The run is 414 passed, 54 subtests passed, and 1 failed. Diagnostics was not implemented, routed through an unreported fallback, or marked as a sandbox skip. Every other final slice suite passed.

The chain suite's first attempt lacked `examples/client_fixture` beside the copied binary. Copying the already-built fixture corrected that test setup, and all 30 tests then passed. The first catalog-derived Python witness selection included commands with required argument groups; it was narrowed to the existing deterministic argument-free database-read subset. Both focused witnesses and the full transaction suite then passed.

All Linux suites ran without skips, including the disposable Secret Service enrollment, rotation, revocation and interruption checks. This Linux sandbox cannot establish native macOS Keychain, Windows Credential Manager or Windows path/executable acceptance. Live providers were not contacted; loopback chain fixtures and the no-egress observer checks passed.

The orchestrator still owns the full census and custody-shadow shards. This
report does not claim those runs. Nothing remains stopped on a conflict.
The branch is local-only; no push or PR was made.

Evidence is in `/tmp/kassiber-r5b-verification/`: `inputs.json`, `replay.json`,
`resolutions.json`, `final-audit.json`, `origin-locks.json`, `refs-final.json`,
`verified-native-results.json` and logs, `final-required-python.log`,
`verified-*-result.json` and partition mutation logs, `final-hybrid-*-result.json`
and test logs, `served-by-final.json`, `final-hybrid-*-served.jsonl`,
`final-hybrid-*-daemon-kinds.jsonl`, `coverage-log-deduplication.json`,
`final-mutation-entry-audit.json`, `backup-quiet-retarget.log`, and
`final-generators-results.json`.
