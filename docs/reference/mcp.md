# MCP server for external agents

`kassiber mcp serve` lets an external agent (Claude Code, Codex, Claude
Desktop, Cursor, or any other MCP host) read a Kassiber book through typed
tools over stdio. It is the agent-facing counterpart of the in-product
Assistant: the same tool catalog, argument validation, AI projections, and
result redaction, with a narrower, strictly read-only allowlist.

## What it is and is not

- **Local stdio only.** The host launches `kassiber mcp serve` as a child
  process. There is no listener, no loopback socket, and no HTTP transport.
  Optional server/REST operation stays outside the accepted scope.
- **Read-only.** Exposed tools never sync wallets, contact backends or rate
  sources, rebuild journals, or write the book. The read-triggered journal
  rebuild and opt-in freshness sync the Assistant performs are disabled, so a
  report with stale inputs returns its stale/blocked state instead of changing
  the book. Tool execution runs with SQLite `query_only` on. Opening the book
  beforehand does what any CLI read does: schema-compatibility checks and the
  project catalog's last-opened marker. The agent is told to ask the user to run `kassiber journals
  process` or a sync.
- **No egress.** Launch, `server/discover`, `tools/list`, and every exposed
  tool are local. A regression test runs launch, listing, and a local read
  under a socket guard that also blocks loopback.
- **No secrets.** Tool results pass the Assistant's AI projections and then
  `redact_ai_tool_result` again at the MCP boundary: no descriptors, xpubs,
  tokens, local paths, or URLs.
- **Not a new authority.** The same OS user could already run the CLI; the
  server exposes a narrower surface than the CLI skill does. Mutations,
  consented tools, custody tools restricted to local providers, on-device chain
  analysis, cross-book reads, attachment analysis, and accounting approval
  handles are excluded until daemon-enforced approval modes exist (see
  [TODO](../../TODO.md)).

## Disclosure

Everything a tool returns goes to the agent host and, through it, to whatever
model the host uses. That model may be remote, and the host may persist
transcripts and the server's stderr. Registering the server with a host is the
user's decision to disclose the book's read projections to that host.
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
`tools/call`, `subscriptions/listen` (acknowledged with an empty filter,
because the tool list never changes at runtime), and `notifications/cancelled`.
Unsupported modern versions get `-32022` with the supported list, missing
request metadata gets `-32602`, and unknown tools get `-32602`. Tool
validation and domain failures are `isError` results carrying Kassiber's error
code, message, and hint, so the agent can act on them.

The tool list is deterministic and returned in one page. Tool names are the
catalog's wire names without the internal `ui_` prefix (for example
`reports_balance_sheet`); `kassiber mcp tools` prints the exact definitions.
Every definition is annotated `readOnlyHint: true`, `destructiveHint: false`,
`idempotentHint: true`, and `openWorldHint: false`. Hosts may auto-approve on
those hints, which is why enforcement stays server-side.

The implementation uses only the standard library (`kassiber/mcp/`). It
elicits nothing, samples nothing, and sends no requests to the client.

## Books and encrypted databases

Every call is a finite, self-contained operation that resolves its project and
book when it runs. Nothing is opened at launch. A missing database, or a
machine with no Kassiber project at all, is reported as `not_initialized`;
neither is ever created.

- `--workspace` and `--profile` pin the book and must be given together (a
  half pin would resolve against whatever is active). A pinned call reads that book
  through a context-local override of the active-context settings. It never
  writes the database-wide active context the desktop and CLI share, and
  writing a pinned key fails closed. Without a pin, each call reads the book
  that is active at call time. Every result names the book it read.
- **Plaintext and `unattended` books** open like an ordinary non-interactive
  CLI command (remembered unlock only in unattended mode) and close after the
  call. They never prompt.
- **`brokered` books** submit `kassiber mcp call` to the
  [operator broker](operator-broker.md) as a `read` operation. The lease's
  capability and project/database identity checks apply, the broker requires
  an explicit book, and the server process never sees the passphrase. A lease
  covers every book in its project; the book is the server pin, else the
  lease's default scope. Without a lease, the call returns `interaction_required` with
  `details.reason = operator_lease_required` and never starts a broker, even if
  one exits between the lease check and the submission. The
  agent asks the user to run `kassiber operator unlock --capability read` in
  their own terminal.
- **`manual` encrypted books** return `interaction_required` with
  `details.reason = database_passphrase`.

A broker from an earlier build that is still running cannot bind caller
context; MCP calls are refused against it (`operator_broker_outdated`) until
the user restarts the broker. At most 32 tool calls may be in flight; more are
refused with a retryable `-32000` error rather than queued without bound.

The agent must never ask for or relay the passphrase. `kassiber mcp serve` is
a long-lived server and is refused as a broker operation, like `chat` and
`daemon`. `kassiber mcp call --tool NAME --arguments JSON` runs one tool
directly from a shell with the same result shape (`kind: mcp.call`).

## Host configuration

Use absolute paths if `kassiber` is not on the host's `PATH`. Pinning the
project and book is recommended:

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
read` in their own terminal for each work session. A successful unlock also
selects brokered mode for the project. `kassiber operator lock` ends the
session.
