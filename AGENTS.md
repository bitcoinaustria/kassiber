# AGENTS.md

## Project shape

- Kassiber is a local-first Bitcoin accounting CLI.
- The main entrypoint is [kassiber/app.py](kassiber/app.py).
- Tax policy definitions live in [kassiber/tax_policy.py](kassiber/tax_policy.py).
- Descriptor handling lives in [kassiber/wallet_descriptors.py](kassiber/wallet_descriptors.py).
- Packaging is defined in [pyproject.toml](pyproject.toml).
- User-facing behavior is documented in [README.md](README.md).
- Third-party dependency and license notes are tracked in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).

Kassiber is currently in **dev mode**: renaming commands, breaking flags, and reshaping subcommand trees is acceptable as long as docs in the tree are updated in the same change. There is no deprecation-alias layer.

## Current architecture

- Data lives in a local SQLite database (system of record).
- The CLI model is:
  - backend (`.env` seed + DB overlay via the `backends` table)
  - workspace
  - profile (carries tax policy defaults)
  - account
  - wallet
  - transactions
  - metadata (notes, tags, inclusion)
  - journals (RP2 processing + quarantine)
  - reports (balance-sheet, portfolio-summary, capital-gains, journal-entries, balance-history)
- Every command accepts `--format {table,plain,json,csv}`, `--output <path>`, `--machine` (= `--format json`), and `--debug`.
- Successful responses use `{kind, schema_version, data}`. Errors use `{kind: "error", schema_version, error: {code, message, hint, details, retryable, debug}}`.
- Live sync kinds implemented: `esplora`, `electrum`, `bitcoinrpc`.
- BIP329 records are stored in SQLite and transaction labels are bridged into Kassiber tags.
- BTCPay CSV/JSON imports become transactions, with comments mapped to notes and labels mapped to tags.
- Profile-level tax defaults are stored on `profiles` as `fiat_currency`, `tax_country`, `tax_long_term_days`, and `gains_algorithm`.
- Wallet-level tax provenance stays in `wallets.config_json`, including the manual `altbestand` flag.

## Command surface

- `init`, `status`, `context {show,current,set}`
- `workspaces {list,create}`
- `profiles {list,create,get,set}`
- `accounts {list,create}`
- `wallets {kinds,list,create,get,update,delete,sync,derive,set-altbestand,set-neubestand,import-json,import-csv,import-btcpay}`
- `backends {kinds,list,get,create,update,delete,set-default,clear-default}`
- `transactions {list}`
- `metadata records {list,get,note {set,clear},tag {add,remove},excluded {set,clear}}`
- `metadata bip329 {import,list,export}`
- `journals {process,list,quarantined,events {list,get},quarantine {show,clear,resolve {price-override,exclude}}}`
- `reports {balance-sheet,portfolio-summary,capital-gains,journal-entries,balance-history}`

## Pagination

List endpoints with `--limit` also accept `--cursor`. The cursor is an opaque base64 urlsafe token built from `<occurred_at>|<created_at>|<id>`. Responses include `next_cursor` (or `null`) and `has_more`.

## Tax engine

- The tax engine is RP2-backed and driven from `kassiber/app.py`.
- Policy selection and RP2 country defaults are centralized in `kassiber/tax_policy.py`.
- RP2 runs wallet-scoped, not globally pooled across the whole profile.
- Supported lot selection: `FIFO`, `LIFO`, `HIFO`, `LOFO`.
- Profiles currently expose the RP2 `generic` tax policy, with explicit `tax_long_term_days`.
- Wallets can be flagged manually as `Altbestand`; disposals from those wallets are treated as tax-free while Neubestand wallets use normal tax treatment.
- Journals must be reprocessed after any transaction, metadata, or exclusion change before reports are trusted.
- Transactions without usable fiat pricing are quarantined during journal processing instead of receiving zero-basis tax treatment.

## Working rules

- Keep the project local-first.
- Prefer standard-library solutions unless a dependency clearly buys a lot.
- Keep `--machine` output deterministic — add a `kind` to every new envelope.
- Keep envelope error shapes consistent: use `AppError(code=..., hint=..., retryable=..., details=...)`.
- Be careful with multi-wallet isolation: avoid mixing accounting state across wallets unless that is explicitly intended.
- Keep wallet-level `Altbestand` handling separate from profile-level country policy unless there is a deliberate migration plan.
- Preserve the default `mempool.space` Esplora backend unless there is a strong reason to change it.
- Prefer additive schema changes that work with `CREATE TABLE IF NOT EXISTS`.
- Prefer lightweight compatibility migrations for existing SQLite databases when adding profile fields.
- When adding a new runtime dependency, update both the README dependency story and `THIRD_PARTY_LICENSES.md`.
- Keep `THIRD_PARTY_LICENSES.md` concise: direct dependencies and notable license constraints matter more than a hand-maintained transitive dump.

## Verification

- Compile check:

```bash
PYTHONPYCACHEPREFIX=/tmp/kassiber-pyc python3 -m py_compile kassiber/*.py
```

- CLI smoke checks:

```bash
python3 -m kassiber --help
python3 -m kassiber --machine status
python3 -m kassiber backends list
python3 -m kassiber wallets kinds
python3 -m kassiber profiles create --help
python3 -m kassiber metadata records --help
python3 -m kassiber journals events --help
python3 -m kassiber reports balance-history --help
```

- Safe local workflow:
  - create a temp data root via `--data-root /tmp/smoke/data`
  - `init`, then create workspace/profile/wallet and seed transactions
  - verify `profiles list` shows `tax_country` and `tax_long_term_days`
  - optionally mark a wallet as `Altbestand`
  - import priced CSV or BTCPay CSV
  - import BIP329 JSONL
  - process journals
  - run each report, including `reports balance-history --interval month`
  - if testing `Altbestand`, verify capital gains are zeroed for that wallet and return after `set-neubestand`

## Known gaps

- Amount storage is still float BTC — migration to INTEGER msat is pending.
- No exchange-rate subsystem (`rates sync/latest/range/pairs`) yet.
- No Phoenix / River Lightning CSV importers yet.
- No `custom` wallet kind CSV mapping DSL yet.
- No account adjustments / per-event rate overrides yet.
- No per-profile Tor proxy configuration yet.
- No descriptor/xpub-native live sync through `bitcoinrpc` yet.
- No self-hosted Liquid `elements_rpc` backend yet.
- No BTCPay Greenfield API yet.
- No Lightning node adapters yet (`coreln`, `lnd`, `nwc` kinds are declared but do not sync).
- No REST/server mode or multi-user auth yet.
- No country-specific RP2 policy plugin yet: profiles currently use the `generic` policy layer.
