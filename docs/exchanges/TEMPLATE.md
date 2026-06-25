# Exchange spec: <Display Name>

> Copy this file to `docs/exchanges/<slug>.md` and fill every section before
> any importer code is written. Driven by the intake interview in
> [skills/kassiber/references/add-exchange.md](../../skills/kassiber/references/add-exchange.md).
> See [strike.md](strike.md) for a filled worked example. Keep secrets and
> personal data out of this file.

- **Display name:**
- **Slug:** <!-- lowercase; becomes <slug>_csv source format, <slug> wallet kind, import-<slug> CLI command, pricing_provider -->
- **Logo:** <!-- URL to an official vector (SVG) brand mark for the connection tile; raster only if no vector exists; note trademark usage terms -->
- **Spec status:** draft <!-- draft | ready-for-implementation | implemented -->
- **Date / author:**

## 1. Custodial model

- **Custodial / non-custodial / both:**
- **Integration shape:** <!-- active custodial ledger (full) | match-existing-only evidence (relevant) | both -->
- **Mirror importer:** <!-- 21bitcoin | strike | bull | coinfinity | pocket -->
- **What the export adds over a plain descriptor wallet:** <!-- exact prices? fees? fiat legs? if "nothing", reconsider building it -->
- **Assets:** <!-- BTC-only? also LBTC? multi-asset? Kassiber imports only BTC/LBTC rows; note how non-BTC legs are handled (skip / evidence / quarantine) -->
- **No-code alternative considered?** <!-- could generic import-csv/import-json cover the user's need without a new importer? -->

## 2. Austrian tax notes

- **Exact execution price + cost basis + fees per trade?** <!-- yes/no; if coarse only, pricing is quarantined for review -->
- **Withheld / reported Austrian KESt?** <!-- not modeled yet — note here as a gap, do not invent a column -->
- **Row types with under-specified tax semantics (must quarantine):**

## 3. Row types

One line per distinct provider row/type value. Every value the provider can
emit needs a decision — enumerate from the provider's **documentation**, not
just the sample (a user's history rarely hits every type). `Action` is one of
import / skip / quarantine. `Source` is `sample` (a real row exists) or `docs`
(documented but unverified).

| Provider type value | Meaning | Kassiber `kind` | `direction` | Action | Source | Notes |
|---|---|---|---|---|---|---|
|  |  |  |  |  |  |  |

- **Fallback for unrecognized types:** sign-based `deposit` / `withdrawal` +
  `<slug>-unmapped-type` review tag (never a guessed buy/sell). Raise `AppError`
  only when a row cannot be safely shaped at all.
- **Types still unverified (`docs`-only):** <!-- confirm with a real sample later -->
- **Types whose meaning is unknown from sample + docs:** <!-- open questions; fail-safe handles them -->

## 4. Sample exports

- **Files / location:** <!-- docs/exchanges/samples/<slug>/ if scrubbed, else describe shape; never commit account numbers/names/balances -->
- **Row types covered by the samples:**
- **Row types NOT yet covered (need more samples):**

## 5. Export format details

- **Format:** <!-- CSV | XLSX (export one sheet as CSV) | JSON (generic import) | PDF (not machine-importable — ask for another export) -->
- **Documentation URL:**
- **Delimiter / encoding:**
- **Timezone of timestamps:** <!-- assume UTC only if documented -->
- **Number locale:** <!-- comma vs dot decimals -->
- **Are amounts signed?** <!-- and which column drives direction -->
- **Stable row id column** (for `txid` / `pricing_external_ref`, so re-import dedupes):
- **On-chain txid column / Lightning hash column:**
- **Fee columns (BTC fee, fiat fee):**

## 6. API connection

- **Has an API?** <!-- yes/no -->
- **Auth model / endpoints / rate limits:**
- **Decision:** ship CSV importer first; API live-sync is a follow-up (no
  generic exchange-API sync backend exists yet — esplora / electrum /
  bitcoinrpc / BTCPay Greenfield only).

## 7. Open questions / follow-ups
