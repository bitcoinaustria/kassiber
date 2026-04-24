# AGENTS.md

## Project shape

- Kassiber is a local-first Bitcoin accounting CLI.
- The CLI entrypoint lives in [kassiber/cli/main.py](kassiber/cli/main.py). The remaining command implementation surface lives in [kassiber/cli/handlers.py](kassiber/cli/handlers.py).
- Desktop planning is captured in [docs/plan/00-overview.md](docs/plan/00-overview.md), [docs/plan/01-stack-decision.md](docs/plan/01-stack-decision.md), and [docs/plan/04-desktop-ui.md](docs/plan/04-desktop-ui.md).
- External-document reconciliation scope and architecture are captured in [docs/plan/08-external-document-reconciliation.md](docs/plan/08-external-document-reconciliation.md).
- The desktop shell lives in [kassiber/ui/dashboard.py](kassiber/ui/dashboard.py), [kassiber/ui/app.py](kassiber/ui/app.py), and [kassiber/ui/viewmodels/](kassiber/ui/viewmodels/).
- Supporting modules (bottom-up — no back-edges into the CLI layer):
  - [kassiber/errors.py](kassiber/errors.py) — `AppError` typed exception carrying `code`, `hint`, `details`, `retryable`.
  - [kassiber/time_utils.py](kassiber/time_utils.py) — timestamp parsing + RFC3339 formatting and `UNKNOWN_OCCURRED_AT`.
  - [kassiber/msat.py](kassiber/msat.py) — `SATS_PER_BTC`, `MSAT_PER_BTC`, `dec`, `btc_to_msat`, `msat_to_btc`.
  - [kassiber/util.py](kassiber/util.py) — tiny type-coercion helpers (`str_or_none`, `parse_bool`, `parse_int`, chain/network normalizers).
  - [kassiber/envelope.py](kassiber/envelope.py) — JSON envelope contract, `emit`, table/plain/csv output writers, and the `_KIND_SUBCOMMAND_ATTRS` kind map.
  - [kassiber/db.py](kassiber/db.py) — SQLite schema, `open_db`, data-root resolution, settings helpers, and msat column migrations.
  - [kassiber/backends.py](kassiber/backends.py) — named sync backends with SQLite as the canonical store plus optional dotenv bootstrap via `config/backends.env`, along with CRUD helpers.
  - [kassiber/sync_btcpay.py](kassiber/sync_btcpay.py) — BTCPay Greenfield API fetcher used by wallet-configured BTCPay sync and `wallets sync-btcpay`; it reshapes confirmed remote wallet-transaction rows into the existing BTCPay import format so Kassiber can reuse the same notes/tags pipeline.
  - [kassiber/cli/handlers.py](kassiber/cli/handlers.py) — remaining CLI command handlers and compatibility-layer imports while deeper decomposition continues.
  - [kassiber/core/attachments.py](kassiber/core/attachments.py) — transaction attachment storage, URL-reference handling, integrity verification, and orphan-file GC for the managed attachment tree.
  - [kassiber/core/engines/__init__.py](kassiber/core/engines/__init__.py) — tax-engine interface/resolver. Both the generic RP2 path and the Austrian (§ 27b EStG) path route through `GenericRP2TaxEngine`; AT profiles surface rp2's `rp2.plugin.country.at.AT` plugin directly so accounting methods and engine semantics come from rp2, while Kassiber keeps Austrian disposal bucketing / Kennzahl mapping on its side.
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
  current execution backlog. Plan docs under [docs/plan/](docs/plan/) are
  orientation and product guardrails, not the task source of truth.

Phase 0 core extraction is green: the CLI/runtime surface is split out of
the old `kassiber/app.py` monolith, the smoke suite passes, and future
work should build on the extracted modules instead of re-growing a shim.

Kassiber is currently in **dev mode**: renaming commands, breaking flags, and reshaping subcommand trees is acceptable as long as docs in the tree are updated in the same change. There is no deprecation-alias layer.

## Current architecture

- Data lives in a local SQLite database (system of record).
- Default user state lives under `~/.kassiber/{data,config,exports,attachments}` unless `--data-root` / `--env-file` overrides it; the managed layout manifest lives at `~/.kassiber/config/settings.json`.
- The CLI model is:
  - backend (canonical SQLite rows in the `backends` table plus optional dotenv bootstrap)
  - workspace
  - profile (carries tax policy defaults)
  - account
  - wallet
  - transactions
  - attachments
  - metadata (notes, tags, inclusion)
  - journals (RP2 processing + quarantine)
  - reports (summary, tax-summary, balance-sheet, portfolio-summary, capital-gains, journal-entries, balance-history, austrian-e1kv, austrian-tax-summary, export-pdf, export-austrian, export-austrian-e1kv-pdf, export-austrian-e1kv-xlsx)
  - rates (local cache + CoinGecko sync + manual override)
  - ui (PySide6 + QML desktop shell over the local store)
