# Rust main sync S12: the R6 crates on main through #714

The R6 line (`kassiber-ipc`, `-operator`, `-ai`, `-mcp` and the daemon's AI
kinds; integration tip c48358377) was written against main #654. Main has
since changed the assistant, its providers, chat and MCP in Python and the
desktop. S12 brings those changes into the Rust crates; item 12 of
[main sync 1](rust-main-sync.md#slices) lists the work, and its
[AI catalog section](rust-main-sync.md#ai-catalog-and-tools-r6-5b-r6-8)
records the catalog delta. Python stays the reference
([plan 21](../plan/21-rust-backend.md)).

The branch `t3code/rust-sync-s12` starts with a pure merge of 610b447c3
(main through #714 plus the S1 and S1b store and registry work) into
c48358377. The two conflicts were the generated test inventory and its
table; both were regenerated from the merged tree. Every R6 crate's tests
pass on the merge (`kassiber-ai` 186, `-ipc` 14, `-mcp` 35, `-operator`
240). The operator backoff tests
`three_failures_start_a_persisted_backoff` and
`test_wrong_passphrases_feed_the_shared_backoff` fail when their temporary
directory is on the 860 EVO under load: a persisted failure takes seconds
to sync, so the five-second lock reads back as one second. They pass when
rerun at lower load; the R6 crates did not change in the merge.

Where S12 ends: everything in the table below is ported except the
attachment and document import tools (#682, #698), which need S8's wallet
completeness, and the document reader, which needs its model call moved
off the request loop. [Main sync's after S12](rust-main-sync.md#after-s12)
summarises it.

## Inventory by pull request

"Rust R6" names where the behaviour lives in the Rust crates. "Before"
is the state on the merge commit, measured by the
[differential run](#differential-run-on-the-merge) unless it says
inspection. "Now" is the state on this branch.

| PR | Python change | Kinds, tools and fields | Rust R6 | Before | Now |
| --- | --- | --- | --- | --- | --- |
| #675 | `ai/provider_checks.py`, `providers.list_with_default`, `discovery_cache.seed`: each provider's last explicit check is stored in `ai_provider_checks` (per base URL, models capped at 1000) and read back; `last_checked_at` and `set_up` on every listed provider; a key or address change clears the checks | `ai.providers.list`, `ai.provider_runtime.status`, `ai.list_models`, `ai.test_connection`, `ai.providers.update/set_api_key` | `crates/ai/src/providers`, `crates/ai/src/service.rs`, `crates/ai/src/discovery` | Differs: rows lack `last_checked_at` and `set_up`; checks are neither stored nor seeded | see [state](#state-on-this-branch) |
| #676 | `core/chat_visuals.py` (997 lines): charts rendered from local report and transaction reads | tool `ui.visuals.render` | none | Missing from the catalog | Ported; see [visuals](#chat-visuals-676) |
| #677 | `ai/tools.py`, `ai/prompt.py`: the tool-area index in the system prompt, `load_tools`, `TOOL_AREA_SUMMARIES`; "what can you do" no longer offers every schema; `/review` selects the review pack; report keywords for gains, income and charts. `tax_estimate.py` core tax figures (S7) | `ai.chat` system prompt and advertised tools; tool `load_tools` | `crates/ai/src/select.rs`, `prompt.rs`, `executor.rs`, `chat/turn.rs` | Differs: selection and prompt are pre-#677 | Ported; see [catalog](#catalog-selection-and-summaries) |
| #678 | `providers.agent_access` (`full` default, `restricted`), `_provider_has_full_agent_access`, `_ai_full_access`: a restricted off-device provider is kept to projected results and away from custody-gap, lineage, chain-analysis source and attachment-value tools; an on-device provider always has full access | provider rows and `create/update --agent-access`; `agent_access_restricted` denials replace `local_provider_required` | `crates/ai/src/providers`, `executor.rs`, `mutate.rs`, `projections.rs`, `previews.rs`, `file_analysis.rs`, `chat/screen.rs`, `select.rs` | Differs: Rust gates on `provider_kind == local` and has no `agent_access` | see [state](#state-on-this-branch) |
| #679 | `custody_components` revise/supersede/undo/activate actions; `ui.transfers.dismissals.list`; qualified txids (S2) | tools `ui.transfers.components.plan/apply` (`action`, `component_id`, `activate`), `ui.transfers.dismissals.list` | `crates/ai/src/summary.rs`, `executor.rs`, `crates/transfers` | Differs: summaries, schemas | Summaries ported; dismissals list refused (owned by S6); component actions follow S11 |
| #681 | quarantine root-cause groups (S5), `transaction_verify.py` (S8), wallet completeness | tools `ui.journals.quarantine.groups`, `ui.transactions.verify`; refresh set gains the groups tool | `executor.rs`, `projections.rs` | Missing; AI reads also lacked the stale-journal projection | Catalog entries, refresh set; the projection is ported (see [MCP grants](#mcp-grants-686)); the tools refused until S5 and S8 |
| #682 | `core/attachment_import.py`, `import_batches.py`: an attached file imported as a consented, plan-bound import with rollback | tools `ui.wallets.import_attachment.plan/apply`, `ui.imports.list`, `ui.imports.rollback(.plan)`, `ui.wallets.request_input`, `ui.attachments.add_from_chat` | none | Missing | Catalog entries, consent and summaries; the import runs, their rollback and the wallet handoff ported (see [import runs](#import-runs-and-the-wallet-handoff-682)); `import_attachment.plan/apply` refused until ported |
| #683 | system prompt evidence rules; journals stale on dismiss (S6) | `ai.chat` system prompt | `prompt.rs` (embedded) | Differs | Prompt regenerated |
| #684 | `ai/undo.py`, `REVERSIBLE_TOOLS`, `AI_PERMISSION_MODES`, `AiToolConsentState.preapproval/set_permission_mode`; custody AI audit decisions `mode_auto`, `mode_full_access` | `ai.chat` `permission_mode`, kind `ai.chat.permission_mode`, `ai.chat.tool_call` `approval`, consent card `reversible`, `egresses`, `can_allow_session`, result `undo` | `crates/ai/src/consent.rs`, `chat/turn.rs`, `mutate.rs`, `crates/daemon/src/ai_chat.rs` | Missing: kind refused, no `reversible` | see [state](#state-on-this-branch) |
| #686 | `agent_access.py` grants (`read`, `operator`, `accounting_decisions`) and access mode; `mcp/tools.py` registry over the broker's exact-path classification; writes through the assistant executor, audited `mcp_host` | `mcp serve` tool list and calls, `mcp enable --grant --agent-access`, `mcp call --grant --client`, `ui.agent_access.configure` | `crates/mcp`, `crates/daemon/src/agent_access.rs`, `crates/setup/src/agent_access.rs` | Differs: Rust lists the 44 read tools and refuses writes | Ported; see [MCP grants](#mcp-grants-686) |
| #689 | `daemon_chat_uploads.py`: pasted files in chunks to an owner-only temporary file, staged like a picked file | kind `ui.chat.attachments.upload`; `internal.document_import.stage` `purpose` and the staged `kind` | none | Refused (`not_implemented`) | Ported; see [chat files](#chat-files-staging-and-uploads-689-698) |
| #690 | hints say "Settings → Assistant" instead of "Settings → AI providers" | provider and transport errors | `crates/ai/src/transport/errors.rs`, `providers` | Differs (hint text) | see [state](#state-on-this-branch) |
| #693, #697 | `core/chat_files.py`: several files per turn; images and PDFs shown to a full-access provider; expired files dropped | `ai.chat` `attachments`, its terminal `expired_attachments`, chat history | `chat/args.rs`, `chat/screen.rs`, `broker` | Differs: one attachment only | Ported; see [chat files](#chat-files-in-the-turn-693-697) |
| #698 | `core/document_chat_import.py`, `document_import.py` read step without a database and its detected API format | tools `ui.wallets.read_document`, `ui.wallets.import_document.plan/apply` | `crates/imports` (document core) | Missing | Staging's chat purpose, the sixteen grants and the preview's provider-first order ported; the three tools refused: the plans need S8's wallet completeness, the reader its model call off the request loop |
| #700, #709 | desktop only: one composer permission control; the model picker folds reasoning, speed and permission mode | `ai.chat.permission_mode` (from #684) | none | — | No Rust change |
| #714 | provider broker (TypeScript): OpenCode 2, Codex fast mode; desktop saved model inventories | none in Python | none | — | No Rust change: the broker script is shared |

## Differential run on the merge

The four R6 parity harnesses ran the merged Python against a test-harness
build of the merge commit (`KASSIBER_RUST_BIN`):
`tests/test_rust_ai_tools.py`, `test_rust_ai_providers.py`,
`test_rust_ai_chat.py` and `test_rust_mcp.py`. 242 tests and subtests
failed and 18 passed. Failing test functions per harness: tools 49 (all
three drift tests, read and mutating parity on every fixture, consent
previews, the review flow, the no-network fence), providers 8, chat 9,
MCP 13. One failure per harness, rerun for its full output, shows the cause:

- providers (`test_daemon_provider_crud_contacts_nothing`): every Python
  provider row carries `agent_access` (#678), `last_checked_at` and `set_up`
  (#675); Rust's rows lack all three.
- chat (`test_validation_errors_contact_nothing`): the Rust session timed
  out before its first line; Python answers every case.
- MCP (`test_only_the_44_read_tools_are_reachable`): the harness itself
  reads `kassiber.mcp.tools.mcp_entries`, which #686 replaced with the
  grant registry, so the MCP harness has to follow main before it can
  compare.
- tools: the drift tests fail on the regenerated catalog and vectors
  ([below](#catalog-selection-and-summaries)); the executor tests fail on
  the 15 new tools and the changed schemas.

Two `kassiber-daemon` tests fail on the merge and equally on c48358377, so
they predate S12: `malformed_requests_match_python` (it still expects
`ui.agent_access.status` to be `not_implemented`, which the daemon serves
since R6-8) and `agent_access::tests::test_ending_the_desktop_session_ends_the_agent_lease`
(the fake broker's last call is `unlock`, not `lock`). Both pass since the
MCP grants step: the first now uses a kind the daemon still lacks, and the
second found a real gap (`daemon.lock` did not end the agents' lease).

## Catalog, selection and summaries

`crates/ai/scripts/gen_catalog.py` now records each tool's `reversible`
flag, the `load_tools` entry, the tool-area index and the Python prompt
with it, and the new sets (`WALLET_IMPORT_TOOL_NAMES`,
`CHAT_DOCUMENT_TOOL_NAMES`, `TOOL_AREA_SUMMARIES`, `REVERSIBLE_TOOLS` with
each inverse, `AI_PERMISSION_MODES`). The catalog grows from 161 to 176
tools; nine egress (`ui.rates.rebuild`, `ui.rates.latest`,
`ui.wallets.sync`, `ui.maintenance.run` and `ui.wallets.read_document`
are new).

The Rust side follows:

- `select.rs`: the larger core and review packs, the chat-document pack
  (`chat_documents`, offered only by the chat loop), wallet-import tools
  outside `operations`, `ui.visuals.render` under reports and
  transactions, the `/review` route, the new report keywords, and no
  full-catalog answer to "what can you do". `build_area_tools` serves
  `load_tools`.
- `prompt.rs`: a tool-enabled turn sends the prompt with the area index;
  a turn without tools sends the plain prompt.
- `executor.rs`: `load_tools` authorizes the requested areas' schemas at
  once and the chat loop offers them from the next round; it is offered to
  HTTP providers outside the `full` profile, never to a broker agent.
- `consent.rs`: three more once-only tools (each import and rollback is its
  own plan-bound decision).
- `summary.rs`: the import, rollback, chat-evidence, quarantine-group and
  component-action consent texts.
- `planner.rs`: `_planned_chat_visuals`, the chart reads a question asks for
  by name (overview, readiness, a transaction list, and for a chart word the
  tax summary, portfolio, cash flow or balance history). The planned
  `ui.visuals.render` reads are refused until the visuals are native.
- `review.rs`: the AI review rules of #678 and #679. A restricted provider
  plans only `price_override`, `exclude`, `unpair` and component creation,
  and gets no receipt holding anything else (`agent_access_restricted`);
  with full access every type passes, including `kind_override`, which was
  refused outright before. A component revision or activation never makes
  a conversion effective: it must clear the inherited review, stay a draft,
  and look the component up in the book when the spec names no mode. The
  plan-size hint names `accept_coarse_prices`. The AI executor routes the
  review kinds through `Book::review`, which carries the turn's access mode
  (`executor::full_access`: `provider_full_access`, else on-device).
- New tools whose service is not native are refused with `not_implemented`
  and the owning work, before the book is touched: reads in
  `executor::REFUSED_READS`, writes in `mutate::REFUSED_MUTATIONS`. The
  Python harness lists them in `tests/test_rust_ai_tools.py`.

## State on this branch

### Providers (#675, #678, #690)

- `providers/mod.rs`: `agent_access` (`full` by default, `restricted`) on
  create, update, the stored row and the redacted output; an unknown stored
  value reads as `restricted`. `ai.providers.list` adds `last_checked_at`
  (the newest check at the current base URL) and `set_up` (a built-in row
  still at its built-in address and name counts only once it is the
  default, checked, or acknowledged off-device). An edit that changes the
  address or the key deletes the saved checks.
- `providers/checks.rs`: `ai_provider_checks` as Python writes it (JSON with
  Python's default separators, models capped at 1000, bound to the base
  URL, malformed rows read as nothing).
- `service.rs`: `ai.provider_runtime.status` for one provider and
  `ai.list_models` save every explicit check and seed the RAM cache from
  the saved one, so a failed probe reports it stale; their `cached_only`
  reads fall back to the saved check without contacting anything
  (`ai.list_models` gained `cached_only`). Non-preference updates and
  `set_api_key` clear the checks. `discovery::Cache::seed` holds a saved
  snapshot already expired.
- Hints say "Settings → Assistant" (#690).

`tests/test_rust_ai_providers.py` now compares `ai_provider_checks` with
the other provider tables, covers `agent_access` on create and update
(daemon and CLI) and the cached and saved model listings, and adds an
encrypted-book scenario; all 9 tests pass (all 8 failed on the merge).

### Agent access, permission modes and undo (#678, #684, #685)

- Full agent access: the chat turn carries `provider_full_access` (on this
  device, or a provider whose `agent_access` is `full`), and every gate that
  checked a local provider now checks `executor::full_access`: custody
  lineage reads and custody-gap writes, chain-analysis source and dataset
  tools and their consent previews, attachment cell values, and the turn's
  tool filter. A restricted provider is refused with
  `agent_access_restricted`, Python's messages and the
  `enable_full_agent_access` remediation.
- Failed tools answer as `_tool_result_from_app_error` does: the reason,
  message, the error's or the code's remediation (replan, reread a case,
  full agent access, ask again in the open book), the hint, and bounded
  details for the allowlisted codes.
- Permission modes: `ai.chat` takes `permission_mode` (`approval` by
  default, `auto`, `full_access`), and `ai.chat.permission_mode` switches
  a running chat, releasing the waiting calls the new mode approves
  (`released_call_ids`; `updated: false`, `reason: not_found` for an
  unknown chat). `auto` approves a reversible tool that stays on the
  machine; `full_access` approves everything. Tool-call records carry
  `approval` (`not_needed`, `required`, `auto`, `full_access`, `session`),
  and consent cards `reversible`, `egresses` and `can_allow_session`.
  "Allow for this chat" is kept per chat session in RAM (256 sessions) and
  seeds the session's next turn. `kassiber chat --permission-mode` sends it.
- Undo: a reversible tool's successful result carries
  `undo: {tool, arguments}` (`undo.rs`, `kassiber/ai/undo.py`); amounts keep
  Kassiber's 32-digit Decimal context. The chat loop still refuses mutating
  tools until the mutating executor is wired into chat, so the hint is
  reached through the executor and `test.ai_mutate` for now.

`tests/test_rust_ai_chat.py` passes 8 of 9: the remaining difference is the
chat-files `attachments` argument. It gains a turn that resumes a provider
cursor inside a saved session.

### MCP grants (#686)

- `crates/setup/src/agent_access.rs`: the preference records `grant`
  (`read`, `operator`, `accounting_decisions`; never admin) and
  `agent_access` (`restricted` by default) together once either is chosen,
  fails closed on unknown values, and reports `access_recorded`.
  `ui.agent_access.configure` takes them as strings; a new grant ends the
  desktop's agent lease, and `ui.agent_access.unlock` leases exactly the
  grant.
- `crates/mcp/src/tools.rs`: Python's registry, each tool classified through
  the broker's capability table (an egressing tool needs `operator`), the
  listing and definitions per grant and access mode, refusals
  (`mcp_grant_insufficient`, `agent_access_restricted`), the instructions
  per grant, `call_capability` and the cleaned client name.
  `crates/mcp/tests/registry.rs` replays `scripts/gen_registry.py`'s
  vectors.
- Calls (`crates/daemon/src/mcp.rs`, `crates/cli/src/mcp.rs`): refused above
  the grant before anything is resolved; a write needs a pinned server
  (`book_pin_required`) and runs through the assistant executor under a
  writable pin (`pin_write_scope`: a temp `settings` copy whose changes,
  except the pinned context, are copied back), audited as `mcp_host` and
  logged in RAM; from `operator` up a read refreshes stale journals and the
  analysis index first, at `read` a stale index is `derived_state_stale`.
  The host's approval of the call is its consent, for writes and for reads
  that leave the machine. The `book` labels are the resolved rows'.
  Brokered calls pass `--grant` and `--client=`, and the broker admits an
  agent lease's `mcp call` by its tool's class and claimed grant.
- `daemon.lock` now ends the agents' lease, as Python's
  `_clear_unlocked_passphrase` does; the Rust lock skipped it before (the
  daemon test `test_ending_the_desktop_session_ends_the_agent_lease` failed
  on c48358377).
- The custody AI audit admits `mode_auto`, `mode_full_access` and
  `mcp_host`; until now an MCP or permission-mode custody write failed with
  `tool_error`.
- #681's `_project_stale_journal_figures_for_ai` is in
  `projections::stale_journal_figures_for_ai`: AI reads of stale or
  never-processed journals (capital gains, journal events and snapshot,
  journal transfers, the overview's fiat figures, quarantine and its groups)
  get `null` figures with `status: unavailable` and the `process_journals`
  remediation instead of numbers the model could quote. Vectors over every
  branch are in `vectors.json` (`stale_journals`).

`tests/test_rust_mcp.py` follows the registry: the transcripts run at the
default grant on every fixture and at every grant and access mode on a
pinned desktop book, writes included (masking generated ids, analysis
references and clocks per side). Requests now go in windows of 16: the old
single burst exceeded the server's 32 calls in flight, so most calls on
both sides were the same overload refusal. All 11 tests pass. Tools whose
replies differ for another slice are listed in its `PENDING` map with their
slice (S2, S4, S5, S6, S7, S8, S10) and are not compared until it lands.

### Chat files: staging and uploads (#689, #698)

- `internal.document_import.stage` takes `purpose`: `chat_attachment` stages
  an export as well as a statement or photo (`chat_uploads::chat_attachment_source`:
  an existing file of a chat type, at most 64 MB, its own errors), and the
  staged `kind` is `file_kind`'s (`spreadsheet`, `delimited`, `pdf`,
  `image`). The daemon keeps sixteen staging grants (#698).
- `ui.chat.attachments.upload` (`crates/daemon/src/chat_uploads.rs`): a
  pasted file in base64 chunks of up to 512 KiB, decoded as Python's
  `b64decode(validate=True)`, into an owner-only file in a private 0700
  directory; two uploads at a time, five minutes idle. The last chunk stages
  it; the file is deleted with the last grant that names it, on lock, and
  the upload root when the daemon ends.
- #698's preview resolves the OCR provider and checks it is contactable
  before the document's arguments, as Python's `preview_document_import`
  now does. `tests/test_rust_document_cli.py` pins the Rust AI clock, since
  that resolution seeds the built-in providers.
- No image codec: Python adds a Pillow `thumbnail` to a staged chat image;
  the Rust daemon, like Python without Pillow, sends none.

`tests/test_rust_wallet_document_import.py` gains the chat picks and the
upload sequence (refusals, renaming, order, completion, analysis, lock);
its five tests pass, as do the document CLI and ingestion harnesses.
`tests/rust_port/repro_stage_purpose_unhashable.py` records a Python crash
on a list or object `purpose` that Rust refuses as validation.

### Chat files in the turn (#693, #697)

- `ai.chat` takes `attachments`: up to eight staging grants with `label` and
  `new` (the message that first sends a file); the single `attachment` of
  before is no longer read. The daemon resolves them to `chat_files`
  (`file-1` .. `file-8` in the order attached, with the file's `kind`); a
  grant that expired or belongs to another book is left out, named in the
  turn's note, and its token returned as the terminal's
  `expired_attachments`.
- `chat/files.rs`: the turn's note (a restricted provider gets the
  extension only, never the name), and for a full-access provider the
  images as themselves and PDFs as their `pdftotext` text or `pdftoppm`
  page images, at most eight images and 12 MB a message. A resumed session
  gets the note on the user's message and earlier files in a message of
  their own. Tool turns and plain ones attach them alike; an export offers
  the wallets pack, any file offers `ui.attachments.add_from_chat`, an image
  or PDF the document reader.
- Tools name a file by `file` (`files::ai_chat_file`); the analysis tool
  reads only exports. `ui.attachments.add_from_chat` keeps any chat file as
  evidence on a transaction (the native `ui.attachments.add` on the grant's
  file); a restricted provider never gets the file's name back. The user row keeps the cards (name, kind, size) of
  the files its message first sent.
- The broker carries a user message's inline images apart from its text
  (`images`, base64 data URLs only), on tool turns and plain ones.
- `kassiber chat --file` repeats (up to eight, with matching
  `--file-context`), stages with the chat purpose and marks the files new
  on the first turn only. It reads the grant as `document_token`; Python's
  CLI reads `token` and so never attaches a file
  (`tests/rust_port/repro_chat_file_token_key.py`).
- Without an image codec an image above 3.7 MB (or a larger rendered PDF
  page) is not shown, where Python reduces it with Pillow; images within
  the limits are sent byte for byte as Python sends them.

`tests/test_rust_ai_chat.py` gains the files scenario (an export read by id,
a PDF's text, an image, an expired grant, a plain follow-up, the stored
cards) and a broker scenario with images over a resumed session; all 11
tests pass. The tools harness now feeds the analysis tool `chat_files` and
compares its by-id cases; they match.

### Import runs and the wallet handoff (#682)

- `crates/ai/src/import_runs.rs`: `ui.imports.list` and
  `ui.imports.rollback.plan` read the book's import runs (a restricted
  provider gets no recorded filename); the plan is `review_rollback`'s,
  bound to the book's input version and a digest, and blocked when nothing
  is left to delete or the rollback would delete reviewed work.
  `ui.imports.rollback` runs only when the consented digest and version
  still hold (`rollback_plan_stale`, `rollback_blocked`).
  `kassiber_imports::Operation::Review` and `InputVersion` supply the plan
  with the version and the version alone.
- `ui.wallets.request_input`: the typed desktop handoff for a wallet the
  assistant must not create (source allowlist, bounded plain-text
  explanation, the packet's digest as its correlation id).
- `previews::plan_consent_preview`: the consent card of a plan-bound apply
  recomputes the plan (`ready`, `blocked`, `stale`, `unavailable`) and its
  summary; the attachment and document imports show as unavailable after
  Python's first checks until they are ported.
  `tests/rust_ai_tool_reference.py` applies the same card as the chat loop.

### Chat visuals (#676)

- `crates/ai/src/visuals.rs` is `core/chat_visuals.py`: the request
  normalizer, every source's projection (overview, balance history,
  portfolio, tax summary, cash flow, transactions, transaction flow,
  readiness) with its caveats (stale journals, quarantines, missing prices,
  truncation; never a zero for an unknown), and `model_digest`. Calendars
  are floored in each moment's own offset and written in UTC, as Python's
  aware datetimes are.
- `ui.visuals.render` reads each source from the page's own native read
  (the dashboard with its pairing for cash flow, the list from both ends
  for a size order, the local graph and resolver for a flow) and the
  report blockers for a report source's caveats; it contacts nothing. Only
  the report sources refresh stale journals first.
- The provider gets the digest, not the rows (`tool_result_for_model` on
  the function output and the auto-read context); the screen gets the
  whole visual.

Vectors (`visuals` in `vectors.json`) cover the normalizer and 49 built
visuals; the tools harness compares the tool on every fixture, masking the
visual's own clock (`as_of`, a cash-flow window's end).

### Tools harness after these steps

`tests/test_rust_ai_tools.py` now checks whole books against the store
manifest (113 tables) instead of a fixed 112 and builds its MCP state from
#686's grant registry (`available_tools(grant="read",
agent_access="restricted")`). One stale use of the removed
`MCP_SOURCE_TOOL_NAMES` stopped two of its tests with a `NameError` until
the MCP grants step; with that fixed, one test function and 14 subtests
fail, 232 subtests pass. Every remaining record difference belongs to
another main-sync slice or to the chat files still to come:

| Difference | Tools | Slice |
| --- | --- | --- |
| `group_id`, "exactly one of transaction or group_id" | `ui.journals.quarantine.resolve` | S5 |
| `completeness_class`, `scan_bounds` | auto-read `ui.wallets.list` | S8 |
| suggestion `exclusions` | auto-read `ui.transfers.suggest` | S6 |
| journals stale after a dismissal (`journal_input_version`) | `ui.transfers.dismiss` | S6 |
| `definitions`, badges `review` | auto-read health and badges, `ui.maintenance.run` | S10, S4 |
| `fiat_currency` on cases | review flow, acquisition receipts | S4 |
| export sections and sheets | `ui.reports.export` | S7 |
| `book_network_review_required` hint | review flow | fixed: the journals stage now keeps a custody refusal's hint and details |
| `swap_candidate_count` and one more index tick after processing | AI-authored `ui.journals.process` (automatic pairing before journals, #707) | S4 |
| the missing-attachment hint ("attach the file") | `ui.wallets.analyze_file` | S12, with the chat files (#693, #697) |

### Python fix round 7 (eabbe6606)

`t3code/python-reference-fixes-r7-7` is merged as a pure merge (2b523c060).
Its assistant changes are ported:

- b2c0d9447: a non-string component `action` is a validation result
  ("Custody component review action is unsupported") before any consent
  card, and the turn continues (`summary.rs`, the pre-consent refusal in
  `chat/turn.rs`). The transfers-side handler check belongs to S4.
- a26567dfa: a tool turn's saved answer is every leg's text, and 7e3e9e286
  binds and looks up the provider cursor the same way: the lookup
  fingerprints the transcript before the new prompt
  (`chat::turn::resume_fingerprint`), so a follow-up resumes the native
  session. `tests/test_rust_ai_chat.py` now asserts the resume.
- 5d8b2658a: a non-string review operation type is a validation result in
  the AI review rules.
- #684's default `tool_profile` is `full`.

The refresh by wire name (09912e4c0) is ported with the MCP grants step:
the executor decides on the catalog entry, so a model's `ui_reports_summary`
refreshes stale journals as the planner's dotted read does (on the desktop
fixture this is what makes those reads refuse `book_network_review_required`
as Python does). The maintenance sync mode (a125129ed) and the CLI chat's
Ctrl-C (c40e8fb32) already held in Rust (`settings.rs` checks the type;
the chat daemon leads its own process group). Not ported here: the wallet
sync's typed `invalid_wallet_config` (7bca137d4); Rust's sync planner still
answers `internal_error` for an unreadable wallet configuration, which is
the wallet-sync owner's.

At this point `tests/test_rust_ai_chat.py` passed 4 of 9; the permission
modes above brought it to 8.

## Python reference notes

`summarize_tool_call` (`kassiber/ai/tools.py`) used to look a component
action up in a dict before the arguments were checked against the schema,
so a model that sent `{"action": ["a"]}` to `ui.transfers.components.plan`
or `.apply` raised `TypeError: unhashable type: 'list'` and the chat turn
ended with `internal_error`. Fix round 7 (b2c0d9447) made it a validation
result; `tests/rust_port/repro_component_summary_unhashable.py` now checks
the fixed behaviour and exits non-zero if the crash returns.
