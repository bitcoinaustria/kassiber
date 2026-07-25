# Kassiber chat provider broker

This bundled Node/TypeScript broker is deliberately chat-only. It adapts the
installed Codex app-server, Claude CLI, and OpenCode server/API to a
small JSONL stream consumed by the Python daemon.

It inherits only provider authentication/configuration environment variables,
runs from an empty Kassiber-owned temporary directory, advertises no native
tools, denies every native tool permission request, and never returns raw
provider errors or credentials. Kassiber accounting tools remain outside this
process and are unavailable for these providers until a separately audited
typed-tool bridge exists.
