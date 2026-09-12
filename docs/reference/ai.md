# AI Reference

The desktop Assistant and `kassiber chat` use the same provider configuration,
tool loop, and daemon authority. External coding/terminal assistants can use
the [checked-in CLI skill](../../skills/kassiber/SKILL.md).

Accounting, imports, matching, and reports remain deterministic and work without
AI. Models can propose classifications, extract document fields, and help resolve
missing evidence; those outputs require the same validation and review as other
inputs. Opt-in organizational work follows the
[general-accounting contract](general-accounting.md) and its separate disclosure
and mutation approvals.

## HTTP transport

HTTP providers must implement `POST /v1/responses`, typed function calls, and
semantic streaming events. The tool loop replays complete response Items with
matching function results so provider reasoning state is not discarded.

Kassiber deliberately does **not** enable provider-managed conversation state.
Every HTTP request sets `store: false`; system guidance is sent through
`instructions`, and the bounded tool loop replays typed output Items in memory.
Persisted chat history remains a Kassiber/SQLCipher concern and is sent again
as input only when the user resumes a session. This keeps the local-first
storage boundary and also works with providers that implement only stateless
Responses.

The implementation keeps those invariants in one request builder. Prepared
tool-loop context is an explicit typed input and cannot be combined with the
legacy message input accidentally. HTTP transport remains in
`kassiber.ai.client`. Fixed external CLI locators route through the bundled
Node/TypeScript broker in `ui-tauri/provider-broker`; Python
supervises that JSONL process through `kassiber.ai.broker_client`. Shared
delta/request contracts remain isolated in `kassiber.ai.contracts`.

### Building the broker from a source checkout

The broker ships as a single bundled `kassiber/ai/provider_broker/index.mjs`,
built by esbuild from `ui-tauri/provider-broker/src`. It is **not committed** —
release builds produce it before packaging. In a source checkout the CLI-backed
providers report `ai_unavailable` until you build it once:

```bash
pnpm --dir ui-tauri run broker:build
```

`pnpm --dir ui-tauri run build` runs that step first, so a desktop build already
covers it. Point `KASSIBER_AI_PROVIDER_BROKER` at another path to override which
script is spawned.

## Privacy warning

Treat AI prompts as sensitive accounting data.

Depending on what you send, prompts may contain:

- wallet labels
- addresses
- transaction notes
- imported document contents
- backend hostnames
- reconciliation context
- accounting or tax annotations

Normal Kassiber `backends ...` and `wallets ...` output is intentionally
narrowed for secret-bearing config values, but that is not a general privacy
guarantee. Do not paste raw credentials, raw private descriptors, Silent
Payments `sp()` / `spscan` scan material, wallet exports, or other sensitive
material into a remote model unless that is acceptable for your threat model.

If in doubt, keep inference local.

Codex, Claude, and OpenCode CLI providers are supported for convenience, but
they are not a local-privacy guarantee. The broker denies provider-native tools
in layers — Claude loads no user/project/local settings, empties its built-in
tool set, disables slash commands, and accepts only its temporary Kassiber MCP
server; OpenCode serves with `--pure` and a deny-all session permission plus
exact Kassiber MCP allows; Codex runs a read-only sandbox with network access
off — and any tool outside the advertised Kassiber catalog aborts the turn.

Kassiber's own capability-scoped schemas do cross into these providers, through
their native typed-tool protocols: Codex `dynamicTools`, and an ephemeral MCP
server for Claude and OpenCode. Only the schemas and already-redacted results
traverse that bridge, over a unix socket inside a private 0700 directory; the
Python daemon stays the authority for capability selection, argument
validation, consent, execution, and the privacy receipt. Provider-native coding
tools remain disabled throughout. Because these providers route to their own
models, that is the point at which accounting data can leave the device —
choose them deliberately.

Codex exposes no tool-free profile, so a local read there can begin before the
abort lands; with its network disabled that content can only surface through
assistant text on a turn Kassiber is already failing. Kassiber reuses their normal local
authentication/config, telemetry, and model-provider routing without reading,
copying, persisting, or displaying provider credentials. Treat them as
off-device unless the underlying configuration proves local or confidential
inference.

## Provider setup

Local inference is the recommended default.

Kassiber seeds first-class local provider rows for:

