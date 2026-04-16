# AGENTS.md

## Project shape

- Kassiber is a local-first Bitcoin accounting CLI.
- The main entrypoint is [kassiber/app.py](/Users/dev/Github/kassiber/kassiber/app.py).
- Tax policy definitions live in [kassiber/tax_policy.py](/Users/dev/Github/kassiber/kassiber/tax_policy.py).
- Packaging is defined in [pyproject.toml](/Users/dev/Github/kassiber/pyproject.toml).
- Examples and user-facing behavior are documented in [README.md](/Users/dev/Github/kassiber/README.md).
- Third-party dependency and license notes are tracked in [THIRD_PARTY_LICENSES.md](/Users/dev/Github/kassiber/THIRD_PARTY_LICENSES.md).

## Current architecture

- Data lives in a local SQLite database.
- The CLI model is:
  - backend
  - workspace
  - profile
  - account
  - wallet
  - transactions
  - journals
  - reports
- Live sync is currently address-based for:
  - `esplora`
  - `electrum`
  - `bitcoinrpc`
- BIP329 records are stored in SQLite and transaction labels are bridged into Kassiber tags.
- BTCPay CSV/JSON imports become transactions, with comments mapped to notes and labels mapped to tags.
- Profile-level tax defaults are stored on `profiles` as `tax_country` and `tax_long_term_days`.
- Wallet-level tax provenance stays in `wallets.config_json`, including the manual `altbestand` flag.

## Tax engine

- The current tax engine is RP2-backed and driven from `kassiber/app.py`.
- Policy selection and RP2 country defaults are centralized in `kassiber/tax_policy.py`.
- It is wallet-scoped, not globally pooled across the whole profile.
- Supported lot selection today:
  - `FIFO`
  - `LIFO`
  - `HIFO`
  - `LOFO`
- Profiles currently expose the RP2 `generic` tax policy, with explicit `tax_long_term_days`.
- Wallets can be flagged manually as `Altbestand`; disposals from those wallets are treated as tax-free while Neubestand wallets use normal tax treatment.
- Journals must be reprocessed after transaction or metadata changes before reports are trusted.
- Transactions without usable fiat pricing are quarantined during journal processing instead of receiving zero-basis tax treatment.

## Working rules

- Keep the project local-first.
- Prefer standard-library solutions unless a dependency clearly buys a lot.
- Be careful with multi-wallet isolation: avoid mixing accounting state across wallets unless that is explicitly intended.
- Keep wallet-level `Altbestand` handling separate from profile-level country policy unless there is a deliberate migration plan.
- Preserve the default `mempool.space` Esplora backend unless there is a strong reason to change it.
- Prefer additive schema changes that work with `CREATE TABLE IF NOT EXISTS`.
- Prefer lightweight compatibility migrations for existing SQLite databases when adding profile fields.
- When adding a new runtime dependency, update both the README dependency story and `THIRD_PARTY_LICENSES.md`.

## Verification

- Compile check:

```bash
PYTHONPYCACHEPREFIX=/tmp/kassiber-pyc python3 -m py_compile kassiber/*.py
```

- CLI smoke checks:

```bash
python3 -m kassiber --help
python3 -m kassiber backends list
python3 -m kassiber profiles create --help
python3 -m kassiber wallets import-btcpay --help
python3 -m kassiber wallets set-altbestand --help
python3 -m kassiber metadata bip329 --help
```

- Safe local workflow checks:
  - create a temp data root
  - create workspace/profile/wallet
  - verify `profiles list` shows `tax_country` and `tax_long_term_days`
  - optionally mark a wallet as `Altbestand`
  - import priced CSV or BTCPay CSV
  - import BIP329 JSONL
  - process journals
  - run reports
  - if testing `Altbestand`, verify capital gains are zeroed for that wallet and return after `set-neubestand`

## Known gaps

- No descriptor/xpub live derivation yet.
- No BTCPay Greenfield API yet.
- No REST/server mode yet.
- No Lightning adapters yet.
- No country-specific RP2 policy plugin is implemented yet: profiles currently use the `generic` policy layer.
