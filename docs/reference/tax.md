# Tax and Journals Reference

Kassiber separates raw transaction storage from processed journal state. Reports should only be trusted after `journals process` has been run on the current data.

## Tax policies

Profiles carry tax defaults through:

- `tax_country`
- `tax_long_term_days`
- `gains_algorithm`

Current policies:

- `generic` -> RP2-backed lot accounting
- `at` -> recognized for legacy profiles, but tax processing is currently unavailable in Kassiber and is planned through the Kassiber-maintained RP2 fork at [bitcoinaustria/rp2](https://github.com/bitcoinaustria/rp2)

## Journal processing

Run:

```bash
python3 -m kassiber journals process
```

Important behavior:

- generic lot accounting currently runs through RP2
- `generic` is the only active tax-processing mode today
- cost basis is pooled per asset across all wallets in a profile
- self-transfers between user-owned wallets become RP2 `IntraTransaction` moves when Kassiber can prove the relationship
- missing or ambiguous tax inputs quarantine instead of being silently guessed

After any transaction change, metadata change, exclusion change, transfer pair change, or quarantine resolution, journals must be reprocessed before reports are trusted again.

## Transfers

Cross-wallet self-transfers are auto-detected when both legs share the same on-chain `txid`.

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
- cross-asset pairs are stored as audit metadata only
- cross-asset carrying-value is not supported yet

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

If transactions do not already include usable fiat pricing, Kassiber first tries to fill them from the local rates cache during journal processing.

Useful commands:

```bash
python3 -m kassiber rates pairs
python3 -m kassiber rates sync --pair BTC-USD --days 30
python3 -m kassiber rates set BTC-EUR 2026-01-01T00:00:00Z 95000
python3 -m kassiber rates latest BTC-EUR
```

Reports still use stored transaction and journal pricing rather than querying the rates cache live.

## Austrian notes

Kassiber does not currently compute Austrian tax results itself.

Current Austrian status:

- Austrian tax processing is unavailable in Kassiber today
- `journals process` and report commands fail fast for Austrian profiles
- future Austrian support is planned through the Kassiber-maintained RP2 fork, while Kassiber keeps normalization, provenance capture, transfer preparation, and review UX

See [../plan/06-austrian-tax-engine.md](../plan/06-austrian-tax-engine.md) for the design direction.
