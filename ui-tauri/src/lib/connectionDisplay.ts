import type { ConnectionKind, ConnectionStatus } from "@/mocks/seed";

export const connectionKindCategoryLabels: Record<ConnectionKind, string> = {
  xpub: "On-chain",
  address: "On-chain",
  descriptor: "On-chain",
  "core-ln": "Lightning",
  lnd: "Lightning",
  nwc: "NWC",
  cashu: "Ecash",
  btcpay: "BTCPay",
  kraken: "Exchange",
  bitstamp: "Exchange",
  coinbase: "Exchange",
  bitpanda: "Exchange",
  river: "Exchange",
  bullbitcoin: "Exchange",
  strike: "Lightning",
  phoenix: "Lightning",
  custom: "Custom",
  csv: "CSV",
  bip329: "BIP329",
};

export const connectionKindLabels: Record<ConnectionKind, string> = {
  xpub: "XPUB",
  address: "Address",
  descriptor: "Descriptor",
  "core-ln": "Core Lightning",
  lnd: "LND",
  nwc: "NWC",
  cashu: "Cashu",
  btcpay: "BTCPay",
  kraken: "Kraken",
  bitstamp: "Bitstamp",
  coinbase: "Coinbase",
  bitpanda: "Bitpanda",
  river: "River",
  bullbitcoin: "Bull Bitcoin",
  strike: "Strike",
  phoenix: "Phoenix",
  custom: "Custom",
  csv: "CSV",
  bip329: "BIP329",
};

export const connectionStatusStyles: Record<ConnectionStatus, string> = {
  synced:
    "bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-400 dark:ring-emerald-400/20",
  syncing:
    "bg-amber-50 text-amber-700 ring-amber-600/20 dark:bg-amber-900/30 dark:text-amber-400 dark:ring-amber-400/20",
  idle: "bg-muted text-muted-foreground ring-border",
  error:
    "bg-red-50 text-red-700 ring-red-600/20 dark:bg-red-900/30 dark:text-red-400 dark:ring-red-400/20",
};

export function connectionKindTone(kind: ConnectionKind) {
  switch (kind) {
    case "core-ln":
    case "lnd":
    case "nwc":
    case "strike":
    case "phoenix":
      return "border-amber-600/20 bg-amber-500/10 text-amber-700 dark:text-amber-300";
    case "kraken":
    case "bitstamp":
    case "coinbase":
    case "bitpanda":
    case "river":
    case "bullbitcoin":
      return "border-violet-600/20 bg-violet-500/10 text-violet-700 dark:text-violet-300";
    case "cashu":
      return "border-sky-600/20 bg-sky-500/10 text-sky-700 dark:text-sky-300";
    case "btcpay":
    case "csv":
    case "bip329":
    case "custom":
      return "border-muted-foreground/20 bg-muted text-muted-foreground";
    case "xpub":
    case "address":
    case "descriptor":
      return "border-emerald-600/20 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300";
    default:
      return assertNeverConnectionKind(kind);
  }
}

function assertNeverConnectionKind(kind: never): never {
  throw new Error(`Unhandled connection kind: ${kind}`);
}
