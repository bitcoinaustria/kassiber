# Tax and Journals Reference

Kassiber separates raw transaction storage from processed journal state. Reports should only be trusted after `journals process` has been run on the current data.

## Tax policies

Profiles carry tax defaults through:

- `tax_country`
- `tax_long_term_days`
- `gains_algorithm`

Current policies:

- `generic` -> RP2-backed lot accounting
- `at` -> RP2-backed Austrian accounting through the Kassiber-maintained fork at [bitcoinaustria/rp2](https://github.com/bitcoinaustria/rp2), with Kassiber-side normalization and current disposal-category / Kennzahl mapping

## Journal processing

Run:

```bash
python3 -m kassiber journals process
```

Important behavior:

- both `generic` and `at` policies currently run through RP2
- `at` profiles use rp2's Austrian country plugin while Kassiber keeps the normalization, provenance, transfer-preparation, and current report-mapping layer
- cost basis is pooled per asset across all wallets in a profile
- self-transfers between user-owned wallets become RP2 `IntraTransaction` moves when Kassiber can prove the relationship
- explicit inbound `kind` values such as `income`, `interest`, `staking`, `mining`, `airdrop`, `hardfork`, `wages`, `lending_interest`, and `routing_income` are promoted into RP2 earn-like receipts; unlabeled inbound rows stay conservative and process as `BUY`
- missing or ambiguous tax inputs quarantine instead of being silently guessed

After any transaction change, metadata change, exclusion change, transfer pair change, or quarantine resolution, journals must be reprocessed before reports are trusted again.

## Transfers

Cross-wallet self-transfers are auto-detected when both legs share the same on-chain `txid`.

Reports do not auto-detect or auto-pair cross-asset swaps. If you have
BTC ↔ LBTC peg-ins / peg-outs or submarine swaps, pair those legs before
trusting `journals process` and downstream reports.

When that signal is missing, you can pair them manually:

```bash
python3 -m kassiber transfers pair \
  --tx-out <OUT_TRANSACTION_ID> \
  --tx-in <IN_TRANSACTION_ID> \
  --kind manual \
  --policy carrying-value

python3 -m kassiber transfers list
python3 -m kassiber transfers unpair --pair-id <PAIR_ID>
```

Current rules:

- same-asset manual pairs support `--policy carrying-value`
- same-asset `--policy taxable` is rejected; leave those legs unpaired if you want normal SELL + BUY treatment
- cross-asset pairs are always stored for audit
- cross-asset `--policy carrying-value` is supported for Austrian (`tax_country=at`) profiles and feeds the swap-basis-carry path
- cross-asset `--policy taxable` keeps the normal SELL + BUY treatment
- non-Austrian profiles still reject cross-asset `--policy carrying-value`
- cross-asset swaps are never auto-paired from time / amount heuristics during report generation; use `transfers pair` when those links matter for tax treatment

Manual pairs override auto-detection.

## Quarantines

Inspect quarantined transactions:

```bash
python3 -m kassiber journals quarantined
python3 -m kassiber journals quarantine show --transaction <TRANSACTION_ID>
```

Typed resolution paths:

```bash
python3 -m kassiber journals quarantine resolve price-override \
  --transaction <TRANSACTION_ID> --fiat-rate 50000

python3 -m kassiber journals quarantine resolve exclude \
  --transaction <TRANSACTION_ID>

python3 -m kassiber journals quarantine clear \
  --transaction <TRANSACTION_ID>
```

Quarantine causes typically include:

- missing spot price
- missing cost basis
- insufficient lots
- ambiguous or unsupported tax semantics

## Rates and tax input quality

If transactions do not already include usable fiat pricing, Kassiber first tries to fill them from the local rates cache during journal processing. When a transaction has a known `confirmed_at` timestamp, Kassiber prices from that confirmation time; otherwise it falls back to `occurred_at`.

Useful commands:

```bash
python3 -m kassiber rates pairs
python3 -m kassiber rates sync --pair BTC-USD --days 30
python3 -m kassiber rates set BTC-EUR 2026-01-01T00:00:00Z 95000
python3 -m kassiber rates latest BTC-EUR
```

Reports still use stored transaction and journal pricing rather than querying the rates cache live.

## Austrian notes

Current Austrian status:

- Austrian profiles process through rp2's `AT` country plugin via the shared RP2 adapter
- Kassiber keeps normalization, provenance capture, transfer preparation, cross-asset carry wiring, and current disposal-category / Kennzahl mapping
- Austrian cross-asset `--policy carrying-value` pairs are supported and feed Kassiber's swap-basis-carry path before RP2
- Austrian E 1kv export is available through `reports austrian-e1kv`, `reports export-austrian-e1kv-pdf`, and `reports export-austrian-e1kv-xlsx`; `reports austrian-tax-summary` and `reports export-austrian` are friendlier aliases for the same annual Austrian handoff
- Austrian output should remain review-gated; Kassiber is not tax advice

The E 1kv export is annual and review-oriented:

```bash
python3 -m kassiber --machine reports austrian-e1kv --year 2024
python3 -m kassiber --machine reports austrian-tax-summary --year 2024
python3 -m kassiber --format csv --output e1kv-2024.csv reports austrian-e1kv --year 2024
python3 -m kassiber reports export-austrian-e1kv-pdf --year 2024 --file e1kv-2024.pdf
python3 -m kassiber reports export-austrian --year 2024 --file austria-2024.pdf
python3 -m kassiber reports export-austrian-e1kv-xlsx --year 2024 --file e1kv-2024.xlsx
```

Kassiber currently maps crypto rows to the ausländisch / self-custody
Kennzahlen 172, 174, and 176. Domestic-provider and withheld-KESt rows such as
171, 173, or 175 need structured provider metadata before Kassiber can populate
them. The structured JSON/PDF report includes Steuerbericht-style sections
1.1-4.5 so unsupported areas such as margin/derivatives, NFTs, gifts, lost
coins, commercial mining, and minting are visible as zero-value placeholders
instead of disappearing from the handoff.
The XLSX export mirrors that structure as an accountant-facing workbook with
an `Übersicht` sheet, separate numbered tabs such as `1.1.`, `2.1.`, and
`3.3.`, plus an `Erläuterungen zum Steuerreport` notes sheet.

See [../plan/06-austrian-tax-engine.md](../plan/06-austrian-tax-engine.md) for the broader design and remaining Austrian backlog, plus [../austrian-handoff.md](../austrian-handoff.md) for the current marker / carry-basis contract.
