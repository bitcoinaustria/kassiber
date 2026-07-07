# Exchange spec: Coinbase

- **Display name:** Coinbase
- **Slug:** coinbase
- **Spec status:** implemented, BTC-focused API subset
- **Integration shape:** API import

## Custodial model

Coinbase is a custodial exchange/wallet. Kassiber imports BTC account rows into
a Coinbase provider wallet and keeps self-custody sync separate.

## Implemented rows

- BTC `buy`, `sell`, `trade`, and `advanced_trade_fill`: exact
  `exchange_execution` pricing when Coinbase provides usable fiat native
  amounts.
- BTC `send` / exchange transfer rows: wallet movement without exact execution
  pricing.
- BTC interest/staking/income-like rows recognized by the API normalizer:
  inbound income/staking rows without exact execution pricing.

## Safety decisions

Unsupported BTC row types raise validation errors instead of being guessed.
Rows with fiat amounts below Coinbase's useful precision are rejected for exact
trade import so they can be reviewed explicitly.
