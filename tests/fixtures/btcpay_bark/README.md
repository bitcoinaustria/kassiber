# Bark (Ark) BTCPay plugin: recorded Greenfield responses

Real answers from a regtest BTCPay Server with the Bark plugin, recorded by
`dev/regtest/btcpay_bark_stack.py` (`scripts/integration-harness.sh btcpay-bark`
runs it end to end). Each `*.json` file holds one request: `request` (method,
path with `{storeId}`/`{plainStoreId}`, the API key's name and permissions),
`status`, and `body`. API keys, passwords and the `capability=` secret in Bark
Lightning connection strings are redacted. Payment preimages are kept: they are
what Kassiber sees. `observations.json` in each directory is derived from the
recordings (kinds, per-kind amount/fee/status shapes, invoice-payment to row
matching, id stability, fields that reveal the network); `snapshots/` holds
`wallet/transactions` reads taken mid-scenario, before confirmations.

## Versions

| Component | Version | Image digest / hash |
| --- | --- | --- |
| BTCPay Server | 2.4.7 | `btcpayserver/btcpayserver@sha256:313b6178d8566784b8ec1811b80836df9157b9ee821098785bb0853ed1ef9df0` |
| NBXplorer | 2.6.21 | `nicolasdorier/nbxplorer@sha256:117b061886f7b1f0a9ebdb0792c0195c77abda855b76dd51b0481bb4cae3fdc3` |
| Bark plugin | 0.13.0 (builder build 16, bundles barkd 0.8.0) | package sha256 `10494a6bf81604f4462d6c0d8d4ba0b79f05bd9d46add5f5a5a2c420cd5789b9`, equal to the builder's buildHash |
| Bark plugin (pre-0.10) | 0.9.6 (builder build 10, bundles barkd 0.7.1) | package sha256 `125d18916a1a02b6cc8542aeedd9a01f0006d266a5e247cbcd2aa5f369b8e2c4`, equal to the builder's buildHash |
| Ark server | captaind 0.8.1 (+3036820) | `secondark/captaind@sha256:837e433aabbd275c2ae0b3376cd06bc4a0e07d8a9445d40c6dfed9e9bf16eb9b` |
| Ark server CLN + hold | v26.06.10-efc7d5f | `secondark/cln-hold@sha256:a7f0a18ac97f2d3f2ca976aa478aae7028ad1fd01957df7687aa9592a500f0ad` |
| External CLN | v26.06.10 | `elementsproject/lightningd@sha256:59f10c4c7429bb80791481c906859f6de1aa58ef224758aa25d50b7d47fc0c04` |
| Counterparty wallet | bark 0.8.1 CLI | `secondark/bark@sha256:461620508b3105a417bbe2369bc16ed486561e7218f41046bb6169c432fe357c` |
| Esplora | mempool/electrs v3.3.0 | `mempool/electrs@sha256:de9dd2f5a328d5f464936dc4128d848f9b8bf72221ab3c9b8e2949df50899b74` |
| bitcoind | 31.0 (captaind 0.8.1 refuses older) | `bitcoin/bitcoin@sha256:a1d05be6939b5c85132a761d2b795f5b8a2612ce53782263da4939778e461792` |

No pre-0.10 build targets anything but BTCPay >= 2.4.4 (builds 1-10 are 0.8.0-0.9.6,
all `>=2.4.4`; build 0 is 0.2.5 for `>=2.3.9`), so the pre-0.10 run also uses 2.4.7.

## Directories

- `0.13.0/`: a fresh store on plugin 0.13.0 after the full scenario below.
- `0.9.6/`: a fresh store on plugin 0.9.6 (payment method `BARK`): on-chain
  receive, board, an Ark-paid invoice, a Lightning-paid invoice offering
  `BARK` + `BTC-LN`, a `BTC-LN`-only invoice, a plain Ark and a plain Lightning
  receive, an Ark send and a Lightning send. 0.9.6 has no wallet API, so the
  wallet files are 404s. Its VTXOs page answers 500 against captaind 0.8.1
  (`ArkInfo.maxVtxoAmount` is null; fixed in 0.10.0), so it ran no refresh.
- `0.9.6-upgraded-to-0.13.0/`: the same store after swapping in plugin 0.13.0
  and restarting BTCPay, plus one new Ark-paid and one Lightning-paid invoice.
  Shows how `BARK`-era invoices read next to `BTC-ARK` ones.

## Scenario (0.13.0)

Store "Bark store" (default currency BTC) set up through the plugin's setup form
(new wallet, Ark server and Esplora from `BARK_ARK_SERVER`/`BARK_ESPLORA_SERVER`,
"accept Lightning & Ark" on, so Bark also serves `BTC-LN`); Branta lookups off.
A second store without Bark gives the 404. In order:

1. on-chain receive: 0.02 and 0.005 BTC to the Bark on-chain wallet;
2. board 1,000,000 sat (barkd `POST /api/v1/boards/board-amount`: the plugin has no board action);
3. invoice 0.002 BTC paid over Ark by the counterparty (`ark-receive`, invoice label);
4. 50,000 sat Ark payment to a plain Receive-page address (no invoice);
5. invoice 0.0015 BTC offering `BTC-ARK` + `BTC-LN`, paid with the `BTC-LN` BOLT11 by the external CLN;
6. invoice 0.0012 BTC restricted to `BTC-LN`, paid by the external CLN;
7. 30,000 sat BOLT11 from the Receive page (no invoice) paid by the external CLN;
8. Send page: 40,000 sat to the counterparty's Ark address (`ark-send`, label `kassiber-ark-send` selected);
9. Send page: 25,000 sat BOLT11 of the external CLN (`lightning-send`);
10. Greenfield `POST .../lightning/BTC/invoices/pay` of a 15,000 sat BOLT11;
11. a 0.00012 BTC `BTC-LN` payout paid by the Lightning automated payout processor;
12. a 0.0001 BTC refund of invoice 3 claimed to a BOLT11, paid by the same processor;
13. refresh of two VTXOs (VTXOs page action);
14. Send page, Ark balance to an external on-chain address, 30,000 sat (`send-onchain`, label `kassiber-send-onchain`);
15. offboard one VTXO to an external address; 16. offboard one VTXO to the Bark on-chain wallet;
17. Send page, on-chain balance to an external address, 20,000 sat (`onchain-send`);
18. Send page, on-chain balance to the wallet's own on-chain address, 15,000 sat;
19. emergency exit of one VTXO, mined through to its claim.

Kinds that appeared: `onchain-receive` 2, `board` 1, `ark-receive` 2,
`lightning-receive` 3, `ark-send` 1, `lightning-send` 4 (Send page, Lightning API
pay, payout, refund), `refresh` 1, `send-onchain` 1, `offboard` 2,
`onchain-send` 2, `exit` 1, `exit-fee` 2, `exit-claim` 1. Never seen:
`onchain-internal`, `unknown`.

## Observations against https://second.tech/docs/btcpay/api

Agreements first, briefly: rows come newest first; `amount` is the net effect
on the wallet including `fee`, and the rows add up to `wallet.balance`;
`statusFilter`/`labelFilter` combine with OR and `skip`/`limit` page after
filtering; 403/404/422 bodies match the Errors table; Ark payments to an invoice
match by payment id `txid-vout` -> `outpoint` with equal amounts; Lightning
payments match by payment hash, and BTCPay's payment `value` is `amount + fee`
of the row; a payment that arrives as two VTXOs gives two `ark-receive` rows
sharing `movementId` (and two invoice payments); `exit-fee`/`exit-claim` carry
the exit's `movementId`; `onchain-receive` has `fee: null`.

Differences and details the docs do not state:

- **`exit` row**: `amount: "0"`, `fee: "0"`, `status: Unconfirmed`,
  `confirmations: 0`, `transactionHash` set, and it stays Unconfirmed after its
  `exit-claim` confirmed (checked up to ~40 blocks later). The exited VTXO's
  value never shows as an amount; only the CPFP fees (`exit-fee`) and the
  claim's fee (`exit-claim`, `amount = -fee`) move the balance. A tool that
  stores only Confirmed rows never stores the `exit` row.
- **Block fields and confirmations beyond the documented kinds**: `exit-fee` and
  `exit-claim` rows carry `blockHash`/`blockHeight` and report block
  confirmations (e.g. 150), although the docs list only boards, offboards to
  the Bark on-chain wallet and that wallet's own transactions.
- **`board`**: `amount = -fee` (1313 sat for 1,000,000 sat boarded: Ark board
  fee 1100 + on-chain fee), not the boarded amount; no `onchain-internal`
  companion row. Before its funding tx confirms it is Unconfirmed with
  `confirmations: 0` and its final amount already set.
- **Send to the wallet's own on-chain address** shows as `onchain-send` with
  `amount = -fee` (155 sat), not `onchain-internal`. No scenario step produced
  `onchain-internal`.
- **Offboard to the Bark on-chain wallet**: `amount = -fee`, block fields and
  block confirmations (as documented). Offboard to an external address:
  `amount = -(VTXO value)`, `fee` is the part kept by the server and miners,
  no block fields, `confirmations: 1`.
- **`send-onchain` and external `offboard` are `Confirmed` (confirmations 1)
  before their on-chain transaction confirms** (snapshot 03): Confirmed means
  "amount final", not "mined". `send-onchain`: `amount = -(sent + fee)`.
- **`refresh`**: one row for two refreshed VTXOs, `amount = -fee`;
  Unconfirmed with `transactionHash` (the round tx) already set while the round
  is pending (snapshot 02), never any block fields.
- **Pending on-chain receive**: listed Unconfirmed with `confirmations: 0` and
  no block fields (snapshot 00).
- **`lightning-receive`**: `amount = invoice amount - fee` (Ark server fee
  100 sat + 0.2%), `paymentHash` set, `transactionHash: null`. **`lightning-send`**:
  `amount = -(invoice amount + fee)`. Lightning API amounts (`lightning.payments`
  `feeAmount`/`totalAmount`, `lightning.invoices` `amount`) are msat strings.
- **Labels**: invoice-paying rows carry `labels.invoice` with
  `{"ref": invoiceId, "text": "invoice", "type": "invoice"}`. The label picked on
  the Send page stuck on `send-onchain` but **not on `ark-send`** (missing in two
  reads and after a restart; same on 0.9.6 after upgrade). Payout and refund
  payments (`lightning-send`) carry **no payout/refund label** and nothing that
  links them to the payout except `paymentHash` = payout `paymentProof.PaymentHash`.
  Plain Ark/Lightning receives carry no label.
- **`outpoint`** only on `ark-receive`; **`paymentHash`** only on Lightning rows;
  **`movementId: null`** on Bark on-chain wallet rows (`onchain-receive`,
  `onchain-send`).
- **Ids**: movement rows `<movementId>-<.NET ticks of creation>`; `ark-receive`
  `<movementId>-<ticks>:<txid>:<vout>` (as in the docs' example); `exit-fee` /
  `exit-claim` `<exit id>:<txid>`; Bark on-chain wallet rows `tx:<txid>`. Ids,
  amounts, fees and order were identical across two reads and across a BTCPay
  restart (also on the upgraded store).
- **Other payment method ids on the wallet endpoints**:
  `.../payment-methods/BTC-LN/wallet/transactions` and `BTC-CHAIN` answer
  503 `not-available` ("BTC doesn't have any derivation scheme set");
  `.../BARK/wallet` and `/wallet/transactions` answer 404 `unknown-paymentMethodId`
  (the `BARK` -> `BTC-ARK` rewrite does not cover the wallet routes), while
  `GET .../payment-methods/BARK?includeConfig=true` answers 200 with
  `"paymentMethodId": "BTC-ARK"`.
- **Plugin 0.9.6**: no wallet API at all: `BTC-ARK` and `BARK` wallet routes
  both 404 `unknown-paymentMethodId`; `server/info` lists `BARK` only.
- **BARK-era invoices after upgrading**: the invoice keeps a `BARK` entry in
  `paymentMethods` (with `BTC-LN`), its Ark payment id stays
  `{invoiceId}#ark-movement-{movementId}-{index}` (here `...#ark-movement-2-0`),
  and the matching `ark-receive` row (movementId 2) carries the invoice label.
  New invoices in the same store use `BTC-ARK` and `txid-vout` ids. The store's
  payment methods move to `BTC-ARK`; `server/info` then lists both `BTC-ARK` and
  `BARK`.
- **Checkout link**: a `BTC-ARK` prompt's `paymentLink` is
  `bitcoin:?amount=...&lightning=<bolt11>&ark=<address>` without an on-chain
  address (`additionalData.onchainAddress: null`); `amount=0.0` once paid.
- **Store defaults after setup**: `defaultPaymentMethod: "BTC-ARK"`,
  `onChainWithLnInvoiceFallback: true`. `lightning/BTC/info` answers 503
  `lightning-node-unavailable`, `lightning/BTC/balance` 200 with only
  `offchain.local` (msat).

## What reveals the Bark network or Bark's Lightning backing

Nothing in the wallet API (`.../BTC-ARK/wallet`, `/wallet/transactions`) names
the network: rows carry only txids, outpoints and payment hashes. The network
shows only through destination strings:

- Ark addresses: HRP `tark` for every non-mainnet network (regtest, signet,
  testnet, mutinynet), `ark` on mainnet. Found in `invoices`
  `paymentMethods[].destination`, `paymentMethods[].additionalData.arkAddress`,
  `paymentMethods[].payments[].destination`, the same paths of
  `invoice.<id>.payment-methods.accounted`, and the `ark=` parameter of the
  `BTC-ARK` `paymentLink`.
- BOLT11 HRP (`lnbcrt` regtest, `lntbs` signet, `lntb` testnet, `lnbc`
  mainnet): `BTC-LN` (and Ark prompts' `lightning=`) `destination`,
  `paymentLink`, `payments[].destination`; `payouts[].destination`;
  `lightning.invoices[].BOLT11`; `lightning.payments[].BOLT11`.
- No config field names the network: `BTC-ARK` config is only
  `{"enabled": true}`; `server/info` has no network field.

`BTC-LN` being served by Bark is visible only with `includeConfig=true`, which
needs `btcpay.store.canmodifystoresettings`: the config's `connectionString` is
`type=bark;capability=...` (`internalNodeRef: null`). A read-only key
(`canviewstoresettings` + `canviewwallet`) gets 403 `missing-permission` for
`includeConfig=true` on the list and on single methods, and the plain
`payment-methods` list shows `BTC-LN` with no hint of Bark. Indirect hints a
read-only key can see: `BTC-ARK`/`BARK` in `server/info`
`supportedPaymentMethods` and the store's payment methods, and Lightning rows
in the `BTC-ARK` wallet transactions whose `paymentHash` equals `BTC-LN`
invoice payment ids.
