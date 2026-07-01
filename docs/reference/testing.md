# Testing

How Kassiber is tested today, and the planned real-node integration harness
(issue [#312](https://github.com/bitcoinaustria/kassiber/issues/312)).

## TL;DR

```bash
# Fastest useful check — the behavior pin (~1s, stdlib unittest, no pytest):
uv run python -m unittest tests.test_cli_smoke -v

# Full Python gate before any push (compile + smoke + regression + CLI help):
./scripts/quality-gate.sh

# Desktop UI (run from ui-tauri/ changes):
pnpm --dir ui-tauri run typecheck && pnpm --dir ui-tauri test --run && pnpm --dir ui-tauri lint
```

Dependencies come from `uv sync` (then prefix commands with `uv run`) or
`pip install -e .` inside an activated venv (then use `python3`). The `rp2`
tax engine is a pinned git dependency, so a network-reachable GitHub is
required for the initial dependency sync.

## How tests are structured today

- **Framework:** 100% Python standard-library `unittest`. There is **no
  pytest** dependency and **no auto-discovery** — `scripts/quality-gate.sh`
  runs an explicit, hand-maintained list of `unittest` modules. Add new
  modules to that list or they will not run in the gate.
- **Single trusted entrypoint:** `./scripts/quality-gate.sh` is the one path
  humans and agents share. It runs `compileall`, the module list, and a live
  CLI smoke workflow against a throwaway `HOME`.
- **Behavior pin:** `tests/test_cli_smoke.py` asserts envelope `kind` +
  `schema_version`, msat fields, importer counts, balance-sheet totals, and
  the error-envelope shape. Prefer **extending it** over adding new files
  (see [CONTRIBUTING.md](../../CONTRIBUTING.md) → Testing philosophy).
- **Drift guards** keep parallel sources of truth in lockstep:
  `test_dependency_drift`, `test_connection_catalog_drift` (daemon-kind
  allowlists), `test_report_contract_drift`, `test_homebrew_cask`.
- **Determinism seams** already in the tree — the foundation the harness
  builds on:
  - `tests/fixtures/generic_rp2_transfer_snapshot.json`, enforced by
    `tests/test_review_regressions.py`, is a golden snapshot of the full
    journal → capital-gains → holdings output. This `assertEqual(actual,
    fixture)` pattern is the template for new end-to-end scenarios.
  - `kassiber/core/report_verify.py` appends a self-verifying XLSX layer:
    live `write_formula` cells carry Kassiber's number as the cached value
    and are checked `OK`/`DIFF` against it. This is a content-level oracle —
    prefer it over byte-comparing exports.
- **Backend tests today stub transports** rather than talking to live
  infrastructure: `test_sync_backends.py` patches a `FakeElectrumClient` /
  `fake_fetch` / `fake_scan`; `test_sync_btcpay_incremental.py` uses an
  in-memory `_Opener`; `test_liquid_electrum_sync.py` uses `_FakeTx` /
  `_FakeInput` / `_FakeOutput`. This is fast and deterministic but does not
  prove behavior against real wallet state — the motivation for #312.

## Reproducibility recommendations (today)

Until the harness lands, these keep output stable across machines:

- **Isolated state:** run against a temp data root, e.g.
  `--data-root "$(mktemp -d)/data"`, so tests never touch `~/.kassiber`.
- **Timezone / locale:** export `TZ=UTC` and `LC_ALL=C.UTF-8` before running
  anything whose output you intend to snapshot. Several code paths format
  timestamps and numbers, and locale (e.g. the Austrian decimal comma) or a
  local timezone will otherwise diverge machine-to-machine.
- **No real funds or secrets:** regtest / watch-only only. Never commit real
  descriptors, xpubs beyond the designated public fixture, wallet files, API
  tokens, or `.env` credentials. Kassiber is watch-only by design; spend keys
  belong only inside disposable test nodes.

## Planned: real-node integration harness (issue #312)

> **Status: proposed / not yet implemented.** The tiers, env vars, and scripts
> below are the agreed design from #312 and its verification pass. They do not
> exist in the tree yet; this section is the roadmap the implementing PRs
> follow. Do not document them as current behavior until they land.

The goal is local-first *provable* correctness: real regtest wallets and
transactions driven through Kassiber's real sync → journal → report → export
paths, reproducible by any contributor, offline, with no funds and no leaks.
The strategy is tiered so **most PRs never need Docker**:

| Tier | Speed | Docker? | Purpose |
| --- | --- | --- | --- |
| **FAST** (default merge gate) | seconds | no | Recorded regtest "tapes" (raw esplora/electrum/rpc wire responses captured once) replayed through the *same* `sync_backends.py` adapters, plus golden snapshots of journal/report/export output |
| **MEDIUM** (opt-in, CPU-only) | ~1 min | no | Large deterministic journal / transfer-matching / tax / report sweeps against committed fixtures, incl. multi-wallet BTC + Liquid cross-asset scenarios |
| **SLOW** (opt-in / nightly / PR-label) | minutes | yes | The real `docker compose` regtest stack (bitcoind, Fulcrum/Electrum, esplora HTTP, Elements/Liquid; BTCPay optional) — the source of truth that generates the tapes |

Proposed opt-in switches (all default off; the FAST tier stays the only
required check):

- `KASSIBER_INTEGRATION=1` — enable the SLOW Docker lane.
- `KASSIBER_MEDIUM=1` — enable the serviceless MEDIUM sweeps.
- `KASSIBER_NO_EGRESS=1` — process-level `socket.connect` kill-switch that
  raises on any non-loopback connection; the machine-checked guarantee that
  no fast/medium test can reach a live exchange or backend.
- `KASSIBER_FROZEN_NOW=<rfc3339>` — freeze every wall-clock read (correlated
  with bitcoind `setmocktime`) so tax math and `generated_at` stamps are
  byte-stable across runs and machines.

### Two waves

**Wave 1 — no Docker, no dependency on the compose stack (land first):**
hermetic env (`PYTHONHASHSEED`/`TZ`/`LC_ALL`, seeded RNG, `KASSIBER_NO_EGRESS`);
the frozen-clock seam + a grep guard (wall-clock is read in ~40 places,
including the exit-tax deemed-disposal fallback, so tax snapshots are not
reproducible until this exists); the three-tier gate arrays; offline pricing
fixtures (incl. a BTC-EUR cent-rounding case); content-level export snapshots;
a mutation suite proving the oracles fail on corruption; and the missing
positive golden — a multi-wallet **BTC + Liquid** transfer/swap → journal →
report snapshot.

**Wave 2 — rides the compose stack:** regtest genesis/chain guard before any
tx generation; the tape *record* phase (with provenance stamps so a tape and
its golden cannot silently diverge); backend-parity SLOW cases; digest-pinned
images + an arm64 replay leg; a label-triggered advisory CI lane; and
scrub-before-upload redaction on CI artifacts.

### Verified constraints and gotchas (do not repeat these mistakes)

- **Core-RPC descriptor parity is blocked on
  [#310](https://github.com/bitcoinaustria/kassiber/pull/310)** (unmerged
  draft). `sync_backends.py` currently rejects descriptor-backed `bitcoinrpc`
  sync. The backend-parity matrix must **skip-with-reason** (not silently
  green) and carry an `expectedFailure` case that flips to required-green when
  #310 lands.
- **Liquid live sync is esplora/electrum only** — there is no self-hosted
  `elements_rpc` backend. For the Liquid regtest guard, assert
  `getblockchaininfo.chain == "elementsregtest"`; do **not** hardcode a genesis
  hash. (The `5ac9f65c…` constant in the tree is the elementsregtest **policy
  asset id**, not a block genesis.) The BTC regtest genesis
  `0f9188f1…466e2206` is a valid anchor.
- **Never byte-compare exports.** XLSX (XlsxWriter timestamps) and ReportLab
  PDFs (`/ID`, `/CreationDate`) are not byte-deterministic, and float/locale
  rendering differs by machine. Extract content and assert on it; lean on
  `report_verify.py`'s `OK`/`DIFF` oracle to prove the tax figures reached the
  export.
- **No wall-clock TTL as a hard gate.** A "fixture too old after date X"
  assertion is a self-inflicted time-bomb that fails on every machine once the
  date passes. Assert *internal* consistency instead (every scenario tx
  resolves a cached rate).
- **There is no scheduled CI.** Record/nightly lanes must be
  `workflow_dispatch` or PR-label triggered (following
  `prerelease-binaries.yml`'s precedent).

### Guardrails (non-negotiable)

- Regtest by default; no mainnet funds, no user wallet files, no production
  descriptors, no real secrets. Kassiber stays watch-only.
- Disposable per-run credentials — never copy static credentials or
  persistent paths from borrowed compose files into the repo.
- Deterministic pricing is local fixture / rate-cache setup, not live
  exchange egress. No new product egress rules; any signet/Tor case is
  explicitly opt-in.
- Logs, diagnostics, export artifacts, and failure artifacts must preserve the
  existing redaction boundaries (`kassiber/redaction.py`,
  `kassiber/log_ring.py`; see [logging.md](logging.md)).
