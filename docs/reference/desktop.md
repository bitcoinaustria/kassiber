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
