# Exchange spec: Strike

> **Illustrative reference**, not a fresh intake. This documents the *already
> shipped* Strike importer (`kassiber/importers.py` `normalize_strike_record`,
> `docs/reference/imports.md` "## Strike") as a worked example of a completed
> spec. The code is the source of truth; this snapshot describes it as of
> 2026-06. Use it as a model when filling a real spec from
> [TEMPLATE.md](TEMPLATE.md).

- **Display name:** Strike
- **Slug:** `strike`
- **Logo:** `ui-tauri/src/assets/integrations/strike.jpg` — raster (no clean
  vector was sourced); a vector SVG would be preferable per the playbook.
- **Spec status:** implemented
- **Date / author:** 2026-06 (reference snapshot)

## 1. Custodial model

- **Custodial / non-custodial / both:** both — Strike is used as an everyday
  Lightning/on-chain wallet *and* an exchange.
- **Integration shape:** active custodial ledger (`full`); fiat-only rows
  skipped. The CLI has no `--mode` flag; the daemon dispatch hardcodes `full`.
- **Mirror importer:** itself (the canonical custodial wallet+exchange example).
- **What the export adds over a plain descriptor wallet:** exact execution
  pricing on buys/sells, plus the BTC-side platform ledger that has no on-chain
  wallet of its own.
- **Assets:** BTC only (Lightning + on-chain). Strike's fiat balance is out of
  scope; fiat-only rows are skipped.
- **No-code alternative considered?** Generic `import-csv` could load a
  hand-mapped Strike CSV, but the dedicated importer is worth it for the exact
  pricing and Lightning/on-chain id handling.

## 2. Austrian tax notes

- **Exact execution price + cost basis + fees per trade?** Yes when the export
  has `BTC Price` or fiat amount columns → `exchange_execution`,
  `pricing_quality="exact"`, provider `Strike`. Buy cost basis includes fiat
  fees; sell proceeds are reduced by fiat fees.
- **Withheld / reported Austrian KESt?** No (Strike is not a domestic provider;
  KESt withholding is not modeled anywhere yet).
- **Row types with under-specified tax semantics (must quarantine):** none
  specific to Strike beyond the generic missing-price quarantine; withdrawals
  are emitted as transfers to pair out, not disposals.

## 3. Row types

Mapped by `_strike_kind(transaction_type, direction)`; direction comes from the
sign of `Amount BTC`.

| Provider type value | Meaning | Kassiber `kind` | `direction` | Action | Source | Notes |
|---|---|---|---|---|---|---|
| buy / purchase | bought BTC | `buy` | inbound | import | sample | fiat cost basis incl. fee |
| sell | sold BTC | `sell` | outbound | import | sample | proceeds less fiat fee |
| receive / received | inbound transfer | `receive` | inbound | import | sample | Lightning or on-chain |
| send / sent | outbound transfer | `send` | outbound | import | sample | |
| withdraw / withdrawal | on-chain withdrawal | `withdrawal` | outbound | import | sample | pair to receiving wallet |
| (fiat-only funding / reversal) | no `Amount BTC` | — | — | skip | docs | not BTC subledger activity |
| (any other type) | unrecognized | passed through verbatim | by sign | import | — | see caveat below |

- **Fallback caveat:** `_strike_kind` currently **passes an unknown type through
  as the `kind`** — this predates the fail-safe guidance in the playbook. A new
  importer should instead map unknowns to sign-based `deposit`/`withdrawal` plus
  a `<slug>-unmapped-type` tag.
- **Types still unverified (`docs`-only):** fiat-only/reversal skip is by amount,
  not an enumerated type list.

## 4. Sample exports

- **Files / location:** none committed (real Strike exports carry personal
  data). Behavior is pinned by fixtures in `tests/test_cli_smoke.py`.
- **Row types covered:** buy, sell, send/receive, withdrawal, Lightning,
  on-chain, fiat-only skip.

## 5. Export format details

- **Format:** CSV.
- **Documentation URL:** <https://strike.me/> (transaction-history CSV export).
- **Delimiter / encoding:** comma, UTF-8.
- **Timezone of timestamps:** UTC (`Date & Time (UTC)` column).
- **Number locale:** dot decimals.
- **Are amounts signed?** Yes — `Amount BTC` sign drives direction.
- **Stable row id column** (for `txid` / `pricing_external_ref`): `Transaction
  Hash` when present (on-chain), else provider-scoped `strike:<Reference>`
  (Lightning / no hash).
- **On-chain txid column / Lightning hash column:** `Transaction Hash`; Lightning
  rows (detected from type/`Destination` `lnbc…`) preserve a 64-hex
  `payment_hash`.
- **Fee columns (BTC fee, fiat fee):** `Fee BTC`; fiat fee column folded into
  cost basis / proceeds.

## 6. API connection

- **Has an API?** Yes (Strike has a public API).
- **Auth model / endpoints / rate limits:** not used by this importer.
- **Decision:** CSV importer shipped; API live-sync is a possible follow-up (no
  generic exchange-API sync backend exists yet — esplora / electrum /
  bitcoinrpc / BTCPay Greenfield only).

## 7. Open questions / follow-ups

- Source a vector logo to replace the raster `strike.jpg`.
- If/when the fail-safe fallback convention is backported, replace the
  pass-through unknown-type behavior in `_strike_kind`.
