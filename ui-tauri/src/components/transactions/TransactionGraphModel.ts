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

export type TransactionSwapRouteLegKey = "out" | "in";

export type GraphRow = TransactionGraphNode & { side: "input" | "output" | "fee" };

const MAX_COMPACT_ROWS = 24;

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
  hiddenLabel = "Hidden",
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
