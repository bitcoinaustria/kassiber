# AGENTS.md

## Project shape

- Kassiber is a local-first Bitcoin accounting CLI.
- The main entrypoint is [kassiber/app.py](/Users/dev/Github/kassiber/kassiber/app.py).
- Packaging is defined in [pyproject.toml](/Users/dev/Github/kassiber/pyproject.toml).
- Examples and user-facing behavior are documented in [README.md](/Users/dev/Github/kassiber/README.md).

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

## Tax engine

- The current tax engine is implemented in `kassiber/app.py`.
- It is wallet-scoped, not globally pooled across the whole profile.
- Supported lot selection today:
  - `FIFO`
  - `LIFO`
- Journals must be reprocessed after transaction or metadata changes before reports are trusted.

## Working rules

- Keep the project local-first.
- Prefer standard-library solutions unless a dependency clearly buys a lot.
- Be careful with multi-wallet isolation: avoid mixing accounting state across wallets unless that is explicitly intended.
- Preserve the default `mempool.space` Esplora backend unless there is a strong reason to change it.
- Prefer additive schema changes that work with `CREATE TABLE IF NOT EXISTS`.

## Verification

- Compile check:

```bash
PYTHONPYCACHEPREFIX=/tmp/kassiber-pyc python3 -m py_compile kassiber/*.py
```

- CLI smoke checks:

```bash
python3 -m kassiber --help
python3 -m kassiber backends list
python3 -m kassiber wallets import-btcpay --help
python3 -m kassiber metadata bip329 --help
```

- Safe local workflow checks:
  - create a temp data root
  - create workspace/profile/wallet
  - import BTCPay CSV
  - import BIP329 JSONL
  - process journals
  - run reports

## Known gaps

- No descriptor/xpub live derivation yet.
- No BTCPay Greenfield API yet.
- No REST/server mode yet.
- No Lightning adapters yet.
- The tax engine is still custom and intentionally simple for now.
