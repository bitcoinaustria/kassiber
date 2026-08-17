# Kassiber chat provider broker

This bundled Node/TypeScript broker adapts the installed Codex app-server,
Claude CLI, and OpenCode SDK/server to a small bidirectional JSONL stream
consumed by the Python daemon. Provider-specific behavior lives behind the
typed adapter registry in `src/index.ts`, so another provider does not need a
new daemon tool loop.

It inherits only provider authentication/configuration environment variables,
runs from an empty Kassiber-owned temporary directory, denies provider-native
coding tools, and never returns raw provider errors or credentials. Only the
capability-scoped Kassiber schemas selected for a turn are advertised through
Codex dynamic tools or a temporary MCP server. The Python daemon remains the
authority for schema validation, consent, execution, redaction, and auditing;
the broker only carries typed calls and redacted results.
