# Desktop Reference

Kassiber ships an early PySide6/QML desktop shell over the same local SQLite store used by the CLI.

Launch it with:

```bash
python3 -m kassiber ui
```

## Current scope

The desktop app is still early. Today it mainly provides:

- the Phase 1 shell and frame
- empty state
- project/profile surface
- placeholder settings and add-connection dialogs
- persisted window size and position

It should be thought of as a thin local UI over the same core functionality the CLI already exposes.

## Architecture

- the UI imports the shared Kassiber core
- long-running work should happen off the UI thread
- the CLI and desktop are meant to be peers over the same local-first runtime

## What to use today

Use the CLI for the real workflow:

- creating workspaces, profiles, accounts, wallets
- syncing transactions
- imports
- journal processing
- reports
- transfer pairing
- metadata and attachments

Use the desktop shell today as an early companion interface, not as the primary control surface.

## Design docs

See the plan docs for the fuller desktop direction:

- [../plan/01-stack-decision.md](../plan/01-stack-decision.md)
- [../plan/04-desktop-ui.md](../plan/04-desktop-ui.md)
- [../design/README.md](../design/README.md)
