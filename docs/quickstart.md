# Quick start

This guide walks through Kassiber's main flows end-to-end via the CLI:
setup, wallet sync, transfer pairing, journal processing, reports,
Austrian E 1kv, and the reviewed source-of-funds and BTCPay reconciliation
workflows. The desktop GUI covers most of the same flows with onboarding,
forms, and inline review — use whichever surface fits the moment. For
deeper per-topic detail, drill into the [reference docs](reference/).

Examples use `python3 -m kassiber`. When running through `uv`, prefix with
`uv run --locked`. The installed prerelease binary exposes the same surface as
`kassiber ...`, and the desktop bundle forwards `--cli ...` to its
bundled CLI sidecar.

## Concepts

Kassiber's user model:

```text
books file / local state
`-- book(s)
    |-- wallet bucket(s)
    `-- wallet(s)

wallets -> transactions -> journals -> reports
```

- **books file** / **local state** — the local Kassiber data root for one
  person, business, or client (default `~/.kassiber/`).
- **book** — one separate accounting and tax scope inside that local state.
- **wallet** — a transaction source that Kassiber syncs or imports.
- **account** — a wallet/reporting bucket that wallets can belong to.

In the CLI and database these are still named `workspace` and `profile`.
The desktop UI uses the friendlier names above: a workspace is a local book
set, and a profile is a book. "My Books" might contain `private` and
`business` books; a company or client should usually live in its own state
root with one main set of BTC books plus buckets such as `events`,
`memberships`, and `store`.

Transactions flow in from wallets, journals process those transactions into
tax and accounting state, and reports read from the processed journal state.
Cost basis is pooled per asset across all wallets in a set of books — per-wallet
output remains an allocation, not a physical-lot answer. Accounts today are
descriptive bucket metadata, not a double-entry chart of accounts: fees and
external counterparties are not posted automatically to separate account rows.

**Reprocess journals after any change** — transactions, metadata, exclusions,
transfer pairs, quarantine resolutions, rate sync, or manual rate overrides —
before reports are trusted again.

## 1. Minimal setup

```bash
python3 -m kassiber init
python3 -m kassiber workspaces create personal
python3 -m kassiber profiles create main \
  --workspace personal \
  --fiat-currency USD \
  --tax-country generic \
  --tax-long-term-days 365 \
  --gains-algorithm FIFO
python3 -m kassiber context set --workspace personal --profile main
```

For Austrian books, use `--tax-country at --fiat-currency EUR`.

Coarse (daily/monthly) fallback pricing is **accepted by default** and booked at
the coarse spot price. To require manual review of coarse-priced events instead,
set `python3 -m kassiber profiles set --profile main --require-coarse-review`
(revert with `--no-require-coarse-review`).

To encrypt the local SQLite database at rest:

```bash
python3 -m kassiber secrets init
```

See [SECURITY.md](../SECURITY.md) for the SQLCipher boundary and recovery
caveats. To produce a single-file portable backup:

```bash
python3 -m kassiber backup export --file ~/backups/kassiber-$(date +%F).kassiber
```

## 2. Create and sync a wallet

```bash
python3 -m kassiber wallets create \
  --label donations --kind address \
  --address bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq

python3 -m kassiber wallets sync --wallet donations
```

