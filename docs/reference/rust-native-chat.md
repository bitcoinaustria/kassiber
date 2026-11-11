# Rust native chat: writes through chat, and no feature gates

> **Historical.** This page records the Python-to-Rust port. Python was removed
> under [plan 22](../plan/22-python-removal.md); the `kassiber/` paths and Python
> tests it cites exist only at the `archive/python-backend` tag.

Branch `t3code/rust-native-chat` (from the integration tip d12d6e29e) makes
the assistant's chat and the MCP server fully native in the product binary.
Python stays the reference ([plan 21](../plan/21-rust-backend.md)); the
contracts are Python's [tool use](ai.md#tool-use),
[agent access](ai.md#agent-access), [permission modes](ai.md#permission-modes)
and [MCP](mcp.md).

Until now chat (`ai.chat`, its controls and history, `kassiber chat`, the
`chats` leaves) and the tool-running MCP leaves (`mcp serve`, `mcp call`) sat
behind the `native-chat` and `native-mcp` cargo features, which only
test-harness builds enabled. Inside a chat turn every consented write went to
an R6-5b stub (`executor::execute_mutating`) and was refused
`not_implemented`, so chat could read but not change anything.

## What changed

**Writes in a chat turn** (`crates/ai/src/chat/turn.rs`, Python's
`_run_ai_chat_tool_loop`):

- After the user's decision (or a permission-mode or "allow for this chat"
  preapproval) the turn binds the consent to the call id and runs the native
  mutating executor (`mutate::execute_mutating`) with the custody consent
  audit Python builds (`CustodyAiConsentAudit`: provider kind or `unknown`,
  model, decision, the proposal and decision times). Successful reversible
  writes carry their `undo` hint, as before through `test.ai_tool`.
- Denied, timed-out and cancelled custody proposals are audited in the book
  (`record_nonexecuted_custody_call`), including a cancel between the call
  record and its card (`cancelled_before_consent`); other tools record
  nothing, as in Python.
- The consent card is the book's, not the model's: the acquisition and
  dataset previews replace the redacted arguments, plan-bound applies
  (`ui.wallets.import_attachment.apply`, `ui.wallets.import_document.apply`,
  `ui.imports.rollback`) show the recomputed plan and its summary, and
  `ui.review.apply` carries `review_preview`. Before, the card showed plain
  redacted arguments and the CLI denied `ui.review.apply` for lack of a
  preview.
- The executor stub and its owner constant are gone.

**The chat worker's book** (`crates/daemon/src/ai_chat.rs`). The worker
reaches the book through calls the request loop serves. The proxy forwarded
only the read calls, so writes, maintenance runs, custody writes and audits,
review and dataset previews, report blockers, the profile's fiat currency and
the egress ledger fell to the trait's `not_implemented` defaults. Each call
is now an owned closure sent to the request loop with its reply channel, so
every executor call crosses; a locked book still answers
`passphrase_required`.

**The gates.** `native-chat` (daemon, CLI) and `native-mcp` (CLI) are
removed: product builds register the nine chat kinds, `chat`, the five
`chats` leaves, `mcp serve` and `mcp call` (now in `describe::IMPLEMENTED`;
`feature_registered` is gone). `crates/net/tests/reviewed-dependencies.txt`
drops the two features. The desktop and the sidecar never depended on them:
the packaged binary simply serves the kinds now.

**Tools that were still refused.** The executor's `REFUSED_READS` and
`mutate::REFUSED_MUTATIONS` are removed; every tool reaches a native service:

