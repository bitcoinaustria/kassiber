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
- **3h-pre** — Self-transfer detection is conservative on purpose: it only
  pairs single-out + single-in rows that share `external_id`. Multi-output
  transactions that fan out to several owned wallets, or 1→N coinjoin-style
  splits, are not auto-paired. The `transfers pair / list / unpair` CLI
  (table `transaction_pairs`) covers the 1↔1 manual case; extending it to
  N-tuples (one outbound consumed by multiple inbound legs, or vice versa)
  is still pending. Detection lives in `kassiber/transfers.py`.
- **3h-pre3** — Cross-asset carrying-value swaps (BTC ↔ LBTC peg, on-chain
  ↔ Lightning submarine swap) are stored today as audit metadata only:
  `transfers pair --policy taxable` is accepted, the legs still process as
  a normal SELL + BUY through the lot engine, and the pair surfaces in
  `cross_asset_pairs` on the ledger state and `journals process` envelope.
  `--policy carrying-value` is rejected at CLI creation time. Properly
  carrying basis across asset boundaries needs unified FIFO that spans
  assets (so disposing the inbound LBTC consumes the original BTC lots,
  with only the network fee realizing a gain) — RP2's `IntraTransaction`
  is same-asset only, so this is either a custom layer on top of RP2 or a
  bespoke cross-asset lot tracker.
- **3h-pre2** — Per-wallet portfolio rows currently show
  `wallet_quantity * asset_avg_residual_basis`. That sums correctly to the
  global residual basis but it is an allocation, not physical-lot tracking
  — a disposal in wallet A may consume basis acquired in wallet B once
  cross-wallet pooling is enabled. If physical-lot wallet attribution is
  ever required (e.g. for jurisdiction-specific reporting), wrap RP2 with
  a per-wallet lot tracker layered on top of the global FIFO/LIFO order.
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
