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
