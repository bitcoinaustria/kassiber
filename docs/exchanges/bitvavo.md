# Exchange spec: Bitvavo

Intake decisions; the contract lives in [the reference](../reference/bitvavo.md),
open questions in TODO.md.

- **Slug:** `bitvavo` (`bitvavo_csv`, `wallets import-bitvavo`,
  `wallets sync-bitvavo`, `pricing_provider` `Bitvavo`); implemented
  2026-11-12. Logo: official marks on <https://bitvavo.com/en/press>; a `BV`
  monogram until the mark is added.
- **Model:** custodial, active ledger (`full`) from the API, the CSV or both
  into one wallet; mirrors Strike (CSV) and Bitpanda (API). BTC only:
  BTC-USDC and altcoin or Convert trades touching BTC are refused by name.
- **Austrian tax:** amount, unit price, quote amount and fee per trade give
  exact execution pricing; no KESt (Dutch provider).
- **Row types:** the API's `type` enum (OpenAPI 2.10.0), shared by the CSV.
  Mapped: `buy`, `sell`, `deposit`, `withdrawal`, `staking`,
  `fixed_staking`, `affiliate`. Any other type with BTC refuses the import
  by name; mapping one later is a one-line change in `kind()`.
- **Samples:** none committed; `BITVAVO_CSV` in the file-import tests is
  synthetic, after the public samples' layout.
- **Format:** comma, UTF-8, optional BOM; wall time in the `Timezone`
  column; dot decimals with exponents; signed amounts; `Transaction ID`; no
  txid. **API:** REST v2, HMAC-SHA256; no endpoint reveals a key's
  permissions, so the guide asks for View access only.
