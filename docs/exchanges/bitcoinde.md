# Exchange spec: Bitcoin.de

Intake decisions; the contract lives in [the reference](../reference/bitcoinde.md),
open questions in TODO.md.

- **Slug:** `bitcoinde` (`bitcoinde_csv`, `wallets import-bitcoinde`,
  `wallets sync-bitcoinde`, `pricing_provider` `Bitcoin.de`); implemented
  2026-11-13, slimmed 2026-11-14. Logo: the Bitcoin.de mark.
- **Model:** custodial peer-to-peer marketplace, active ledger (`full`) from
  the API, the statement CSV or both into one wallet. BTC only: the API reads
  the BTC statement and BTC/EUR trades; other coins' statements (BCH, ETH,
  LTC…) and trades touching BTC on another pair are refused by name.
- **Austrian tax:** the euros that changed hands per trade (agreed amount
  minus half the Bitcoin.de fee; the bank's fee too for older express trades)
  give exact execution pricing; no KESt (German provider).
- **Row types:** the statement's `type` values, with the German and English
  CSV names in `classify()`. Mapped: buy, sell, inpayment, payout with its
  network fee, affiliate, kickback, welcome credit; zero registrations are
  skipped. Any other type refuses the import by name.
- **Identity:** trades by trade id; other bookings by second and running
  balance (no booking ids exist), so imports need the whole history from
  zero and are append-only.
- **Samples:** none committed; `BITCOINDE_CSV` in the file-import tests is
  synthetic, after the public samples' layout (raccoin, CoinTracer,
  cryptotracker parsers).
- **Format:** `;`, UTF-8, German wall time without offset, dot decimals,
  German or English headings. **API:** Trading API v4, HMAC-SHA256 with a
  rising nonce; `showPermissions` lets Kassiber refuse a key that can write.
