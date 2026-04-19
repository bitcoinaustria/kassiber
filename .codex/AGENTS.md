# Codex guidance for kassiber

This supplements the root `AGENTS.md` with Codex-specific workflow guidance.

## Default posture

- Prefer the repo-local kassiber skill surface under `skills/kassiber/` before generic habits.
- For non-trivial changes, gather evidence first, then restate the requirement, risks, and steps before editing.
- Run `./scripts/quality-gate.sh` before calling work push-ready.
- Review diffs as a separate pass after implementation.

## Recommended roles

- `explorer` for read-only codebase inspection and evidence gathering
- `reviewer` for correctness, regressions, docs drift, and missing tests
- `docs_researcher` only when external docs or tax/API verification actually matters

## Review priorities

1. user-visible CLI contract drift
2. tax and reporting correctness
3. security/privacy disclosure changes
4. missing test coverage for changed behavior
5. README / AGENTS / TODO / skill reference drift

## What not to do

- Do not create command forests or prompt machinery that duplicates the real Kassiber CLI.
- Do not introduce hidden hook behavior when an explicit script or CI check will do.
- Do not chase abstract coverage numbers over contract and regression safety.
