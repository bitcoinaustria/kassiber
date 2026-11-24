# Exchange spec: Bitstamp

Intake decisions; the contract lives in [the reference](../reference/bitstamp.md),
open questions in TODO.md.

- **Slug:** `bitstamp` (`bitstamp_csv`, `wallets import-bitstamp`,
  `wallets sync-bitstamp`, `pricing_provider` `Bitstamp`, `pricing_method`
  `bitstamp`); implemented 2026-11-12, slimmed 2026-11-14. Logo: Bitstamp's
  own mark from bitstamp.net (`#003b2f`), on the connection tile only.
- **Model:** custodial, active ledger (`full`) from the API, the CSV or both
  into one wallet; mirrors Bitvavo. BTC only: trades against altcoins or
  stablecoins and third-asset fees are refused by name.
- **Austrian tax:** amount, rate, fiat amount and fee per trade give exact
  execution pricing; no KESt (Bitstamp is not an Austrian withholding agent).
- **Row types:** the API's numeric `type` list, shared by the CSV through
  its type names. Mapped: 0, 1, 2, 14, 35, 36, 32. Any other type with BTC
  refuses the import by name.
- **Samples:** `samples/bitstamp/example.csv`, synthetic, in the column
  layout third-party parsers document.
- **Format:** comma, UTF-8; ISO-8601 times; dot decimals, unsigned (the type
  gives the direction); `ID`; no txid. **API:** v2, HMAC-SHA256 with a
  random nonce; no read reveals a key's permissions, so the guide names
  every permission to leave off.
