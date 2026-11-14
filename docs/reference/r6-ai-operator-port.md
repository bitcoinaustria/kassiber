# R6 port design: AI, MCP and the operator broker

> **Historical.** This page records the Python-to-Rust port. Python was removed
> under [plan 22](../plan/22-python-removal.md); the `kassiber/` paths and Python
> tests it cites exist only at the `archive/python-backend` tag.

Design for stage R6 of [plan 21](../plan/21-rust-backend.md#stages): AI chat,
the provider broker client, AI tool execution with consent and the AI
projections, the MCP server, and the operator broker (queues, agent sessions,
Touch ID). On 2026-11-02 the owner brought R6 into scope ("do it all"), so
Python leaves the product, and every surface below needs a Rust implementation
with Python parity. This document is a plan. It ports no code.

It was researched on 2026-11-02 at `3d9f49efc`, the tip of the integration
branch holding all native R4/R5 work so far. Line numbers refer to that
commit. "R6" here means the plan-21 stage, not the older integration branch
of the same name.

## Summary

- Nothing in R6 is native yet: none of the 31 CLI leaves or the 27 daemon kinds.
  Rust already has these pieces:
  - the parsed grammar and `commands describe` metadata for every leaf;
  - the capability classes, which the broker authorizes by;
  - Python-compatible project ownership locks;
  - the envelope kinds;
  - the secret-store control channel;
  - an AI route in `kassiber-net`;
  - a non-streaming AI HTTP client used by document import.
- The in-product AI catalog has 161 tools. 81 of them call a service with no
  native daemon kind yet, and 4 more are only partly native. Those
  [shared R5 dependencies](#2-shared-r5-dependencies) decide when chat
  can be registered.
- The work splits into [eight slices](#3-slices) in two independent tracks, an
  operator track and an AI track, which meet at MCP. Two slices need two worker
  runs each.
- No new external crate is needed. Two `windows-sys` features are added. Of
  the four new workspace crates, only `crates/ipc` allows `unsafe`, and it
  holds all the platform code.

## 1. Census

### 1.1 Surface counts

The inventory at `c1e19df4c` counted 38 excluded CLI leaves and 34 excluded
daemon kinds. Seven of each are PDF exports (now a separate stage), which
leaves 31 R6 leaves and 27 R6 kinds. A recount against `3d9f49efc` finds the
same names, all still present in Python (313 leaves in
`tests/fixtures/rust_port/commands_describe.json`, 303 kinds in
`daemon_kinds.json`) and none registered natively: no R6 leaf is in
`IMPLEMENTED` (`crates/cli/src/describe.rs:16`, 188 entries), and no daemon
family registers an R6 kind (`crates/daemon/src/lib.rs:72`).

| Surface | Count | Native today |
| --- | ---: | --- |
| CLI leaves (operator 10, mcp 6, chat 1, chats 5, ai 9) | 31 | parsed, described, classified; execution answers `not_implemented` |
| Daemon kinds (ai 16, ui.chat 5, ui.agent_access 5, ui.chain_analysis.ai_context 1) | 27 | classified only; answer `not_implemented` |
| Supervisor control records (`supervisor.ai_secret_store.request` / `.response`) | 2 | transport native (`crates/daemon/src/control.rs`), only a test kind calls it |
| Entry paths: `--operator-broker-server`, `--operator-auth-fd`, brokered routing, operator child mode | 4 | refusal stubs (`crates/cli/src/run.rs:192`, `:819`, `:852`) |
| Operator protocol actions | 14 | none |
| MCP methods (both protocol eras) | 8 | none |
| MCP tools | 44 | none; 25 call a native kind (2 with AI-path gaps), 18 depend on R5, 1 is R6 itself |
| Provider broker messages (4 commands, 6 event types) | 10 | none |
| AI tools | 161 | none of the executor; 76 call a native kind, 17 of them with AI-path gaps |
| AI projections outside the catalog | about 15 | none |

The inventory files `ui.review.worklist` under unassigned R4/R5 work: its
direct route answers `daemon_unavailable` (`kassiber/daemon.py:18700`), and only
the AI tool works. This plan treats its AI payload as R6 work
(slice [R6-5](#r6-5-ai-tool-executor-catalog-and-projections)).

### 1.2 CLI leaves

All 31 are in the generated parser (`crates/cli/src/spec_data.rs`), have
capability classes (`crates/runtime/src/capabilities.rs`) and kind attributes
(`crates/envelope/src/lib.rs:377`), and fail with `not_implemented` before
touching a project (`crates/cli/src/run.rs:322`).

| Leaf | Python | Capability | Behaviour | Slice |
| --- | --- | --- | --- | --- |
| `operator unlock` | `kassiber/operator/cli.py:140` | admin | password or Touch ID unlock; starts a broker | R6-3, R6-4 (Touch ID) |
| `operator status` | `operator/cli.py:129` | read | lease and mode status; never starts a broker | R6-3 |
| `operator lock` | `operator/cli.py:138` | admin | ends a lease | R6-3 |
| `operator stop` | `operator/cli.py:109` | admin | idle-only broker stop | R6-3 |
| `operator mode` | `operator/cli.py:169` | admin | `manual`/`brokered`/`unattended` with a fresh passphrase | R6-3 |
| `operator operation status` | `operator/cli.py:104` | read | queued-operation status | R6-3 |
| `operator operation cancel` | `operator/cli.py:104` | admin | cancels queued work | R6-3 |
| `operator touch-id status` | `operator/cli.py:113`, `native_auth.py:197` | read | runs the helper `status` in the CLI process | R6-4 |
| `operator touch-id enroll` | `operator/cli.py:116` | admin | stores the passphrase behind Touch ID | R6-4 |
| `operator touch-id forget` | `operator/cli.py:116` | admin | deletes it | R6-4 |
| `mcp serve` | `kassiber/mcp/cli.py:513` | read | stdio MCP server | R6-8 |
| `mcp tools` | `mcp/cli.py:151` | read | prints the 44 definitions | R6-8 |
| `mcp status` | `mcp/cli.py:81`, `agent_access.py:61` | read | reads `agent-access.json` | R6-8 |
| `mcp enable` | `mcp/cli.py:120` | admin | terminal-only confirmation | R6-8 |
| `mcp disable` | `mcp/cli.py:81` | operator | turns access off | R6-8 |
| `mcp call` | `mcp/cli.py:92` | read | one tool call; brokerable | R6-8 |
| `chat` | `kassiber/cli/chat.py:1176` | operator | spawns a private daemon child and streams | R6-7 |
| `chats list`/`show`/`delete`/`clear`/`config` | `kassiber/cli/main.py:5560`–`5596`, `cli/handlers.py:1217`–`1240` | read ×2, admin ×3 | chat history | R6-7 |
| `ai providers list`/`get` | `cli/main.py:6505`, `:6507` | read | redacted rows; first read seeds built-in rows | R6-6 |
| `ai providers create`/`update`/`delete`/`set-default`/`clear-default` | `cli/main.py:6509`–`6553` | admin | provider configuration; inline keys only | R6-6 |
| `ai providers test` | `cli/main.py:282` | admin | connection test (explicit egress) | R6-6 |
| `ai models` | `cli/main.py:6556` | read | lists models, bypassing the cache (explicit egress) | R6-6 |

### 1.3 Daemon kinds and control records

| Kind | Python | Capability | Notes | Slice |
| --- | --- | --- | --- | --- |
| `ai.providers.list`, `ai.providers.get` | `daemon.py:18047`, `:18265` | read | may seed rows; desktop keychain `exists` over the bridge | R6-6 |
| `ai.providers.create`/`update`/`delete`/`set_default`/`clear_default`/`acknowledge` | `daemon.py:18058`–`18279` | admin | configuration; invalidates the discovery cache | R6-6 |
| `ai.providers.set_api_key`, `ai.providers.move_api_key` | `daemon.py:18176`, `:18199` | admin | inline or native store via `supervisor.ai_secret_store.*` | R6-6 |
| `ai.provider_runtime.status` | `daemon.py:18292` | read | probes CLI runtimes on a cold cache unless `cached_only` | R6-6 |
| `ai.list_models` | `daemon.py:18372` | read | probes on a cold or expired cache, 60 s TTL | R6-6 |
| `ai.test_connection` | `daemon.py:18410` | admin | up to one `GET /models` and six `POST`s | R6-6 |
| `ai.chat` | `daemon.py:18583`, worker `:9770` | operator | the only streaming R6 kind | R6-7 |
| `ai.chat.cancel`, `ai.tool_call.consent` | `daemon.py:15289`, `:15309` | operator | control branch, served while a chat streams | R6-7 (consent state in R6-5) |
| `ui.chat.sessions.list`/`get`/`delete`/`clear` | `daemon.py:9115`–`9153` | read ×2, admin ×2 | history | R6-7 |
| `ui.chat.history.configure` | `daemon.py:9136` | admin | `auto`/`on`/`off` | R6-7 |
| `ui.agent_access.status` | `daemon.py:15354`, `daemon_agent_session.py:83` | read | no broker contact unless `refresh` with a live lease | R6-3 |
| `ui.agent_access.configure`/`unlock`/`lock`/`pairing` | `daemon.py:15379`, `daemon_agent_session.py:153`–`346` | admin | desktop "unlock for agents" lease | R6-3 |
| `ui.chain_analysis.ai_context` | `core/chain_analysis_api.py:45`, `daemon.py:16876` | read | desktop-only projection for the chain-analysis assistant | R6-5 (needs chain analysis, R5) |
| `supervisor.ai_secret_store.request`/`.response` | `daemon.py:1487` | control | 15 s wait, deferred stdin lines | R6-6 (transport exists) |

### 1.4 Operator protocol

The protocol is version 3 over `~<passwd home>/.kassiber/run/operator-v1.sock`
on Unix or `\\.\pipe\kassiber-operator-<sha256(SID)[:24]>` on Windows. Frames
are a `!cI` header (one kind byte and a big-endian u32 length). `J` frames hold
compact JSON with sorted keys, at most 8 MiB. `S` frames hold
`challenge\0secret`, at most 16 KiB. There is one request per connection, and
multi-step actions continue on the same connection with `continue: secret`,
`secrets` or `argv` replies that name the broker's build. The broker sends no
unsolicited events. All code is in `kassiber/operator/`; nothing exists in Rust
apart from ownership (below).

| Action | Python | Auth beyond same-user peer | Slice |
| --- | --- | --- | --- |
| `ping` | `server.py:247` | — | R6-1 (client), R6-2 (server) |
| `status` | `server.py:271`, `service.py:1332` | — | R6-2 |
| `unlock` (+ secret frame, optional agent control frame) | `server.py:279`, `service.py:706` | passphrase opens the book; auth backoff | R6-2 |
| `lock` | `server.py:397`, `service.py:1254` | — | R6-2 |
| `submit` (+ argv frame, secret frames in sorted label order, admin frame) | `server.py:411`, `:492`; `service.py:1395` | lease, capability, admin fresh auth, agent token and pid | R6-2 |
| `operation_status` | `server.py:413` | — | R6-2 |
| `operation_cancel` | `server.py:420` | — | R6-2 |
| `set_mode` (+ fresh auth frame) | `server.py:424`, `service.py:1178` | fresh passphrase | R6-2 |
| `restart_for_native_auth` (also `operator stop`) | `server.py:269`, `service.py:1815` | broker must be idle | R6-2 |
| `agent_session_open` (connection held, heartbeats) | `server.py:206`, `service.py:2203` | peer pid required, agent-scope lease | R6-3 |
| `agent_session_heartbeat` | `server.py:232` | — | R6-3 |
| `agent_session_decide` (+ control frame) | `server.py:372`, `service.py:2289` | sha256 of the control secret | R6-3 |
| `unlock_touch_id` | `server.py:340`, `native_auth.py:77` | signed helper identity, policy binding | R6-4 |
| `touch_id_configure` (+ fresh auth frame) | `server.py:448`, `service.py:1214` | helper identity, fresh passphrase | R6-4 |

Related state is persisted only in project settings (`operator_unlock_mode`,
`operator_policy_project_identity`, `operator_policy_database_identity`,
`operator_native_auth_generation`), `config/auth_backoff.json` and the global
`config/agent-access.json`. Leases, queues, operations and agent sessions live in RAM.

What Rust already has (`crates/ownership/src/lib.rs`, 647 lines):

- **Ownership:**
  - canonical project identity and owner locks matching Python's;
  - the broker and desktop roles, aliases and admission locks;
  - `verify_current`;
  - Python interoperability tests (`tests/test_rust_store_interop.py:354`, `:362`).
- **Missing from ownership:**
  - the seven-day prune janitor (`project.py:382`);
  - the `project_in_use` hint naming the holder's pid;
  - inheritable child duplicates (`duplicate_for_child` returns close-on-exec clones);
  - Python's `OPEN_ALWAYS` probe on Windows.
- **Elsewhere:**
  - the effective unlock mode (`crates/kassiber/src/real_store.rs:404`);
  - `Policy::operator_status` and native-auth generation rotation (`crates/secrets/src/policy.rs:99`, `:406`);
  - the auth backoff file (`real_store.rs:642`).

### 1.5 MCP protocol and tools

`mcp serve` speaks newline-delimited JSON-RPC on stdio. Frames are written with
`ensure_ascii`, compact separators and insertion order. There are no batches,
and the limits are 16 MiB per message, 32 calls in flight and 16 subscriptions.
One worker runs tool calls in order, while the reader answers other methods at
once, so replies can arrive out of order. At EOF, accepted calls get 30 s.
The server claims stdio: it keeps private copies of stdin and stdout, then
points fds 0, 1 and 2 at `/dev/null` before anything can write to stderr
(`mcp/cli.py:537`).

| Method | Era | Python |
| --- | --- | --- |
| `server/discover` | modern `2026-07-28` | `mcp/protocol.py:237` |
| `initialize` | legacy `2025-11-25`, `2025-06-18` | `protocol.py:277` |
| `ping` | legacy | `protocol.py:261` |
| `tools/list` | both | `protocol.py:247`, `:263` |
| `tools/call` | both | `protocol.py:306`, `mcp/cli.py:249` |
| `subscriptions/listen` | modern | `protocol.py:436` |
| `notifications/cancelled` | both | `protocol.py:402` |
| `notifications/initialized` and other notifications | both | ignored, `protocol.py:143` |
| anything else | both | `-32601` |

The 44 tools are the read-only allowlist `MCP_SOURCE_TOOL_NAMES`
(`kassiber/mcp/tools.py:24`). Each takes its schema and description from the AI
catalog, and its name is the catalog's wire name without the `ui_` prefix.
Calls run through the AI read executor in remote-provider mode: no journal
refresh or sync, no cross-book reads, and SQLite `query_only`. Plaintext and
`unattended` books are opened directly; `brokered` books go through the broker
as `--machine mcp call` without ever starting one; `manual` encrypted books
answer `interaction_required`. [Appendix B](#appendix-b-mcp-tools) lists every
tool with its native status.

### 1.6 Provider broker protocol

The TypeScript broker (`ui-tauri/provider-broker`) stays a subprocess. Python
spawns it as `[node, index.mjs]` (`kassiber/ai/broker_client.py:177`) with the
full environment plus `KASSIBER_AI_BROKER_TMPDIR`, in a new session. Each
process takes exactly one JSONL request on the first stdin line; there is no
handshake. The overrides `KASSIBER_AI_BROKER_NODE` and
`KASSIBER_AI_PROVIDER_BROKER` select Node and the script, and the Python tests
use them to install fake brokers. [Plan 23](../plan/23-rust-provider-broker.md)
later replaced the TypeScript broker with `kassiber --provider-broker`; the
protocol below is unchanged.

| Message | Direction | Python | Notes |
| --- | --- | --- | --- |
| `status` (`provider` or `providers`) | daemon → broker | `broker_client.py:290` | 35 s; an empty list never spawns |
| `models` | daemon → broker | `broker_client.py:366` | full status probe; `min(timeout, 35)` |
| `chat` | daemon → broker | `broker_client.py:519` | options allowlisted; tools carry `read_only`/`destructive` hints |
| `tool_results` | daemon → broker | `broker_client.py:553` | every pending call id; denials encoded in `output` |
| `status` event | broker → daemon | `broker_client.py:600` | only notice phases are forwarded |
| `delta` | broker → daemon | same | content or reasoning |
| `tool_call` | broker → daemon | same | the client returns after one call and keeps the process alive |
| `done` | broker → daemon | same | stores the session cursor |
| `error` | broker → daemon | `broker_client.py:234` | maps to `ai_auth_failed`, `ai_unavailable`, `ai_request_invalid` |
| `result` | broker → daemon | same | status rows or models |

Cancellation is signals only: SIGTERM to the process group, then SIGKILL.
Windows has no group kill.

### 1.7 AI tools

The catalog is `TOOL_CATALOG` (`kassiber/ai/tools.py:3442`), which joins three
lists in `tools.py` with the 36 generated chain-analysis tools
(`kassiber/ai/chain_analysis_tools.py:73`) and the 5 source tools
(`chain_analysis_sources_tools.py:5`). Consent follows from two fields:
`kind_class` (`read_only` or `mutating`) and `egresses`. A tool needs consent if
it mutates or egresses (`tools.py:54`).

| Class | Count | Consent |
| --- | ---: | --- |
| read, no egress | 98 | none |
| read with egress (`ui.reports.lightning_profitability`, `ui.connections.node.snapshot`) | 2 | yes; runs through the read executor |
| mutating, no egress | 59 | yes |
| mutating with egress (`ui.chain_analysis.acquire.apply`, `ui.chain_analysis.sources.run`) | 2 | yes |

Further gates:

- **Once-only consent** (10 tools, `daemon.py:784`): "allow for this chat" becomes "allow once".
- **Local provider only** (7 custody tools, `daemon.py:677`, `:690`): denied with `local_provider_required`.
- **On-device provider only** (11 chain-analysis source, acquire and dataset tools, `daemon.py:6551`, `:9464`): stripped from the advertised list for other providers.

Exposure:

- The desktop advertises capability packs per turn (profile `scoped`, `tools.py:3553`).
- `kassiber chat` defaults to the 60 `core` tools.
- MCP gets 44 read tools.
- The auto-read planner may run 36 read tools without consent.
- The provider broker receives exactly the tools advertised that turn.

The executor (`daemon.py:6569` for reads, `:7471` for mutations) does the following:

- It validates arguments against the catalog schema (`daemon.py:6330`), refusing anything over 65,536 code points.
- It runs every scoped operation with a book-scope and stale-context check (`daemon.py:7134`).
- It sends each database callback to the daemon's single main-thread connection (`daemon.py:1310`).
- It redacts every result (`tools.py:3694`).
- It hands the model compact JSON with sorted keys.

[Appendix A](#appendix-a-ai-tool-census) lists all 161 tools.

Projections outside the catalog (none native):

- the transaction review context and review worklist composites (`daemon.py:6048`, `:5889`);
- `ui.chain_analysis.ai_context` and the HMAC `ca-ref:` handle projection (`core/chain_analysis_ai.py:26`–`249`);
- the `_for_ai` builders for wallets, profiles, Lightning and file analysis;
- the evidence and source-of-funds key filters (`daemon.py:2518`, `:2533`);
- the AI-safe egress snapshot (`daemon.py:5807`);
- the per-turn provenance and privacy receipt (`daemon.py:7996`);
- the custody AI audit writer (`core/custody_ai_audit.py:66`).

The handle registry already exists natively in
`crates/wallets/src/analysis_index/handles.rs`, but it is unregistered.

### 1.8 Chat, providers and history

- **Transports.** Every `http(s)` provider uses one urllib client:
  - it speaks Responses or Chat Completions;
  - it ignores system proxies;
  - it follows redirects only to the same origin;
  - it falls back between the two formats on a missing route.
- **CLI locators.** The four `*-cli://default` locators go through the broker. There is no native Anthropic or Ollama API.
- **API keys.** Keys are either inline in the book, or in the desktop's keychain reached only through `supervisor.ai_secret_store.*`. Python never touches a keyring.
- **Persisted state:**
  - `ai_providers` and `ai_provider_secret_refs` hold provider configuration;
  - `ai_chat_sessions` and `ai_chat_messages` hold history;
  - the `ai_chat_history` setting holds the history policy.
- **RAM-only state:** the discovery cache, provider session cursors, consent
  state and pending cancels.

`ai.chat` emits these records in a fixed order:

- `ai.chat.status`
- `ai.chat.delta`
- `ai.chat.tool_call`
- `ai.chat.tool_consent_required`
- `ai.chat.tool_result`
- a terminal `ai.chat`, or an error envelope

Consent waits up to 300 s. A cancel that arrives before the chat registers is
held for 30 s (at most 128 are held). A cancelled turn still ends with a
terminal record.

Rust already has a document-import AI client (`crates/wallets/src/file_import/document/provider*.rs`, about 1,400 lines plus tests). It covers:

- seeding;
- the enabled and acknowledged gates;
- request bodies with Python-identical JSON;
- error mapping;
- the format fallback.

`kassiber-net` has the AI route with Python's offline exception for local
loopback providers (`crates/net/src/lib.rs:384`, `:403`), but responses are
fully buffered. There is no streaming reader.

## 2. Shared R5 dependencies

A tool can only be ported when the service behind it is native, and chat can
only be registered when every tool is. The 81 tools without a native kind,
grouped by the R5 work that owns the service:

| Owning work | Tools | In MCP |
| --- | --- | ---: |
| Chain analysis (unassigned R5) | all 36 `ui.chain_analysis.*` tools, the 5 `ui.chain_analysis.sources.*` tools, and the `ui.chain_analysis.ai_context` kind | 0 |
| Workspace overview and health, readiness (`core/ui_snapshot.py`) | `ui.overview.snapshot`, `ui.workspace.overview.snapshot`, `ui.workspace.health`, `ui.next_actions`, `ui.report.blockers`, `ui.review.badges`, `ui.audit.changes_since_last_answer`, `ui.audit.evidence.summary` | 4 |
| Journals (R4 reads, R4-15 desktop processing) | `ui.journals.snapshot`, `.quarantine`, `.quarantine.resolve`, `.events.list`, `.transfers.list`, `.process` | 4 |
| Review workflow (`t3code/r5-transfers-17`) | `ui.review.cases`, `.request_input`, `.plan`, `.apply` | 2 |
| Transactions and graph projections | `ui.transactions.resolve`, `ui.transactions.graph` | 2 |
| Transfers and swap review | `ui.transfers.review_context` | 1 |
| Freshness and maintenance | `ui.maintenance.settings`, `.configure`, `.run` | 1 |
| Backend configuration | `ui.backends.list` (the CLI leaf is native, the kind is not) | 1 |
| Reports | `ui.reports.capital_gains` (kind only), `ui.reports.privacy_mirror` (Mirror branches) | 1 |
| Wallets, ownership verification, file analysis | `ui.wallets.utxos`, `ui.wallets.identify`, `ui.wallets.analyze_file` | 0 |
| Custody projections | `ui.custody.coverage.snapshot`, `ui.custody.lineage.snapshot` | 0 |
| Rates | `ui.rates.rebuild` (`crates/rates/src/desktop.rs:371` refuses it) | 0 |
| Commercial evidence | `ui.btcpay.provenance.review` | 0 |
| Lightning node snapshot | `ui.connections.node.snapshot` | 0 |
| Diagnostics and egress ledger | `ui.egress.snapshot` | 0 |
| PDF stage | `ui.source_funds.export` (PDF and bundle targets) | 0 |
| R6 itself | `read_skill_reference`, plus the AI-only composites `ui.transactions.review_context` and `ui.review.worklist`, which need the rows above | 3 |

Four more tools are partial. `ui.reports.export` lacks the PDF and audit-package
targets (native since the [audit and Mirror port](rust-audit-mirror.md), as is
`ui.source_funds.export`); the other three lack an AI-only piece:

- `ui.profiles.snapshot`: the `_for_ai` wrapper.
- `ui.reports.lightning_profitability`: the aggregate-only projection.
- `ui.custody.review.apply`: the custody AI audit rows.

Seventeen tools call a native kind but rely on AI-specific behaviour the native
route lacks:

- **Authorship.** The native transfers routes hard-code `authored_source="gui"`
  (`crates/daemon/src/transfers.rs:204`, `:383`, `:398`, `:411`). The AI path
  needs `ai_tool` authorship and the custody conversion guard
  (`daemon.py:1881`).
- **Metadata edits.** They must accept the `ai_tool` source.
- **`ui.review.receipt`** must refuse `kind_override`.
- **`ui.loans.list`** applies its own limit and summary.
- **`ui.wallets.sync`** must pass `strict=True, auto_pair_journals=False`.

R6-5 adds these as parameters on the native services, not as a second
interpreter. That means small additions to `crates/transfers`,
`crates/transactions` and `crates/daemon`.

The operator track has no R5 dependency. A Rust broker runs its children
from its own executable, so a command the binary has not ported answers
`not_implemented` inside the broker exactly as it does directly.

## 3. Slices

Each slice follows the [service slice contract](../plan/21-rust-backend.md#service-slice-contract-r4-onward):
one branch from the current stack tip, domain logic in its own crate,
registration lines in `crates/cli` and `crates/daemon`, parity through the
hybrid candidate and differential drivers, and mutation entries killed by named
tests. New crates: `crates/ipc` (platform primitives, `unsafe` allowed and
audited), `crates/operator`, `crates/ai` and `crates/mcp`, all with
`unsafe_code = "forbid"` except `crates/ipc`.

| Slice | Scope | Depends on | Runs |
| --- | --- | --- | ---: |
| R6-1 | Local IPC, child-process primitives, operator protocol and client | — | 1 |
| R6-2 | Broker service, server and broker children | R6-1 | 1 |
| R6-3 | Operator CLI, brokered routing, agent sessions and agent access | R6-2 | 1 |
| R6-4 | Native auth and Touch ID | R6-2 | 1 |
| R6-5 | AI tool executor, catalog and projections | — (tool wiring follows §2) | 2 |
| R6-6 | Providers, HTTP streaming, broker client and discovery | — | 2 |
| R6-7 | Chat: `ai.chat`, cancel, consent, history, `chat` and `chats` | R6-5a, R6-6 | 1 |
| R6-8 | MCP server | R6-5a; R6-3 for brokered books | 1 |

Parallel waves:

1. R6-1, R6-5a and R6-6a.
2. R6-2, R6-5b and R6-6b.
3. R6-3, R6-4, R6-7 and the direct-book half of R6-8.
4. The brokered half of R6-8.

R6-4's hardware acceptance needs a signed build on a Mac, so it can stay open
while the rest proceeds.

### R6-1 Local IPC, child-process primitives and the operator protocol

Done on branch `t3code/r6-1-ipc-protocol`; the
[checkpoint](rust-shell.md#operator-protocol-local-ipc-and-ownership-r6-1)
records its verification and what is left for R6-2.

**Scope:**

- **`crates/ipc`:**
  - Unix listener and stream, with peer credentials: `SO_PEERCRED` on Linux;
    `getpeereid` plus `LOCAL_PEERPID` on macOS.
  - Non-blocking connect with a timeout.
  - The Windows named-pipe server and client:
    - an SDDL DACL for the user's SID;
    - `FILE_FLAG_FIRST_PIPE_INSTANCE` and `PIPE_REJECT_REMOTE_CLIENTS`;
    - overlapped I/O with deadlines, and `CancelIoEx`, keeping a buffer alive
      until the cancelled I/O drains;
    - client and server pid and SID checks.
  - Child spawn with an explicit list of inherited fds or handles:
    - POSIX: `pre_exec` clears close-on-exec on the listed fds, and `setsid` detaches.
    - Windows: `CreateProcessW` with `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`.
  - Signal-to-flag delivery.
  - A pipe poll with a timeout.
- **`crates/operator`, protocol layer:**
  - frames and limits;
  - the challenge;
  - runtime-directory resolution from the passwd home with mode checks, plus
    the test override, admitted only in `test-harness` builds;
  - the start-lock election and inode-checked stale-socket removal;
  - build identity;
  - the complete client wire for all 14 actions.
- **Ownership completion:** inheritable duplicates through `crates/ipc`, the
  prune janitor, the `project_in_use` hint and the Windows probe mode.

**Oracle:**

- `tests/test_operator_protocol.py` (23), `test_operator_client.py` (25) and
  `test_operator_project.py` (26) are ported as Rust tests.
- A golden corpus of frames generated by `kassiber/operator/protocol.py`, with
  a drift test.
- Cross-language checks:
  - Python's `BrokerClient.ping` and `status` against a Rust test listener;
  - Rust's client against the Python broker started by `tests/operator_server_fixture.py`.
    These two actions need no same-build proof.
- `tests/test_rust_store_interop.py` grows a case where a Rust-spawned child
  holds inherited owner locks that block Python.

**Mutation targets:**

- frame limits at the boundaries;
- the challenge comparison;
- the peer uid and SID checks;
- runtime-directory mode and owner checks;
- the stale-socket unlink condition;
- build-identity equality;
- connect timeouts.

**Privacy fences:**

- The only sockets are `AF_UNIX` and named pipes, in `crates/ipc`. The
  chokepoint scanner (`crates/net/tests/chokepoint.rs`) gets an IPC allowance
  for that crate only.
- Secrets travel only in `S` frames, never in JSON, environment or argv. Their
  buffers are zeroized.
- The client never starts a broker except from `unlock`, `mode` and
  `touch-id enroll`/`forget`.

### R6-2 Broker service, server and broker children

Done on branch `t3code/r6-2-broker-service`; the
[checkpoint](rust-shell.md#operator-broker-service-server-and-children-r6-2)
records its verification, reviewed differences and what is left for R6-3
and R6-4.

**Scope:**

- **`crates/operator` service:**
  - leases;
  - per-project FIFO queues (64 deep);
  - the operation state machine, with retention and tombstones (256 results,
    16 MiB, 1,024 tombstones);
  - replay by operation id;
  - single-use 60 s admin authorization;
  - auth backoff scopes, reusing the existing backoff file code;
  - the lease janitor and retried owner releases;
  - idle restart and close.
- **Modes and policy:** `set_unlock_mode`, `bind_project_policy`.
- **The runner:**
  - lease passphrase and secret-label pipes, plus owner duplicates;
  - child environment bindings and the caller working directory;
  - UTF-8-lossy output and stderr redaction;
  - `operator_result_too_large` measured on the ASCII-escaped length.
- **The server:**
  - at most 64 connection threads;
  - Linux login-session lifetime checks;
  - signal stop;
  - the `--operator-broker-server` entry.
- **Argv classification** uses `crates/cli`'s argparse emulation and
  capability tables. It needs a `classify` API that reports declared
  destinations, because Python tests `hasattr(parsed, "workspace")`.
- **The CLI child side** replaces the `not_implemented` stubs in
  `crates/cli/src/run.rs:592`, `:761` and `:819`. The child:
  - enters the caller's directory;
  - verifies the project identity;
  - drains the lease pipe eagerly;
  - verifies the database identity before bootstrap.

**Oracle:**

- `test_operator_service.py` (96), `test_operator_server.py` (35) and
  `test_operator_modes.py` (9) become Rust tests, with a fake runner seam.
- `test_operator_integration.py` (6) runs against the Rust broker through a
  new fixture switch (see [verification](#6-verification-plan)).
- A differential driver compares brokered commands' envelopes with direct runs
  of the same commands on copies of the same book.

**Mutation targets:**

- capability checks and admission order;
- the scope-required and project-mismatch checks;
- the replay fingerprint;
- retention limits;
- lease and idle expiry;
- admin TTL and single use;
- the `failed` versus `result_unknown` split for read and mutating commands;
- lease-ending commands;
- argv pinning, which strips `--data-root` and `--project`.

**Privacy fences:**

- The passphrase reaches children only through pipes.
- `KASSIBER_NO_EGRESS=1` is propagated when requested.
- The status payload passes the public-safe detail filter.
- The broker opens no network route.

### R6-3 Operator CLI, brokered routing, agent sessions and agent access

Done on branch `t3code/r6-3-operator-cli`; the
[checkpoint](rust-shell.md#operator-cli-brokered-routing-and-agent-access-r6-3)
records its verification, reviewed differences and what is left for R6-4
and R6-8.

**Scope:**

- **Operator leaves:** the seven non-Touch-ID `operator` leaves, plus `--operator-auth-fd`.
- **`route_brokered_command`** in `crates/cli`:
  - fd-to-label conversion;
  - the `operator.operation.accepted` line on stderr;
  - waiting for the result, and cancel on Ctrl-C with exit 130;
  - typed result mapping.
- **Agent sessions:** open, heartbeat and decide, with peer process labels
  (`operator/peers.py`).
- **`config/agent-access.json`:**
  - schema exactly integer 1;
  - 1,024-byte cap;
  - fails closed to off;
  - written with sorted keys and default separators.
- **Daemon:** the five `ui.agent_access.*` kinds, and the desktop agent-lease
  state machine ticking in the daemon request loop.

**Oracle:**

- `test_operator_cli.py` (22), `test_operator_agent_sessions.py` (23),
  `test_daemon_agent_session.py` (25) and `test_cli_agent_contract.py` (11).
  Their subprocess-scenario tests become CLI and daemon drivers.
- The hybrid candidate serves the operator leaves, and its log shows Rust served them.
- A Rust daemon session (`KASSIBER_HYBRID_RUST_DAEMON=1`) runs the agent-access kinds.

**Mutation targets:**

- the agent-access parsing rules and reason precedence;
- the pid and token check;
- denial memory;
- idle and heartbeat timeouts;
- the conditional lock with `restore_manual`.

**Privacy fences:**

- `ui.agent_access.status` contacts no broker unless `refresh` is set and a
  desktop lease exists.
- MCP and status paths never start a broker.
- Agent-scoped leases admit only `mcp.call`.

### R6-4 Native auth and Touch ID

**Scope:**

- **Leaves:** `operator touch-id status`/`enroll`/`forget` and
  `operator unlock --auth touch-id`.
- **Broker side:**
  - `unlock_touch_id` and `touch_id_configure`;
  - the restart handoff, so a broker started from a signed CLI inherits
    `KASSIBER_NATIVE_AUTH_HELPER`.
- **The helper launch:**
  - inherited result, ready and go pipes;
  - a 5 s wait for `R`;
  - live-pid code-signing validation before `G`.
- **Code identity:** inspect it by running `/usr/bin/codesign` on a path and on
  a pid, as Python does (`native_auth.py:334`).
- **Account derivation:** derive the account from the project identity and the
  native-auth generation.
- **No Tauri changes.** The Tauri helper (`ui-tauri/src-tauri/src/lib.rs:4103`)
  keeps its check that its direct parent is
  `Contents/Resources/binaries/kassiber-cli-<arch>-apple-darwin`, signed with
  that identifier and team. The Rust sidecar keeps that name and identifier.

**Oracle:**

- `test_operator_native_auth.py` (10) as Rust tests, through a code-identity seam.
- The Tauri unit tests at `lib.rs:4831` and `:4844`.
- A signed hardware run on a Mac. This is the existing R0 TODO item, now
  against the real flow.

**Mutation targets:**

- the identifier, team and cdhash comparisons;
- the requirement string;
- the opaque identity hash;
- the parent pid check;
- exit code 4 mapping to `operator_native_auth_not_configured`.

**Privacy fences:**

- The secret moves only through a pipe and is at most 16 KiB.
- The helper must be the broker's direct child.
- Linux and Windows answer `native_auth_unavailable` without spawning anything.

### R6-5 AI tool executor, catalog and projections

R6-5a is done on branch `t3code/r6-5a-ai-tools`; the
[checkpoint](rust-shell.md#ai-tool-executor-and-read-tools-r6-5a) records its
verification, the read tools still refused and what is left for R6-5b. On that
branch the chain-analysis kinds are already native, so §2's chain-analysis row
no longer blocks their read tools. R6-5b is done on `t3code/r6-5b-ai-tools`;
its [checkpoint](rust-shell.md#ai-tool-writes-previews-and-projections-r6-5b)
lists the tools that still refuse and what chat still needs. §2 and
Appendix A record the census when this design was written.

Two runs:

- **R6-5a:** the framework and the read tools.
- **R6-5b:** the mutating tools, consent previews and the remaining projections.

**Scope:**

- **The catalog in `crates/ai`:**
  - All 161 definitions as generated data, embedded in the binary: names, wire
    names, descriptions, schemas in their literal key order, `kind_class`,
    `egresses`, daemon kinds and summary templates.
  - A drift test against Python, the way `capabilities.json` is checked today.
- **Tool selection:** capability packs, the keyword rules and the
  review-checkpoint rule (`tools.py:3524`–`3640`).
- **The schema validator,** keeping Python's check order and its `re.search`
  semantics, translated for `fancy-regex`.
- **Redaction:** `redact_tool_arguments` and `redact_ai_tool_result`, reusing
  the redaction already in `kassiber-envelope`.
- **`summarize_tool_call`** templates.
- **The consent state machine.**
- **The read and mutating executors.** Database work is marshalled to the
  daemon's request loop, and every scoped operation gets the scope and
  stale-context check.
- **The auto-read planner and context trimming.**
- **Provenance and the privacy receipt.**
- **Skill references,** embedded from `crates/ai/skill_references/`.
- **The projections** in §1.7. The `ca-ref:` handles reuse the existing
  registry.
- **The custody AI audit writer.**
- **The AI-path parameters** on native routes listed in §2.

A test-harness kind, `test.ai_tool`, runs any tool with a given executor state,
so each tool can be proven on its own before chat is registered.

**Oracle:**

- The catalog, consent and redaction parts of `test_ai_core.py`.
- `test_chain_analysis_ai.py`, `test_custody_ai_audit.py`,
  `test_review_cli_ai.py`, `test_cli_review_consent.py`.
- The catalog and schema classes in `test_mcp_server.py`.
- A new driver, `tests/test_rust_ai_tools.py`, compares the Python executor
  through a reference wrapper with `test.ai_tool`, per tool and per fixture
  book, on separate copies. It compares the sorted compact bytes the model sees.

**Mutation targets:**

- the validator's check order;
- the `$` behaviour before a trailing newline;
- the sensitive-key lists;
- the path-redaction cut-off;
- the once-only set and the local and on-device gates;
- the advertised check;
- the stale-context comparison;
- code-point size limits;
- the 300 s consent timeout;
- duplicate call ids.

**Privacy fences:**

- No tool runs unless it was advertised that turn.
- Egressing tools run only after consent, through a route bound to that consent.
- Once-only grants are never remembered.
- Results are redacted before they leave the executor.
- On-device-only tools are never offered to other providers.
- There is no shell, filesystem, descriptor or configuration tool.
- Skill references come only from the allowlist.

### R6-6 Providers, HTTP streaming, broker client and discovery

Two runs:

- **R6-6a:** provider configuration, the HTTP transport and discovery.
- **R6-6b:** the provider broker client.

R6-6a is done on branch `t3code/r6-6a-providers`; the
[checkpoint](rust-shell.md#ai-providers-http-streaming-and-discovery-r6-6a)
records its verification, its reviewed differences and what is left for R6-6b.
R6-6b is done on branch `t3code/r6-6b-broker-client`; its
[checkpoint](rust-shell.md#ai-provider-broker-client-r6-6b) records the
process route, the parity drivers, the mutation entries and the reviewed
differences. §7 item 3 is settled as recommended (observer only, no ledger
row); item 8 uses the recommended default until R7 packaging settles it.
§7 item 4 is settled as recommended: 64 MiB per response, 16 MiB per SSE line.

**R6-6a scope:**

- **Shared provider store in `crates/ai`.** The document-import provider code
  moves from `crates/wallets` into `crates/ai`, so one store serves both:
  seeding, resolution, gates, output redaction.
- **Provider kinds and leaves:** the 10 `ai.providers.*` kinds and 7 of the
  `ai providers` leaves.
- **Native keys** through the existing control channel: `set`, `get`,
  `delete`, `exists`.
- **The discovery cache:**
  - in RAM, with a 60 s TTL;
  - one probe per key, shared by concurrent waiters;
  - last-good results returned as stale;
  - `checked_at` with microseconds.
- **Model-metadata sanitising.**
- **`ai.provider_runtime.status`, `ai.list_models` and `ai.test_connection`,**
  plus `ai providers test` and `ai models`.
- **The streaming HTTP client:**
  - a streaming response reader in `kassiber-net`, with incremental chunked
    decoding and a bounded line reader;
  - cancellation by shutting down the socket;
  - the SSE splitter, keeping Python's quirks (§7);
  - Responses and Chat Completions stream parsers and tool-call accumulation;
  - format detection, the retry relaxations and error mapping.

**R6-6b scope:**

- **A gated process route in `kassiber-net`** (`Subsystem::Ai`, `Operation::Process`).
  It requires consent and the online gate, as `require_online` does today.
- **Node spawn** in a new process group, with the `XDG_RUNTIME_DIR` temp-root checks.
- **The broker exchange:** JSONL commands and events, keep-alive between tool
  calls, per-leg watchdogs, and the TERM-then-KILL sequence.
- **The same three environment overrides,** so the Python fake brokers keep working.

**Oracle:**

- `test_ai_core_http_transport.py` (62), `test_ai_core_api_format.py` (65),
  `test_ai_core_provider_settings.py` (35), `test_ai_discovery_cache.py` (3),
  `test_ai_broker.py` (24) and `test_ai_fast_mode.py` (18). Parser and
  transport units become Rust tests.
- Daemon and CLI scenarios run Python and Rust against the same loopback
  `ScriptedProvider` scripts and the same fake broker.
- The hybrid candidate serves the `ai` leaves.

**Mutation targets:**

- missing-route phrase detection;
- relaxations happening at most once;
- the status-code mapping;
- the same-origin redirect rule;
- the acknowledgement gates;
- local-kind inference;
- the discovery TTL;
- each SSE quirk;
- the `store` refusal regex.

**Privacy fences:**

- Listing, reading and editing providers contacts nothing.
- Contact happens only on the explicit kinds listed in §5.
- Offline mode blocks everything except a local loopback provider.
- Keys never appear in argv or logs.
- Error bodies use the 512-character redacted preview.
- Each broker spawn is gated and seen by the attempt observer.

### R6-7 Chat

R6-7 is done on branch `t3code/r6-7-chat`; the
[checkpoint](rust-shell.md#ai-chat-r6-7) records its verification and the
reviewed differences. §7 item 1 is followed: the chat kinds and leaves are
compiled behind the `native-chat` feature, which only test-harness builds
enabled, and the checkpoint lists what had to be native before product
builds turned it on; [native chat](rust-native-chat.md) removed the gate.
§7 item 11 is settled as recommended.

**Scope:**

- **`ai.chat`:**
  - a worker thread, with database work on the request loop;
  - record order;
  - the tool loop with consent;
  - call ids;
  - provider session cursors and the history fingerprint;
  - the persistence policy;
  - provenance.
- **`ai.chat.cancel`,** including early cancel, plus `ai.tool_call.consent`.
- **History kinds:** the four `ui.chat.sessions.*` kinds and `ui.chat.history.configure`.
- **`kassiber chat`:**
  - re-executes the binary as a daemon child, using the `daemon.unlock` bootstrap;
  - rendering, `--stream-json`, `--machine`, the REPL;
  - consent prompts and transcripts.
- **The five `chats` leaves.**

**Oracle:**

- `test_cli_chat.py` (41): its 30 boundary-fixture tests run unchanged through
  `KASSIBER_TEST_CLI`.
- The chat parts of `test_ai_core.py`, and the 79 AI tests in
  `test_daemon_smoke.py`.
- A stream driver compares the whole JSONL output of Python and Rust daemons
  byte for byte. Both run against the same scripted provider and fake broker.

**Mutation targets:**

- record order;
- consent registered before the tool-call record is written;
- cancel becoming a terminal `cancelled`;
- the persistence policy, where `auto` means encrypted books only;
- the 16-iteration budget rule;
- the cross-book phrase match.

**Privacy fences:**

- Nothing is contacted before the user's message.
- History is stored only under the policy.
- The privacy receipt counts what was sent.
- Cancelling revokes the turn's network consent (`Consent::revoke`).

### R6-8 MCP server

Done on branch `t3code/r6-8-mcp`; the
[checkpoint](rust-shell.md#mcp-server-and-cooperative-chat-ctrl-c-r6-8)
records its verification, the reviewed differences and what is left. §7
item 1 is followed as for chat: `mcp serve` and `mcp call` register behind
the `native-mcp` feature until every MCP tool was native (removed by
[native chat](rust-native-chat.md)), while `mcp tools`,
`status`, `enable` and `disable` need no tool and always register. The same
run gave `kassiber chat` its cooperative Ctrl-C.

**Scope:**

- **`mcp serve`:**
  - the stdio claim;
  - both protocol eras;
  - the single tool worker;
  - limits and cancellation;
  - the 30 s drain at EOF.
- **Definitions** derived from the catalog.
- **The direct path:**
  - `query_only`;
  - a per-call scope pin that never writes the active context;
  - opening and closing the book per call.
- **The brokered path:**
  - the operator client;
  - agent sessions with 10 s heartbeats;
  - polling every 0.1 s;
  - cancelling queued work.
- **`mcp tools`, `status`, `enable`, `disable` and `call`.**

The direct path can land after R6-5a. The brokered path waits for R6-3.

**Oracle:**

- `test_mcp_server.py` (62).
- A new stdio transcript driver compares Python and Rust replies byte for byte
  on the same fixture books.
- The MCP case in `test_operator_integration.py`.

**Mutation targets:**

- era selection;
- the error codes;
- in-flight limits;
- request-id typing (`1` versus `"1"`);
- `result_too_large` on the escaped length;
- the access-gate reason precedence.

**Privacy fences:**

- Launch, `initialize`, `server/discover` and `tools/list` touch no files and
  no sockets, including loopback.
- The server never starts a broker.
- Only the 44 read tools are reachable.

## 4. Dependencies

No new external crate is needed.

| Need | Choice | Why not something else |
| --- | --- | --- |
| JSON-RPC (MCP) and JSONL (broker, daemon) | hand-rolled over `kassiber-pyjson` and `kassiber-pytext` | the bytes must match CPython's `json.dumps`/`json.loads`, which no JSON-RPC crate reproduces |
| Unix sockets | `std::os::unix::net` | — |
| Peer credentials, non-blocking connect, `poll`, `sigaction`, `setsid`, `fcntl` | `libc` (already a reviewed direct dependency of `kassiber-secrets`) | std's `peer_cred` is unstable; `nix` would need new reviewed features |
| Windows named pipes and overlapped I/O | `windows-sys` 0.61 (already direct) plus features `Win32_System_Pipes` and `Win32_System_IO` | std has no named pipes; Tokio's need an async runtime the daemon does not use |
| Windows handle-list inheritance | `windows-sys` `Win32_System_Threading` (already enabled) | std always inherits every inheritable handle and its attribute API is unstable |
| Windows share modes for owner locks | already in `crates/ownership` and `crates/paths` | — |
| Process groups for the provider broker | `std::os::unix::process::CommandExt::process_group` | stable std; `killpg` through `libc` |
| Linux login lifetime | parse `/run/systemd/users/<uid>` and `/proc/self/mountinfo` | avoids `dlopen` of `libsystemd`; see §7 |
| macOS code identity | run `/usr/bin/codesign`, as Python does | `security-framework` would become a direct dependency for no behavioural gain |
| Entropy (tokens, challenges, uuid4) | `getrandom` (direct), `base64` (direct elsewhere), `uuid` (direct in setup) | — |
| HMAC for `ca-ref:` handles | the existing `bitcoin::hashes` HMAC in `crates/wallets` | — |
| Python-`re` patterns | `fancy-regex` (already used by redaction) | — |
| Threads | std threads, `Mutex`, `Condvar`; no async runtime | the daemon and `kassiber-net` are blocking by design |

Every new direct dependency edge and process launch gets an entry in
`crates/net/tests/reviewed-dependencies.txt` and `reviewed-process-calls.txt`.

Process launches to review:

- the provider broker (Node);
- broker children, i.e. the binary re-executed by the runner;
- the detached broker launch;
- the CLI chat daemon child;
- the Touch ID helper;
- `codesign`;
- `/bin/ps` for macOS process labels.

## 5. Egress and privacy

Every path, with the gate it maps to:

| Path | Trigger (the only one) | Rust gate | Ledger |
| --- | --- | --- | --- |
| Provider `GET /models` | `ai.list_models` (cold/expired cache or `refresh`), `ai models`, `ai.test_connection`, `ai providers test` | `kassiber-net` AI route, explicit-trigger consent bound to the request; online gate; local loopback allowed offline | `ai`, as Python |
| Provider `POST /responses`, `/chat/completions` | `ai.chat`, `chat`, connection probes | same | `ai`; body length feeds the privacy receipt |
| Document OCR inference | `wallets preview-document` and the desktop preview (native already) | same | `ai` |
| Provider broker (Node) and its CLIs | `ai.chat`, `ai.list_models`/`ai models` on a CLI row, `ai.provider_runtime.status` without `cached_only`, connection tests | new gated process route; online gate blocks all broker use offline, as Python | none, as Python; the attempt observer records the launch |
| Egressing tools (Lightning reads, chain-analysis acquisition and source runs) | consent for that tool call | the owning R5 route, with consent bound to the call | the owning subsystem |
| Operator broker socket or pipe | operator leaves, brokered commands, desktop agent unlock, MCP calls on brokered books | local IPC in `crates/ipc`, not a network route | none |
| Broker children | `submit` | each child runs its own gate; `KASSIBER_NO_EGRESS` propagated | none in the parent |
| Touch ID helper, `codesign`, `/bin/ps` | Touch ID actions; agent-session labels on macOS | local processes | none |
| CLI chat daemon child | `kassiber chat` | child has its own gate | none |
| MCP | stdio only | none needed | none |
| Desktop keychain | provider key operations | the existing daemon control channel | none |

There are no probes on launch, idle or status:

- No AI code runs at daemon start.
- `ai.providers.list`/`get` only read, seed rows, and ask the desktop keychain over stdio.
- `mcp status` reads one file.
- `operator status`, `lock` and `stop` only connect to an existing local endpoint.
- `ui.agent_access.status` stays in memory unless `refresh` is set and a lease exists.
- The desktop agent-lease tick checks the broker every 30 s only while the user's lease is live.
- `ai.provider_runtime.status` with `cached_only` never probes, and the desktop
  sends that flag when the settings page opens.

Two Python behaviours look like probes but are bound to an explicit action:

- `ai.list_models` contacts a provider on a cold cache without `refresh`.
- `ai.provider_runtime.status` without `cached_only` contacts the CLI runtimes.

The desktop sends both only on "Check models" or "Refresh". Rust keeps this
and pins it with regressions.

`KASSIBER_NO_EGRESS` does not cover AI and update traffic in Python, which
uses `require_online` instead. `kassiber-net` already matches this
(`crates/net/src/lib.rs:384`). This commit also corrects plan 21, which said
Rust offline mode blocks local AI. It permits an explicitly local loopback
provider, as Python does.

How each slice proves no traffic before consent:

- **R6-1 to R6-4:**
  - Scenarios run under the strict attempt observer (`KASSIBER_TEST_NO_EGRESS=strict`, loopback included).
  - The chokepoint scanner proves the only socket constructors are in `crates/ipc` and are local.
  - CI's network-namespace lane is the second layer.
- **R6-5:**
  - Each read tool runs under the strict observer.
  - For each egressing tool, a test shows zero attempts before consent and
    exactly the bound request after.
  - A remote-provider test shows on-device-only tools are neither advertised nor executable.
- **R6-6:**
  - Provider CRUD, `cached_only` status and warm-cache listing show zero
    attempts and zero process launches.
  - Offline mode refuses before connecting.
  - A test pins that `KASSIBER_NO_EGRESS` leaves AI alone, as in Python.
  - The fake broker proves no spawn happens for an empty runtime list.
- **R6-7:**
  - A cancel before the provider call leaves zero attempts.
  - A tools-off chat against the loopback fake makes exactly the expected requests.
- **R6-8:**
  - The port of `test_launch_listing_and_local_reads_open_no_socket_even_loopback`
    covers launch and listing.
  - A brokered call with no broker running makes no connection beyond the
    status probe and never launches one.

## 6. Verification plan

**Byte parity.**

- Daemon records and stream chunks are compared as raw bytes, including line
  ends, the way `tests/test_rust_shell_parity.py` compares responses.
- Newly generated identities are compared through declared mappings, not
  masked: chat session and message uuid4s, `mcp-` call ids, and broker
  generations and tokens.
- Clocks and entropy come from the existing `KASSIBER_RUST_TEST_*` seams on
  the Rust side, and from a patched Python reference wrapper on the other.
  `tests/test_rust_document_cli.py` already does this for its AI provider.

**Running the existing tests against Rust.**

- **Boundary fixtures and subprocess scenarios that spawn through
  `tests/cli_runner.py`** run unchanged with `KASSIBER_TEST_CLI` naming the
  hybrid candidate:
  - `test_cli_chat.py`;
  - the CLI classes of `test_ai_core_api_format.py` and `test_ai_core_provider_settings.py`;
  - `test_review_cli_ai.py`;
  - the AI group of `test_daemon_smoke.py`.

  The hybrid log must show that the slice's own commands were served by Rust.
- **Daemon sessions are all or nothing** in the hybrid
  (`KASSIBER_HYBRID_RUST_DAEMON=1`). R6 daemon scenarios must therefore set up
  books with native kinds or seeded SQL only.
- **The subprocess-scenario tests** (131 across these modules) that today
  call the daemon or CLI in process are rewritten as subprocess drivers, as
  their disposition says.
- **The 573 rust-test entries** become Rust tests in the owning crate.
- **`test_operator_integration.py`** gets a broker switch. Its fixture can start
  either `tests/operator_server_fixture.py` or the Rust binary with
  `--operator-broker-server`. The Rust binary needs test-harness equivalents
  for two fixture overrides:
  - the gated runtime-directory override;
  - the logind override.
- **Mixed brokers.** Python and Rust brokers refuse each other's secrets by
  design (different build origin). The cross-language checks therefore stop at
  `ping` and `status`, and each broker is otherwise tested with its own client.

**New differential drivers:**

- `tests/test_rust_ai_tools.py`: per-tool executor parity (R6-5).
- `tests/test_rust_ai_providers.py`: provider kinds and leaves, discovery, and
  connection tests against loopback fakes (R6-6).
- `tests/test_rust_ai_chat.py`: whole `ai.chat` streams, cancellation and
  consent (R6-7).
- `tests/test_rust_mcp.py`: stdio transcripts (R6-8).
- `tests/test_rust_operator.py`: brokered command envelopes, lease and queue
  behaviour, agent sessions (R6-2, R6-3).

**Fakes.**

- Provider fakes are the existing `ScriptedProvider` (a `ThreadingHTTPServer`
  on `127.0.0.1:0`) and `_ToolChatHandler`. They run on loopback only, with the
  non-strict observer.
- The fake broker is a Python script selected by `KASSIBER_AI_BROKER_NODE`
  and `KASSIBER_AI_PROVIDER_BROKER`. The Rust client honours both.
- No test starts a real provider, Node CLI or network.

**Recordings.** The session harness has no network or subprocess tapes, and
live fakes in the test process are absent on replay. Recorded sessions are
used only for provider-free surfaces:

- provider configuration;
- chat history;
- agent access;
- `mcp status`/`tools`.

Streams are compared live against the fakes.

**Catalog and text goldens.** These are generated from Python and pinned by
sha256, with drift tests while Python exists:

- the 161-tool catalog;
- the MCP definitions (38,378 bytes at this commit);
- `INSTRUCTIONS`;
- `DEFAULT_KASSIBER_SYSTEM_PROMPT`;
- the skill references.

**Done means:**

- the plan-21 slice checklist passes;
- the slice's served-by counts show no fallback;
- `python -m tests.rust_port.classify_tests update` and the `preflight` shard pass.

## 7. Risks and owner decisions

Parity that is impossible or undesirable, with the recommendation for each:

1. **When chat and MCP are registered.**
   - Chat can only match Python once all 161 tools work, and 81 wait on R5.
   - Recommendation: register `ai.chat`, `chat` and the `ui.chat` kinds in
     product builds only when the whole catalog is native; until then,
     register them only in `test-harness` builds.
   - Register MCP when its 44 tools are native (18 wait on R5).
   - Register operator, provider and history surfaces as soon as their slices
     pass, since they have no R5 dependency.
   - **Owner decision.**
2. **Python and Rust brokers on one machine.**
   - Both use the one per-user endpoint, and each refuses the other's work as
     a different build. On the planned side-by-side macOS test app, whichever
     broker starts first serves, and `kassiber operator stop` switches.
   - Recommendation: keep parity and document this. A separate Rust endpoint
     would change the protocol contract.
   - **Owner decision.**
3. **Provider-broker launches in the egress ledger.**
   - Python records no ledger row for the Node broker or its CLIs.
   - Recommendation: match Python now (observer only); propose a ledger row
     later as a visible improvement.
   - **Owner decision.**
4. **Response size.**
   - Python reads AI responses without a bound. `kassiber-net` caps bodies at
     16 MiB by default and 64 MiB at most.
   - Recommendation: a 64 MiB cap per streamed response and 16 MiB per SSE
     line, recorded as a reviewed difference. Exact parity would mean unbounded
     memory.
5. **`unsafe` code.**
   - Named pipes, handle lists, peer credentials and signals need it.
   - Recommendation: confine it to the new `crates/ipc`, with every other new
     crate forbidding `unsafe`.
   - **Owner decision**, because it changes the workspace's lint posture.
6. **Linux login lifetime.**
   - Python calls `sd_uid_get_state` from `libsystemd` via ctypes.
   - Recommendation: read `/run/systemd/users/<uid>` directly. That is the
     file `sd_uid_get_state` parses, including its missing-file answer of
     `offline`, so this avoids `dlopen`.
   - Keep "systemd not running" as "unknown", and pin the mapping with vectors
     taken from systemd's source.
7. **Build identity of the Rust binary.**
   - Python reports `frozen` and an `origin` hashed from the executable path.
   - Recommendation:
     - release builds report `frozen: true` with the same `origin` rule;
     - `test-harness` builds report `frozen: false`, so the gated test
       runtime directory works.
8. **Where the binary finds the provider broker bundle.**
   - Python ships `index.mjs` as package data next to `kassiber/ai/`.
   - Recommendation: default to `<executable dir>/provider_broker/index.mjs`,
     with `KASSIBER_AI_PROVIDER_BROKER` still overriding. Settle it with R7
     packaging.
   - **Owner decision** together with packaging. Superseded by
     [plan 23](../plan/23-rust-provider-broker.md): the binary is the broker.
9. **Skill references and the catalog are embedded** in the binary. Python
   reads them from disk, never from the working directory. Nothing visible
   changes. At retirement the generated catalog file becomes the source of
   truth.
10. **Python quirks kept for parity.** The recommendation is to keep each; none
    is a privacy problem:
    - SSE `data:` loses all leading spaces, and a bare CR does not end a line.
    - Schema patterns accept a value followed by a trailing newline.
    - Key-substring redaction blanks harmless fields such as `authored_source`
      and `seed_count` in AI results.
    - `timeout_seconds` accepts numeric strings.
    - Unknown `ai.chat` arguments are ignored.
    - Decimals become floats in tool results.
    - The 8 MiB and 4 MiB limits are measured on ASCII-escaped length.
11. **A latent Python bug.** `kassiber/cli/chat.py:629` references an undefined
    `provider_name` when a provider row has no name. Rows always have one, so it
    is unreachable. Recommendation: Rust uses the row's name; fix the reference
    in its own commit only if a test can reach it.
12. **Consent summaries** put model-supplied strings into the text shown to the
    user without redaction (`tools.py:3731`). This is local display only.
    Recommendation: keep parity.
13. **Re-entrant locks.** The broker service relies on Python's re-entrant
    locks. The Rust service needs restructured locking, which is a correctness
    risk rather than a parity question. R6-2's mutation entries and the ported
    96 service tests cover it.
14. **Hard dependency on R5 progress.**
    - Chain analysis is the largest block (41 tools plus `ai_context`) and has
      no owner yet.
    - Recommendation: assign it before R6-5b, or chat registration slips with it.
    - **Owner decision.**

## Appendix A: AI tool census

Columns:

- **Python:** where the tool is defined. `chain_analysis_tools.py` and
  `chain_analysis_sources_tools.py` are in `kassiber/ai/`.
- **Class:** `kind_class`, plus egress.
- **Gates:**
  - `once`: once-only consent;
  - `local`: local provider only;
  - `on-device`: on-device provider only;
  - `refresh`: an automatic journal refresh before the read, disabled under MCP.
- **Exposure:**
  - `core`: the default `kassiber chat` profile;
  - `MCP`;
  - `auto`: may be auto-read without consent.

  Every tool is reachable from desktop chat with the right capability pack.
- **Native kind:**
  - `yes`: the tool's daemon kind is registered in Rust;
  - `yes, AI path gaps`: as `yes`, but the AI-only behaviour in §2 is missing;
  - `partial`;
  - `no`.

  No tool's AI executor path is native yet.

| # | Tool | Python | Class | Gates | Exposure | Native kind |
| ---: | --- | --- | --- | --- | --- | --- |
| 1 | `status` | tools.py:469 | read | — | core, MCP | yes |
| 2 | `ui.overview.snapshot` | tools.py:477 | read | refresh | core, MCP | no |
| 3 | `ui.transactions.list` | tools.py:490 | read | — | core, MCP, auto | yes |
| 4 | `ui.transactions.extremes` | tools.py:594 | read | — | core, MCP, auto | yes |
| 5 | `ui.transactions.search` | tools.py:634 | read | — | core, MCP, auto | yes |
| 6 | `ui.wallets.list` | tools.py:693 | read | — | core, MCP, auto | yes |
| 7 | `ui.wallets.utxos` | tools.py:705 | read | — | — | no |
| 8 | `ui.wallets.identify` | tools.py:730 | read | — | core | no |
| 9 | `ui.wallets.analyze_file` | tools.py:772 | read | — | core | no |
| 10 | `ui.backends.list` | tools.py:843 | read | — | core, MCP, auto | no |
| 11 | `ui.profiles.snapshot` | tools.py:856 | read | — | core, auto | partial |
| 12 | `ui.reports.capital_gains` | tools.py:868 | read | refresh | core, MCP, auto | no |
| 13 | `ui.reports.summary` | tools.py:877 | read | refresh | core, MCP, auto | yes |
| 14 | `ui.reports.balance_sheet` | tools.py:899 | read | refresh | core, MCP, auto | yes |
| 15 | `ui.reports.portfolio_summary` | tools.py:912 | read | refresh | core, MCP, auto | yes |
| 16 | `ui.reports.lightning_profitability` | tools.py:926 | read + egress | — | — | partial |
| 17 | `ui.connections.node.snapshot` | tools.py:965 | read + egress | — | — | no |
| 18 | `ui.reports.tax_summary` | tools.py:1003 | read | refresh | core, MCP, auto | yes |
| 19 | `ui.reports.balance_history` | tools.py:1027 | read | refresh | core, MCP, auto | yes |
| 20 | `ui.reports.privacy_hygiene` | tools.py:1075 | read | — | auto | yes |
| 21 | `ui.reports.privacy_mirror` | tools.py:1090 | read | — | auto | no |
| 22 | `ui.journals.snapshot` | tools.py:1108 | read | refresh | core, MCP, auto | no |
| 23 | `ui.journals.quarantine` | tools.py:1120 | read | refresh | core, MCP, auto | no |
| 24 | `ui.journals.quarantine.resolve` | tools.py:1159 | mutating | once | core | no |
| 25 | `ui.journals.events.list` | tools.py:1211 | read | — | MCP | no |
| 26 | `ui.journals.transfers.list` | tools.py:1238 | read | refresh | core, MCP, auto | no |
| 27 | `ui.rates.summary` | tools.py:1270 | read | — | core, MCP, auto | yes |
| 28 | `ui.rates.coverage` | tools.py:1279 | read | refresh | core, MCP, auto | yes |
| 29 | `ui.rates.rebuild` | tools.py:1302 | mutating | — | core | no |
| 30 | `ui.report.blockers` | tools.py:1337 | read | refresh | core, MCP, auto | no |
| 31 | `ui.audit.changes_since_last_answer` | tools.py:1351 | read | refresh | core, auto | no |
| 32 | `ui.transactions.history` | tools.py:1372 | read | — | core, MCP | yes |
| 33 | `ui.activity.history` | tools.py:1397 | read | — | — | yes |
| 34 | `ui.maintenance.settings` | tools.py:1424 | read | — | core, MCP, auto | no |
| 35 | `ui.workspace.health` | tools.py:1436 | read | refresh | core, MCP, auto | no |
| 36 | `ui.next_actions` | tools.py:1450 | read | refresh | core, MCP, auto | no |
| 37 | `ui.source_funds.sources.list` | tools.py:1462 | read | — | MCP | yes |
| 38 | `ui.source_funds.links.list` | tools.py:1475 | read | — | MCP | yes |
| 39 | `ui.source_funds.preview` | tools.py:1502 | read | — | — | yes |
| 40 | `read_skill_reference` | tools.py:1519 | read | — | core, MCP, auto | no |
| 41 | `ui.wallets.sync` | tools.py:1540 | mutating | — | core | yes, AI path gaps |
| 42 | `ui.journals.process` | tools.py:1559 | mutating | — | core | no |
| 43 | `ui.maintenance.configure` | tools.py:1571 | mutating | once | — | no |
| 44 | `ui.maintenance.run` | tools.py:1617 | mutating | — | core | no |
| 45 | `ui.source_funds.sources.create` | tools.py:1642 | mutating | — | — | yes |
| 46 | `ui.source_funds.links.create` | tools.py:1672 | mutating | — | — | yes |
| 47 | `ui.source_funds.links.review` | tools.py:1732 | mutating | — | — | yes |
| 48 | `ui.source_funds.suggest` | tools.py:1776 | mutating | — | — | yes |
| 49 | `ui.source_funds.links.bulk_review` | tools.py:1804 | mutating | — | — | yes |
| 50 | `ui.transfers.suggest` | tools.py:1837 | read | — | core, MCP, auto | yes |
| 51 | `ui.custody.coverage.snapshot` | tools.py:1903 | read | local | core, auto | no |
| 52 | `ui.custody.lineage.snapshot` | tools.py:1920 | read | local | core, auto | no |
| 53 | `ui.custody.gaps.list` | tools.py:1958 | read | local | core, auto | yes |
| 54 | `ui.custody.gaps.review_context` | tools.py:1985 | read | local | core | yes |
| 55 | `ui.custody.gaps.history` | tools.py:2014 | read | local | core | yes |
| 56 | `ui.custody.review.plan` | tools.py:2036 | read | local | core | yes, AI path gaps |
| 57 | `ui.custody.review.apply` | tools.py:2049 | mutating | once, local | core | partial |
| 58 | `ui.transfers.review_context` | tools.py:2061 | read | refresh | core, MCP, auto | no |
| 59 | `ui.transfers.list` | tools.py:2127 | read | — | core, MCP, auto | yes |
| 60 | `ui.transfers.payouts.list` | tools.py:2139 | read | — | auto | yes |
| 61 | `ui.transfers.components.list` | tools.py:2151 | read | — | core | yes |
| 62 | `ui.transfers.components.plan` | tools.py:2180 | read | — | core | yes |
| 63 | `ui.transfers.components.apply` | tools.py:2219 | mutating | once | core | yes, AI path gaps |
| 64 | `ui.transfers.payouts.create` | tools.py:2250 | mutating | — | — | yes, AI path gaps |
| 65 | `ui.transfers.payouts.delete` | tools.py:2296 | mutating | — | — | yes, AI path gaps |
| 66 | `ui.transfers.update` | tools.py:2310 | mutating | — | — | yes, AI path gaps |
| 67 | `ui.transfers.rules.list` | tools.py:2329 | read | — | core, MCP, auto | yes |
| 68 | `ui.saved_views.list` | tools.py:2341 | read | — | auto | yes |
| 69 | `ui.transfers.pair` | tools.py:2362 | mutating | — | core | yes, AI path gaps |
| 70 | `ui.transfers.unpair` | tools.py:2453 | mutating | — | — | yes, AI path gaps |
| 71 | `ui.transfers.bulk_pair` | tools.py:2473 | mutating | — | — | yes, AI path gaps |
| 72 | `ui.transfers.dismiss` | tools.py:2506 | mutating | — | — | yes, AI path gaps |
| 73 | `ui.transfers.rules.create` | tools.py:2532 | mutating | — | — | yes, AI path gaps |
| 74 | `ui.transfers.rules.delete` | tools.py:2564 | mutating | — | — | yes, AI path gaps |
| 75 | `ui.transfers.rules.set_enabled` | tools.py:2578 | mutating | — | — | yes, AI path gaps |
| 76 | `ui.transfers.rules.apply` | tools.py:2595 | mutating | — | — | yes, AI path gaps |
| 77 | `ui.saved_views.create` | tools.py:2608 | mutating | — | — | yes |
| 78 | `ui.saved_views.delete` | tools.py:2629 | mutating | — | — | yes |
| 79 | `ui.workspace.overview.snapshot` | tools.py:2654 | read | — | — | no |
| 80 | `ui.transactions.resolve` | tools.py:2672 | read | — | core, MCP | no |
| 81 | `ui.transactions.graph` | tools.py:2686 | read | — | core, MCP | no |
| 82 | `ui.transactions.review_context` | tools.py:2703 | read | refresh | core, MCP | no |
| 83 | `ui.activity.stale` | tools.py:2727 | read | — | — | yes |
| 84 | `ui.attachments.list` | tools.py:2736 | read | — | core, MCP | yes |
| 85 | `ui.audit.evidence.summary` | tools.py:2756 | read | refresh | core | no |
| 86 | `ui.review.badges` | tools.py:2779 | read | — | — | no |
| 87 | `ui.review.worklist` | tools.py:2788 | read | refresh | core, MCP, auto | no |
| 88 | `ui.loans.list` | tools.py:2813 | read | — | MCP, auto | yes, AI path gaps |
| 89 | `ui.loans.mark` | tools.py:2831 | mutating | — | — | yes |
| 90 | `ui.loans.link` | tools.py:2853 | mutating | — | — | yes |
| 91 | `ui.loans.unmark` | tools.py:2876 | mutating | — | — | yes |
| 92 | `ui.transactions.metadata.update` | tools.py:2893 | mutating | — | — | yes, AI path gaps |
| 93 | `ui.transactions.history.revert` | tools.py:2931 | mutating | — | — | yes |
| 94 | `ui.attachments.copy` | tools.py:2950 | mutating | — | — | yes |
| 95 | `ui.source_funds.evidence.list` | tools.py:2968 | read | — | — | yes |
| 96 | `ui.source_funds.sources.attach` | tools.py:2984 | mutating | — | — | yes |
| 97 | `ui.source_funds.links.attach` | tools.py:3004 | mutating | — | — | yes |
| 98 | `ui.source_funds.coverage` | tools.py:3024 | read | — | MCP | yes |
| 99 | `ui.source_funds.cases.list` | tools.py:3040 | read | — | MCP | yes |
| 100 | `ui.source_funds.review_context` | tools.py:3049 | read | — | — | yes |
| 101 | `ui.source_funds.request_input` | tools.py:3057 | read | — | — | yes |
| 102 | `ui.source_funds.assemble` | tools.py:3072 | mutating | — | — | yes |
| 103 | `ui.source_funds.cases.save` | tools.py:3093 | mutating | — | — | yes |
| 104 | `ui.source_funds.export` | tools.py:3107 | mutating | — | — | no |
| 105 | `ui.transactions.commercial_context` | tools.py:3123 | read | — | — | yes |
| 106 | `ui.btcpay.provenance.list` | tools.py:3132 | read | — | — | yes |
| 107 | `ui.btcpay.provenance.suggest` | tools.py:3148 | mutating | — | — | yes |
| 108 | `ui.btcpay.provenance.links` | tools.py:3169 | read | — | — | yes |
| 109 | `ui.documents.list` | tools.py:3185 | read | — | core, MCP | yes |
| 110 | `ui.btcpay.provenance.review` | tools.py:3198 | mutating | — | — | no |
| 111 | `ui.documents.create` | tools.py:3218 | mutating | — | — | yes |
| 112 | `ui.reports.exit_tax_preview` | tools.py:3243 | read | refresh | MCP | yes |
| 113 | `ui.rates.latest` | tools.py:3260 | mutating | — | — | yes |
| 114 | `ui.reports.export` | tools.py:3282 | mutating | — | — | partial |
| 115 | `ui.egress.snapshot` | tools.py:3312 | read | — | — | no |
| 116 | `ui.review.cases` | tools.py:3395 | read | — | core, MCP | no |
| 117 | `ui.review.request_input` | tools.py:3403 | read | — | core | no |
| 118 | `ui.review.plan` | tools.py:3416 | read | — | core, MCP | no |
| 119 | `ui.review.apply` | tools.py:3424 | mutating | once | core | no |
| 120 | `ui.review.receipt` | tools.py:3432 | read | — | core, MCP | yes, AI path gaps |
| 121 | `ui.chain_analysis.query` | chain_analysis_tools.py:75 | read | — | — | no |
| 122 | `ui.chain_analysis.entropy` | chain_analysis_tools.py:76 | read | — | — | no |
| 123 | `ui.chain_analysis.entropy.start` | chain_analysis_tools.py:77 | read | — | — | no |
| 124 | `ui.chain_analysis.jobs.get` | chain_analysis_tools.py:78 | read | — | — | no |
| 125 | `ui.chain_analysis.jobs.cancel` | chain_analysis_tools.py:79 | read | — | — | no |
| 126 | `ui.chain_analysis.psbt.analyze` | chain_analysis_tools.py:80 | read | — | — | no |
| 127 | `ui.chain_analysis.psbt.compare` | chain_analysis_tools.py:81 | read | — | — | no |
| 128 | `ui.chain_analysis.psbt.entropy.start` | chain_analysis_tools.py:82 | read | — | — | no |
| 129 | `ui.chain_analysis.datasets.list` | chain_analysis_tools.py:83 | read | — | — | no |
| 130 | `ui.chain_analysis.datasets.get` | chain_analysis_tools.py:84 | read | — | — | no |
| 131 | `ui.chain_analysis.datasets.query` | chain_analysis_tools.py:85 | read | — | — | no |
| 132 | `ui.chain_analysis.datasets.preview` | chain_analysis_tools.py:86 | read | on-device | — | no |
| 133 | `ui.chain_analysis.datasets.import` | chain_analysis_tools.py:87 | mutating | once, on-device | — | no |
| 134 | `ui.chain_analysis.datasets.revoke` | chain_analysis_tools.py:88 | mutating | — | — | no |
| 135 | `ui.chain_analysis.cases.list` | chain_analysis_tools.py:89 | read | — | — | no |
| 136 | `ui.chain_analysis.cases.get` | chain_analysis_tools.py:90 | read | — | — | no |
| 137 | `ui.chain_analysis.cases.compare` | chain_analysis_tools.py:91 | read | — | — | no |
| 138 | `ui.chain_analysis.cases.save` | chain_analysis_tools.py:92 | mutating | — | — | no |
| 139 | `ui.chain_analysis.cases.delete` | chain_analysis_tools.py:93 | mutating | — | — | no |
| 140 | `ui.chain_analysis.labels.list` | chain_analysis_tools.py:94 | read | — | — | no |
| 141 | `ui.chain_analysis.labels.upsert` | chain_analysis_tools.py:95 | mutating | — | — | no |
| 142 | `ui.chain_analysis.labels.delete` | chain_analysis_tools.py:96 | mutating | — | — | no |
| 143 | `ui.chain_analysis.acquire.plan` | chain_analysis_tools.py:97 | read | on-device | — | no |
| 144 | `ui.chain_analysis.acquire.apply` | chain_analysis_tools.py:98 | mutating + egress | once, on-device | — | no |
| 145 | `ui.chain_analysis.watches.preview` | chain_analysis_tools.py:102 | read | — | — | no |
| 146 | `ui.chain_analysis.watches.create` | chain_analysis_tools.py:103 | mutating | — | — | no |
| 147 | `ui.chain_analysis.watches.list` | chain_analysis_tools.py:104 | read | — | — | no |
| 148 | `ui.chain_analysis.watches.inbox` | chain_analysis_tools.py:105 | read | — | — | no |
| 149 | `ui.chain_analysis.watches.configure` | chain_analysis_tools.py:106 | mutating | — | — | no |
| 150 | `ui.chain_analysis.watches.delete` | chain_analysis_tools.py:107 | mutating | — | — | no |
| 151 | `ui.chain_analysis.watches.acknowledge` | chain_analysis_tools.py:108 | mutating | — | — | no |
| 152 | `ui.chain_analysis.watches.evaluate` | chain_analysis_tools.py:109 | mutating | — | — | no |
| 153 | `ui.chain_analysis.datasets.discard` | chain_analysis_tools.py:111 | mutating | — | — | no |
| 154 | `ui.chain_analysis.datasets.preview.start` | chain_analysis_tools.py:112-114 (derived) | read | on-device | — | no |
| 155 | `ui.chain_analysis.datasets.import.start` | chain_analysis_tools.py:112-114 (derived) | mutating | once, on-device | — | no |
| 156 | `ui.chain_analysis.datasets.discard.start` | chain_analysis_tools.py:112-114 (derived) | mutating | — | — | no |
| 157 | `ui.chain_analysis.sources.plan` | chain_analysis_sources_tools.py:14 | read | on-device | — | no |
| 158 | `ui.chain_analysis.sources.authorize` | chain_analysis_sources_tools.py:15 | mutating | once, on-device | — | no |
| 159 | `ui.chain_analysis.sources.list` | chain_analysis_sources_tools.py:16 | read | on-device | — | no |
| 160 | `ui.chain_analysis.sources.revoke` | chain_analysis_sources_tools.py:17 | mutating | on-device | — | no |
| 161 | `ui.chain_analysis.sources.run` | chain_analysis_sources_tools.py:18 | mutating + egress | once, on-device | — | no |

## Appendix B: MCP tools

The order is the `tools/list` order (`kassiber/mcp/tools.py:24`). Source
lines are in `kassiber/ai/tools.py`. "Native kind" uses Appendix A's meaning.

| # | MCP name | Source tool | Python | Native kind |
| ---: | --- | --- | --- | --- |
| 1 | `status` | `status` | tools.py:469 | yes |
| 2 | `overview_snapshot` | `ui.overview.snapshot` | tools.py:477 | no |
| 3 | `workspace_health` | `ui.workspace.health` | tools.py:1436 | no |
| 4 | `next_actions` | `ui.next_actions` | tools.py:1450 | no |
| 5 | `report_blockers` | `ui.report.blockers` | tools.py:1337 | no |
| 6 | `transactions_list` | `ui.transactions.list` | tools.py:490 | yes |
| 7 | `transactions_extremes` | `ui.transactions.extremes` | tools.py:594 | yes |
| 8 | `transactions_search` | `ui.transactions.search` | tools.py:634 | yes |
| 9 | `transactions_resolve` | `ui.transactions.resolve` | tools.py:2672 | no |
| 10 | `transactions_graph` | `ui.transactions.graph` | tools.py:2686 | no |
| 11 | `transactions_review_context` | `ui.transactions.review_context` | tools.py:2703 | no |
| 12 | `transactions_history` | `ui.transactions.history` | tools.py:1372 | yes |
| 13 | `wallets_list` | `ui.wallets.list` | tools.py:693 | yes |
| 14 | `backends_list` | `ui.backends.list` | tools.py:843 | no |
| 15 | `reports_summary` | `ui.reports.summary` | tools.py:877 | yes |
| 16 | `reports_balance_sheet` | `ui.reports.balance_sheet` | tools.py:899 | yes |
| 17 | `reports_portfolio_summary` | `ui.reports.portfolio_summary` | tools.py:912 | yes |
| 18 | `reports_capital_gains` | `ui.reports.capital_gains` | tools.py:868 | no |
| 19 | `reports_tax_summary` | `ui.reports.tax_summary` | tools.py:1003 | yes |
| 20 | `reports_balance_history` | `ui.reports.balance_history` | tools.py:1027 | yes |
| 21 | `reports_exit_tax_preview` | `ui.reports.exit_tax_preview` | tools.py:3243 | yes |
| 22 | `journals_snapshot` | `ui.journals.snapshot` | tools.py:1108 | no |
| 23 | `journals_quarantine` | `ui.journals.quarantine` | tools.py:1120 | no |
| 24 | `journals_transfers_list` | `ui.journals.transfers.list` | tools.py:1238 | no |
| 25 | `journals_events_list` | `ui.journals.events.list` | tools.py:1211 | no |
| 26 | `rates_summary` | `ui.rates.summary` | tools.py:1270 | yes |
| 27 | `rates_coverage` | `ui.rates.coverage` | tools.py:1279 | yes |
| 28 | `maintenance_settings` | `ui.maintenance.settings` | tools.py:1424 | no |
| 29 | `transfers_suggest` | `ui.transfers.suggest` | tools.py:1837 | yes |
| 30 | `transfers_review_context` | `ui.transfers.review_context` | tools.py:2061 | no |
| 31 | `transfers_list` | `ui.transfers.list` | tools.py:2127 | yes |
| 32 | `transfers_rules_list` | `ui.transfers.rules.list` | tools.py:2329 | yes |
| 33 | `review_cases` | `ui.review.cases` | tools.py:3395 | no |
| 34 | `review_plan` | `ui.review.plan` | tools.py:3416 | no |
| 35 | `review_receipt` | `ui.review.receipt` | tools.py:3432 | yes, AI path gaps |
| 36 | `review_worklist` | `ui.review.worklist` | tools.py:2788 | no |
| 37 | `attachments_list` | `ui.attachments.list` | tools.py:2736 | yes |
| 38 | `documents_list` | `ui.documents.list` | tools.py:3185 | yes |
| 39 | `loans_list` | `ui.loans.list` | tools.py:2813 | yes, AI path gaps |
| 40 | `source_funds_sources_list` | `ui.source_funds.sources.list` | tools.py:1462 | yes |
| 41 | `source_funds_links_list` | `ui.source_funds.links.list` | tools.py:1475 | yes |
| 42 | `source_funds_coverage` | `ui.source_funds.coverage` | tools.py:3024 | yes |
| 43 | `source_funds_cases_list` | `ui.source_funds.cases.list` | tools.py:3040 | yes |
| 44 | `read_skill_reference` | `read_skill_reference` | tools.py:1519 | no (R6 itself) |
