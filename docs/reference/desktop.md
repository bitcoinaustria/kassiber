# Desktop Reference

Kassiber's desktop shell uses Tauri 2 + React + TypeScript with the Python
core running as a long-lived sidecar daemon over JSONL. See
[../plan/01-stack-decision.md](../plan/01-stack-decision.md) for the stack
decision and [../plan/04-desktop-ui.md](../plan/04-desktop-ui.md) for the
implementation plan.

The desktop shell is in active development. Until it ships, use the CLI as
the primary control surface — see [../../README.md](../../README.md) for
the quick start and [machine-output.md](machine-output.md) for the JSON
envelope contract that the future desktop shell will consume through the
daemon.

Current development modes:

- `pnpm dev` in `ui-tauri/` runs the browser dashboard against mock daemon
  fixtures.
- `pnpm tauri:dev` runs the Tauri shell, starts `python -m kassiber daemon`,
  and calls the Rust `daemon_invoke` boundary. The command allowlists the
  current UI data kinds; `status` is a real daemon round-trip, while UI
  snapshot kinds return `daemon_unavailable` until typed read models land.
  The supervisor uses `.venv/bin/python` when present, then `python3`, unless
  `KASSIBER_DAEMON_PYTHON` is set. `KASSIBER_REPO_ROOT` can point a dev
  shell at a different checkout; production packaging replaces this repo-root
  lookup with a bundled sidecar path.