Descriptor, BSMS, and xpub wallets work the same way — `wallets kinds` lists
the supported kinds. See [reference/backends.md](reference/backends.md) for
configuring sync backends (the built-in defaults are listed in
[SECURITY.md](../SECURITY.md#the-big-gotcha-not-running-your-own-node)).

For CSV / JSON imports (BTCPay, Phoenix, River, Bull Bitcoin, Coinfinity,
21bitcoin, Pocket Bitcoin, generic), see
[reference/imports.md](reference/imports.md).

## 3. Pair transfers and swaps

Same-chain self-transfers between two wallets in the same book are detected
automatically. Source-qualified, unique, equal-principal Lightning payments
between owned nodes (including circular same-node payments) are automatic too.
Liquid peg-in/peg-out, submarine-swap, and incomplete Lightning/channel
evidence stays in review before reports are trusted.

```bash
# Surface unpaired candidates (exact only from complete cryptographic/whole-row
# evidence; incomplete provider metadata and time + amount remain strong):
python3 -m kassiber transfers suggest

# Auto-apply every solo exact match without further review:
python3 -m kassiber transfers bulk-pair --confidence exact

# Or apply saved non-conflicted auto-pair rules:
python3 -m kassiber transfers rules apply

# Or pair one specific pair by id:
python3 -m kassiber transfers pair --tx-out <out-id> --tx-in <in-id> \
  --kind submarine-swap --policy carrying-value
```

For 1:N, N:1, N:M, or multi-wallet histories with missing intermediate
wallets, use the atomic custody-component resolver instead of forcing a chain
of ambiguous pairs:

```bash
python3 -m kassiber transfers components plan --action create \
  --file migrations.json
python3 -m kassiber transfers components apply --action create \
  --file migrations.json \
  --expected-input-version <input-version-from-preview>
```

The same workflow is available under **Swap Matching → Close gaps**. See
[reference/custody-components.md](reference/custody-components.md) for the JSON
contract, `untracked_wallet` placeholders, allocation rules, and audit model.

For a failed swap that refunds the same asset back to the same wallet, pair the
send and refund legs with `--policy carrying-value` (use `--kind swap-refund` to
label it) when the imported principal and separately evidenced fees conserve.
Kassiber then avoids treating the send as a sale and the refund as a fresh
acquisition. For different physical transactions, an explicit reviewed pair can
allocate a bounded refund shortfall as its fee. A residual inside one physical
txid, or any multi-leg/ambiguous shortfall, is never guessed; close that with a
custody component containing an explicit fee/external leg.
When the refund is swept on-chain through a Boltz HTLC, chain sync links it to
its funding send automatically and `transfers suggest` surfaces it as an exact
`swap-refund` candidate when the funding outpoint and whole-row evidence are
unique — even within a single wallet. Legacy txid-only or batched witness
evidence remains a manual-review candidate.

Boltz v2 cooperative Taproot key-path spends do not reveal swap scripts or
preimages on-chain. Provider evidence is exact only for a unique 1:1 provider
key with canonical send/receive route txids and explicit integer-msat
principals covering both complete rows. Route-only or ID-only metadata stays
strong/manual; chain-only rows stay in the heuristic/manual review lane.

For a direct swap payout where the provider pays an external recipient and
no owned inbound leg exists:

```bash
# --payout-fiat-value is the reviewed sale/disposal proceeds for ordinary
# taxable reviews. Cross-asset carrying-value keeps supported swap legs neutral.
python3 -m kassiber transfers payouts create --tx-out <out-id> \
  --payout-asset BTC --payout-amount 0.24990000 \
  --payout-fiat-value 12495 --payout-external-id <recipient-txid> \
  --counterparty "recipient or exchange" --policy carrying-value
```

When one on-chain spend both returns funds to an owned wallet and pays a
provider/recipient, add `--out-amount <btc>` to record only the payout portion
of the source transaction. The remaining source amount can still resolve as the
same-transaction-id self-transfer instead of being absorbed as a giant fee.

Cross-asset BTC ↔ LBTC peg-ins/peg-outs and submarine swaps:

- **Generic profiles** — BTC/LBTC Bitcoin-rail pairs default to
  `carrying-value` while the profile's Bitcoin-rail setting is enabled;
  disable it with `profiles set --no-bitcoin-rail-carrying-value`, or pass
  `--policy taxable` on a specific pair.
- **Austrian profiles** — reviewed `--policy carrying-value` pairs get
  Austrian swap markers and run through RP2's native multi-asset hook;
  `--policy taxable` stays on the SELL + BUY path.

## 4. Process journals and run reports

```bash
python3 -m kassiber journals process

python3 -m kassiber reports summary
python3 -m kassiber reports tax-summary
python3 -m kassiber reports balance-sheet
python3 -m kassiber reports capital-gains

python3 -m kassiber reports export-pdf --file report.pdf
python3 -m kassiber reports export-summary-pdf --file summary.pdf \
  --start 2024-01-01T00:00:00Z --end 2024-12-31T23:59:59Z
python3 -m kassiber reports export-csv --file report.csv
python3 -m kassiber reports export-xlsx --file report.xlsx
```

`reports balance-history --interval month` and `reports portfolio-summary`
round out the standard set. The desktop Reports screen exposes the Summary
PDF with wallet scope controls and an optional live snapshot cover.

For transaction pricing, the `rates` command tree maintains a local
BTC-USD / BTC-EUR cache: Coinbase Exchange and CoinGecko live providers,
Kraken OHLCVT local archive (`rates sync --source kraken-csv --path ...`), and
manual overrides (`rates set BTC-USD <ts> <rate>`). Desktop maintenance can use
the configured live market-rate provider for automatic latest-price refresh and
default pricing-cache rebuilds; Coinbase Exchange remains the default when no
provider is configured.
The repository also ships a BTC-only Kraken offline history bundle for EUR and
USD hourly values under `kassiber/data/rates/kraken/btc_hourly`, which
freshness/rate-coverage jobs seed automatically when missing. It can also be
imported with the same `kraken-csv` path flow, and Desktop Settings exposes it
as `Kraken offline history: hourly values` for offline fallback coverage. The
earliest rows are daily-derived from the existing bundled daily cache, which
uses Coin Metrics BTC-USD history and official ECB USD/EUR FX so cached
BTC-EUR/BTC-USD coverage starts at `2011-01-01`.
Bundled hourly values are stored at candle close timestamps. Early
daily-derived rows should be treated as coarse fallback pricing, not exact
intraday pricing.

## 5. Austrian E 1kv reports

For Austrian books (`--tax-country at --fiat-currency EUR`):

```bash
python3 -m kassiber --machine reports austrian-e1kv --year 2024
python3 -m kassiber --machine reports austrian-tax-summary --year 2024

python3 -m kassiber reports export-austrian-e1kv-pdf \
  --year 2024 --file e1kv-2024.pdf
python3 -m kassiber reports export-austrian \
  --year 2024 --file austria-2024.pdf
python3 -m kassiber reports export-austrian-e1kv-xlsx \
  --year 2024 --file e1kv-2024.xlsx
python3 -m kassiber reports export-austrian-e1kv-csv \
  --year 2024 --dir e1kv-2024-csv
```

The styled PDF output includes Steuerbericht-style summary/detail pages,
holdings, Besonderheiten, explanations, a transaction appendix, a
FinanzOnline-style Kennzahl summary, and FAQ. The XLSX and CSV bundles
include an `Übersicht`, numbered section tabs/files, and
`Erläuterungen zum Steuerreport`.

Current limits: the export targets the ausländisch / self-custody Kennzahlen;
domestic-provider withheld KESt metadata is not modeled yet. See
[reference/tax.md](reference/tax.md) and
[plan/06-austrian-tax-engine.md](plan/06-austrian-tax-engine.md).

## 6. Source-of-funds report

The source-of-funds workstation produces a reviewed, path-scoped provenance
report for a target transaction — typically a planned exchange sale, a
broker deposit, or a relationship-bank disclosure. It is not chain
surveillance: links must be reviewed, sources must be attested, and
descriptors / xpubs / wallet files / seeds / backend tokens are never
exposed.

```bash
# Pick a purpose. For a planned exchange sale, the target transaction is the
# current funds-history anchor, not the future exchange deposit txid.
python3 -m kassiber --machine reports source-funds \
  --purpose planned_exchange_sale \
  --target-transaction <current-funds-txid-or-id> \
  --target-amount 1.00000000 \
  --planned-destination "Exchange or broker" \
  --planned-note "Pre-disclosure before expected bank proceeds"

# Seed target-scoped suggestions from canonical transfers, reviewed pairs, and
# provider/import hints. Provider ids and broad heuristics need
# --include-broad-hints.
python3 -m kassiber source-funds suggest \
  --target-transaction <txid-or-id>

# Bulk-accept deterministic links (canonical scoped transaction identity with
# equal whole-row principal,
# transaction input/output structure, source-qualified equal-principal
# Lightning hashes, and reviewed transaction_pairs) for this target path;
# provider ids and weak matches stay manual.
python3 -m kassiber source-funds links bulk-review \
  --target-transaction <target-txid-or-id>

# Add reviewed root evidence and explicit flow allocations.
python3 -m kassiber source-funds sources create \
  --type fiat_purchase --label "Reviewed exchange purchase" \
  --asset BTC --amount 0.10000000
python3 -m kassiber source-funds links create \
  --from-source <source-id> --to-transaction <transaction-id> \
  --type manual_source --allocation-amount 0.10000000

# Optional: save recipient-specific disclosure defaults for repeats.
python3 -m kassiber source-funds recipients create \
  --label "Relationship bank" --kind bank \
  --default-reveal-mode standard

# Auto-assemble everything provable from local evidence (scoped tx
# inputs/outputs of synced wallets, source-qualified Lightning hashes, and
# reviewed pairs). Provider/platform ids remain manual suggestions.
python3 -m kassiber source-funds assemble \
  --target-transaction <target-txid-or-id>

# Preview gates and disclosure; save an immutable case before export.
python3 -m kassiber --machine reports source-funds \
  --target-transaction <target-txid-or-id> \
  --reveal-mode standard --save-case

# Export only renders the saved case snapshot, never live mutable tables.
python3 -m kassiber reports export-source-funds-pdf \
  --case <case-id> --file source-of-funds.pdf
```

Reports carry overview metrics, deterministic narrative text, a simplified
reviewed flow path, data-source rollups (including how each row entered
Kassiber: chain sync, platform export, or manual import), source mix with
root-source details, level-by-level flow rows, per-level transaction detail
tables (date, source, in/out amount, fee, fiat value, txid, data source),
review gates, a missing-history section when gaps exist, and disclosure
notes including the wallets the report names and what sharing it reveals. The
simplified flow chart follows reviewed local source, wallet-transfer, and
consolidation-style links; CoinJoin/PayJoin traversal is deferred and shown
as a privacy boundary rather than ownership proof through unrelated
participant inputs. Opening balances are attested prior-history stops rather
than real root sources.

Export gates reject cycle paths, self-transfer asset mismatches,
source/edge asset mismatches, concrete sources without amounts, cumulative
source over-allocation, and reviewed paths that require more value from a
transaction than it contains.

For Austrian books, the PDF uses the `Mittelherkunftsnachweis / Source of
Funds Report` title, includes Austria/EUR report context, and renders an
evidence checklist covering fiat-purchase proof, reviewed wallet-transfer /
consolidation hops, target broker or exchange deposit, and immutable
saved-case export. Full German localization, country-specific legal
templates, and CoinJoin/PayJoin traversal remain deferred.

A fictitious AT/EUR sample report can be generated locally:

```bash
uv run --locked python scripts/generate-source-funds-demo-report.py \
  --output /tmp/kassiber-source-funds-demo.pdf \
  --json-output /tmp/kassiber-source-funds-demo.json
```

The generator removes its temporary CSV/evidence inputs and default
temporary data root after a successful run. Pass `--keep-workdir` or
`--data-root` to inspect the generated working data. The design intent and
boundary are documented in [plan/09-source-of-funds.md](plan/09-source-of-funds.md).

## 7. BTCPay invoice/payment reconciliation

BTCPay-backed wallets persist their `backend` / `store_id` /
`payment_method_id` config on the wallet itself, so later sync runs and the
desktop GUI can reuse the same source without retyping store details. When
no explicit payment method is supplied, Kassiber stores the default BTC
on-chain payment method internally.

Invoice/payment provenance uses a separate review path from wallet balances:

```bash
python3 -m kassiber btcpay provenance sync \
  --backend btcpay-prod --store-id <store-id>
python3 -m kassiber btcpay provenance suggest
python3 -m kassiber btcpay provenance review \
  --link <link-id> --state reviewed --commercial-kind income

python3 -m kassiber reports export-commercial-subledger-csv \
  --file commercial-subledger.csv
```

The sync stores BTCPay invoice/payment ids, raw snapshots, and exact fiat
facts without creating wallet transactions. Reviewed links are the gate
that applies BTCPay pricing and commercial meaning to existing wallet rows.

If you use multiple BTCPay stores, only model them as multiple Kassiber
wallets when they are actually different underlying wallets — if two stores
point at the same wallet, creating both in Kassiber would duplicate
holdings.

Scope and design intent for external-document reconciliation live in
[plan/08-external-document-reconciliation.md](plan/08-external-document-reconciliation.md).

## Where to next

- [reference/backends.md](reference/backends.md) — configure your own
  Bitcoin Core / Esplora / Electrum / Liquid endpoints
- [reference/imports.md](reference/imports.md) — CSV / JSON / BIP329 details
- [reference/tax.md](reference/tax.md) — tax policies, journal semantics,
  quarantine
- [reference/ai.md](reference/ai.md) — assistant providers, consent gates,
  read-only tool surface
- [reference/desktop.md](reference/desktop.md) — desktop preview install,
  dev modes, supervisor behavior
- [reference/machine-output.md](reference/machine-output.md) — JSON envelope
  contract for scripting
- [TODO.md](../TODO.md) — current gaps and active backlog