- Every command accepts `--format {table,plain,json,csv}`, `--output <path>`, `--machine` (= `--format json`), and `--debug`.
- Successful responses use `{kind, schema_version, data}`. Errors use `{kind: "error", schema_version, error: {code, message, hint, details, retryable, debug}}`.
- Live sync kinds implemented: `esplora`, `electrum`, `bitcoinrpc`. BTCPay Greenfield confirmed on-chain wallet history sync is available through wallet config and `wallets sync-btcpay`.
- BIP329 records are stored in SQLite and transaction labels are bridged into Kassiber tags.
- BTCPay CSV/JSON imports become transactions, with comments mapped to notes and labels mapped to tags. Wallet-configured BTCPay sync and `wallets sync-btcpay` reuse that same normalization for confirmed Greenfield wallet history.
- Transaction attachments are stored in a managed `attachments/` state sibling; file attachments are copied locally and URL attachments remain literal strings with no fetching or indexing.
- Profile-level tax defaults are stored on `profiles` as `fiat_currency`, `tax_country`, `tax_long_term_days`, and `gains_algorithm`.

## Command surface

- `init`, `status`, `ui`, `context {show,current,set}`
- `workspaces {list,create}`
- `profiles {list,create,get,set}`
- `accounts {list,create}`
- `wallets {kinds,list,create,get,update,delete,sync,sync-btcpay,derive,import-json,import-csv,import-btcpay,import-phoenix}`
- `backends {kinds,list,get,create,update,delete,set-default,clear-default}`
- `transactions {list}`
- `attachments {add,list,remove,verify,gc}`
- `metadata records {list,get,note {set,clear},tag {add,remove},excluded {set,clear}}`
- `metadata bip329 {import,list,export}`
- `journals {process,list,transfers {list},quarantined,events {list,get},quarantine {show,clear,resolve {price-override,exclude}}}`
- `transfers {pair,list,unpair}`
- `reports {summary,tax-summary,balance-sheet,portfolio-summary,capital-gains,journal-entries,balance-history,austrian-e1kv,austrian-tax-summary,export-pdf,export-austrian,export-austrian-e1kv-pdf,export-austrian-e1kv-xlsx}`
- `rates {pairs,sync,latest,range,set}`

## Pagination

List endpoints with `--limit` also accept `--cursor`. The cursor is an opaque base64 urlsafe token built from `<occurred_at>|<created_at>|<id>`. Responses include `next_cursor` (or `null`) and `has_more`.

## Tax engine

