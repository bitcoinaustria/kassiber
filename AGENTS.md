# AGENTS.md

## Project shape

- Kassiber is a local-first Bitcoin accounting CLI.
- The CLI entrypoint lives in [kassiber/cli/main.py](kassiber/cli/main.py). The remaining command implementation surface lives in [kassiber/cli/handlers.py](kassiber/cli/handlers.py).
- Supporting modules (bottom-up — no back-edges into the CLI layer):
  - [kassiber/errors.py](kassiber/errors.py) — `AppError` typed exception carrying `code`, `hint`, `details`, `retryable`.
  - [kassiber/time_utils.py](kassiber/time_utils.py) — timestamp parsing + RFC3339 formatting and `UNKNOWN_OCCURRED_AT`.
  - [kassiber/msat.py](kassiber/msat.py) — `SATS_PER_BTC`, `MSAT_PER_BTC`, `dec`, `btc_to_msat`, `msat_to_btc`.
  - [kassiber/util.py](kassiber/util.py) — tiny type-coercion helpers (`str_or_none`, `parse_bool`, `parse_int`, chain/network normalizers).
  - [kassiber/envelope.py](kassiber/envelope.py) — JSON envelope contract, `emit`, table/plain/csv output writers, and the `_KIND_SUBCOMMAND_ATTRS` kind map.
  - [kassiber/db.py](kassiber/db.py) — SQLite schema, `open_db`, data-root resolution, settings helpers, and msat column migrations.
  - [kassiber/backends.py](kassiber/backends.py) — dotenv (`config/backends.env`) seed + DB overlay for named sync backends, plus CRUD helpers.
  - [kassiber/cli/handlers.py](kassiber/cli/handlers.py) — remaining CLI command handlers and compatibility-layer imports while deeper decomposition continues.
  - [kassiber/core/attachments.py](kassiber/core/attachments.py) — transaction attachment storage, URL-reference handling, integrity verification, and orphan-file GC for the managed attachment tree.
  - [kassiber/core/engines/__init__.py](kassiber/core/engines/__init__.py) — tax-engine interface/resolver; selects the generic RP2 engine or the experimental Austrian engine by profile tax policy.
  - [kassiber/core/engines/austria.py](kassiber/core/engines/austria.py) — experimental Austrian ledger builder on the shared engine seam; processes supported flows conservatively and quarantines unsupported provenance.
  - [kassiber/core/tax_events.py](kassiber/core/tax_events.py) — in-memory normalization seam between raw transaction rows and tax-engine inputs, including early quarantine classification for under-specified tax semantics.
  - [kassiber/core/sync.py](kassiber/core/sync.py) — wallet sync orchestration above backend-specific transport details.
  - [kassiber/core/sync_backends.py](kassiber/core/sync_backends.py) — descriptor target discovery plus `esplora`, `electrum`, and `bitcoinrpc` live-sync adapters.
  - [kassiber/core/reports.py](kassiber/core/reports.py) — extracted report builders, balance-history calculations, and PDF export assembly behind hookable journal/runtime dependencies.
  - [kassiber/tax_policy.py](kassiber/tax_policy.py) — profile tax-policy layer.
  - [kassiber/wallet_descriptors.py](kassiber/wallet_descriptors.py) — descriptor normalization, chain/network validation.
- Packaging is defined in [pyproject.toml](pyproject.toml).
- User-facing behavior is documented in [README.md](README.md).
- Third-party dependency and license notes are tracked in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
- In-flight and deferred work is tracked in [TODO.md](TODO.md) — it is the
  current execution backlog for core extraction, attachments, tax-engine
  cleanup, Austrian tax support, and the later desktop UI work.

Phase 0 core extraction is green: the CLI/runtime surface is split out of
the old `kassiber/app.py` monolith, the smoke suite passes, and future
work should build on the extracted modules instead of re-growing a shim.

