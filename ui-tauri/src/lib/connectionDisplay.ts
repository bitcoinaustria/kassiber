import type { Connection, ConnectionKind, ConnectionStatus } from "@/mocks/seed";

/**
 * Layer-aware category: the chain wins over the kind so a Liquid
 * descriptor wallet reads "Liquid", not "On-chain".
 */
export function connectionCategoryLabel(
  connection: Pick<Connection, "kind" | "chain" | "role">,
): string {
  if (connection.role === "backend" || connection.kind === "backend") {
    return "Infrastructure";
  }
  if (connection.chain === "liquid") return "Liquid";
  return connectionKindCategoryLabels[connection.kind];
}

export const connectionKindCategoryLabels: Record<ConnectionKind, string> = {
  xpub: "On-chain",
  address: "On-chain",
  descriptor: "On-chain",
  "silent-payment": "On-chain",
  samourai: "On-chain",
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
  coinfinity: "Exchange",
  strike: "Custodial platform",
  phoenix: "Lightning",
  custom: "Custom",
  csv: "CSV",
  bip329: "BIP329",
  backend: "Infrastructure",
};

const connectionCategorySortOrder = [
  "On-chain",
  "Liquid",
  "Lightning",
  "NWC",
  "Ecash",
  "BTCPay",
  "Exchange",
  "Custodial platform",
  "CSV",
  "BIP329",
  "Custom",
  "Infrastructure",
] as const;

const connectionCategorySortRanks = new Map<string, number>(
  connectionCategorySortOrder.map((category, index) => [category, index]),
);

export function connectionCategorySortRank(
  connection: Pick<Connection, "kind" | "chain" | "role">,
): number {
  return (
    connectionCategorySortRanks.get(connectionCategoryLabel(connection)) ??
    connectionCategorySortOrder.length
  );
}

export const connectionKindLabels: Record<ConnectionKind, string> = {
  xpub: "Wallet export",
  address: "Address",
  descriptor: "Wallet export",
  "silent-payment": "Silent Payments",
  samourai: "Samourai",
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
  coinfinity: "Coinfinity",
  strike: "Strike",
  phoenix: "Phoenix",
  custom: "Custom",
  csv: "CSV",
  bip329: "BIP329",
  backend: "Backend",
};

export type ConnectionTypeInput = Pick<Connection, "kind"> &
  Partial<
    Pick<
      Connection,
      "paymentMethodId" | "sourceFormat" | "syncMode" | "syncSource" | "role"
    >
  >;

const sourceFormatLabels: Record<string, string> = {
  csv: "Generic CSV",
  json: "Generic JSON",
  phoenix_csv: "Phoenix CSV",
  river_csv: "River CSV",
  bullbitcoin_csv: "Bull Bitcoin CSV",
  bullbitcoin_wallet_csv: "Bull Bitcoin Wallet CSV",
  coinfinity_csv: "Coinfinity CSV",
  pocketbitcoin_csv: "Pocket Bitcoin CSV",
  "21bitcoin_csv": "21bitcoin CSV",
  strike_csv: "Strike CSV",
  ledgerlive_csv: "Ledger Live CSV",
  binance_supplemental_csv: "Binance supplemental CSV",
  wasabi_bundle: "Wasabi export",
  generic_ledger: "Generic ledger",
};

export function connectionTypeLabel(connection: ConnectionTypeInput): string {
  if (connection.role === "backend" || connection.kind === "backend") {
    return connection.syncSource?.trim() || "Backend endpoint";
  }
  const sourceFormat = connection.sourceFormat?.trim();
  if (sourceFormat) {
    return sourceFormatLabels[sourceFormat] ?? sourceFormat;
  }
  if (connection.syncMode === "btcpay" || connection.syncSource === "btcpay") {
    return connection.paymentMethodId
      ? `BTCPay API · ${connection.paymentMethodId}`
      : "BTCPay API";
  }
  switch (connection.kind) {
    case "xpub":
    case "descriptor":
      return "Wallet export";
    case "silent-payment":
      return "Silent Payments watch-only";
    case "address":
      return "Address list";
    case "samourai":
      return "Samourai watch-only";
    case "core-ln":
      return "Core Lightning API";
    case "lnd":
      return "LND API";
    case "nwc":
      return "NWC API";
    case "cashu":
      return "Cashu wallet";
    case "btcpay":
      return "BTCPay API";
    case "csv":
      return "Generic CSV";
    case "bip329":
      return "BIP329 labels";
    case "custom":
      return "Custom source";
    case "kraken":
    case "bitstamp":
    case "coinbase":
    case "bitpanda":
    case "river":
    case "bullbitcoin":
    case "coinfinity":
    case "strike":
    case "phoenix":
      return connectionKindLabels[connection.kind];
    default:
      return assertNeverConnectionKind(connection.kind);
  }
}

export const connectionStatusStyles: Record<ConnectionStatus, string> = {
  synced:
    "bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-400 dark:ring-emerald-400/20",
  syncing:
    "bg-amber-50 text-amber-700 ring-amber-600/20 dark:bg-amber-900/30 dark:text-amber-400 dark:ring-amber-400/20",
  idle: "bg-muted text-muted-foreground ring-border",
  error:
    "bg-red-50 text-red-700 ring-red-600/20 dark:bg-red-900/30 dark:text-red-400 dark:ring-red-400/20",
};

export type ConnectionAssetLabel = "BTC" | "LBTC" | "LN-BTC";

export type ConnectionAssetInput = Pick<Connection, "kind"> &
  Partial<
    Pick<
      Connection,
      | "asset"
      | "chain"
      | "network"
      | "policyAsset"
      | "paymentMethodId"
      | "sourceFormat"
      | "syncMode"
      | "syncSource"
      | "label"
    >
  >;

const lightningConnectionKinds = new Set<ConnectionKind>([
  "core-ln",
  "lnd",
  "nwc",
  "phoenix",
]);

const liquidSignals = ["liquid", "liquidv1", "lbtc", "l-btc"];
const lightningSignals = ["lightning", "ln-btc", "btc-ln", "btc_lightning"];

function hasSignal(values: Array<string | null | undefined>, signals: string[]) {
  return values.some((value) => {
    const normalized = value?.trim().toLowerCase();
    return Boolean(
      normalized &&
        signals.some((signal) =>
          normalized.split(/[^a-z0-9]+/).includes(signal) ||
          normalized.includes(signal),
        ),
    );
  });
}

export function connectionAssetLabel(
  connection: ConnectionAssetInput,
): ConnectionAssetLabel {
  const signals = [
    connection.asset,
    connection.chain,
    connection.network,
    connection.policyAsset,
    connection.paymentMethodId,
    connection.sourceFormat,
    connection.syncMode,
    connection.syncSource,
    connection.label,
  ];
  if (hasSignal(signals, liquidSignals)) return "LBTC";
  if (lightningConnectionKinds.has(connection.kind)) return "LN-BTC";
  if (hasSignal(signals, lightningSignals)) return "LN-BTC";
  return "BTC";
}

export function connectionAssetIconKind(asset: ConnectionAssetLabel) {
  switch (asset) {
    case "BTC":
    case "LN-BTC":
      return "bitcoin";
    case "LBTC":
      return "liquid";
  }
}

function assertNeverConnectionKind(kind: never): never {
  throw new Error(`Unhandled connection kind: ${kind}`);
}