| Tool | Python | Rust |
| --- | --- | --- |
| `ui.journals.quarantine.groups` | `build_journals_quarantine_groups_snapshot` | `quarantine::groups::snapshot` (argument checks in Python's order), served through `journal_view` for the assistant only |
| `ui.transactions.verify` | `_transaction_verify_payload_for_ai` | `ai_tools::transaction_verify_for_ai` over the native verifier |
| `ui.transfers.dismissals.list`, `.delete` | `_ui_transfers_payload` | the transfers route |
| `ui.btcpay.provenance.review` | `core_commercial.review_link` | the commercial route, evidence-redacted |
| `ui.connections.node.snapshot` | `_lightning_node_snapshot_payload_for_ai` | the desktop snapshot projected by `projections::node_snapshot_for_ai` |

The node snapshot projection keeps fields by name (`snapshot_to_dict_for_ai`:
no operator pubkey, no channel, peer, forward or connection identifier), so
the AI view stays closed if the full projection grows a field.

**Test clock seam.** The store's transfers route now takes the test-harness
sources (journal clock and ids) as its review route already did, matching
the Python reference's bound `transfer_matching` clock. A dismissal's expiry
is counted from the wall clock (`SystemSources`), as Python's
`dismiss_pair_candidate` counts it from `datetime.now()` while stamping
`created_at` through `now_iso()`. Product builds read the wall clock for
both, so nothing changes outside harnesses.

That wall-clock read (`sql::wall_clock`) has its own seam:
`Sources::pinned_wall_clock`, compiled only with `kassiber-transfers`'
`test-harness` feature and unpinned by default, as the CLN client's
`pin_clock` is. `crates/transfers/tests/authoring.rs` pins it to each
case's instant, because `export_authoring.py` froze Python's `datetime`
there too; without the pin its recorded expiries (`created_at` + 90 days)
could never match. `a_dismissal_expiry_counts_from_the_system_clock` pins
the product path: with `now_iso()` frozen and nothing pinned, the expiry is
the system clock plus the days. Mutation entries `dismiss-expiry-system-clock`
and `dismiss-expiry-harness-pin` (`crates/transfers/scripts/mutations.json`)
are killed by those two tests. The kassiber harness's sources pin nothing,
so the AI and transfer parity drivers keep Python's wall-clock expiry.

## Per tool

Every mutating tool and every egressing read, as a chat turn runs it. All
need consent; "once-only" tools never take "allow for this chat", reversible
ones are approved by the `auto` mode when they stay on this machine, and
`full_access` approves everything. Writes and egressing reads run only with
the consent bound to their call id.

| Tool | Consent | Undo hint names | Chat route (Rust) |
| --- | --- | --- | --- |
| `ui.wallets.import_attachment.apply` | once-only, reversible, plan card | ui.imports.rollback.plan | plan-bound import (`chat_imports`) |
| `ui.imports.rollback` | once-only, plan card | — | plan-bound rollback (`import_runs`) |
| `ui.reports.lightning_profitability` | once or for the chat, egresses | — | read executor, consent bound to the call |
| `ui.connections.node.snapshot` | once or for the chat, egresses | — | read executor, consent bound to the call |
| `ui.journals.quarantine.resolve` | once-only, reversible | ui.transactions.history.revert | metadata route, authored `ai_tool` |
| `ui.rates.rebuild` | once or for the chat, egresses | — | desktop route |
| `ui.wallets.sync` | once or for the chat, egresses | — | strict sync, native freshness plan, no pairing |
| `ui.journals.process` | once or for the chat, reversible | rerun; derived state only | journal refresh without pairing |
| `ui.maintenance.configure` | once-only | — | desktop route |
| `ui.maintenance.run` | once or for the chat, egresses | — | maintenance run (skips a refresh the turn ran) |
| `ui.source_funds.sources.create` | once or for the chat | — | source-of-funds route, redacted |
| `ui.source_funds.links.create` | once or for the chat | — | source-of-funds route, redacted |
| `ui.source_funds.links.review` | once or for the chat | — | source-of-funds route, redacted |
| `ui.source_funds.suggest` | once or for the chat | — | source-of-funds route, redacted |
| `ui.source_funds.links.bulk_review` | once or for the chat | — | source-of-funds route, redacted |
| `ui.custody.review.apply` | once-only, reversible | ui.custody.review.plan reopen | custody write in `SAVEPOINT custody_ai_assisted_write` + audit row |
| `ui.transfers.components.apply` | once-only, reversible | ui.transfers.components.plan supersede/undo | transfers route (`ai_tool`) |
| `ui.transfers.payouts.create` | once or for the chat, reversible | ui.transfers.payouts.delete | transfers route (`ai_tool`) |
| `ui.transfers.payouts.delete` | once or for the chat | — | transfers route (`ai_tool`) |
| `ui.transfers.update` | once or for the chat | — | transfers route (`ai_tool`) |
| `ui.transfers.pair` | once or for the chat, reversible | ui.transfers.unpair | transfers route (`ai_tool`) |
| `ui.transfers.unpair` | once or for the chat, reversible | ui.transfers.pair with the removed pair's legs | transfers route (`ai_tool`) |
| `ui.transfers.bulk_pair` | once or for the chat | — | transfers route (`ai_tool`) |
| `ui.transfers.dismiss` | once or for the chat, reversible | ui.transfers.dismissals.delete | transfers route (`ai_tool`) |
| `ui.transfers.dismissals.delete` | once or for the chat, reversible | ui.transfers.dismiss with the same legs | transfers route (`ai_tool`) |
| `ui.transfers.rules.create` | once or for the chat, reversible | ui.transfers.rules.delete | transfers route (`ai_tool`) |
| `ui.transfers.rules.delete` | once or for the chat, reversible | ui.transfers.rules.create with the removed rule | transfers route (`ai_tool`) |
| `ui.transfers.rules.set_enabled` | once or for the chat, reversible | ui.transfers.rules.set_enabled | transfers route (`ai_tool`) |
| `ui.transfers.rules.apply` | once or for the chat | — | transfers route (`ai_tool`) |
| `ui.saved_views.create` | once or for the chat, reversible | ui.saved_views.delete | transfers route (`ai_tool`) |
| `ui.saved_views.delete` | once or for the chat | — | transfers route (`ai_tool`) |
| `ui.loans.mark` | once or for the chat, reversible | ui.loans.unmark | desktop route |
| `ui.loans.link` | once or for the chat | — | desktop route |
| `ui.loans.unmark` | once or for the chat | — | desktop route |
| `ui.transactions.metadata.update` | once or for the chat, reversible | ui.transactions.history.revert | metadata route, authored `ai_tool` |
| `ui.transactions.history.revert` | once or for the chat, reversible | ui.transactions.history.revert | metadata route, authored `ai_tool` |
| `ui.attachments.copy` | once or for the chat | — | commercial/evidence route, redacted |
| `ui.attachments.add_from_chat` | once or for the chat | — | commercial/evidence route, redacted |
| `ui.wallets.read_document` | once or for the chat, egresses | — | read executor, consent bound to the call |
| `ui.wallets.import_document.apply` | once-only, plan card | — | plan-bound import (`chat_imports`) |
| `ui.source_funds.sources.attach` | once or for the chat | — | source-of-funds route, redacted |
| `ui.source_funds.links.attach` | once or for the chat | — | source-of-funds route, redacted |
| `ui.source_funds.assemble` | once or for the chat | — | source-of-funds route, redacted |
| `ui.source_funds.cases.save` | once or for the chat | — | source-of-funds route, redacted |
| `ui.source_funds.export` | once or for the chat | — | `ui.source_funds.export_pdf`/`_bundle` |
| `ui.btcpay.provenance.suggest` | once or for the chat | — | commercial/evidence route, redacted |
| `ui.btcpay.provenance.review` | once or for the chat | — | commercial/evidence route, redacted |
| `ui.documents.create` | once or for the chat | — | commercial/evidence route, redacted |
| `ui.rates.latest` | once or for the chat, egresses | — | desktop route |
| `ui.reports.export` | once or for the chat | — | export target → `ui.reports.export_*` (saved locally) |
| `ui.review.apply` | once-only, review preview card | — | review workflow (`review::execute_as`, `ai_tool`) |
| `ui.chain_analysis.datasets.import` | once-only, validated dataset card | — | chain-analysis ops (`chain_analysis_for_ai`) |
| `ui.chain_analysis.datasets.revoke` | once or for the chat | — | chain-analysis ops (`chain_analysis_for_ai`) |
| `ui.chain_analysis.cases.save` | once or for the chat, reversible | ui.chain_analysis.cases.delete | chain-analysis ops (`chain_analysis_for_ai`) |
| `ui.chain_analysis.cases.delete` | once or for the chat | — | chain-analysis ops (`chain_analysis_for_ai`) |
| `ui.chain_analysis.labels.upsert` | once or for the chat | — | chain-analysis ops (`chain_analysis_for_ai`) |
| `ui.chain_analysis.labels.delete` | once or for the chat | — | chain-analysis ops (`chain_analysis_for_ai`) |
| `ui.chain_analysis.acquire.apply` | once-only, egresses, recomputed plan card | — | chain-analysis ops (`chain_analysis_for_ai`) |
| `ui.chain_analysis.watches.create` | once or for the chat, reversible | ui.chain_analysis.watches.delete | chain-analysis ops (`chain_analysis_for_ai`) |
| `ui.chain_analysis.watches.configure` | once or for the chat | — | chain-analysis ops (`chain_analysis_for_ai`) |
| `ui.chain_analysis.watches.delete` | once or for the chat | — | chain-analysis ops (`chain_analysis_for_ai`) |
| `ui.chain_analysis.watches.acknowledge` | once or for the chat | — | chain-analysis ops (`chain_analysis_for_ai`) |
| `ui.chain_analysis.watches.evaluate` | once or for the chat | — | chain-analysis ops (`chain_analysis_for_ai`) |
| `ui.chain_analysis.datasets.discard` | once or for the chat | — | chain-analysis ops (`chain_analysis_for_ai`) |
| `ui.chain_analysis.datasets.import.start` | once-only, validated dataset card | — | chain-analysis ops (`chain_analysis_for_ai`) |
| `ui.chain_analysis.datasets.discard.start` | once or for the chat | — | chain-analysis ops (`chain_analysis_for_ai`) |
| `ui.chain_analysis.sources.authorize` | once-only, recomputed plan card | — | chain-analysis ops (`chain_analysis_for_ai`) |
| `ui.chain_analysis.sources.revoke` | once or for the chat | — | chain-analysis ops (`chain_analysis_for_ai`) |
| `ui.chain_analysis.sources.run` | once-only, egresses | — | chain-analysis ops (`chain_analysis_for_ai`) |

## Evidence

- `crates/ai/tests/chat.rs` (28 tests): an allowed write runs the mutating
  executor through the desktop route authored `ai_tool`; a denied write runs
  nothing and audits nothing; `full_access` writes without a card; an
  allowed custody write carries its consent audit; denied and timed-out
  custody proposals, and ones cancelled while the card waits or before it,
  are audited; the review, rollback and acquisition cards show the book's
  previews.
- `crates/ai/tests/fences.rs`: the node snapshot reaches its route with a
  bound consent and comes back without pubkeys, peers or ids; the BTCPay
  review reaches its route.
- `tests/test_rust_ai_chat.py::test_writes_ask_apply_and_undo_through_the_executor`:
  one session per runtime on the desktop fixture book against the scripted
  provider. Turn one asks for a note edit and a saved view, applies them,
  and the provider's next leg (computed from each side's own tool output)
  calls both undo hints: the revert is applied, the view's deletion denied.
  Turn two runs in `auto`: a reversible edit runs without a card, a
  plan-bound rollback shows the book's (unavailable) plan and is denied, a
  non-reversible deletion asks. Turn three runs in `full_access`: the
  journal rebuild and a once-only settings change run without cards. Every
  record, every provider request and every table of the book match Python
  (generated ids mapped, fresh clocks one token). All 12 chat scenarios
  pass.
