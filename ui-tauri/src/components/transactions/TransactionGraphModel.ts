import { formatShortTxid } from "./model";

export type TransactionGraphAnnotation = {
  code: string;
  label?: string;
  severity?: "info" | "warning" | "error";
  groupId?: string | null;
  amountMsat?: number;
  amountBtc?: number;
  residualMsat?: number;
  residualBtc?: number;
};

export type TransactionGraphNode = {
  id: string;
  index?: number;
  outpoint?: string;
  txid?: string;
  vout?: number;
  address?: string;
  scriptType?: string;
  valueSats?: number | null;
  valueBtc?: number | null;
  valueState?: "known" | "missing" | "confidential";
  label?: string;
  wallet?: string;
  walletId?: string | null;
  ownership?: string;
  role?: string;
  overflow?: boolean;
  overflowCount?: number;
  annotations?: TransactionGraphAnnotation[];
};

export type TransactionSwapRouteLeg = {
  id?: string;
  externalId?: string | null;
  txid?: string | null;
  direction?: string | null;
  role?: "consolidation" | "spend" | "receive" | null;
  asset?: string | null;
  network?: string | null;
  amountMsat?: number | null;
  amountBtc?: number | null;
  feeMsat?: number | null;
  feeBtc?: number | null;
  occurredAt?: string | null;
  confirmedAt?: string | null;
  kind?: string | null;
  counterparty?: string | null;
  description?: string | null;
  wallet?: {
    id?: string | null;
    label?: string | null;
    kind?: string | null;
  } | null;
};

export type TransactionSwapRoute = {
  id?: string;
  kind?: string | null;
  routeKind?: string | null;
  policy?: string | null;
  pairSource?: string | null;
  confidence?: string | null;
  createdAt?: string | null;
  currentLeg?: "out" | "in" | null;
  swapFeeMsat?: number | null;
  swapFeeBtc?: number | null;
  swapFeeKind?: string | null;
  outAmountMsat?: number | null;
  outAmountBtc?: number | null;
  outFullAmountMsat?: number | null;
  outFullAmountBtc?: number | null;
  out: TransactionSwapRouteLeg;
  in: TransactionSwapRouteLeg;
};

export type TransactionGraphPayload = {
  transaction: {
    id: string;
    txid?: string | null;
    externalId?: string | null;
    asset?: string | null;
    inputCount?: number | null;
    outputCount?: number | null;
    version?: number | null;
    locktime?: number | null;
    size?: number | null;
    vsize?: number | null;
    weight?: number | null;
    feeRateSatVb?: number | null;
    // Resolved by the daemon (`_row_chain_network`); do not infer either of
    // these from asset or wallet labels.
    chain?: string | null;
    network?: string | null;
  } | null;
  supportLevel: "full" | "partial" | "graphless" | "unsupported";
  unsupportedReason?: string | null;
  warnings?: Array<{ code: string; level?: string; message: string }>;
  inputs: TransactionGraphNode[];
  outputs: TransactionGraphNode[];
  fee?: TransactionGraphNode | null;
  annotations?: TransactionGraphAnnotation[];
  accounting?: {
    quarantine?: { reason?: string | null; detail?: Record<string, unknown> } | null;
    linkedPairs?: TransactionGraphAnnotation[];
    transferGroupIds?: string[];
  };
  swapRoute?: TransactionSwapRoute | null;
};

export type TransactionGraphIssueTarget = "bitcoin" | "liquid";

export type TransactionRouteKind = "swap" | "coinjoin" | "transfer" | "pair";

function lowerJoin(parts: Array<string | null | undefined>) {
  return parts.filter(Boolean).join(" ").toLowerCase();
}

export function looksLiquid(...parts: Array<string | null | undefined>) {
  const text = lowerJoin(parts);
  return text.includes("liquid") || text.includes("lbtc") || text.includes("l-btc");
}

export function looksLightning(...parts: Array<string | null | undefined>) {
  const text = lowerJoin(parts);
  return text.includes("lightning") || text.includes("ln-btc");
}

/** "Liquid" / "Bitcoin" for display, or the raw asset when neither applies. */
export function routeNetworkLabel(
  asset?: string | null,
  ...walletHints: Array<string | null | undefined>
) {
  if (looksLiquid(asset, ...walletHints)) return "Liquid";
  if (String(asset || "").toUpperCase() === "BTC") return "Bitcoin";
  return asset || undefined;
}