Kassiber is currently in **dev mode**: renaming commands, breaking flags, and reshaping subcommand trees is acceptable as long as docs in the tree are updated in the same change. There is no deprecation-alias layer.

## Current architecture

- Data lives in a local SQLite database (system of record).
- Default user state lives under `~/.kassiber/{data,config,exports,attachments}` unless `--data-root` / `--env-file` overrides it; the managed layout manifest lives at `~/.kassiber/config/settings.json`.
- The CLI model is:
  - backend (dotenv seed + DB overlay via the `backends` table)
  - workspace
  - profile (carries tax policy defaults)
  - account
  - wallet
  - transactions
  - attachments
  - metadata (notes, tags, inclusion)
  - journals (RP2 processing + quarantine)
  - reports (balance-sheet, portfolio-summary, capital-gains, journal-entries, balance-history)
  - rates (local cache + CoinGecko sync + manual override)
- Every command accepts `--format {table,plain,json,csv}`, `--output <path>`, `--machine` (= `--format json`), and `--debug`.
- Successful responses use `{kind, schema_version, data}`. Errors use `{kind: "error", schema_version, error: {code, message, hint, details, retryable, debug}}`.
- Live sync kinds implemented: `esplora`, `electrum`, `bitcoinrpc`.
- BIP329 records are stored in SQLite and transaction labels are bridged into Kassiber tags.
- BTCPay CSV/JSON imports become transactions, with comments mapped to notes and labels mapped to tags.
- Transaction attachments are stored in a managed `attachments/` state sibling; file attachments are copied locally and URL attachments remain literal strings with no fetching or indexing.
- Profile-level tax defaults are stored on `profiles` as `fiat_currency`, `tax_country`, `tax_long_term_days`, and `gains_algorithm`.
- Wallet-level tax provenance stays in `wallets.config_json`, including the manual `altbestand` flag.

## Command surface

- `init`, `status`, `context {show,current,set}`
- `workspaces {list,create}`
- `profiles {list,create,get,set}`
- `accounts {list,create}`
- `wallets {kinds,list,create,get,update,delete,sync,derive,set-altbestand,set-neubestand,import-json,import-csv,import-btcpay,import-phoenix}`
- `backends {kinds,list,get,create,update,delete,set-default,clear-default}`
- `transactions {list}`
- `attachments {add,list,remove,verify,gc}`
- `metadata records {list,get,note {set,clear},tag {add,remove},excluded {set,clear}}`
- `metadata bip329 {import,list,export}`
- `journals {process,list,quarantined,events {list,get},quarantine {show,clear,resolve {price-override,exclude}}}`
- `transfers {pair,list,unpair}`
- `reports {balance-sheet,portfolio-summary,capital-gains,journal-entries,balance-history}`
- `rates {pairs,sync,latest,range,set}`

## Pagination

List endpoints with `--limit` also accept `--cursor`. The cursor is an opaque base64 urlsafe token built from `<occurred_at>|<created_at>|<id>`. Responses include `next_cursor` (or `null`) and `has_more`.

## Tax engine