- `tests/test_rust_ai_tools.py::test_formerly_refused_tools_match_python`:
  every read case of the four formerly refused reads and every write case of
  the dismissal and BTCPay review tools on the three fixture books, records
  and all tables against Python. AI wallet sync is compared on every fixture
  now (its direct-sync exclusion was stale). The whole module fails the same
  12 tests on this branch as on its base d12d6e29e, for differences recorded
  before (the capital-gains projection's `rows`, journal events'
  `explanationReference`, the profile counters after the default refresh),
  while comparing the formerly refused tools too.
- `tests/test_rust_lightning_service.py::test_assistant_node_snapshot_matches_python_without_identifiers`:
  `ui.connections.node.snapshot` through the AI executor against the fake
  LND and Core Lightning nodes: same bytes and node requests as Python, no
  identifiers. A call without a bound consent reaches no node in Rust
  (Python's executor leaves that to the chat loop, an R6-5b reviewed
  difference).
- `tests/test_rust_mcp.py`: the transcripts at every grant and access mode
  now compare the formerly refused tools instead of skipping them. With
  Python round 13 merged (198c1dfae, below) every grant and access mode
  matches; only the reports book's `journals_events_list` differs (2 of 284
  replies), as on the base.
- The Privacy Mirror over MCP. Before round 13 the *Python* reference
  answered `reports_privacy_mirror`, `chain_analysis_query` and
  `chain_analysis_entropy` with `tool_error` from the operator grant up: its
  `begin_write_savepoint` reserved the writer with a no-op `UPDATE`, which
  the MCP read's `PRAGMA query_only` refuses, and the analysis index read
  opens that savepoint. Rust's `write_lock::reservation` already skips a
  connection that cannot write, so Rust answered with the mirror all along.
  Round 13 gives Python the same skip; merged here, both runtimes answer
  identically. `wire-mcp-analysis-reads-skip-reservation` (the AI mutation
  catalog) drops Rust's `query_only` skip and is killed by
  `test_every_grant_lists_and_runs_what_it_covers`.