function normalizedAsset(asset?: string | null) {
  return String(asset || "").trim().toUpperCase();
}

/**
 * Classify a paired route.
 *
 * The daemon ships `routeKind` and per-leg `role` for current payloads; this is
 * the single client heuristic used for everything that predates them (older
 * snapshots, and the pair fallback built from a transactions-list row). It
 * deliberately mirrors `_paired_route_kind` in `core/transaction_graph.py`.
 */
export function classifyRouteKind({
  kind,
  policy,
  outAsset,
  inAsset,
}: {
  kind?: string | null;
  policy?: string | null;
  outAsset?: string | null;
  inAsset?: string | null;
}): TransactionRouteKind {
  const text = String(kind || "").toLowerCase();
  if (text.includes("coinjoin") || text.includes("whirlpool")) return "coinjoin";
  if (
    text.includes("swap") ||
    text.startsWith("peg-") ||
    normalizedAsset(outAsset) !== normalizedAsset(inAsset)
  ) {
    return "swap";
  }
  if (policy === "carrying-value") return "transfer";
  return "pair";
}

/**
 * Whether the outgoing leg of a swap reads as a consolidation rather than a
 * plain spend: a Liquid-side leg feeding a different network, or a leg the
 * source data already describes as one.
 */
export function classifyRouteOutRole({
  kind,
  policy,
  description,
  outAsset,
  inAsset,
  outWallet,
  inWallet,
}: {
  kind?: string | null;
  policy?: string | null;
  description?: string | null;
  outAsset?: string | null;
  inAsset?: string | null;
  outWallet?: string | null;
  inWallet?: string | null;
}): "consolidation" | "spend" {
  if (classifyRouteKind({ kind, policy, outAsset, inAsset }) !== "swap") return "spend";
  if (lowerJoin([kind, description]).includes("consolidat")) return "consolidation";
  const outNetwork = routeNetworkLabel(outAsset, outWallet);
  const inNetwork = routeNetworkLabel(inAsset, inWallet);
  return outNetwork === "Liquid" && outNetwork !== inNetwork ? "consolidation" : "spend";
}

export type TransactionSwapRouteLegKey = "out" | "in";

export type GraphRow = TransactionGraphNode & { side: "input" | "output" | "fee" };

export const MAX_COMPACT_ROWS = 24;

export function compactGraphRows(
  nodes: TransactionGraphNode[],
  side: "input" | "output",
  maxRows = MAX_COMPACT_ROWS,
): GraphRow[] {
  const rows = nodes.map((node) => ({ ...node, side }));
  if (rows.length <= maxRows) return rows;
  const visible = rows.slice(0, Math.max(1, maxRows - 1));
  const hidden = rows.slice(visible.length);
  // A hidden node may itself be a server-side overflow node, so count the legs
  // it represents rather than the strand.
  const hiddenCount = hidden.reduce((sum, node) => sum + (node.overflowCount ?? 1), 0);
  // Only advertise a concrete value when every hidden leg has a known amount;
  // otherwise leave it amountless so a partial sum isn't shown as a total.
  const allKnown = hidden.every((node) => typeof node.valueSats === "number");
  const totalSats = allKnown
    ? hidden.reduce((sum, node) => sum + (node.valueSats ?? 0), 0)
    : null;
  return [
    ...visible,
    {
      id: `${side}-overflow`,
      side,
      label: `+${hiddenCount} more`,
      role: "overflow",
      ownership: "overflow",
      overflow: true,
      overflowCount: hiddenCount,
      valueSats: totalSats,
      valueBtc: totalSats != null ? totalSats / 100_000_000 : null,
      annotations: [
        {
          code: "overflow",
          label: `${hiddenCount} compacted ${side} rows`,
        },
      ],
    },
  ];
}

export function sensitiveGraphText(
  value: string | null | undefined,
  hidden: boolean,
  hiddenLabel: string,
) {
  if (!value) return "";
  return hidden ? hiddenLabel : value;
}

function nodeTitle(node: TransactionGraphNode) {
  if (node.overflow) return node.label ?? "More";
  return node.wallet || node.address || node.outpoint || node.label || "Transaction leg";
}

export function nodeTooltipTitle(node: TransactionGraphNode) {
  const title = nodeTitle(node);
  if (node.outpoint && title === node.outpoint) return formatShortTxid(node.outpoint);
  if (title.length > 48) return formatShortTxid(title);
  return title;
}
