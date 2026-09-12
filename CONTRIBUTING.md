# Contributing

Keep changes small, explicit, and easy to verify. Read [AGENTS.md](AGENTS.md)
for cross-cutting invariants and links to subsystem guidance.

## Setup

Python development uses the locked `uv` environment:

```sh
./scripts/bootstrap-dev-env.sh
```

The script checks system prerequisites, runs `uv sync --locked`, and verifies
imports. Use `uv run --locked` for Python commands so a stale lockfile fails
instead of being rewritten. End-user pip installation remains a packaging
contract, not a second contributor workflow.

For frontend development, use the pnpm version pinned in
[ui-tauri/package.json](ui-tauri/package.json):

```sh
pnpm --dir ui-tauri install --frozen-lockfile
```

The daemon-backed preview and disposable regtest setup are documented in
[Testing](docs/reference/testing.md). Keep development commands off real books.

## Verification and review

During development, run the smallest relevant checks. For example:

```sh
uv run --locked python -m pytest tests/test_cli_smoke.py -q
```

Focused frontend checks, from `ui-tauri/`:

```sh
pnpm typecheck
pnpm test --run
pnpm lint
```

Before push or PR, run the full gate:

```sh
./scripts/quality-gate.sh
```

It compiles Python, validates the test inventory, runs all Python tests once,
and runs TypeScript, ESLint, and Vitest. CI partitions that Python inventory
into disjoint shards; the required aggregate and specialized integration lanes
are explained in [Testing](docs/reference/testing.md#pull-request-ci).
The full gate is required before calling work push-ready.

Stage explicit paths; do not use `git add -A`. Review staged and unstaged diffs
separately after implementation. When a second
agent is available, have it review the same change; otherwise perform a manual
second pass. Fix P1/P2 correctness or consistency findings before push and
report deferred lower-severity concerns.

Prefer existing behavior-pin and regression tests when they cover the change.
Choose tests for contract and regression safety rather than coverage numbers.
Shared changes need proof across the CLI, daemon, desktop, and applicable AI
or provider projections, not only the surface that was edited.

## Documentation

Update the reference that owns changed behavior. Check the quick start,
relevant in-product AI references, and any affected CLI skill instructions.
Update [TODO.md](TODO.md) when an item is completed or materially reshaped.
Keep the README an introduction and AGENTS.md contributor guidance; detailed
feature descriptions belong in [docs/reference/](docs/reference/).

UI strings and terminology follow [the localization guide](docs/reference/i18n.md)
and [Austrian German glossary](docs/reference/i18n-glossary.md). External I/O
and disclosure changes also require [privacy documentation](docs/reference/privacy-and-security.md).

## Pull requests and releases

PR descriptions explain the problem, resulting behavior, verification, and
intentional follow-up. When committing is in scope, use coherent, reviewable
commits and separate refactors from behavior changes where practical.

For branch/tester binaries, manually run the packaging workflow and keep its
outputs as workflow artifacts. Do not create a release for a tester build or
broaden PR packaging triggers without an explicit policy change.

Tagged releases follow [prerelease packaging](docs/reference/prerelease-binaries.md)
and [local signing and notarization](docs/reference/macos-release.md). Never put
Apple signing private keys in CI or publish failed platform checks. Raw bundled
sidecars are internal package contents, not standalone release assets.