- `tests/test_cli_chat.py` through the hybrid candidate with a Rust daemon:
  43 of 43 pass, all 232 calls served by Rust (R6-7 recorded 39 of 41).
  The AI tests of `tests/test_daemon_smoke.py` with a Rust daemon: 87 of 87,
  every daemon served by Rust.
- A product build (`cargo build -p kassiber`, no features) registers
  `ai.chat`, answers the chat kinds and the `chats` leaves, serves
  `mcp serve`, and has no test kinds; `tests/test_packaged_cli_no_egress.py`
  passes against it (6 of 6).
- `crates/ai/scripts/mutations.json`: 18 new entries (chat consent binding,
  the executor call, each custody audit path, each consent card, the node
  snapshot projection, the worker's write forwarding, the two new reads, the
  dismissal clock and the transfers test seam), each killed by a named test;
  the stub's refusal entry is gone.

## Reviewed differences

- **Consent reply and denied result order.** A decision's control reply
  (`ai.tool_call.consent`) and the denied call's `ai.chat.tool_result` reach
  the output in either order: the request loop answers the decision while
  the chat worker it released writes the result. Python's GIL makes the
  reply first nearly always; the Rust loop and worker run in parallel. The
  desktop and the stream driver group records by request id, so
  `test_ai_chat_mutating_tool_consent_preview_redacts_secrets` now reads
  both in any order and still checks each: the card's redacted descriptor
  and configuration, the recorded decision, the denied result for its call,
  and no secret in any record after it (10 of 10 runs on each runtime).
- Otherwise none for the chat records, requests or stored rows the
  scenarios cover. A metadata history read straight after several edits in
  one second orders them by their random ids in both runtimes; the scenario
  compares the stored rows instead.

## What remains

- `test_only_the_granted_tools_are_reachable` (`tests/test_rust_mcp.py`)
  can see the server's 32-calls-in-flight refusal (`-32000`) instead of
  the expected `-32602` when the machine is loaded; it passes alone. Its
  burst can exceed the limit, which Python enforces the same way.
- Failing on the base and untouched here: the three tool-harness
  differences above, `kassiber-journals`' `persistence` (15) and `crash` (1)
  vector tests.
- AI `ui.wallets.sync` still refuses a selection the native freshness plan
  cannot take (`not_implemented`); no fixture book reaches it any more.
  File-sourced wallets' direct sync stays with the wallet-sync gaps work.
- Windows: the chat and MCP Windows arms are still not compiled (R6-7, R6-8).
