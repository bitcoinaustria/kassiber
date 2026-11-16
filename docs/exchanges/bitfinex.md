# Exchange spec: Bitfinex

Intake decisions; the contract lives in [the reference](../reference/bitfinex.md),
open questions in TODO.md.

- **Slug:** `bitfinex` (`bitfinex_csv`, `wallets import-bitfinex`,
  `wallets sync-bitfinex`, `pricing_provider` `Bitfinex`); implemented
  2026-11-12, slimmed 2026-11-14. Logo: `ui-tauri/src/assets/integrations/bitfinex.svg`, the leaf
  mark cut from Bitfinex's published logo
  (<https://commons.wikimedia.org/wiki/File:Bitfinex_logo.svg>); the name and
  leaf are trademarks of iFinex Inc., used only to name the integration.
- **Model:** custodial, active ledger (`full`) from the API, the Reports
  exports or both into one wallet. BTC spot against USD, EUR, GBP, JPY
  only, plus funding interest and transfers between the exchange and
  funding wallets; margin, derivatives, other funding entries and BTC against
  other coins are refused by name.
- **Austrian tax:** amount, price and fee per trade give exact execution
  pricing; no KESt (non-Austrian provider).
- **Row types:** trades and movements are the records; the BTC ledger is
  the cross-check, with fixed description grammars paired one-to-one.
  Unknown descriptions refuse; adding one is a new arm in `describe()`.
- **Samples:** `samples/bitfinex/` (synthetic, the API tests' events).
- **Format:** Reports zip or single CSVs, comma, UTF-8; `YYYY-MM-DD` UTC
  dates only. **API:** REST v2, HMAC-SHA384 over signed POSTs; the
  permissions endpoint lets Kassiber refuse a key that can write.