- The tax engine now goes through `kassiber/core/engines.build_tax_engine(...)`; the current implementation behind that seam is still the generic RP2 engine in `kassiber/core/engines/rp2.py`.
- Journal processing first normalizes raw transaction rows into in-memory tax events via `kassiber/core/tax_events.py`; raw `transactions` rows remain the source of truth and no derived regime state is persisted back onto them.
- Under-specified tax semantics that used to fall through raw-row handling should quarantine at the normalization boundary instead of being guessed. That includes malformed same-asset transfers, missing required pricing, and unsupported tax directions.
- The generic RP2 engine now owns the per-profile journal orchestration behind the engine seam: transfer detection, manual-pair application, per-asset grouping, normalized event preparation, and holdings aggregation all live in `kassiber/core/engines/rp2.py`, while CLI handlers only load rows and persist the resulting journal state.
- Snapshot coverage for the current generic transfer path lives in [tests/fixtures/generic_rp2_transfer_snapshot.json](tests/fixtures/generic_rp2_transfer_snapshot.json) and is enforced by `tests/test_review_regressions.py` in addition to the CLI smoke suite.
- Policy selection and RP2 country defaults are centralized in `kassiber/tax_policy.py`.
- RP2 runs per-asset (pooled across all wallets of a profile) so `IntraTransaction` (MOVE) carries cost basis between user-owned wallets. Wallet identity is preserved by setting RP2's `exchange` to the wallet label and recovering per-wallet quantity buckets via `BalanceSet`.
- Self-transfer detection lives in `kassiber/transfers.py`. The detector pairs same-`external_id` outbound + inbound rows across two wallets of the same profile; the journal pipeline turns each pair into an `IntraTransaction` plus `transfer_out` / `transfer_in` (and, when there's a fee, `transfer_fee`) ledger entries.
- Manual pairing via `transfers pair / list / unpair` (table `transaction_pairs`) overrides auto-detection: `apply_manual_pairs` in `kassiber/transfers.py` filters out any auto-pair that touches a manually-paired row. Same-asset manual pairs currently support `--policy carrying-value` and feed the existing IntraTransaction path; same-asset `--policy taxable` is rejected and users should leave those legs unpaired to preserve normal SELL + BUY treatment. Cross-asset pairs (BTC ↔ LBTC peg-ins/peg-outs, submarine swaps) are stored as audit metadata and surfaced via `cross_asset_pairs` in the ledger state and the `journals process` envelope; the legs themselves still process as a normal SELL + BUY because RP2 `IntraTransaction` is same-asset only. Cross-asset `--policy carrying-value` is rejected at CLI creation time — implementing it requires unified FIFO across assets and is deferred (see TODO.md).
- Liquid peg-in/peg-out detection must not lean on hardcoded federation addresses (per-claim tweaked, federation keys rotate). Use the manual pair CLI or non-address heuristics (time + amount + direction inversion + same-profile constraint) instead.
- Per-wallet portfolio rows show that wallet's residual quantity at the asset's average residual basis — an allocation, not a physical-lot answer.
- Supported lot selection: `FIFO`, `LIFO`, `HIFO`, `LOFO`.
- Profiles expose the RP2 `generic` tax policy and an explicitly experimental Austrian `at` policy registration. Austrian profiles normalize to EUR and keep the legacy `tax_long_term_days` field shape for `Altbestand`; supported Austrian journal flows now process through `kassiber/core/engines/austria.py`, while ambiguous provenance quarantines instead of being guessed. Austrian JSON report envelopes carry top-level `experimental` / `review_required` markers so automation does not lose the review gate after journals are processed.
- Wallets can be flagged manually as `Altbestand`; disposals from those wallets are treated as tax-free while Neubestand wallets use normal tax treatment.
- Journals must be reprocessed after any transaction, metadata, or exclusion change before reports are trusted.
- Transactions without usable fiat pricing are quarantined during journal processing instead of receiving zero-basis tax treatment.

## Working rules

- Keep the project local-first.
- Prefer standard-library solutions unless a dependency clearly buys a lot.
- Keep `--machine` output deterministic — add a `kind` to every new envelope.
- Keep envelope error shapes consistent: use `AppError(code=..., hint=..., retryable=..., details=...)`.
- Per-asset pooling is intentional so RP2 `IntraTransaction` works across wallets; per-wallet output remains via `BalanceSet`. Do not regress to per-wallet RP2 calls without thinking through the transfer story first.
- Keep wallet-level `Altbestand` handling separate from profile-level country policy unless there is a deliberate migration plan.
- Preserve the default `mempool.space` Esplora backend unless there is a strong reason to change it.
- Prefer additive schema changes that work with `CREATE TABLE IF NOT EXISTS`.
- Prefer lightweight compatibility migrations for existing SQLite databases when adding profile fields.
- When a `TODO.md` item is completed or materially reshaped, update
  `TODO.md` in the same change and check or split the item so the backlog
  stays truthful.
- When adding a new runtime dependency, update both the README dependency story and `THIRD_PARTY_LICENSES.md`.
- Keep `THIRD_PARTY_LICENSES.md` concise: direct dependencies and notable license constraints matter more than a hand-maintained transitive dump.

## Verification

All commands below assume project dependencies are installed — either via `uv sync` (then prefix with `uv run`) or via `pip install -e .` inside an activated venv (then use `python3` directly). The examples use `uv run python` because it works without pre-activation; swap in `python3` when working inside an activated venv.

- Compile check:

```bash
PYTHONPYCACHEPREFIX=/tmp/kassiber-pyc uv run python -m py_compile kassiber/*.py
```

- End-to-end CLI smoke test (stdlib `unittest`, no pytest dep, ~1s):

```bash
uv run python -m unittest tests.test_cli_smoke -v
```

  This is the behavior pin. If you refactor internals the suite MUST
  still pass unchanged — it asserts envelope `kind` + `schema_version`,
  msat fields, Phoenix import counts, balance-sheet totals, and
  error-envelope shape. Prefer extending this suite to adding new test
  files.

- CLI smoke checks:

```bash
uv run python -m kassiber --help
uv run python -m kassiber --machine status
uv run python -m kassiber backends list
uv run python -m kassiber wallets kinds
uv run python -m kassiber profiles create --help
uv run python -m kassiber metadata records --help
uv run python -m kassiber attachments list --help
uv run python -m kassiber journals events --help
uv run python -m kassiber reports balance-history --help
uv run python -m kassiber rates --help
```

- Safe local workflow:
  - create a temp data root via `--data-root /tmp/smoke/data`
  - `init`, then create workspace/profile/wallet and seed transactions
  - verify `profiles list` shows `tax_country` and `tax_long_term_days`
  - optionally mark a wallet as `Altbestand`
  - import priced CSV, BTCPay CSV, or Phoenix CSV
  - import BIP329 JSONL
  - process journals
  - run each report, including `reports balance-history --interval month`
  - if testing `Altbestand`, verify capital gains are zeroed for that wallet and return after `set-neubestand`
  - exercise the rates cache: `rates pairs`, `rates set BTC-USD <ts> <rate>`, `rates latest BTC-USD`, `rates range BTC-USD --start <ts>`; optionally `rates sync --pair BTC-USD --days 7` when network access is acceptable

## Known gaps

- BTC-denominated amounts are stored as INTEGER msat in SQLite. Machine envelopes expose both `amount` (BTC float) and `amount_msat` (integer), and the same for `fee` / `quantity`. Fiat columns (`fiat_value`, `fiat_rate`, etc.) are still REAL.
- Rates cache (`rates pairs/sync/latest/range/set`) stores BTC-USD / BTC-EUR samples from CoinGecko or manual upsert, but journal processing still derives fiat rates from priced transactions rather than the cache.
- Phoenix Lightning wallet CSV import is implemented (`wallets import-phoenix`). River CSV importer is not implemented yet.
- No `custom` wallet kind CSV mapping DSL yet.
- No account adjustments / per-event rate overrides yet.
- No per-profile Tor proxy configuration yet.
- No descriptor/xpub-native live sync through `bitcoinrpc` yet.
- No self-hosted Liquid `elements_rpc` backend yet.
- No BTCPay Greenfield API yet.
- No Lightning node adapters yet (`coreln`, `lnd`, `nwc` kinds are declared but do not sync).
- No REST/server mode or multi-user auth yet.
- Austrian journal processing exists for supported acquisitions, disposals, and self-transfers, but the path remains experimental, quarantines unclear provenance, keeps report envelopes visibly experimental, and does not yet ship E 1kv export.
