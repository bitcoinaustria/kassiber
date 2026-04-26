# Contributing

Kassiber is a local-first Bitcoin accounting project. Keep changes small, explicit, and easy to verify.

## Setup

```bash
uv sync
```

Or use a virtualenv and `pip install -e .`, but the repo examples and lockfile assume `uv`.

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

That covers:
- compile sanity
- CLI smoke suite
- review regression suite
- key CLI help/smoke checks

## Prerelease binaries

`.github/workflows/prerelease-binaries.yml` builds unsigned CLI binaries on
macOS Apple Silicon, macOS Intel, and Linux. Manual workflow runs upload
`.tar.gz` artifacts; `v*` tag pushes also attach those artifacts and their
SHA-256 files to a GitHub prerelease. Linux CLI binaries are built on Ubuntu
22.04 to match the AppImage portability floor. CLI archives use
`kassiber-cli-<target>.tar.gz` filenames and contain an executable named
`kassiber`. The workflow also builds unsigned desktop previews for universal
macOS (`.app` zip / `.dmg`), Linux (`.AppImage`), and Windows (`.msi` plus
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
- `AGENTS.md`
- `TODO.md`
- `skills/kassiber/`
- `SECURITY.md` when privacy or external I/O changes

## Testing philosophy

Kassiber cares more about contract and regression safety than vanity metrics. Prioritize:
- `tests.test_cli_smoke`
- `tests.test_review_regressions`
- focused additions when user-visible behavior changes