- The tax engine now goes through `kassiber/core/engines.build_tax_engine(...)`; both `generic` and `at` profiles route through `kassiber/core/engines/rp2.py`, with Austrian profiles selecting `rp2.plugin.country.at.AT` through the shared seam.
- Journal processing first normalizes raw transaction rows into in-memory tax events via `kassiber/core/tax_events.py`; raw `transactions` rows remain the source of truth and no derived regime state is persisted back onto them.
- Under-specified tax semantics that used to fall through raw-row handling should quarantine at the normalization boundary instead of being guessed. That includes malformed same-asset transfers, missing required pricing, and unsupported tax directions.
- The generic RP2 engine now owns the per-profile journal orchestration behind the engine seam: transfer detection, manual-pair application, per-asset grouping, normalized event preparation, and holdings aggregation all live in `kassiber/core/engines/rp2.py`, while CLI handlers only load rows and persist the resulting journal state.
- Snapshot coverage for the current generic transfer path lives in [tests/fixtures/generic_rp2_transfer_snapshot.json](tests/fixtures/generic_rp2_transfer_snapshot.json) and is enforced by `tests/test_review_regressions.py` in addition to the CLI smoke suite.
- Policy selection and RP2 country defaults are centralized in `kassiber/tax_policy.py`.
- RP2 runs per-asset (pooled across all wallets of a profile) so `IntraTransaction` (MOVE) carries cost basis between user-owned wallets. Wallet identity is preserved by setting RP2's `exchange` to the wallet label and recovering per-wallet quantity buckets via `BalanceSet`.
- Self-transfer detection lives in `kassiber/transfers.py`. The detector pairs same-`external_id` outbound + inbound rows across two wallets of the same profile; the journal pipeline turns each pair into an `IntraTransaction` plus `transfer_out` / `transfer_in` (and, when there's a fee, `transfer_fee`) ledger entries.
- Manual pairing via `transfers pair / list / unpair` (table `transaction_pairs`) overrides auto-detection: `apply_manual_pairs` in `kassiber/transfers.py` filters out any auto-pair that touches a manually-paired row. Same-asset manual pairs currently support `--policy carrying-value` and feed the existing IntraTransaction path; same-asset `--policy taxable` is rejected and users should leave those legs unpaired to preserve normal SELL + BUY treatment. Cross-asset pairs (BTC ↔ LBTC peg-ins/peg-outs, submarine swaps) are always surfaced via `cross_asset_pairs` in the ledger state and the `journals process` envelope. For `generic` profiles they still process as normal SELL + BUY because RP2 `IntraTransaction` is same-asset only. For Austrian (`at`) profiles, cross-asset `--policy carrying-value` pairs now run through Kassiber's two-pass swap-basis carry path before reaching RP2, while cross-asset `--policy taxable` pairs stay on the normal SELL + BUY path. Same-wallet cross-asset pairs are allowed so manual peg-ins/peg-outs can be recorded without forcing duplicate wallet records.
- Liquid peg-in/peg-out detection must not lean on hardcoded federation addresses (per-claim tweaked, federation keys rotate). Use the manual pair CLI or non-address heuristics (time + amount + direction inversion + same-profile constraint) instead.
- Per-wallet portfolio rows show that wallet's residual quantity at the asset's average residual basis — an allocation, not a physical-lot answer.
- Supported lot selection: `FIFO`, `LIFO`, `HIFO`, `LOFO`.
- Profiles support `generic` and `at` (Austrian, § 27b EStG) tax policies. AT profiles delegate engine defaults to `rp2.plugin.country.at.AT` (`moving_average_at`, accepted accounting methods, `open_positions`, English fallback), while Kassiber consumes rp2's `classify_disposal()` API to persist Austrian semantic buckets and current Kennzahl mappings. Typed Austrian fields on `NormalizedTaxEvent` (`at_regime`, `at_pool`, `at_swap_link`, `carried_basis_fiat`) are Kassiber's internal source of truth for the marker wire format. See [docs/austrian-handoff.md](docs/austrian-handoff.md) for the full current carry-basis contract.
- Journals must be reprocessed after any transaction, metadata, or exclusion change before reports are trusted.
- Transactions without usable fiat pricing are quarantined during journal processing instead of receiving zero-basis tax treatment.

## Working rules

- Keep the project local-first.
- Treat code, README, AGENTS.md, and TODO.md as current truth. Treat
  `docs/plan/` as concise guardrails; if code and plans drift, inspect code and
  update the docs in the same change.
- Keep Kassiber as the BTC-side subledger and reconciliation layer; invoice issuance, VAT workflow, and the company general ledger stay outside Kassiber.
- For merchant and document-linked flows, keep provenance capture, commercial matching, and RP2-facing tax normalization as separate layers.
- Prefer standard-library solutions unless a dependency clearly buys a lot.
- Keep `--machine` output deterministic — add a `kind` to every new envelope.
- Keep envelope error shapes consistent: use `AppError(code=..., hint=..., retryable=..., details=...)`.
- Per-asset pooling is intentional so RP2 `IntraTransaction` works across wallets; per-wallet output remains via `BalanceSet`. Do not regress to per-wallet RP2 calls without thinking through the transfer story first.
- RP2 owns tax primitives and computation; do not push invoice, ERP, or broader business-workflow concepts into RP2 unless the tax math itself truly requires them.
- Austrian tax semantics live on the rp2 side (plugin: `rp2.plugin.country.at`). Kassiber emits typed markers, carries basis across swaps, and maps rp2's disposal categories onto current Austrian report buckets / Kennzahlen; it does not re-implement Alt/Neu classification or moving-average math beyond the documented marker/quarantine contract in [docs/austrian-handoff.md](docs/austrian-handoff.md).
- Preserve the default `mempool.space` Esplora backend unless there is a strong reason to change it.
- Prefer additive schema changes that work with `CREATE TABLE IF NOT EXISTS`.
- Prefer lightweight compatibility migrations for existing SQLite databases when adding profile fields.
- When a `TODO.md` item is completed or materially reshaped, update
  `TODO.md` in the same change and check or split the item so the backlog
  stays truthful.
- Before pushing a code or docs change, review both `git diff --cached`
  and any unstaged `git diff` separately from the implementation pass.
  When second-agent tooling is available, have that reviewer inspect the
  same diff; otherwise do a manual second-pass review yourself. Fix any
  P1/P2 correctness or consistency issues before push, and mention any
  deferred lower-severity concerns in the handoff.
- For non-trivial changes touching CLI behavior, tax logic, schema,
  reports, or multiple docs, gather repo evidence first, then restate the
  requirement, risks, and step plan before editing.
- Prefer the repo-local `skills/kassiber/` references before generic
  agent habits when working on Kassiber-specific flows.
- Before calling work push-ready, run `./scripts/quality-gate.sh`.
- When adding a new runtime dependency, update both the README dependency story and `THIRD_PARTY_LICENSES.md`.
- Keep `THIRD_PARTY_LICENSES.md` concise: direct dependencies and notable license constraints matter more than a hand-maintained transitive dump.

## Verification

All commands below assume project dependencies are installed — either via `uv sync` (then prefix with `uv run`) or via `pip install -e .` inside an activated venv (then use `python3` directly). The examples use `uv run python` because it works without pre-activation; swap in `python3` when working inside an activated venv. For the baseline push/PR pass, use `./scripts/quality-gate.sh` as the single trusted entrypoint; the commands below are the underlying pieces.

- Compile check:

```bash
PYTHONPYCACHEPREFIX=/tmp/kassiber-pyc uv run python -m py_compile kassiber/*.py kassiber/ui/*.py kassiber/ui/viewmodels/*.py
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
uv run python -m kassiber wallets sync-btcpay --help
uv run python -m kassiber profiles create --help
uv run python -m kassiber metadata records --help
uv run python -m kassiber attachments list --help
uv run python -m kassiber journals events --help
uv run python -m kassiber journals transfers list --help
uv run python -m kassiber reports summary --help
uv run python -m kassiber reports tax-summary --help
uv run python -m kassiber reports austrian-e1kv --help
uv run python -m kassiber reports austrian-tax-summary --help
uv run python -m kassiber reports export-austrian --help
uv run python -m kassiber reports export-austrian-e1kv-xlsx --help
uv run python -m kassiber reports balance-history --help
uv run python -m kassiber rates --help
uv run python -m kassiber ui --help
```

- Safe local workflow:
  - create a temp data root via `--data-root /tmp/smoke/data`
  - `init`, then create workspace/profile/wallet and seed transactions
  - verify `profiles list` shows `tax_country` and `tax_long_term_days`
  - import priced CSV, BTCPay CSV, or Phoenix CSV
  - import BIP329 JSONL
  - process journals
  - run each report, including `reports summary`, `reports tax-summary`, and `reports balance-history --interval month`
  - exercise the rates cache: `rates pairs`, `rates set BTC-USD <ts> <rate>`, `rates latest BTC-USD`, `rates range BTC-USD --start <ts>`; optionally `rates sync --pair BTC-USD --days 7` when network access is acceptable

## Known gaps

- BTC-denominated amounts are stored as INTEGER msat in SQLite. Machine envelopes expose both `amount` (BTC float) and `amount_msat` (integer), and the same for `fee` / `quantity`. Fiat columns (`fiat_value`, `fiat_rate`, etc.) are still REAL.
- Rates cache (`rates pairs/sync/latest/range/set`) stores BTC-USD / BTC-EUR samples from CoinGecko or manual upsert. `journals process` can auto-fill missing transaction prices from the cache when a matching sample exists at or before the transaction timestamp, but reports still use stored transaction and journal pricing rather than querying the cache live.
- Phoenix Lightning wallet CSV import is implemented (`wallets import-phoenix`). River CSV importer is not implemented yet.
- No `custom` wallet kind CSV mapping DSL yet.
- No account adjustments yet.
- No per-profile Tor proxy configuration yet.
- No descriptor/xpub-native live sync through `bitcoinrpc` yet.
- No self-hosted Liquid `elements_rpc` backend yet.
- No BTCPay invoice/payment provenance ingest yet beyond confirmed on-chain wallet history plus comment/label carry-through from wallet-configured BTCPay sync.
- No Lightning node adapters yet (`coreln`, `lnd`, `nwc` kinds are declared but do not sync).
- No REST/server mode or multi-user auth yet.
- Generic cross-asset carrying-value is still unsupported: outside Austrian profiles, BTC ↔ LBTC peg-ins/peg-outs and submarine swaps remain audit-linked SELL + BUY pairs rather than a cost-basis-carry primitive.
