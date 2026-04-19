# Onboarding

Use this reference when the user is setting up Kassiber state, creating the first workspace or profile, or confirming the active paths and context.

## Core idea

`kassiber init` is non-interactive. It creates the managed state tree and reports the active paths. It does not create a workspace or profile.

## Fresh setup

```bash
kassiber init
kassiber status
```

For repo-local development where `kassiber` is not on `PATH`, use:

```bash
uv run kassiber init
uv run kassiber status
```

Common follow-up setup:

```bash
kassiber workspaces create personal
kassiber profiles create main \
  --workspace personal \
  --fiat-currency EUR \
  --tax-country at \
  --tax-long-term-days 365 \
  --gains-algorithm FIFO
kassiber context set --workspace personal --profile main
```

## Context and scope

Check scope before mutating data:

```bash
kassiber status
kassiber context show
kassiber context current
```

Use explicit scope flags when the current context is unclear:

```bash
kassiber profiles list --workspace personal
kassiber accounts list --workspace personal --profile main
```

## Profile behavior

Profiles carry tax defaults:

- `--fiat-currency`
- `--tax-country {generic,at}`
- `--tax-long-term-days`
- `--gains-algorithm {FIFO,LIFO,HIFO,LOFO}`

Creating a profile also creates default accounts:

- `treasury`
- `fees`
- `external`

## Paths

Default state root is usually `~/.kassiber`, but older machines may resolve to a legacy XDG path. Always verify with:

```bash
kassiber status
```

Override roots only when the user explicitly wants a custom location:

```bash
kassiber --data-root /custom/root/data status
kassiber --data-root /custom/root/data init
```
