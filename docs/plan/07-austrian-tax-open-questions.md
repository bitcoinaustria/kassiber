# Austrian Tax Open Questions

**Status:** Live backlog of unresolved or practitioner-sensitive Austrian tax
assumptions.
**Current source of truth:** this file plus `docs/austrian-handoff.md`,
`kassiber/core/austrian.py`, RP2 AT plugin behavior, and tests.
**Rule:** if provenance is insufficient, quarantine. Do not silently apply these
defaults.

These defaults are planning inputs and report-review notes, not tax advice.
`reports austrian-e1kv` and the E 1kv PDF/XLSX exports surface invoked defaults
so a Steuerberater can review them.

## Defaults Summary

| ID | Question | Current default | Gate |
|---|---|---|---|
| AT-001 | Lightning routing fees | laufende Einkünfte at FMV, 27.5% | review required |
| AT-002 | L-BTC under §27b | treat as Kryptowährung like BTC | review if Liquid used |
| AT-003 | consolidation sweep | weighted-average pooling across source containers | review recommended |
| AT-004 | inheritance basis | Buchwertfortführung: basis/date carry over | explicit provenance required |
| AT-005 | CoinJoin | self-transfer if explicitly known | quarantine unless provenance exists |
| AT-006 | RBF/CPFP | final confirmed fee only | closed-ish |
| AT-007 | small spends FMV granularity | use best available transaction/date rate; do not imply hourly coverage unless cache supports it | low priority |
| AT-008 | late first sync | on-chain acquisition timestamp governs | closed |
| AT-009 | Altvermögen declaration | computed by date, user declaration as warning/audit signal if added | future UI flow |
| AT-010 | provider KESt withholding | outside MVP automation until withholding metadata exists | warning/review |
| AT-011 | staking classification | classic staking: no laufende Einkünfte at receipt, zero acquisition cost; yield/lending/DeFi products labelled "staking" may be taxable Überlassung | product provenance required |

## Implementation Rules

- Defaults apply only to explicitly supported event types with enough facts.
- Missing price, missing source basis, unknown income/gift/inheritance status, or
  unsupported privacy/mixing provenance must quarantine.
- Report output must name which AT-00x defaults affected the period.
- If a default changes, update this file, tests, and any persisted versioning or
  report footer behavior in the same change.

## Short Notes

### AT-001 Lightning Routing Fees

No clear BMF position located. Treat as laufende Einkünfte by analogy to
mining/staking/lending-like in-kind income. If guidance reclassifies it as
non-taxable until disposal or as progressive-rate income, surface separately
instead of forcing it into the current 27.5% bucket.

### AT-002 L-BTC

Default is that L-BTC satisfies the §27b Kryptowährung definition. Do not use
hardcoded Liquid federation addresses for detection.

### AT-003 Consolidation Sweep

Weighted-average pooling preserves total basis and avoids treating self-custody
hygiene as a disposal. If future BMF guidance demands stricter
wallet-address-level tracing, this may need narrower tax containers.

### AT-004 Inheritance

Practitioner consensus points to basis/date carryover, but Kassiber should not
infer inheritance from an inbound transaction. Require explicit provenance or
quarantine.

### AT-005 CoinJoin

Treat as self-transfer only with explicit CoinJoin/user-owned provenance. Avoid
automatic classification from shape alone.

### AT-006 RBF / CPFP

Only confirmed transactions are ingested. Replaced mempool transactions should
not create tax events.

### AT-007 FMV Granularity

Current rates behavior is bounded by available cache samples. Phrase report
notes as "best available transaction/date rate" unless and until hourly coverage
is implemented and tested.

### AT-008 Late First Sync

Discovery date does not change acquisition date. User must still be able to
prove ownership externally if audited.

### AT-009 Altvermögen Declaration

Automatic date classification is the default. A future UI may capture user
declarations as audit/warning metadata, not as a blind override.

### AT-010 Provider KESt

Kassiber does not yet persist provider domicile or withheld-KESt metadata.
Until that exists, keep provider withholding outside automation and surface a
review note.

### AT-011 Staking Classification

The current BMF crypto tax page distinguishes classic staking from products
that are only labelled "staking". Classic staking, where existing crypto is
used mainly for transaction processing or validation, is not laufende
Einkünfte at receipt; the received crypto is taxed later with acquisition cost
zero. If the product mechanics are really paid Überlassung of crypto, such as
lending, yield, liquidity-pool, or other DeFi arrangements, receipt can be
taxable laufende Einkünfte under § 27b Abs 2 Z 1 EStG even if the provider
calls it staking.

Kassiber should not treat the word `staking` alone as enough provenance for
either side of that line. Until imports capture product mechanics explicitly,
review staking-labelled rows before relying on the E 1kv handoff.

Reference: BMF, [Steuerliche Behandlung von Kryptowährungen](https://www.bmf.gv.at/themen/steuern/sparen-veranlagen/steuerliche-behandlung-von-kryptowaehrungen.html).

## Sources To Recheck When Touching This File

- BMF crypto tax page
- § 27a / § 27b EStG
- KryptowährungsVO § 2
- current E 1kv form
- Steuerberater/practitioner guidance used by the project owner
