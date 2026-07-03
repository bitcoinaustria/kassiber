---
name: kassiber-regtest-mode
description: Use this skill when Codex needs to run, review, debug, or explain Kassiber's local regtest integration harness, including scripts/integration-harness.sh fast, bitcoin-core, demo-full, demo-up, demo-tick, demo-down, Docker-backed local node tests, persistent regtest demo books, and PR work around issue #312 regtest/demo tooling.
---

# Kassiber Regtest Mode

Use the fastest lane that proves the user's request. Do not start Docker unless
the user asks for a live/local-node check, persistent demo mode, or a failure
cannot be reproduced in the fast replay lane.

## Lane Selection

| Need | Command | Docker |
| --- | --- | --- |
| Fast validation, PR sanity, no live node | `./scripts/integration-harness.sh fast` | no |
| Real Bitcoin Core sync smoke | `./scripts/integration-harness.sh bitcoin-core` | yes |
| Full disposable accounting scenario | `./scripts/integration-harness.sh demo-full` | yes |
| Persistent local demo book for UI/dev | `./scripts/integration-harness.sh demo-up` | yes |
| Add fresh activity to persistent demo | `./scripts/integration-harness.sh demo-tick [N]` | yes, existing demo node |
| Stop or remove persistent demo | `./scripts/integration-harness.sh demo-down [--purge]` | yes |

Use `demo-full` for test proof. Use `demo-up` for interactive development; it
must leave reports immediately readable. Use `demo-tick` when the app refresh
path needs new chain activity to import.

## Workflow

1. Start in the Kassiber repo root.
2. Run `bash -n scripts/integration-harness.sh` after editing the harness.
3. Run `./scripts/integration-harness.sh fast` first unless the request is
   explicitly about live Docker/regtest behavior.
4. Before any Docker-backed lane, check Docker cheaply with
   `docker info` or `docker ps`.
5. If Docker is unavailable, do not keep retrying. Tell the user Docker is
   required for `bitcoin-core`, `demo-full`, `demo-up`, `demo-tick`, and
   `demo-down`, and suggest installing Docker Desktop on macOS/Windows or
   Docker Engine/Compose on Linux. Still run the `fast` lane if it is useful.
6. For live lanes, prefer the harness defaults. They use disposable regtest
   credentials, loopback-only ports, and per-worktree Compose projects except
   the persistent `demo-up` project.
7. After Docker-backed tests, verify cleanup with `docker ps` unless the lane
   intentionally keeps the node running (`demo-up` or `KASSIBER_REGTEST_KEEP=1`).

## Multi-Agent Demo Backends

The desired default is one persistent demo backend per worktree, not one
machine-global demo singleton. Multiple Codex/Claude agents should be able to
run `demo-up`, `demo-tick`, `demo-down`, and `pnpm dev:demo` concurrently from
different worktrees without port, Compose project, data-root, or manifest
collisions.

When implementing this, prefer a deterministic namespace derived from the
worktree path, for example:

- `KASSIBER_REGTEST_DEMO_NAMESPACE`: explicit override for humans and CI.
- `KASSIBER_REGTEST_COMPOSE_PROJECT`: default
  `kassiber-regtest-demo-<hash>`.
- `KASSIBER_REGTEST_DEMO_HOME`: default
  `~/.kassiber/regtest-demo-<hash>`.
- `KASSIBER_REGTEST_RPC_PORT`: default to a stable per-worktree port block.
  The existing derived ports can continue to use offsets from that base.

Keep `KASSIBER_REGTEST_SHARED_DEMO=1` (or an equivalently explicit opt-in) for
the old single shared backend behavior. Do not make accidental sharing the
default; it causes agents to stop, purge, tick, or mutate each other's books.

The manifest should remain the source of truth after first startup. Persist the
namespace, Compose project, data root, all loopback URLs, and generated RPC
credentials there, then have `demo-tick`, `demo-down`, `dev:demo`, and helper
scripts read the manifest before inventing defaults. If the manifest exists,
reuse it even if the current default hash algorithm later changes.

When reviewing or patching this area, check all of these together:

- `scripts/integration-harness.sh`: namespace, port block, manifest read/write,
  `demo-up`, `demo-tick`, `demo-down`.
- `ui-tauri/package.json` and `ui-tauri/vite.config.ts`: `dev:demo` must point
  at the same worktree-scoped demo home unless explicitly overridden.
- `dev/regtest/bitcoin-cli.sh`: should pick the current worktree manifest or
  accept the same namespace override before falling back to defaults.
- `docs/reference/testing.md`: document shared mode as opt-in and name the
  exact override knobs.

For host-browser explorer inspection, the local mempool/esplora-compatible HTTP
endpoints should stay bound to host loopback but answer browser preflight
requests. Keep `KASSIBER_REGTEST_EXPLORER_CORS_ORIGIN` available for narrowing
or disabling the default regtest-only CORS headers.

For containerized Kassiber processes that need the host's Ollama instance, the
regtest Compose stack should provide `host.docker.internal`, and the Kassiber
AI provider seed can use
`KASSIBER_DEFAULT_AI_BASE_URL=http://host.docker.internal:11434/v1`.

## Faster Chat Startup

For chat work, avoid treating `demo-up` as a mandatory cold build. The fast
path should be:

1. Look for the worktree-scoped `demo-manifest.json`.
2. Probe the recorded Core RPC URL with the recorded credentials.
3. If reachable and the scenario checksum still matches, skip Docker startup,
   skip book rebuild, refresh live-rate cache only if requested, and print the
   ready data root immediately.
4. If the container exists but is stopped, run Compose `up -d` for the recorded
   project and wait only for the probe.
5. Rebuild the book only when the scenario checksum changed, the data root is
   missing, or `KASSIBER_REGTEST_DEMO_REBUILD=1` is set.

For even faster "start this chat on a real regtest book" flows, add or use a
cheap status lane before doing expensive work:

```bash
./scripts/integration-harness.sh demo-status
```

`demo-status` should be read-only: print whether the manifest exists, whether
the Compose project is running, whether RPC is reachable, the data root for
`pnpm dev:demo`, and the exact next command (`demo-up`, `demo-tick`, or
`demo-down`). It must not start Docker or mutate the book.

## Guardrails

- Never use mainnet funds, real wallet files, production descriptors, or public
  explorers for this harness. It is regtest-only.
- Do not expose RPC passwords in summaries. The harness stores persistent demo
  credentials in the local manifest with mode `0600`.
- If ports collide, follow the harness message: stop the persistent demo with
  `./scripts/integration-harness.sh demo-down` or choose another
  `KASSIBER_REGTEST_RPC_PORT`.
- If `uv` cache or Docker socket access fails because of sandboxing, rerun the
  same command with the needed approval rather than changing the command shape.
- Keep `demo-full` and `demo-up` semantics distinct: `demo-full` proves the
  built-in post-sync refresh path; `demo-up` is the reusable interactive book.

## References

Read `docs/reference/testing.md` when changing the harness behavior, explaining
the full scenario, debugging persistent demo mode, or deciding whether a lane's
coverage is sufficient.
