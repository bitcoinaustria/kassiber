# Plan 23: the provider broker in Rust

Status: in progress.

## Why

The provider broker adapts the installed agent CLIs (Codex app-server,
Claude CLI, OpenCode, ACP agents such as GitHub Copilot) to the daemon's
JSONL broker protocol. [Plan 21](21-rust-backend.md) kept it in TypeScript
because it was never Python. That left three costs:

- CLI-backed AI needs Node.js 20+ on the user's machine. Kassiber runs
  whatever `node` it finds on `PATH` or in a fallback directory, with the
  daemon's full environment (`NODE_OPTIONS` included), so the runtime in
  the AI path is unpinned and outside the product.
- The bundle ships npm code (`@agentclientprotocol/sdk`,
  `@modelcontextprotocol/sdk`, `@opencode-ai/sdk`, `zod`) in the one
  subprocess that sits between Kassiber and remote agents.
- Every artifact carries `provider_broker/index.mjs` beside the binary,
  and redaction logic exists twice (TypeScript `safeErrorMessage` and the
  Rust credential-echo handling).

## Requirement

The `kassiber` binary serves the broker itself. Behaviour, the JSONL
protocol, and every security boundary of the TypeScript broker carry over
unchanged; Node.js is no longer needed for any Kassiber feature.

## Design

- **Process boundary stays.** The daemon still starts a separate process
  per request, now `kassiber --provider-broker` (exact argv, dispatched in
  `crates/kassiber/src/main.rs` before the CLI, like
  `--operator-broker-server`). Crash isolation, the per-leg watchdog, the
  process-group kill and the online gate in `kassiber-net` are unchanged.
- **Protocol stays byte-compatible.** Requests (`status`, `models`, `chat`,
  `tool_results`) and events (`status`, `delta`, `tool_call`, `done`,
  `error`, `result`) keep their fields and meanings, so
  `crates/ai/src/broker/` needs no protocol change.
- **New crate `crates/provider-broker` (`kassiber-provider-broker`).** One
  module per TypeScript module: `protocol` (types, `statusScope`, option
  checks, session-cursor shape, `safeErrorMessage`), `env`
  (`providerEnvironment` allowlists and drops), `executables`, `cleanup`,
  `workdir` (`KASSIBER_AI_BROKER_TMPDIR`), `prompt`, `bridge` (tool bridge
  and MCP child), and one module per adapter: `codex`, `claude`, `acp`,
  `opencode` (both OpenCode generations). Threads and `std::process`, no
  async runtime. JSON through `serde_json`. No new third-party crates;
  crates already in `Cargo.lock` only.
- **Tool bridge.** Unchanged design: the broker listens on a Unix socket in
  a 0700 temporary directory (Windows: a named pipe with an unguessable
  name), the provider CLI starts the MCP child, and the child forwards each
  call over the socket. The child is now `kassiber --provider-broker-mcp
  <socket> <manifest>` and serves MCP through `kassiber_mcp::protocol`
  instead of the TypeScript SDK. Calls outside the advertised catalog and
  repeated call ids are refused in the broker, as before. Argument schemas
  are advertised as given; the daemon stays the authority for schema
  validation.
- **OpenCode without the SDK.** A minimal HTTP/1.1 client (requests and the
  server-sent event stream) restricted to the loopback server the adapter
  itself started. The provider's own outbound traffic stays the provider's,
  as today.
- **Signals.** SIGTERM, SIGINT and SIGHUP still stop tracked agents and
  remove tracked directories before exit. The daemon's temporary root
  covers anything a hard kill leaves behind.
- **Locating the broker.** `BrokerProgram` becomes a program plus
  arguments. Default: the running executable with `--provider-broker`.
  `KASSIBER_AI_PROVIDER_BROKER` still overrides it, now naming an
  executable run without arguments (tests use fake brokers).
  `KASSIBER_AI_BROKER_NODE` and the Node lookup go away.

## Steps

1. Crate skeleton: protocol, redaction, environment, executables, cleanup,
   working directory, prompt helpers, the dispatcher, the tool bridge and
   the MCP child, `--provider-broker` dispatch, and the daemon-side
   `BrokerProgram` switch. Status and model probes answer through the
   adapter registry.
2. Adapters, in parallel once step 1 lands: Claude and Codex; ACP; OpenCode.
   Each ports the TypeScript adapter's behaviour and its tests, and is
   checked against the TypeScript broker on the same fake provider CLIs
   before the TypeScript code goes.
3. Removal: `ui-tauri/provider-broker`, `broker:build`, the npm packages
   only it used, the `provider_broker/` packaging in every artifact and
   build script, and the Node hint in `broker_unavailable`. Update
   [AI](../reference/ai.md), [Rust packaging](../reference/rust-packaging.md),
   [prerelease binaries](../reference/prerelease-binaries.md),
   `THIRD_PARTY_LICENSES.md` and `TODO.md`.

## Risks

- **Silent behaviour drift** in a security boundary (environment
  allowlists, tool denial flags, sharing off, session-cursor shape,
  redaction). Mitigation: port the TypeScript tests, and run both brokers
  against the same fake CLIs during the port and compare their events.
- **OpenCode API drift** without the SDK's types. Mitigation: only the
  endpoints the adapter uses, the existing version check, and failures
  that end the turn rather than guess.
- **Windows paths** (named pipe, `PATHEXT`, `TerminateProcess`) remain
  unrun locally, as before (`TODO.md`).

## Out of scope

New providers, new options, protocol changes, UI changes beyond removing
the Node hint, and linking the broker into the daemon process.
