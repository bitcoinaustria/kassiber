# MCP server for external agents

`kassiber mcp serve` lets an external agent (Claude Code, Codex, Claude
Desktop, Cursor, or any other MCP host) work in a Kassiber book through typed
tools over stdio. It is the agent-facing counterpart of the in-product
Assistant: the same tool catalog, argument validation, AI projections,
executors, and result redaction, within the access the user grants.

## What it is and is not

- **Local stdio only.** The host launches `kassiber mcp serve` as a child
  process. There is no listener, no loopback socket, and no HTTP transport.
  Optional server/REST operation stays outside the accepted scope.
- **Within the user's grant.** The user chooses what agents may do, using the
  [operator broker's](operator-broker.md#capabilities) cumulative capabilities:
  - `read`: every read-only Assistant tool. Reads are strictly read-only:
    no journal rebuild, no freshness sync, SQLite `query_only` on, so a report
    with stale inputs returns its stale/blocked state and the agent is told to
    ask the user to run `kassiber journals process` or a sync. Reads served
    from the derived chain-analysis index (queries, entropy, Privacy Mirror)
    work while that index is current and otherwise return
    `derived_state_stale`.
  - `operator`: also journal processing, maintenance settings, report and
    source-of-funds exports, source-of-funds sources, links and evidence,
    documents, attachment copies, saved views, local chain-analysis cases,
    labels and watches, and the tools that leave the machine: wallet sync,
    rate rebuild and live rates, maintenance runs, node and Lightning reads,
    and chain-analysis acquisition. File imports are chat-only (they need an
    attachment), so agents have none. Before a read, stale journals (and the
    derived chain-analysis index) are refreshed locally, the same refresh the
    desktop runs when a report opens.
  - `accounting_decisions`: also quarantine resolution, review batches,
    transfer pairing and dismissal, custody and component reviews, loan
    marks, source-of-funds and BTCPay provenance reviews, transaction
    metadata edits (notes, tags, exclusions, classification) and reverts, and
    auto-pair rules.

  Each tool is classified through the broker's exact-path registry by the
  daemon kind it executes (`call_capability` and `mcp_tools` in
  [the MCP tools module](../../kassiber/mcp/tools.py)). Read tools whose
  daemon kind is `operator` only because a direct read refreshes journals
  (reports, report blockers, review contexts) are reads here; anything that
  leaves the machine needs at least `operator`. Tools above the grant are not
  listed, and a call to one returns `mcp_grant_insufficient` naming the grant
  it needs. Admin-class work (secrets, backups, passphrases, deletes,
  `imports rollback`, provider and agent configuration) is never exposed at
  any grant, and neither is a tool whose kind the registry does not classify.
- **Agent access.** `full` or `restricted`, with the
  [per-provider meaning](ai.md#agent-access): `full` gives unprojected results
  and the custody-gap, coverage, lineage, and chain-analysis source and
  acquisition tools; `restricted` hides those tools and projects
  chain-analysis results. The agent's model counts as off-device either way.
- **Not chat-only tools.** Tools that need a chat attachment
  (`wallets_analyze_file`, attachment import), a desktop handoff card (the
  `request_input` tools), a chat visual, a cross-book view, a PSBT or dataset
  the desktop staged, the desktop daemon's background jobs, or its per-process
  egress ledger are not exposed. Book pinning still applies: one call reads or
  changes one book.
- **Writes use the Assistant's executor.** A mutating tool runs through the
  same executor and plan-bound validation as the in-app Assistant
  (`_execute_mutating_ai_tool`): review batches need their unchanged artifact
  and digest, custody and component reviews their expected input version,
  acquisition its server-recomputed plan, exactly as in chat. The MCP host's
  own tool approval is the consent; Kassiber shows no card. SQLite
  `query_only` is turned off only for the duration of an allowed write or of
  the local refresh before an `operator` read; the read itself stays
  `query_only`.
- **Attributable.** Where the core records an author (transaction metadata
  history, review receipts, transfer and custody decisions), writes are
  authored as AI-tool writes, as in chat. A custody review is also recorded
  in the custody AI audit with
  provider kind `mcp`, the host's self-reported client name (from
  `clientInfo`, display only, or `unknown`), and the consent decision
  `mcp_host`. Each write also leaves a redacted entry in the RAM log ring.
- **Egress only at `operator` and above.** Launch, `server/discover`,
  `tools/list`, and every tool not annotated `openWorldHint: true` are local
  at every grant; reads never run the opt-in freshness sync, even when the
  book enables it. The annotated tools (the syncs, rates, maintenance runs,
  the node and Lightning reads, and chain-analysis acquisition) need at least
  `operator` and contact only the destinations their CLI equivalents use.
  Two of them grant later egress instead: maintenance settings can turn on
  the book's background refresh and report-read sync for the desktop, and a
  recurring source authorization lets the desktop daemon acquire. Offline
  mode and `KASSIBER_NO_EGRESS` still block all of them (see
  [privacy and security](privacy-and-security.md#external-requests-complete-list)).
  A regression test runs launch, listing, and reads at the widest grant, with
  the book's report-read sync enabled, under a socket guard that also blocks
  loopback.
- **No secrets.** Tool results pass the Assistant's AI projections and then
  `redact_ai_tool_result` again at the MCP boundary: no descriptors, xpubs,
  wallet configuration, tokens, local paths, or URLs. Nothing offers shell,
  filesystem, or CLI dispatch. BTC amounts stay exact integer msat.
- **Not a new authority.** The same OS user could already run the CLI under
  the same capability levels through the broker; the grant caps what an agent
  reaches, and only the user can widen it.

## Turning it on

External agents are off by default. Until the user turns them on, the server
still lists the tools of the recorded grant but every call returns
`mcp_disabled`. Two ways turn them on:

- the desktop app: **Settings → Assistant → External agents**, which shows
  **Allow agents**, the grant (**Read**, **Read and operate**, **Accounting
  decisions**), a **Restrict results** switch, what the selected grant allows,
  and then the command for the current book. A first enable preselects
  **Accounting decisions** with full access; a recorded choice is kept.
  Changing the grant while agents are on applies to the next call;
- `kassiber mcp enable [--grant read|operator|accounting_decisions]
  [--agent-access full|restricted]` in the user's own terminal. It describes
  what the chosen grant allows, asks for confirmation, and refuses
  `--machine` and non-interactive callers, so an agent can neither turn
  itself on nor widen its own grant. Omitted flags keep the recorded choice
  (`read` and `restricted` for a first enable).

`kassiber mcp disable` or the desktop switch turn agents off again, and
`kassiber mcp status` reports the state with `grant` and `agent_access`.
`kassiber mcp tools` prints the definitions at the current grant. AI features
are the master switch: while they are off in the desktop, calls return
`ai_features_disabled`, and the external-agents choice is kept for when AI is
turned back on.

The choice lives in `config/agent-access.json` in Kassiber's per-user state
directory, beside the update-check consent. The file is owner-only and holds no
secrets. The desktop writes it through `ui.agent_access.configure`
(`mcp_enabled`, `ai_features_enabled`, `grant`, `agent_access`), and every
call re-reads it before resolving any project; an unreadable file counts as
off. A file without `grant` and `agent_access`, written before grants
existed, keeps its earlier behaviour: `read` and `restricted`. So does any
value this build does not recognise, `admin` included.
## Disclosure

Everything a tool returns goes to the agent host and, through it, to whatever
model the host uses. That model may be remote, and the host may persist
transcripts and the server's stderr. Registering the server with a host is the
user's decision to disclose the book's read projections to that host, and the
grant is their decision about what that host may change. At `operator` and
above, the host's own tool approval is the only prompt before a change, so its
approval settings matter; a host that auto-approves tools turns the grant into
standing permission.
Kassiber cannot see or control the host's model or provider. The server writes
nothing to stderr and logs only to the bounded RAM ring; see
[logging](logging.md).

Notes, labels, and imported descriptions reach the agent verbatim as data. The
server instructions mark them as untrusted text, never instructions, but a
host with shell or web tools should still be treated as exposed to prompt
injection from book contents.

## Protocol

The server is dual-era under the MCP versioning rules:

| Client | How it speaks | Server behavior |
| --- | --- | --- |
| Modern (`2026-07-28`) | Per-request `_meta` with `io.modelcontextprotocol/protocolVersion` and `clientCapabilities`; no handshake | Served statelessly; results carry `resultType` and `serverInfo`; lists carry `ttlMs`/`cacheScope` |
| Legacy (`2025-11-25`, `2025-06-18`) | `initialize` handshake | Echoes a supported version (otherwise offers `2025-11-25`) and serves that revision's shapes for the rest of the process |

`2025-03-26` is not offered because that revision requires JSON-RPC batch
support. `ping` is answered for legacy clients only; `2026-07-28` removed it.
Implemented methods: `server/discover`, `initialize`, `ping`, `tools/list`,
`tools/call`, `subscriptions/listen` (acknowledged with an empty filter: the
server sends no list-change notifications), and `notifications/cancelled`.
Unsupported modern versions get `-32022` with the supported list, missing
request metadata gets `-32602`, and unknown tools get `-32602`. Tool
validation and domain failures are `isError` results carrying Kassiber's error
code, message, and hint, so the agent can act on them.

The tool list is deterministic and returned in one page. It follows the
recorded grant and access mode when the host asks for it, so a changed grant
appears the next time the host lists tools (modern lists carry a 60-second
`ttlMs`; legacy hosts usually list once per launch). Calls are authorized when
they run, never from the list. Tool names are the catalog's wire names without
the internal `ui_` prefix (for example `reports_balance_sheet`);
`kassiber mcp tools` prints the exact definitions at the current grant.

Annotations are truthful so hosts can prompt appropriately: `readOnlyHint` is
true exactly for read-only tools; `destructiveHint` marks writes Kassiber
cannot undo (deleting a payout, rule, saved view, watch, case, or incomplete
dataset import, removing a loan mark, and review batches, which have no single
undo); a dismissal is not destructive, since
`transfers_dismissals_delete` undoes it; `idempotentHint` marks calls that a repeat with
the same arguments does not change further; and `openWorldHint` marks the
tools that contact the network. A mutating catalog tool without a reviewed
annotation entry (`MUTATING_TOOL_HINTS`) is not exposed. Hosts may
auto-approve on these hints, which is why enforcement stays server-side.

The server instructions describe the grant in force: at `read`, that nothing
can change; above it, that changes follow the user's request, plans are
passed unchanged, and stale plans are replanned.

The implementation uses only the standard library (`kassiber/mcp/`). It
elicits nothing, samples nothing, and sends no requests to the client. The
client's self-reported name (legacy `initialize` `clientInfo.name`, or a
modern request's `io.modelcontextprotocol/clientInfo` metadata) is used only
to attribute writes.

## Books and encrypted databases

Every call is a finite, self-contained operation that resolves its project and
book when it runs. Nothing is opened at launch. A missing database, or a
machine with no Kassiber project at all, is reported as `not_initialized`;
neither is ever created.

- `--workspace` and `--profile` pin the book and must be given together (a
  half pin would resolve against whatever is active). A pinned call uses that book
  through a context-local override of the active-context settings. It never
  writes the database-wide active context the desktop and CLI share, and
  writing a pinned key fails closed. Without a pin, each call reads the book
  that is active at call time, so a task could read one book and change
  another: writes therefore need a pinned server and otherwise return
  `book_pin_required` (on a brokered book the lease's default scope counts as
  the pin). Every result names the book it used.
- **Plaintext and `unattended` books** open like an ordinary non-interactive
  CLI command (remembered unlock only in unattended mode) and close after the
  call. They never prompt.
- **`brokered` books** submit `kassiber mcp call` to the
  [operator broker](operator-broker.md). The broker classifies each call by
  its tool, raised to the `--grant` the server passes (the narrower of the
  user's grant and the lease, which lets a read refresh journals), so the
  lease's capability covers what the call can do; project/database identity
  checks apply, the broker requires an explicit book, and the server process
  never sees the passphrase. The child runs with no more than the broker
  admitted. A lease covers every book in its project; the book is the server
  pin, else the lease's default scope. Without a lease, the call returns
  `interaction_required` with `details.reason = operator_lease_required` and
  never starts a broker, even if one exits between the lease check and the
  submission. The agent asks the user to run
  `kassiber operator unlock --capability <grant>` in their own terminal. A
  write the lease does not cover returns `interaction_required` with
  `details.reason = operator_capability_insufficient` and the unlock command
  for the capability it needs; nothing is queued.
- **`manual` encrypted books** return `interaction_required` with
  `details.reason = database_passphrase`.
- **From the desktop:** with external agents on, the External agents row in
  Settings offers *Unlock for agents* for the open encrypted book when agents
  cannot open it on their own (not for plaintext or `unattended` books). It
  uses the passphrase the desktop already holds after its own unlock (typed,
  Touch ID, or the OS credential store); the renderer never receives it. The
  result is an **agent session**, narrower than a terminal lease:
  - It carries the user's External agents grant as its capability, never
    admin. Changing the grant ends the session, so the next unlock leases
    exactly the new grant.
  - It admits only `mcp call` tool calls, each within that capability.
    Ordinary CLI commands such as `kassiber transactions list` are refused
    (`agent_session_scope`).
  - It admits them only from `kassiber mcp serve` processes the user
    allowed. Each server registers itself on its first call and holds that
    registration open with a heartbeat. The broker takes the process id from
    the OS, not from the caller, and hands that connection a token the server
    must present with every call, so a process that later reuses the id is a
    stranger. The registration ends when the process exits. Until the user allows it, calls return `interaction_required` with
    `details.reason = agent_pairing_required` and a hint to allow the agent
    in Kassiber; the row lists it as "*claude* wants to use this book" with
    Allow and Deny. The name is the program that started the server and is
    display only.
  - Allowing needs a random control secret that only the desktop holds, so a
    program talking to the broker directly cannot allow itself. Deny also
    withdraws an earlier approval, closes that registration, cancels its
    queued calls, withholds the results of any it already ran, and keeps the
    process refused (`agent_pairing_denied`) until the book is unlocked for
    agents again. The session covers the whole project, so switching books
    within it keeps it; opening another project ends it.
  - It ends after 15 minutes without a tool call, when the desktop locks,
    quits, opens another project, rotates the passphrase, or restores a backup, when
    external agents or AI features are turned off, and after at most 8 hours.
    A new session never inherits an earlier approval.

  The row shows each allowed agent's call count. While the session exists
  the desktop asks the broker for waiting agents and activity (every few
  seconds while the row is on screen, every 30 seconds otherwise); without a
  session it contacts no broker. Until the broker confirms the lock, the row
  keeps showing the session and the desktop retries. A lease that is already
  open, for example from a terminal, is neither replaced nor locked. Granting
  binds the book to `brokered` mode. A book that was `manual` returns to
  `manual` when the desktop's session ends, through whichever broker is
  running then, unless someone chose another mode since. If no broker is
  running by then, the book stays `brokered`, and a terminal needs
  `kassiber operator unlock`. The broker it starts runs the desktop's build,
  so point agents at the command the row shows.

  Pairing stops other programs from simply connecting to the broker. It is
  not a boundary against a program running as the same OS user that goes
  after the allowed server itself: on Linux such a program can reach the
  server's input and output through `/proc`, and, with
  `kernel.yama.ptrace_scope=0`, its memory. See
  [privacy and security](privacy-and-security.md#external-agents-mcp).

A broker from an earlier build that is still running cannot bind caller
context; MCP calls are refused against it (`operator_broker_outdated`) until
the user restarts the broker. Calls go only to a broker of the server's own
build; another build's broker is refused with `operator_broker_build_mismatch`
(see [several builds](operator-broker.md#several-builds-on-one-machine)).
Arguments are validated against the tool's schema before anything is queued,
and a brokered result is redacted again in the server process. At most 32 tool calls may be in flight; more are
refused with a retryable `-32000` error rather than queued without bound, and
at most 16 `subscriptions/listen` streams may be open.

The agent must never ask for or relay the passphrase. `kassiber mcp serve` is
a long-lived server and is refused as a broker operation, like `chat` and
`daemon`. `kassiber mcp call --tool NAME --arguments JSON` runs one tool
directly from a shell with the same result shape (`kind: mcp.call`), within
the user's grant; `--grant` can only narrow it, and `--client NAME` sets the
attribution for writes. `commands describe` lists `mcp call` as `mutating`
because its tool may write.

## Host configuration

Once agents are enabled, the desktop shows a ready `claude mcp add` command
for the current book and copies an equivalent JSON config for other hosts. Both
use the installed terminal command, or the app's own launcher with `--cli`,
which forwards stdio to the bundled CLI. By hand, use absolute paths if
`kassiber` is not on the host's `PATH`. Pinning the project and book is
recommended:

```json
{
  "mcpServers": {
    "kassiber": {
      "command": "kassiber",
      "args": ["--project", "<project-id>", "mcp", "serve",
               "--workspace", "<book set>", "--profile", "<book>"]
    }
  }
}
```

For Claude Code: `claude mcp add kassiber -- kassiber mcp serve --workspace
<book set> --profile <book>`. Claude Code and Codex use the legacy handshake
for local stdio servers by default and opt into `2026-07-28` separately; both
eras work.

For an encrypted book, the user runs `kassiber operator unlock --capability
<grant>` (the grant they chose for agents) in their own terminal for each work
session, or uses *Unlock for agents* in the desktop. A successful unlock also
selects brokered mode for the project. `kassiber operator lock` ends the
session.
