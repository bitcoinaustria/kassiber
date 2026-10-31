import type { TFunction } from "i18next";

// Daemon entry-type CODES map to translated labels; unknown codes fall back to
// a structural humanizer of the code itself (the code is the stable id).
const ENTRY_TYPE_LABEL_KEYS: Record<string, string> = {
  acquisition: "ledger.entryType.acquisition",
  disposal: "ledger.entryType.disposal",
  income: "ledger.entryType.income",
  fee: "ledger.entryType.fee",
  transfer_fee: "ledger.entryType.transferFee",
  transfer: "ledger.entryType.transfer",
  transfer_in: "ledger.entryType.transferIn",
  transfer_out: "ledger.entryType.transferOut",
  neutral_swap: "ledger.entryType.neutralSwap",
};

export function humanizeEntryCode(code: string) {
  return code
    .replaceAll("_", " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}

export function localizedEntryType(type: string, t: TFunction<"journals">) {
  const key = ENTRY_TYPE_LABEL_KEYS[type];
  // dynamic key
  return key ? t(key as never) : humanizeEntryCode(type);
}
