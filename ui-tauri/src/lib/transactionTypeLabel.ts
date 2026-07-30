import type { TFunction } from "i18next";

// The daemon emits stable English display labels for a transaction's derived
// type (`_transaction_type` in core/ui_snapshot.py). Per docs/reference/i18n.md
// the daemon is never locale-aware, so the UI maps those labels to copy here.
//
// Values that are not in this map are user-authored metadata tags (the chip
// prefers a real tag over the derived label) — render those verbatim.
//
// `as const` keeps the values literal, so a key that does not exist in the
// English bundle is a compile error rather than a label that renders raw.
const TRANSACTION_TYPE_LABEL_KEYS = {
  // inbound
  Buy: "type.buy",
  Deposit: "type.deposit",
  Income: "type.income",
  Wages: "type.wages",
  Mining: "type.mining",
  Staking: "type.staking",
  Interest: "type.interest",
  Airdrop: "type.airdrop",
  "Hard fork": "type.hardFork",
  Acquired: "type.acquired",
  "LN invoice": "type.lnInvoice",
  "Channel close": "type.channelClose",
  // outbound
  Sell: "type.sell",
  Withdrawal: "type.withdrawal",
  Spend: "type.spend",
  Gift: "type.gift",
  Donation: "type.donation",
  Lost: "type.lost",
  Stolen: "type.stolen",
  Expense: "type.expense",
  "LN payment": "type.lnPayment",
  "Channel open": "type.channelOpen",
  // either direction
  Transfer: "type.transfer",
  Swap: "type.swap",
  Fee: "type.fee",
} as const;

export function transactionTypeLabel(
  t: TFunction<"transactions">,
  value: string | null | undefined,
): string {
  if (!value) return "";
  const key =
    TRANSACTION_TYPE_LABEL_KEYS[
      value as keyof typeof TRANSACTION_TYPE_LABEL_KEYS
    ];
  return key ? t(key) : value;
}
