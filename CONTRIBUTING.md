# Contributing

Kassiber is a local-first Bitcoin accounting project. Keep changes small, explicit, and easy to verify.

## Setup

```bash
uv sync --locked
```

`uv` owns the repository environment, locked dependency installation, and
Python command execution. `--locked` fails when `uv.lock` is stale instead of
rewriting it. `pip install kassiber` remains a supported packaging
contract for end users, not a parallel development workflow.

## Workflow

1. Read `AGENTS.md` before non-trivial changes.
2. For changes touching CLI behavior, tax logic, schema, reports, or multiple docs, start with a short plan: requirement, risks, steps.
3. Prefer extending existing behavior-pin tests over adding sprawling new test surfaces.
4. Keep docs in lockstep with behavior changes.
5. Review the diff as a separate pass before push.

## Quality gate

Run this before push or PR:

```bash
./scripts/quality-gate.sh
```

For a focused Python test during development:

```bash
uv run --locked python -m pytest tests/test_cli_smoke.py -q
```

That covers:
- compile sanity
- every Python test module exactly once through pytest (including unittest tests)
- in-process CLI help coverage plus a small real-subprocess entrypoint smoke
- TypeScript typechecking and ESLint
- the complete desktop Vitest suite

Pull-request CI uses the same Python manifest but splits it into disjoint
domain shards. Safe shards use two pytest-xdist workers; socket-sensitive,
daemon-listener, broad-regression, and process/integration modules use isolated
serial jobs. JUnit artifacts and the 50
slowest tests are retained per shard.

For local-first wallet-sync proof beyond the default gate, use the opt-in
integration harness:

```bash
./scripts/integration-harness.sh fast          # no Docker, no egress
./scripts/integration-harness.sh bitcoin-core  # disposable Bitcoin Core regtest
./scripts/integration-harness.sh demo-full     # full multi-wallet accounting demo
```

For interactive development against a real book instead of mock fixtures
(needs Docker, `uv`, and `pnpm`):

```bash
./scripts/integration-harness.sh demo-up       # persistent regtest node + demo book
cd ui-tauri && pnpm dev:demo                   # dev preview backed by the real daemon
```

See [`docs/reference/testing.md`](docs/reference/testing.md) for the tiered
FAST/SLOW model, the developer demo environment, and Docker/regtest
guardrails.

### Desktop UI (frontend)

The full quality gate covers the Tauri/React checks. For focused changes under
`ui-tauri/`, run them directly from that directory:

```bash
pnpm typecheck      # incl. type-safe i18n keys
pnpm test --run     # incl. the en/de key-parity guard
pnpm lint
```

The UI is bilingual (English + Austrian German). When you touch a user-facing
string, update `ui-tauri/src/i18n/locales/en/<ns>.json` **and**
`de/<ns>.json` in lockstep (the parity test enforces it) and follow the term
glossary. See [`docs/reference/i18n.md`](docs/reference/i18n.md) (dev workflow)
and [`docs/reference/i18n-glossary.md`](docs/reference/i18n-glossary.md). The
CLI/daemon stay English and machine-deterministic — do not localize them.

## Prerelease binaries

`.github/workflows/prerelease-binaries.yml` builds unsigned CLI binaries on
macOS Apple Silicon, macOS Intel, and Linux. Manual workflow runs upload
`.tar.gz` artifacts; `v*` tag pushes also attach those artifacts and their
SHA-256 files to a GitHub prerelease. Linux CLI binaries are built on Ubuntu
22.04 to match the AppImage portability floor. CLI archives use
`kassiber-cli-<target>.tar.gz` filenames and contain an executable named
`kassiber`. The workflow also builds unsigned desktop previews for Apple
Silicon macOS (`.app` zip / `.dmg`), Linux (`.AppImage`), and Windows (`.msi` plus
NSIS setup `.exe`) with `kassiber-desktop-<target>-...` filenames. Desktop
previews bundle a one-file Kassiber CLI sidecar, so they do not expect an
external Kassiber-capable Python environment for normal daemon calls. The GUI
executable also forwards `--cli ...` to the bundled CLI sidecar for
installed-app CLI use. Raw bundled sidecar files are internal to desktop
packages and must not be published as release assets. The desktop shell
displays the build commit beside the version number; CLI artifact filenames and
`.sha256` sidecars still do not embed the commit hash.

Pull requests intentionally do not build binaries automatically. If a maintainer
asks for binaries for a PR or branch, run `prerelease-binaries` manually against
that branch and leave the result as workflow artifacts. Only publish to GitHub
Releases for real `v*` prerelease tags. See
[`docs/reference/prerelease-binaries.md`](docs/reference/prerelease-binaries.md)
for the exact commands and commit-hash caveats.

## Pull requests

PRs should say:
- what changed
- why it changed
- what verification ran
- any intentional follow-up left out

## Documentation surfaces

If behavior changes, check whether these also need updates:
- `README.md`
- `docs/quickstart.md`
- `AGENTS.md`
- `TODO.md`
- `kassiber/ai/skill_references/` and the external CLI Agent Skill at
  `bitcoinaustria/kassiber-skill`
- `docs/reference/i18n.md` + `docs/reference/i18n-glossary.md` when UI strings or German terminology change
- `SECURITY.md` when privacy or external I/O changes

## Testing philosophy

Kassiber cares more about contract and regression safety than vanity metrics. Prioritize:
- `tests.test_cli_smoke`
- `tests.test_review_regressions`
- focused additions when user-visible behavior changes