- [Ollama](https://ollama.com/) at `http://localhost:11434/v1`
- [oMLX](https://omlx.ai/) at `http://127.0.0.1:8000/v1`

Ollama remains the default provider for compatibility, but oMLX appears as a
built-in local provider and Settings preset. Run the server (`ollama serve`, or
`omlx start` / the oMLX menu-bar app) and use **Test connection** in Settings
before saving a provider change.

Settings **Test connection** checks reachability/model discovery or CLI binary
presence. It does not prove inference, tool calling, or streaming. Verify those
through an explicitly started chat workflow. A Chat Completions-only endpoint
is insufficient for HTTP chat; compatibility depends on the installed server.

If Kassiber itself is running inside a container and Ollama is running on the
host, seed the provider with the Docker host alias instead:

```bash
KASSIBER_DEFAULT_AI_BASE_URL=http://host.docker.internal:11434/v1
```

This only affects first-time provider seeding. For an existing book, update the
`ollama` provider's `base_url` instead.

To make oMLX the default for a brand-new book, set:

```bash
KASSIBER_DEFAULT_AI_PROVIDER=omlx
KASSIBER_DEFAULT_AI_BASE_URL=http://127.0.0.1:8000/v1
```

Per-provider seed overrides are also supported:

```bash
KASSIBER_OMLX_AI_BASE_URL=http://127.0.0.1:8000/v1
KASSIBER_OLLAMA_AI_BASE_URL=http://host.docker.internal:11434/v1
```

Choose a model that supports the required tool or image input contract and
verify it against the intended workflow. The photo/PDF transaction importer
requires a local loopback provider and an installed vision/OCR model. Remote,
TEE, Codex, Claude, and OpenCode CLI providers are hard-disabled for that path.

Codex, Claude, and OpenCode appear automatically through fixed provider
locators:

```bash
codex-cli://default
claude-cli://default
opencode-cli://default
```

No Settings row or API-token entry is required. The broker discovers installed
executables, reports `ready`, `missing executable`, or `authentication
required`, and tells the user to run the provider's normal login command
outside Kassiber when necessary. It uses Codex `app-server`, the Claude
executable's `--output-format stream-json` event stream with strict MCP config,
and the OpenCode SDK v2 against an ephemeral loopback OpenCode server. A local
Node.js executable meeting the [broker package requirements](../../ui-tauri/package.json) is required to run the bundled broker.

Model and reasoning-effort selection are forwarded through each provider's
native protocol.

Model discovery follows each provider's protocol. Where discovery supplies
aliases, forward those aliases rather than maintaining versioned model IDs in
this document. The provider's configured default applies when no model is sent.
Provider session cursors are kept only in daemon memory and
are reused only while the visible Kassiber transcript remains unchanged;
editing or branching starts a clean native provider session.

## In-app surface

The desktop assistant lives at the bottom of every authenticated screen. Its
provider/model picker is fed by `ai.providers.list` and `ai.list_models` over
the daemon protocol; chat streaming is wired through Tauri events
(`daemon://stream`) so the UI can render loading status, reasoning
(`<think>`), and the answer in real time without blocking navigation.

The picker uses a provider rail plus a searchable model list. **Check models**
discovers only the selected provider through `ai.list_models`; checking one
native CLI must not refresh global runtime status or start other providers.
Stored configuration and cached models remain visible without discovery.
First use still requires Kassiber's off-device acknowledgement.

The **All / Local** control filters the inventory by the posture Kassiber can
prove. Model rows show both their source namespace and privacy posture.
OpenCode model IDs identify their source provider (`omlx/...`, `ollama/...`,
`opencode/...`). OpenCode's model list carries no endpoint — `api.url` is empty
for locally served models — so the route is judged by that source provider's
resolved endpoint from `opencode debug config`, reading
`provider.<id>.options.baseURL` and nothing else (the same options block can
hold an `apiKey`, which never leaves the broker). A loopback endpoint (`127.0.0.0/8` matched as a literal IPv4
address, `localhost`, `::1`) is labelled local, because the model cannot leave
the machine. A loopback address that fronts a reverse proxy is the known
limit of this check: it reports local while the proxy may forward inference
off-machine. Everything else stays remote, including LAN addresses and
any provider for which OpenCode reports no endpoint. The posture is proven from
configuration, never guessed from a provider's name.

Provider and model discovery uses a daemon-owned, in-memory last-good cache.
Snapshots include `checked_at`, `stale`, and sanitized `error` metadata, and
concurrent refreshes for the same provider share one probe. Failed refreshes
keep the last successful inventory visible. Opening an AI surface or its model
picker does not contact any provider, including loopback HTTP services and
native CLIs. Refresh requires **Check models** for that provider; testing a
provider in Settings is also an explicit action. Provider configuration changes
invalidate the daemon cache but do not authorize a probe.

Assistant answers, reasoning and restored transcripts render Markdown image
references as inert buttons, with no image or preload resource. Opening an image
requires an explicit click and uses the existing external-browser action; only
absolute HTTP(S) URLs without embedded credentials are accepted. Unsupported
image references remain text. This applies in the native app and the supported
browser development UI, independently of the native Content Security Policy.

Kassiber's HTTP AI client ignores operating-system and environment proxy
settings and rejects redirects outside the configured provider origin. Route a
remote HTTP provider through its explicitly configured base URL. Native CLI
providers retain their own executable transport settings, as described below.

Each broker probe or chat runs from a fresh Kassiber-owned empty temporary
directory that is removed afterward. Provider subprocesses receive only their
own authentication/configuration environment plus the shared network/runtime
minimum; unrelated provider and Kassiber secrets are excluded. Claude gets an
empty built-in tool set, no filesystem setting sources, the file/exec/network
tools named in `--disallowed-tools`, strict MCP config, and an allow-list of
only the advertised `mcp__kassiber__*` names; OpenCode gets a deny-all
permission ruleset with exact allows for the temporary Kassiber MCP tools;
Codex runs read-only and untrusted with network access disabled and
capability-scoped `dynamicTools`. A tool request outside the advertised
Kassiber catalog aborts the turn for Claude and OpenCode; Codex returns a
failed tool result instead, so its turn continues without the tool. The broker receives no repository, database, attachment, wallet,
browser, terminal, or source-control capability, and no ability to widen its
own catalog: it forwards a tool call to the daemon and waits.

Claude and OpenCode reach Kassiber through a child MCP process, and the bridge
carrying its calls is a unix socket inside a private 0700 directory removed
with the turn. A loopback TCP port would be reachable by every local process
and would need a shared secret; the only place to hand one to the child is
`argv`, which is world-readable on Linux, so filesystem permissions replace the
secret. Codex needs no child process — its adapter answers `item/tool/call`
in-process and never touches the socket. On Windows the bridge is a named pipe
with an unguessable name in the global pipe namespace rather than a file in the
private directory, so it is protected by the default per-logon pipe security
descriptor, not by directory permissions. In every case the boundary is other
operating-system accounts; a process already running as the desktop user can
reach Kassiber's data by many other routes and is not defended against here.

Tool results that reach these providers also land in their own session stores —
Codex threads, Claude sessions, OpenCode sessions — which Kassiber neither
encrypts nor prunes. Resuming a chat depends on those stores, so the accounting
context of a tool-enabled turn persists outside SQLCipher until the provider's
own retention removes it. Local providers avoid this entirely.

For browser-driven development, the Vite dev server also exposes a loopback-only
daemon bridge. Run:

```bash
pnpm --dir ui-tauri run dev:bridge
```

Then open `http://127.0.0.1:5173`. In bridge mode, the browser talks to the
same local Python daemon protocol through Vite: normal calls use
`/__kassiber__/daemon`, and `ai.chat` streams NDJSON records from
`/__kassiber__/daemon/stream`. This is a development-only convenience for
testing real local AI, tool cards, cancellation, and consent from an ordinary
browser tab. Packaged builds and `tauri dev` do not use the bridge.

Provider configuration is mirrored in the CLI:

```bash
kassiber ai providers list
printf '%s\n' "$OPENAI_API_KEY" | kassiber ai providers create openai --base-url https://api.openai.com/v1 --kind remote --acknowledge --api-key-stdin --default-model gpt-4o-mini
kassiber ai providers create claude-cli --base-url claude-cli://default --kind remote --acknowledge --default-model default
kassiber ai providers set-default openai
kassiber ai models
kassiber chat "Summarise the last week of imports."
git log --oneline -20 | kassiber chat -
kassiber chat
```

`kassiber chat` is the CLI client for the same daemon-backed assistant used by
the desktop UI — there is exactly one chat surface and one protocol. It starts
a local daemon transport, sends `ai.chat` requests with `tools_enabled=true`,
renders streaming deltas in the terminal, and sends `ai.tool_call.consent`
decisions when mutating tools ask for approval. Pass a prompt positionally or
with `--prompt` for one turn; `kassiber chat -` reads the one-shot prompt from
stdin for pipelines and heredocs. After each rendered turn a dim provenance
footer shows provider/model, the tools that actually ran, and whether journals
were auto-refreshed — the same provenance the desktop Assistant records.
`--no-tools` disables the tool loop for a provider-only exchange, and
`--system "..."` replaces the built-in Kassiber system prompt with a raw one
(`system_prompt_kind="raw"`).

Use `--tool-profile` to select the exposed catalog; defaults and scope behavior
are documented under [Tool use](#tool-use).

Use `--timeout SECONDS` for harnesses or local models that need a shorter or
longer wait. It caps daemon startup and provider stream inactivity (default
120 seconds). The value is local transport control only: Kassiber does not put
it in system prompts, user messages, tool schemas, tool results, or transcripts.

Omit the prompt for REPL mode, which has line editing and in-session history
on real terminals, and these commands:

- `/help` — command help; `/exit`, `/quit`, or Ctrl-D leaves.
- `/tools` — the daemon tool catalog with consent classes.
- `/model [id]`, `/provider [name]` — show or switch mid-session; a provider
  switch re-resolves that provider's default model and rolls back on error.
- `/allow <tool>`, `/allowed` — manage which mutating tools are pre-approved
  for this session.
- `/new` — start a fresh conversation without restarting the daemon.

Ctrl-C during a reply cancels that turn cooperatively and keeps the session.
Daemon-side `allow_session` consent spans a single `ai.chat` request, so the
REPL carries an interactive "[s] session" answer across turns client-side and
re-sends it for that tool.

Output modes cover scripting:

- default rendered text for humans. On a TTY, the model's markdown is
  rendered with ANSI styling — bold, inline code, headers, bullets, fenced
  code, and pipe tables re-drawn as box-aligned tables — while preserving
  token streaming, and successful tool results draw a compact deterministic
  table straight from the daemon envelope, so tabular numbers on screen never
  depend on the model retyping them correctly. `--plain` turns both off.
  With piped stdout, the raw answer text is the only thing on stdout —
  progress labels, tool announcements, tool tables, consent UI, and the
  provenance footer move to stderr;
- `--machine` / `--format json` (one-shot only) emits a single `chat` envelope
  with the final message, `finish_reason`, provenance, and tool-call summary;
- `--stream-json` (one-shot only, mutually exclusive with `--machine`) emits
  the raw daemon stream records — `ai.chat.status`, `ai.chat.delta`,
  `ai.chat.tool_call`, `ai.chat.tool_consent_required`, `ai.chat.tool_result`,
  then the terminal `ai.chat` — as NDJSON, mirroring what the desktop bridge
  streams;
- `--transcript PATH` (any mode, REPL included) appends every daemon request
  and stream record for the session to PATH as NDJSON — a local audit trail
  for debugging model answers and tool behavior. The file is plaintext and
  contains prompts and redacted tool results; treat it like notes, not like
  the encrypted database.

For automation, `kassiber chat --yes "..."` approves mutating tool requests for
that chat session without prompting, except `ui.review.apply`: each custody
review requires a fresh interactive answer to the daemon-validated preview.
Neither `--allow-tool`, `/allow`, nor prior session consent bypasses that
requirement, and non-interactive chat denies this tool. Use the explicit
`review plan/apply` CLI for externally reviewed artifacts. Prefer the narrower
`--allow-tool ui.journals.process` form when a script should approve only one
tool. Machine and `--stream-json` runs never prompt interactively even on a
TTY; there, and without a TTY in rendered mode, unapproved mutating tools are
denied and the denial is fed back to the model as `user_denied`.

If the local database is SQLCipher-encrypted and the CLI cannot unlock it,
`kassiber chat` fails with `passphrase_required` before contacting the model.
Headless scripts should pass the global `--db-passphrase-fd <FD>` flag from a
parent process. The passphrase is consumed by the local daemon only; it is not
embedded in prompts or tool output.

Test the assistant/tool path through the CLI, not only through the desktop GUI:
`tests/test_cli_chat.py` pins the daemon-backed chat loop, consent behavior,
locked-database handling, timeout controls, and the tool profiles.

Tool-profile selection is described under [Tool use](#tool-use).
Live backend checks should also be CLI-first, with explicit user-approved
endpoints and a fresh temporary data root.

## Chat history

Chat sessions can persist — inside the SQLite/SQLCipher database, next to the
data the answers were derived from, never as separate plaintext files. The
policy setting `ai_chat_history` has three values, managed via
`kassiber chats config [--history auto|on|off]`:

- `auto` (default) — persist only when the database file is
  SQLCipher-encrypted. A plaintext database stays ephemeral; running
  `kassiber secrets init` is what unlocks history.
- `on` — persist regardless of encryption (an explicit user choice).
- `off` — never persist.

`kassiber chat --incognito` skips persistence for one session regardless of
the setting. `kassiber chat --continue` resumes the most recently updated
session (the stored messages are replayed to the model as context);
`--session <id>` resumes a specific one. In the REPL, `/new` starts a fresh
session. Stored exchanges keep the user prompt, the assistant answer, the
`finish_reason`, and the answer provenance — not full tool result envelopes,
which remain reproducible from the database (use `--transcript` for
full-fidelity capture).

Answer provenance includes a UI-only `privacy_receipt`: provider kind,
local/remote classification, screen route, number of advertised schemas and
executed tools, plus outbound event/endpoint/byte counts recorded during that
turn. The receipt is computed after the provider call and is never fed back to
the model. Exact hosts remain available only on the dedicated local Egress
screen; the AI-facing egress tool receives aggregate subsystem counts.

Manage stored sessions with `kassiber chats list`, `chats show <id>`,
`chats delete <id>`, and `chats clear`. Machine chat envelopes and the
terminal `ai.chat` record carry `session_id` (null when nothing persisted).

On the wire, persistence is per request: `ai.chat` accepts
`persist: true | false | "auto"` plus `session_id` to append to an existing
session; unknown session ids fail before streaming starts. A request opts in
by sending `persist` true/`"auto"` or a `session_id`; with neither, nothing
persists, so existing clients are unchanged. The stored policy stays
authoritative over every write — `off` never persists and `auto` persists
only on encrypted databases, even for continuations of an existing session. Session management is exposed as the daemon
kinds `ui.chat.sessions.list`, `ui.chat.sessions.get`,
`ui.chat.sessions.delete`, and `ui.chat.sessions.clear`, profile-scoped like
the rest of the UI surface, plus `ui.chat.history.configure` for reading or
setting the policy (the desktop Settings control). The desktop Assistant
sends `persist: "auto"`, round-trips `session_id`, and surfaces history in
its toolbar; these kinds stay usable while the AI runtime toggle is off,
because seeing and deleting stored history is a privacy control, not an AI
feature. Chat history is intentionally **not** an AI tool:
the model cannot browse or search prior sessions on its own. Only a session
the user explicitly resumes is replayed as normal chat context, so
prompt-injection risk stays scoped to the resumed conversation. Diagnostics
reports and audit packages do not include chat content.

Provider API-key entry supports `--api-key-stdin` and `--api-key-fd FD`. The
legacy `--api-key <value>` form still works as a warning-on-use compatibility
shim, but docs and tests avoid it because argv can land in shell history and
process listings. Desktop Settings uses the narrow `ai.providers.set_api_key`
daemon kind to rotate/re-enter a key and `ai.providers.move_api_key` to move a
stored key between `sqlcipher_inline` and the selected native store. The daemon
rejects `api_key` on `ai.providers.create`, `ai.providers.update`, and
`ai.test_connection`; connection tests use the stored provider key after it has
been saved.

Provider envelopes expose only `has_api_key` plus
`secret_ref.{store_id,state}`. `sqlcipher_inline` keeps the key in the
SQLCipher database. Native desktop storage records only non-secret
`ai_provider_secret_refs` metadata and stores the value in macOS Keychain,
Windows user-scope Credential Manager/DPAPI, or Linux Secret Service when
platform policy selects that store. Backup export records only ref metadata for
non-inline AI keys and refuses inconsistent rows where a non-inline ref still
has an inline `api_key`; backup import surfaces a non-fatal
`secret_ref_unavailable` warning so Settings can prompt for re-entry. Backend
tokens, descriptors, xpubs, and blinding keys stay SQLCipher-protected. See
[`../plan/10-secret-management.md`](../plan/10-secret-management.md).

Reasoning-capable models surface provider-exposed reasoning summaries or
model-authored thinking text through one of two channels, and both are split
into collapsible reasoning pane(s) above the answer:

- Inline `<think>...</think>` tags inside the content stream — emitted by
  DeepSeek-R1 and QwQ.
- Structured Responses reasoning-summary events — emitted by supported OpenAI
  reasoning models and by compatible Ollama/oMLX thinking models.

Each user turn gets its own assistant message. Inside a tool-using turn,
each provider completion round (`waiting_for_model` before the next model
call) opens a fresh reasoning segment, so Ollama/oMLX traces stay
per-round instead of one continuous blob. Models that don't emit either
channel pass through unchanged.

Settings → AI providers exposes a **Test connection** action. It calls the
daemon's `ai.test_connection` kind with the *currently entered* base URL and
API key (or, when editing without changing the API-key field, the saved key)
and reports the model count without persisting anything. For built-in CLI
locators, model discovery and authentication readiness come from the broker's
native runtime probes. For HTTP providers, the
connection test probes `/v1/models`; it does not spend tokens on a generation,
so the first chat remains the final check that `/v1/responses` is enabled.

Remote, TEE, Codex, Claude, and OpenCode CLI providers require explicit
acknowledgement before chat. The CLI uses
`kassiber ai providers update <name> --acknowledge` (or `--acknowledge` during
`create`), and the desktop Settings form prompts before saving an off-device
provider. Without that acknowledgement, `ai.chat` returns
`ai_remote_ack_required` before sending any prompt content.

Streaming is demuxed by `request_id`: the Tauri supervisor keeps one daemon
process and one stdout reader, but routes each JSON envelope to the matching
request. While a chat is streaming, unrelated daemon calls can complete
independently.

Before the first token arrives, `ai.chat` may emit `ai.chat.status` records
with phases such as `preparing`, `connecting`, and `waiting_for_model`. These
records are UI progress hints only; model-authored thinking is shown only when
the provider emits inline `<think>` content or structured reasoning-summary
deltas.

Pressing **Stop** in the desktop UI, choosing cancel at a terminal consent
prompt, or interrupting `kassiber chat` sends `ai.chat.cancel` with
`args.target_request_id = <active ai.chat request_id>`. Cancellation is
best-effort and cooperative: Kassiber stops forwarding deltas once the Python
worker returns between provider chunks, then emits the terminal `ai.chat`
envelope with `finish_reason: "cancelled"`. For metered remote providers, any
tokens already generated or in flight may still be billed.

## Tool use

The desktop assistant and `kassiber chat` opt into a bounded tool loop with
`ai.chat` top-level args:

```json
{
  "tools_enabled": true,
  "tool_loop_max_iterations": 8,
  "system_prompt_kind": "kassiber"
}
```

Tool control stays top-level; generation options still live under `options`.
When enabled, Kassiber prepends a compact Kassiber skill-aware system prompt,
sends flat Responses function definitions, emits `ai.chat.tool_call`,
`ai.chat.tool_consent_required`, and
`ai.chat.tool_result` stream records as needed, feeds tool results back as
typed `function_call_output` Items with matching `call_id` values, and finishes
with the normal terminal `ai.chat` envelope. The complete provider output is
replayed between tool rounds so reasoning Items are not discarded.

Before the provider is called, Kassiber also runs a small deterministic
read-only router for Kassiber questions. It looks for common accounting intents
such as pending work, sync readiness, totals, inflow/outflow, balances, tax
summaries, largest/smallest transactions, transaction search, quarantine,
the combined review worklist, loans, book-set views, transfers/direct payouts,
swap-review context, saved review filters, auto-pair rules, and pricing.
Matching read-only tool results are streamed to the UI and inserted into the
model context as exact local data, so answers use program-derived facts. That auto-read context is sent as
untrusted accounting data, not as system instructions, and is bounded per tool
so large reports cannot silently crowd out everything else.
When one of those reads needs current reports or journal-derived state,
Kassiber refreshes stale local journals first and includes the
`ui.journals.process` result in the tool result metadata. This refresh is local
and deterministic, and the same refresh path applies when the desktop GUI reads
journal-derived daemon kinds such as `ui.reports.*` or `ui.report.blockers`
directly. Watch-only source refresh remains explicit unless the active profile has
enabled automatic refresh-before-report maintenance, because refresh can contact
external services and import new transactions. Automatic refresh results are
redacted before they enter AI/UI tool metadata; exact backend URLs are not sent
to the provider. If any source refresh row fails, the maintenance/report-blocker
path returns a blocking `sync_failed` item instead of saying reports are ready.

Quoted or search-like questions can trigger `ui.transactions.search` before the
provider is called. Matching transaction notes, descriptions, counterparties,
tags, and values may then be included in the provider context; keep inference
local when those fields are sensitive.

Change-audit reads require an explicit previous-answer timestamp. Without a
baseline, `ui.audit.changes_since_last_answer` returns
`status: "baseline_required"` rather than claiming that nothing changed.
When a baseline is provided, the answer includes transaction metadata edit
events alongside transaction, wallet, journal, quarantine, and rate changes.
Bounded edit-history reads are available only through the safe
`ui.transactions.history` and `ui.activity.history` daemon tools; the assistant
does not get raw SQLite or CLI access.

The in-app prompt is a digest, not a full Agent Skill dump. It teaches the model the local-first accounting
role, the normal workflow order, the journal reprocessing rule, and the
boundary between read-only information and mutating actions. The assistant is
skill-aware, but it is not shell-powered or CLI-powered: there is no raw command
execution, raw filesystem access, arbitrary daemon dispatch, or generic
Kassiber CLI tool.

Clients should upsert tool cards by `call_id`. Mutating tools emit an initial
`ai.chat.tool_call` with `needs_consent: true`, followed by
`ai.chat.tool_consent_required`. If the user approves the call, the daemon emits
another `ai.chat.tool_call` for the same `call_id` with `needs_consent: false`
before `ai.chat.tool_result`; that second record marks the approved call as
running and must not create a duplicate card.

Live chats advertise capability packs selected from the latest question and
typed `screen_context`. `tool_profile=core` intersects those packs with the
common catalog; `scoped` includes relevant specialist tools; `full` skips
capability scoping. Omitted `tool_profile` defaults to `scoped` at the daemon;
the CLI supplies `core` by default. Profile selection limits exposure and
context size, not the validation or consent required to execute a tool.

The authoritative tool names, schemas, effects, and daemon mappings live in
[TOOL_CATALOG and get_tool](../../kassiber/ai/tools.py), with capability selection
in `select_tool_capabilities` in the same module. These are distinct from
CLI `commands describe` and the renderer allowlist. Do not reconstruct a tool
schema or assume that a daemon kind is available to the model.

Desktop chat builds an ephemeral `screen_context` from a positive registry of
canonical routes and capability packs. It contains only a route and,
when available, a typed entity id, bounded filters, or explicit capability
hints. Sensitive keys and oversized filters are rejected. The context is
inserted as untrusted navigation state immediately before the current user
turn; it never grants filesystem access and is not a replacement for a typed
read tool.

Only schemas advertised for that turn may be requested by the provider. The
small deterministic pre-read router remains separately bounded and read-only.
The daemon validates tool
arguments against the catalog again at execution time, including required
fields, types, enums, bounds, and `additionalProperties`; provider output cannot
smuggle a hidden network or mutation argument into a narrower tool.

Read tools return bounded, redacted projections. The following boundaries
matter across tool additions:

- UTXO/privacy projections omit addresses, scripts, derivation details, wallet
  material and backend endpoints. Privacy Mirror and chain analysis share the
  same snapshot and opaque provider references; use `ui.chain_analysis.ai_context`
  for handoffs. Dataset import requires a validated source/recipe/hash-bound
  preview, on-device provider and once-only consent. See
  [chain analysis](local-chain-analysis.md) and [Privacy Mirror](privacy-mirror.md).
- Cross-book context requires an explicit book-set request within the chat's
  original workspace. Keep each book's tax/lot scope and mixed fiat values
  separate. Ordinary health remains active-book scoped.
- Report tools return processed journal facts. Rates remain local unless live
  access is enabled; a rate rebuild can complete while journal processing fails,
  and must report those outcomes separately. [Tax and journals](tax.md) owns
  report semantics and readiness.
- Transaction review combines independently degradable evidence sections.
  Attachment lists omit paths and URL targets, merchant reads omit raw provider
  payloads/document bytes, and egress summaries omit destination identities and
  request contents. [Source-of-funds review](source-of-funds-review.md) owns the
  provenance workflow; evidence never acquires custody or tax authority.
- Transfer suggestions do not write decisions. Ownership intent remains
  necessary for Bitcoin rail changes; an external payment is not a self-transfer.
  `ownership_graph` candidates require individual review and cannot enter bulk
  or rule pairing. A bounded review packet may omit competing rows: use
  `conflict_set_id` and the full cluster before approving a choice. Candidate
  evidence retains contradictions and exact declared amounts so a `strong`
  result is explainable. [Swap matching](../../kassiber/ai/skill_references/swap-matching.md)
  owns the review procedure and filter semantics.
- Loan open-lock hints remain heuristics, never liquidation proof or automatic
  marks. Bounded lists must disclose full counts and truncation.

`ui.workspace.health` summarizes the active books set and book
(`workspace`/`profile` internally), wallet and transaction counts,
journal freshness, quarantine count,
and report-readiness hints from the current database. `ui.next_actions` returns structured
recommendations such as create a wallet, sync/import, process journals, review
quarantine, or run reports. It only advises; it does not execute those actions.

`read_skill_reference` is a virtual tool. `read_skill_reference("index")`
returns a compact routing document derived from the Kassiber skill concepts and
points the model to deeper allowlisted references. The deeper references are
restricted to packaged files under `kassiber/ai/skill_references/`:
`command-templates`,
`journal-processing`, `metadata`, `onboarding`, `reports`,
`secrets-and-backup`, `swap-matching`, `troubleshooting`, `verification`, and
`wallets-backends`.

Tools marked mutating or egressing require consent. Their catalog metadata,
not a copied list of names, determines the execution path. Reviewed same-asset
pairing can express one-to-many or many-to-one ownership hops; cross-asset and
layer-transition links remain one-to-one on that pairing surface.

Natural-language pairing ("pair the Phoenix payment with the Liquid receipt") is
supported on the `core` tool profile through `ui_transfers_pair` only.
`ui_transfers_bulk_pair` is excluded so a single consent can never sweep a whole
review queue, and `ui_transfers_unpair` is excluded too — the assistant is told
not to promise an undo it may not be able to perform, since the user can always
unpair from the desktop review screen.

`ui_transfers_dismiss` is excluded on stronger grounds: it is the one review-queue
write with neither a read-back nor an undo. `transaction_pair_dismissals` has a
single upsert and no `DELETE` anywhere in the codebase, no list/read daemon kind
and no AI tool, and the desktop undo path only calls `unpair`. With
`expires_in_days: 0` the matcher would stop offering a candidate permanently while
neither the assistant nor the user could discover that the dismissal exists — and
`ui_transfers_suggest` explicitly documents that dismissed legs are silently
absent. It should join the profile once a dismissal read path exists.

Three properties of `ui_transfers_pair` are documented on the tool because the
model would otherwise have to guess them, and each guess is a real error class:

- `policy` decides tax treatment (`carrying-value` carries basis across the move
  and realizes nothing; `taxable` realizes a disposal). Both values now carry an
  explanation and the model is told to take the candidate's `default_policy` and
  state which one it used rather than inferring.
- `out_amount` is a decimal **BTC** string — the only non-msat amount in the AI
  surface, next to `out_amount_msat` fields the model has just read. The
  parameter doc carries an explicit unit warning.
- unlike `ui_transfers_bulk_pair`, which refuses any candidate with
  `conflict_size > 1`, this tool *will* pair one member of a conflict cluster and
  thereby consume both legs, making the competing candidates unauthorable. The
  description requires the model to check `conflict_size`, list the competing
  candidates, and let the user choose. Blocking it outright would break the
  desktop flow, where the human resolves the cluster by choosing; disclosure at
  the consent surface is the remaining gap.
The desktop/CLI custody-component resolver handles 1:N, N:1, N:M, multi-hop,
and missing-wallet histories atomically; those authored component mutations are
not generic AI pairing shortcuts.
Source-funds evidence writes are also consent-gated:
`ui_source_funds_sources_create`,
`ui_source_funds_links_create`, `ui_source_funds_links_review`,
`ui_source_funds_suggest`, `ui_source_funds_links_bulk_review`,
`ui_source_funds_sources_attach`, and `ui_source_funds_links_attach`. Evidence
attach tools accept only existing managed attachment ids, never paths. These tools
create/review provenance evidence only; they do not mutate tax/journal
`transaction_pairs` and they support non-CoinJoin link types such as
self-transfer, exchange transfer, trade, swap, peg-in/peg-out, Lightning hops,
manual source, and missing-history edges. CoinJoin/PayJoin links should stay
explicit about privacy-hop ambiguity unless the user has reviewed stronger
evidence. `ui_source_funds_assemble`, `ui_source_funds_cases_save`, and the
virtual `ui_source_funds_export` complete that workflow; export results sent to
the model include the filename/format but not the managed local path.

Transaction review writes (`ui_transactions_metadata_update`,
`ui_transactions_history_revert`, `ui_attachments_copy`) preserve the
append-only edit/evidence audit trail and run only after consent. Commercial
review writes (`ui_btcpay_provenance_review`, `ui_documents_create`) are also
consent-gated. The virtual `ui_reports_export` maps a small report/format enum
onto existing deterministic PDF/XLSX/CSV/audit-package exporters and likewise
withholds managed paths from model context. Stale journals may also be
refreshed automatically before read/report tools as local maintenance. Wallet
sync before report reads is disabled by default; it runs automatically only
after `ui_maintenance_configure` enables that active-profile setting, or when
the user explicitly approves a maintenance/sync call. Tool-call arguments are
redacted before previews, stream events, auto-context entries, and tool-result
content are returned to the model/UI. When a model requests a mutating tool, the
daemon emits
`ai.chat.tool_consent_required` with a short summary and redacted argument
preview, then waits for:

Loan review writes (`ui_loans_mark`, `ui_loans_link`, `ui_loans_unmark`) are
also consent-gated and invalidate journals. Open-lock heuristics never create
marks automatically.

Suggestion seeding (`ui_btcpay_provenance_suggest`, `ui_source_funds_suggest`)
is consent-gated too: both upsert unreviewed link rows and commit, so neither
counts as a read. `ui_review_worklist` stays read-only by listing pending
suggestions through `ui.btcpay.provenance.links` instead of seeding new ones.

The daemon freezes the project/database and active workspace/profile ids when
the chat starts. Every read and approved mutation rechecks that scope on the
main SQLite thread immediately before execution. If the user switches projects
or books mid-turn, the operation fails with `stale_context`; history persistence
still targets the original book and never the newly active one.

```json
{
  "kind": "ai.tool_call.consent",
  "args": {
    "target_request_id": "<active ai.chat request_id>",
    "call_id": "<tool call id>",
    "decision": "allow_once"
  }
}
```

`decision` can be `allow_once`, `allow_session`, or `deny`. Session consent is
in-memory and lasts only for the current `ai.chat` request; it applies only to
subsequent calls to the same tool name in that chat. If the user denies or does
not respond before the consent timeout, the daemon feeds a tool result back to
the model with `ok: false` and `reason: "user_denied"` or
`"consent_timeout"`. Unknown tools still return `tool_not_allowed` and are not
executed.

The terminal `ai.chat` record includes a compact `provenance` object with the
provider/model, generation timestamp, local tool names used, journal refresh
status, sync-attempt status, successful versus denied tool attempts, and counts
learned from health/report-blocker tools. Denied calls never count as executed
or as cross-book disclosure. The GUI uses that object and the exact tool payloads to render source
chips beside the assistant answer, making the result traceable to
program-derived facts.

## Remote inference

Remote inference requires an explicit privacy decision. Provider branding or a
local proxy alone does not establish confidentiality. Verify the selected
transport and provider protections against the
[privacy model](privacy-and-security.md#ai-provider-configuration); selected
financial context follows the stricter [general-accounting disclosure contract](general-accounting.md).

## Related files

- [Kassiber CLI skill](../../skills/kassiber/SKILL.md)
- [`../../kassiber/ai/client.py`](../../kassiber/ai/client.py)
- [`../../kassiber/ai/broker_client.py`](../../kassiber/ai/broker_client.py)
- [`../../ui-tauri/provider-broker/`](../../ui-tauri/provider-broker/)
- [`../../kassiber/ai/contracts.py`](../../kassiber/ai/contracts.py)
- [`../plan/08-external-document-reconciliation.md`](../plan/08-external-document-reconciliation.md)
- [Privacy & security](privacy-and-security.md)
