# TODO

Backlog for later sessions. In-progress work is called out at the top.

## In progress

### Finish Step 7: extract `importers.py`
`kassiber/importers.py` is written on disk (untracked, ~339 lines) but the
duplicate code has **not** been removed from `app.py` and nothing is
committed yet.

To finish:
1. Remove the following from `kassiber/app.py`:
   - `load_import_records`
   - `parse_btcpay_amount`, `parse_btcpay_labels`, `normalize_btcpay_record`,
     `load_btcpay_export_records`, `is_btcpay_format`
   - `_PHOENIX_REQUIRED_COLUMNS`, `_PHOENIX_OUTBOUND_TYPES`,
     `_PHOENIX_INBOUND_TYPES`, `parse_phoenix_fiat_amount`,
     `normalize_phoenix_record`, `load_phoenix_csv_records`, `is_phoenix_format`
   - `normalize_bip329_record`, `load_bip329_file`
2. Add `from .importers import load_import_records` (plus any format
   probes still referenced) at the top of `app.py`.
3. Run `uv run python -m py_compile kassiber/*.py` and the CLI smoke
   suite (`uv run python -m unittest tests.test_cli_smoke`).
4. Commit surgically (not `git add -A`): `git add kassiber/importers.py kassiber/app.py`.

The importers module is "parsers-only" on purpose — `ensure_tag_row`,
`invalidate_journals`, `resolve_scope`, and `resolve_wallet` stay in
`app.py` until later extraction steps.

## Remaining module split

`app.py` is down from 6,535 → 5,609 lines. Continue bottom-up extraction
so the DAG stays clean (no back-edges into `app.py`):

- **Step 8** — `rates.py`: local rates cache (CoinGecko sync + manual override).
- **Step 9** — `journals.py` + `metadata.py` + `reports.py`.
- **Step 10** — `wallets.py` plus sync adapters
  (`sync_esplora.py`, `sync_bitcoinrpc.py`, `sync_electrum.py`, `sync_liquid.py`).
- **Step 11** — `cli.py`: argparse tree + dispatch. After this `app.py`
  becomes a thin `main()` entry point.

Keep the smoke suite green after every step.

## Phase 3 follow-ups

- **3c** — Build `csv_mapping` DSL for the custom wallet kind so users
  can describe arbitrary CSV exports without code changes.
- **3d** — Wire the rates cache into journal processing (use cached
  rates for cost-basis lookups instead of per-call fetches).
- **3e** — Add account adjustments and rate overrides surface.
- **3f** — Per-profile Tor proxy configuration (the backends table
  already supports per-backend timeout; extend to SOCKS proxy).
- **3g** — Replace BTCPay file-based imports with Greenfield API-backed
  sync/import flow. Follow-up once that lands: attach optional BTCPay
  `InvoiceId` metadata to imported on-chain wallet addresses instead of
  trying to infer invoice/address matches from CSV exports.
- **3h** — At-rest encryption for sensitive fields. Nothing on disk is
  encrypted today (SQLite DB, `config/backends.env` / legacy `.env`, exports). Target seamless OS
  keychain integration — macOS Keychain, Linux freedesktop
  secret-service / libsecret, Windows DPAPI / Credential Manager — so
  SLIP77 blinding keys, backend tokens, auth headers, and RPC
  credentials are sealed by default and unlocked on demand without a
  separate passphrase. Call it out prominently in
  [SECURITY.md](SECURITY.md) until it lands.

## Phase 4 — Skills bundle

Author `skills/kassiber/` with `SKILL.md`, `references/`, and `scripts/`
so an agent can drive the CLI from a single skill invocation. The JSON
envelope contract is already stable enough to script against.

## Phase 5 — Optional server/REST mode

Wrap the CLI commands in an HTTP layer (stdlib `http.server` or a tiny
ASGI app) so remote agents can issue envelope-returning calls. Keep
local-first as the default; server mode stays opt-in.

## Bugs

- **`rates set ETH-USD` validation bug.** `rates set` accepts asset
  codes that should be rejected. Bitcoin-only reproduction:
  `rates set BTC-JPY 0.0` (valid pair, exercises the same path) vs.
  `rates set BTCUSD 0.0` (bad syntax — missing dash — currently
  swallowed). Spawned task exists separately; verify with the smoke
  suite once fixed.

## Tech debt

- `_emit_error` helper in `app.py` should move to `envelope.py` — it is
  part of the envelope contract and currently straddles modules.
- `_RP2_MODULES` / `get_rp2_modules` in `app.py` belongs alongside the
  eventual `rp2_engine.py` (or temporarily in `journals.py`) — it is the
  only RP2-facing surface still living in `app.py`.

## Conventions (reminders)

- Internal unit is **millisatoshi (msat)**, 1 BTC = 100_000_000_000 msat.
- Kassiber is pre-release: rename/remove freely, no BC shims, but keep
  docs (README, AGENTS) in lockstep with code changes.
- Bitcoin-only in tests, CSV fixtures, docs, and repros — never ETH or
  altcoins. Use BTC-JPY, BTCUSD (bad syntax), etc. for variety.
- Do not mention the closed-source reference product by name anywhere.
- Surgical `git add <paths>` — never `git add -A` (we already leaked
  `log/` once; it is now ignored).
