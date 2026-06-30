import {
  AlertTriangle,
  ArrowRight,
  ArrowRightLeft,
  Expand,
  ExternalLink,
  Info,
  Maximize2,
} from "lucide-react";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { formatBtc } from "@/lib/currency";
import { cn } from "@/lib/utils";

import { copyText, formatShortTxid } from "./model";

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

type GraphRow = TransactionGraphNode & { side: "input" | "output" | "fee" };

const MAX_COMPACT_ROWS = 5;

export function compactGraphRows(
  nodes: TransactionGraphNode[],
  side: "input" | "output",
  maxRows = MAX_COMPACT_ROWS,
): GraphRow[] {
  const rows = nodes.map((node) => ({ ...node, side }));
  if (rows.length <= maxRows) return rows;
  const visible = rows.slice(0, Math.max(1, maxRows - 1));
  const hidden = rows.slice(visible.length);
  const totalSats = hidden.reduce(
    (sum, node) => sum + (typeof node.valueSats === "number" ? node.valueSats : 0),
    0,
  );
  return [
    ...visible,
    {
      id: `${side}-overflow`,
      side,
      label: `+${hidden.length} more`,
      role: "overflow",
      ownership: "overflow",
      overflow: true,
      overflowCount: hidden.length,
      valueSats: totalSats || null,
      valueBtc: totalSats ? totalSats / 100_000_000 : null,
      annotations: [
        {
          code: "overflow",
          label: `${hidden.length} compacted ${side} rows`,
        },
      ],
    },
  ];
}

export function sensitiveGraphText(value: string | null | undefined, hidden: boolean) {
  if (!value) return "";
  return hidden ? "Hidden" : value;
}

function formatNodeAmount(node: TransactionGraphNode, hidden: boolean) {
  if (hidden) return "Hidden";
  if (node.valueState === "confidential") return "Confidential amount";
  if (typeof node.valueBtc === "number") {
    return formatBtc(node.valueBtc);
  }
  if (typeof node.valueSats === "number") {
    return `${node.valueSats.toLocaleString("de-AT")} sats`;
  }
  return "";
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

function roleLabel(role?: string) {
  const labels: Record<string, string> = {
    input: "Input",
    output: "Output",
    change: "Change",
    external_recipient: "External recipient",
    incoming_payment: "Incoming payment",
    owned_destination: "Owned destination",
    op_return: "OP_RETURN / non-address",
    fee: "Fee",
    overflow: "More",
    ambiguous_owned_output: "Ambiguous owned output",
  };
  return labels[role ?? ""] ?? (role ? role.replace(/_/g, " ") : "Leg");
}

function ownershipBoundaryLabel(node: TransactionGraphNode) {
  if (node.role === "fee" || node.ownership === "network_fee") return "Network fee";
  if (node.ownership === "owned") {
    return node.role === "change" ? "Internal change output" : "Internal wallet leg";
  }
  if (node.ownership === "external") return "External wallet leg";
  if (node.ownership === "ambiguous") return "Ambiguous wallet ownership";
  if (node.ownership === "unspendable") return "Unspendable output";
  if (node.ownership === "overflow") return "Aggregated legs";
  return "Ownership unknown";
}

type DrawableGraphRow = GraphRow & {
  outerY: number;
  innerY: number;
  thickness: number;
  weight: number;
  offset: number;
  visualValueSats: number;
  estimatedVisualValue: boolean;
};

type GraphHoverDetail = {
  node: DrawableGraphRow;
};

function positiveKnownSats(node: TransactionGraphNode) {
  return typeof node.valueSats === "number" && node.valueSats > 0
    ? node.valueSats
    : 0;
}

function visualSatsForNode(node: TransactionGraphNode, fallbackSats: number) {
  if (typeof node.valueSats === "number") {
    return Math.max(0, node.valueSats);
  }
  return Math.max(1, fallbackSats);
}

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function buildDrawableRows(
  rows: GraphRow[],
  totalSats: number,
  height: number,
  combinedWeight: number,
  curveWidth: number,
  fallbackSats: number,
): DrawableGraphRow[] {
  if (!rows.length) return [];
  const centerY = height / 2;
  const visualValues = rows.map((node) => visualSatsForNode(node, fallbackSats));
  const unknownCount = rows.filter((node) => typeof node.valueSats !== "number").length;
  const knownTotal = rows.reduce(
    (sum, node) => sum + (typeof node.valueSats === "number" ? Math.max(0, node.valueSats) : 0),
    0,
  );
  const unknownShare =
    unknownCount > 0
      ? Math.max(1, (Math.max(totalSats, knownTotal) - knownTotal) / unknownCount)
      : 0;
  const weights = rows.map((node, index) => {
    if (!totalSats) return combinedWeight / rows.length;
    const value =
      typeof node.valueSats === "number"
        ? Math.max(0, node.valueSats)
        : unknownShare || visualValues[index];
    return (combinedWeight * value) / Math.max(1, totalSats);
  });
  const lines = rows.map((node, index) => {
    const knownZero = typeof node.valueSats === "number" && node.valueSats <= 0;
    const weight = weights[index] ?? 0;
    return {
      ...node,
      outerY: centerY,
      innerY: centerY,
      thickness: knownZero
        ? 3
        : Math.min(combinedWeight + 0.5, Math.max(2, weight) + 1),
      weight,
      offset: 0,
      visualValueSats: visualValues[index] ?? 0,
      estimatedVisualValue: typeof node.valueSats !== "number",
    };
  });
  const visibleWeight = lines.reduce((sum, line) => sum + line.thickness, 0);
  const spacing =
    lines.length <= 1
      ? 0
      : Math.max(8, (Math.max(120, height - 80) - visibleWeight) / Math.max(1, lines.length - 1));
  const innerTop = centerY - combinedWeight / 2;
  const innerBottom = innerTop + combinedWeight + 0.5;
  let lastOuter = 40;
  let lastInner = innerTop;
  let offset = 0;
  let minOffset = 0;
  let maxOffset = 0;
  let lastWeight = 0;
  let pad = 0;
  lines.forEach((line) => {
    if (lines.length === 1) {
      line.outerY = centerY;
    } else {
      line.outerY = lastOuter + line.thickness / 2;
    }
    line.innerY = clamp(
      lastInner + line.weight / 2,
      innerTop + line.thickness / 2,
      innerBottom - line.thickness / 2,
    );
    lastOuter += line.thickness + spacing;
    lastInner += line.weight;

    const t = (lastWeight + line.weight) / 2;
    const dx = Math.max(1, 0.75 * curveWidth);
    const dy = 1.5 * (line.innerY - line.outerY);
    const angle = Math.atan2(dy, dx);
    if (Math.sin(angle) !== 0) {
      offset += clamp((t * (1 - Math.cos(angle))) / Math.sin(angle), -t, t);
    }
    line.offset = offset;
    minOffset = Math.min(minOffset, offset);
    maxOffset = Math.max(maxOffset, offset);
    pad = Math.max(pad, line.thickness / 2);
    lastWeight = line.weight;
  });

  return lines.map((line) => ({
    ...line,
    offset: line.offset - minOffset + pad + (maxOffset - minOffset),
  }));
}

function strandStroke(node: GraphRow, active = false) {
  if (node.role === "fee") {
    return active
      ? "url(#transaction-flow-fee-hover-gradient)"
      : "url(#transaction-flow-fee-gradient)";
  }
  if (node.side === "input") {
    return active
      ? "url(#transaction-flow-input-hover-gradient)"
      : "url(#transaction-flow-input-gradient)";
  }
  return active
    ? "url(#transaction-flow-output-hover-gradient)"
    : "url(#transaction-flow-output-gradient)";
}

function makeBowtiePath(
  node: DrawableGraphRow,
  side: "input" | "output",
  canvasWidth: number,
  edgePadding: number,
  centerX: number,
  midWidth: number,
) {
  const start = edgePadding;
  const end = centerX - midWidth * 0.9 + 1;
  const maxOffset = Math.max(0, end - start - 44);
  const offset = Math.min(node.offset, maxOffset);
  const curveStart = Math.min(Math.max(start + 5, edgePadding + offset), end - 28);
  const curveEnd = clamp(end - offset - 10, curveStart + 18, end - 4);
  const midpoint = (curveStart + curveEnd) / 2;
  let outerY = node.outerY;
  if (Math.round(outerY) === Math.round(node.innerY)) {
    outerY -= 1;
  }

  if (side === "input") {
    return `M ${start} ${outerY} L ${curveStart} ${outerY} C ${midpoint} ${outerY}, ${midpoint} ${node.innerY}, ${curveEnd} ${node.innerY} L ${end} ${node.innerY}`;
  }
  return `M ${canvasWidth - start} ${outerY} L ${canvasWidth - curveStart} ${outerY} C ${
    canvasWidth - midpoint
  } ${outerY}, ${canvasWidth - midpoint} ${node.innerY}, ${canvasWidth - curveEnd} ${
    node.innerY
  } L ${canvasWidth - end} ${node.innerY}`;
}

function copyReference(node: GraphRow) {
  return node.outpoint || node.address || node.txid || null;
}

function copyAriaLabel(side: GraphRow["side"]) {
  if (side === "input") return "Copy input outpoint";
  if (side === "fee") return "Copy fee reference";
  return "Copy output reference";
}

function GraphStrand({
  node,
  path,
  markerPath,
  testId,
  active,
  onHover,
  onLeave,
}: {
  node: DrawableGraphRow;
  path: string;
  markerPath: string;
  testId?: string;
  active?: boolean;
  onHover: (node: DrawableGraphRow) => void;
  onLeave: () => void;
}) {
  const reference = copyReference(node);
  const ariaLabel = reference
    ? copyAriaLabel(node.side)
    : `${roleLabel(node.role)} graph leg`;
  const handleCopy = () => {
    if (reference) copyText(reference);
  };
  const handleKeyDown = (event: KeyboardEvent<SVGPathElement>) => {
    if (!reference) return;
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      handleCopy();
    }
  };

  return (
    <>
      <path
        d={markerPath}
        role={reference ? "button" : "img"}
        tabIndex={reference ? 0 : undefined}
        aria-label={ariaLabel}
        className={cn(
          "fill-none stroke-transparent outline-none",
          reference && "cursor-pointer",
        )}
        strokeWidth={Math.max(18, node.thickness + 12)}
        strokeLinecap="round"
        onClick={handleCopy}
        onKeyDown={handleKeyDown}
        onPointerEnter={() => onHover(node)}
        onPointerMove={() => onHover(node)}
        onMouseLeave={onLeave}
        onFocus={() => onHover(node)}
        onBlur={onLeave}
      />
      <path
        d={path}
        data-testid={testId}
        aria-hidden="true"
        className={cn(
          "pointer-events-none fill-none transition-opacity",
          active ? "opacity-100" : "opacity-90",
        )}
        stroke={strandStroke(node, active)}
        strokeWidth={node.thickness}
        strokeLinecap="butt"
      />
    </>
  );
}

function GraphStrandHoverHighlight({
  node,
  path,
  testId,
}: {
  node: DrawableGraphRow;
  path: string;
  testId?: string;
}) {
  return (
    <path
      d={path}
      data-testid={testId}
      aria-hidden="true"
      className="pointer-events-none fill-none opacity-100"
      stroke={strandStroke(node, true)}
      strokeWidth={node.thickness + 3}
      strokeLinecap="butt"
      filter="url(#transaction-flow-hover-glow)"
    />
  );
}

function AnnotationStrip({
  annotations,
  hideSensitive,
}: {
  annotations?: TransactionGraphAnnotation[];
  hideSensitive: boolean;
}) {
  const items = (annotations ?? []).slice(0, 8);
  if (!items.length) return null;
  return (
    <div className="flex flex-wrap gap-1.5">
      {items.map((annotation, index) => (
        <Badge
          key={`${annotation.code}-${annotation.groupId ?? ""}-${index}`}
          variant={annotation.severity === "warning" ? "destructive" : "secondary"}
          className={cn("rounded-md", hideSensitive && annotation.groupId && "sensitive")}
        >
          {annotation.label ?? annotation.code}
          {annotation.groupId ? ` · ${annotation.groupId}` : ""}
        </Badge>
      ))}
    </div>
  );
}

function formatRouteAmount(
  amountBtc: number | null | undefined,
  asset: string | null | undefined,
  hidden: boolean,
) {
  if (hidden) return "Hidden";
  if (typeof amountBtc !== "number") return "";
  const assetText = asset ? ` ${asset}` : "";
  return `${formatBtc(amountBtc)}${assetText}`;
}

function legReference(leg: TransactionSwapRouteLeg) {
  return leg.txid || leg.externalId || leg.id || "";
}

function routeCounterparty(route: TransactionSwapRoute) {
  return (
    route.out.counterparty ||
    route.in.counterparty ||
    route.out.description ||
    route.in.description ||
    route.kind ||
    ""
  );
}

function swapRouteOutLooksLikeConsolidation(route: TransactionSwapRoute) {
  if (route.out.role === "consolidation") return true;
  const text = `${route.out.kind || ""} ${route.out.description || ""}`.toLowerCase();
  if (text.includes("consolidat")) return true;
  const kind = String(route.kind || "").toLowerCase();
  const outNetwork = String(route.out.network || route.out.asset || "").toLowerCase();
  const inNetwork = String(route.in.network || route.in.asset || "").toLowerCase();
  return kind.includes("swap") && outNetwork.includes("liquid") && outNetwork !== inNetwork;
}

function SwapRouteLeg({
  label,
  leg,
  active,
  selectedLabel,
  unknownLabel,
  hideSensitive,
}: {
  label: string;
  leg: TransactionSwapRouteLeg;
  active: boolean;
  selectedLabel: string;
  unknownLabel: string;
  hideSensitive: boolean;
}) {
  const amount = formatRouteAmount(leg.amountBtc, leg.asset, hideSensitive);
  const wallet = sensitiveGraphText(leg.wallet?.label, hideSensitive);
  const reference = sensitiveGraphText(formatShortTxid(legReference(leg)), hideSensitive);
  return (
    <div
      className={cn(
        "min-w-0 rounded-md border bg-background px-3 py-2",
        active && "border-primary/60 bg-primary/10",
      )}
    >
      <div className="flex min-w-0 items-center justify-between gap-2">
        <div className="min-w-0">
          <div className="text-[10px] font-medium uppercase text-muted-foreground">
            {label}
          </div>
          <div className="mt-1 flex min-w-0 items-center gap-1.5 text-sm font-medium">
            <span>{leg.network || leg.asset || unknownLabel}</span>
            {leg.asset ? (
              <Badge variant="secondary" className="rounded-md px-1.5 py-0 text-[10px]">
                {leg.asset}
              </Badge>
            ) : null}
          </div>
        </div>
        {active ? (
          <Badge variant="outline" className="shrink-0 rounded-md px-1.5 py-0 text-[10px]">
            {selectedLabel}
          </Badge>
        ) : null}
      </div>
      {amount ? (
        <div className={cn("mt-1 text-sm tabular-nums", hideSensitive && "sensitive")}>
          {amount}
        </div>
      ) : null}
      {wallet ? (
        <div className={cn("mt-1 truncate text-xs text-muted-foreground", hideSensitive && "sensitive")}>
          {wallet}
        </div>
      ) : null}
      {reference ? (
        <div className={cn("mt-1 truncate font-mono text-[11px] text-muted-foreground", hideSensitive && "sensitive")}>
          {reference}
        </div>
      ) : null}
    </div>
  );
}

function SwapRouteStrip({
  route,
  hideSensitive,
}: {
  route?: TransactionSwapRoute | null;
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("transactions");
  if (!route) return null;
  const counterparty = sensitiveGraphText(routeCounterparty(route), hideSensitive);
  const fee = formatRouteAmount(route.swapFeeBtc, route.out.asset, hideSensitive);
  const routeLabel = `${route.out.asset || t("graph.swapRouteAsset")} -> ${
    route.in.asset || t("graph.swapRouteAsset")
  }`;
  const outLabel =
    swapRouteOutLooksLikeConsolidation(route)
      ? t("graph.swapRouteConsolidation")
      : t("graph.swapRouteOut");
  const inLabel =
    route.in.role === "consolidation"
      ? t("graph.swapRouteConsolidation")
      : t("graph.swapRouteIn");
  return (
    <div className="rounded-md border bg-muted/25 p-3" data-testid="swap-route-strip">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <ArrowRightLeft className="size-4 shrink-0 text-sky-500" aria-hidden="true" />
          <div className="min-w-0">
            <div className="text-sm font-medium">{t("graph.swapRouteTitle")}</div>
            <div className="truncate text-xs text-muted-foreground">{routeLabel}</div>
          </div>
        </div>
        {fee ? (
          <Badge variant="secondary" className={cn("rounded-md", hideSensitive && "sensitive")}>
            {t("graph.swapRouteFee", { fee })}
          </Badge>
        ) : null}
      </div>
      <div className="mt-3 grid items-stretch gap-2 md:grid-cols-[minmax(0,1fr)_minmax(190px,0.9fr)_minmax(0,1fr)]">
        <SwapRouteLeg
          label={outLabel}
          leg={route.out}
          active={route.currentLeg === "out"}
          selectedLabel={t("graph.swapRouteSelected")}
          unknownLabel={t("graph.swapRouteUnknownLeg")}
          hideSensitive={hideSensitive}
        />
        <div className="flex min-w-0 items-center justify-center gap-2 px-3 py-2 text-center">
          <ArrowRight className="hidden size-4 shrink-0 text-muted-foreground md:block" aria-hidden="true" />
          <div className="min-w-0">
            <div className="text-[10px] font-medium uppercase text-muted-foreground">
              {t("graph.swapRouteMiddle")}
            </div>
            <div className={cn("mt-1 text-sm font-medium leading-snug", hideSensitive && "sensitive")}>
              {counterparty || t("graph.swapRoutePair")}
            </div>
            {route.policy ? (
              <div className="mt-1 truncate text-[11px] text-muted-foreground">
                {route.policy}
              </div>
            ) : null}
          </div>
          <ArrowRight className="hidden size-4 shrink-0 text-muted-foreground md:block" aria-hidden="true" />
        </div>
        <SwapRouteLeg
          label={inLabel}
          leg={route.in}
          active={route.currentLeg === "in"}
          selectedLabel={t("graph.swapRouteSelected")}
          unknownLabel={t("graph.swapRouteUnknownLeg")}
          hideSensitive={hideSensitive}
        />
      </div>
    </div>
  );
}

export function TransactionFlowDiagram({
  graph,
  hideSensitive,
  expanded = false,
}: {
  graph: TransactionGraphPayload;
  hideSensitive: boolean;
  expanded?: boolean;
}) {
  const preferredCanvasWidth = expanded ? 1120 : 960;
  const shellRef = useRef<HTMLDivElement | null>(null);
  const [hoverDetail, setHoverDetail] = useState<GraphHoverDetail | null>(null);
  const [measuredCanvasWidth, setMeasuredCanvasWidth] = useState<number | null>(null);
  const inputRows = expanded
    ? graph.inputs.map((node) => ({ ...node, side: "input" as const }))
    : compactGraphRows(graph.inputs, "input");
  const outputRowsBase = expanded
    ? graph.outputs.map((node) => ({ ...node, side: "output" as const }))
    : compactGraphRows(graph.outputs, "output");
  const outputRows: GraphRow[] = outputRowsBase;
  const feeRow: GraphRow | null = graph.fee ? { ...graph.fee, side: "fee" } : null;
  const destinationRows = feeRow ? [feeRow, ...outputRows] : outputRows;
  const rowCount = Math.max(inputRows.length, destinationRows.length, 2);
  const height = Math.max(280, rowCount * 58 + 90);
  const viewportHeight = expanded
    ? `min(72vh, ${Math.max(440, Math.min(height + 24, 760))}px)`
    : `${Math.max(340, Math.min(height + 8, 430))}px`;
  useEffect(() => {
    const element = shellRef.current;
    if (!element) return undefined;
    const updateWidth = () => {
      const width = Math.floor(element.clientWidth);
      if (width > 0) setMeasuredCanvasWidth(Math.max(420, width));
    };
    updateWidth();
    if (typeof ResizeObserver === "undefined") return undefined;
    const observer = new ResizeObserver(updateWidth);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const canvasWidth = measuredCanvasWidth ?? preferredCanvasWidth;
  const centerY = height / 2;
  const centerX = canvasWidth / 2;
  const edgePadding = expanded ? 84 : 64;
  const midWidth = Math.min(10, Math.ceil(canvasWidth / 100));
  const curveWidth = centerX - edgePadding - midWidth - 12;
  const inputKnownTotal = inputRows.reduce(
    (sum, node) => sum + positiveKnownSats(node),
    0,
  );
  const outputKnownTotal = destinationRows.reduce(
    (sum, node) => sum + positiveKnownSats(node),
    0,
  );
  const visualTotal = Math.max(inputKnownTotal, outputKnownTotal, 1);
  const fallbackSats = Math.max(1, visualTotal / rowCount);
  const combinedWeight = Math.min(expanded ? 96 : 82, Math.max(26, Math.floor((canvasWidth - 2 * edgePadding) / 9)));
  const inputDrawRows = buildDrawableRows(
    inputRows,
    visualTotal,
    height,
    combinedWeight,
    curveWidth,
    fallbackSats,
  );
  const outputDrawRows = buildDrawableRows(
    destinationRows,
    visualTotal,
    height,
    combinedWeight,
    curveWidth,
    fallbackSats,
  );
  const activeNode =
    hoverDetail
      ? [...inputDrawRows, ...outputDrawRows].find(
          (node) =>
            node.id === hoverDetail.node.id &&
            node.side === hoverDetail.node.side,
        )
      : undefined;
  const pathFor = (node: DrawableGraphRow) =>
    makeBowtiePath(node, node.side === "input" ? "input" : "output", canvasWidth, edgePadding, centerX, midWidth);
  const markerPathFor = (node: DrawableGraphRow) =>
    makeBowtiePath(
      { ...node, thickness: Math.max(18, node.thickness + 12) },
      node.side === "input" ? "input" : "output",
      canvasWidth,
      edgePadding,
      centerX,
      midWidth,
    );
  const showHoverDetail = (node: DrawableGraphRow) => {
    setHoverDetail({ node });
  };
  return (
    <div
      ref={shellRef}
      className={cn(
        "overflow-hidden rounded-md border bg-background",
      )}
      style={{ height: viewportHeight }}
      data-testid="transaction-flow-diagram"
    >
      <span className={cn("sr-only", hideSensitive && "sensitive")}>
        {hideSensitive
          ? "Hidden graph references"
          : "Graph references are available from each strand"}
      </span>
      <div className="h-[calc(100%-64px)] overflow-auto">
        <div
          className="relative"
          data-testid="transaction-flow-canvas"
          data-transaction-flow-canvas
          style={{ width: canvasWidth, height }}
        >
          <svg
            className="absolute inset-0"
            width={canvasWidth}
            height={height}
            viewBox={`0 0 ${canvasWidth} ${height}`}
            role="img"
            aria-label="Transaction flow diagram"
          >
            <defs>
              <linearGradient id="transaction-flow-input-gradient" x1="0%" y1="0%" x2="100%" y2="0%">
                <stop offset="0%" stopColor="rgb(59 130 246)" />
                <stop offset="100%" stopColor="rgb(14 165 233)" />
              </linearGradient>
              <linearGradient id="transaction-flow-input-hover-gradient" x1="0%" y1="0%" x2="100%" y2="0%">
                <stop offset="0%" stopColor="rgb(96 165 250)" />
                <stop offset="100%" stopColor="rgb(34 211 238)" />
              </linearGradient>
              <linearGradient id="transaction-flow-output-gradient" x1="0%" y1="0%" x2="100%" y2="0%">
                <stop offset="0%" stopColor="rgb(14 165 233)" />
                <stop offset="100%" stopColor="rgb(59 130 246)" />
              </linearGradient>
              <linearGradient id="transaction-flow-output-hover-gradient" x1="0%" y1="0%" x2="100%" y2="0%">
                <stop offset="0%" stopColor="rgb(34 211 238)" />
                <stop offset="100%" stopColor="rgb(96 165 250)" />
              </linearGradient>
              <linearGradient id="transaction-flow-fee-gradient" x1="0%" y1="0%" x2="100%" y2="0%">
                <stop offset="0%" stopColor="rgb(14 165 233)" />
                <stop offset="52%" stopColor="rgb(245 158 11)" />
                <stop offset="100%" stopColor="transparent" />
              </linearGradient>
              <linearGradient id="transaction-flow-fee-hover-gradient" x1="0%" y1="0%" x2="100%" y2="0%">
                <stop offset="0%" stopColor="rgb(34 211 238)" />
                <stop offset="58%" stopColor="rgb(251 191 36)" />
                <stop offset="100%" stopColor="transparent" />
              </linearGradient>
              <filter
                id="transaction-flow-hover-glow"
                x="-18%"
                y="-70%"
                width="136%"
                height="240%"
                colorInterpolationFilters="sRGB"
              >
                <feDropShadow
                  dx="0"
                  dy="0"
                  stdDeviation="3.2"
                  floodColor="rgb(125 211 252)"
                  floodOpacity="0.7"
                />
                <feDropShadow
                  dx="0"
                  dy="0"
                  stdDeviation="1"
                  floodColor="rgb(255 255 255)"
                  floodOpacity="0.55"
                />
              </filter>
            </defs>
          {inputDrawRows.map((node) => (
            <GraphStrand
              key={`curve-${node.id}`}
              node={node}
              path={pathFor(node)}
              markerPath={markerPathFor(node)}
              testId="transaction-input-strand"
              active={hoverDetail?.node.id === node.id && hoverDetail.node.side === node.side}
              onHover={showHoverDetail}
              onLeave={() => setHoverDetail(null)}
            />
          ))}
          {outputDrawRows.map((node) => (
            <GraphStrand
              key={`curve-${node.id}`}
              node={node}
              path={pathFor(node)}
              markerPath={markerPathFor(node)}
              testId={node.side === "fee" ? "transaction-fee-strand" : "transaction-output-strand"}
              active={hoverDetail?.node.id === node.id && hoverDetail.node.side === node.side}
              onHover={showHoverDetail}
              onLeave={() => setHoverDetail(null)}
            />
          ))}
          <path
            d={`M ${centerX - midWidth - 1} ${centerY + 0.25} L ${centerX + midWidth + 1} ${centerY + 0.25}`}
            data-testid="transaction-flow-middle-band"
            className="pointer-events-none fill-none opacity-95"
            stroke="url(#transaction-flow-output-gradient)"
            strokeWidth={combinedWeight + 1}
            strokeLinecap="butt"
          />
          {activeNode ? (
            <GraphStrandHoverHighlight
              node={activeNode}
              path={pathFor(activeNode)}
              testId="transaction-hover-strand"
            />
          ) : null}
        </svg>
        </div>
      </div>
      <div
        data-testid="transaction-graph-hover-detail"
        className="h-16 border-t border-white/10 bg-[#101114] px-3 py-2 text-xs text-white"
      >
        {hoverDetail ? (
          <div className="grid h-full min-w-0 content-center gap-1">
            <div className="truncate font-medium">
              {sensitiveGraphText(nodeTooltipTitle(hoverDetail.node), hideSensitive)}
            </div>
            <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-white/70">
              {formatNodeAmount(hoverDetail.node, hideSensitive) ? (
                <span>{formatNodeAmount(hoverDetail.node, hideSensitive)}</span>
              ) : null}
              <span>{roleLabel(hoverDetail.node.role)}</span>
              <span>{ownershipBoundaryLabel(hoverDetail.node)}</span>
              {copyReference(hoverDetail.node) ? (
                <span className={cn("truncate font-mono", hideSensitive && "sensitive")}>
                  {hideSensitive ? "Hidden" : formatShortTxid(copyReference(hoverDetail.node) ?? "")}
                </span>
              ) : null}
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function GraphEmptyState({
  graph,
  loading,
  error,
}: {
  graph?: TransactionGraphPayload;
  loading?: boolean;
  error?: string | null;
}) {
  const { t } = useTranslation("transactions");
  const reason = graph?.unsupportedReason;
  const title = loading
    ? t("graph.loading")
    : error
      ? t("graph.error")
      : graph?.supportLevel === "unsupported"
        ? t("graph.unsupported")
        : reason === "liquid_reference_graph_not_local"
          ? t("graph.liquidGraphless")
          : t("graph.graphless");
  const body = loading
    ? t("graph.loadingBody")
    : error
      ? error
      : reason === "liquid_reference_graph_not_local"
        ? t("graph.liquidBody")
        : t("graph.graphlessBody");
  const txid = graph?.transaction?.txid ?? graph?.transaction?.externalId;
  const liquidExplorerHref =
    reason === "liquid_reference_graph_not_local" && txid
      ? `https://liquid.network/tx/${encodeURIComponent(txid)}`
      : null;
  const alertWarnings = (graph?.warnings ?? []).filter(
    (warning) => warning.level === "warning" || warning.level === "error",
  );
  return (
    <div className="space-y-2">
      <div className="rounded-md border bg-muted/35 p-4">
      <div className="flex items-start gap-3">
        <span className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md bg-background text-muted-foreground">
          {error ? <AlertTriangle className="size-4" /> : <Info className="size-4" />}
        </span>
        <div>
          <div className="text-sm font-medium">{title}</div>
          <div className="mt-1 text-sm text-muted-foreground">{body}</div>
          {liquidExplorerHref ? (
            <Button asChild variant="outline" size="sm" className="mt-3 gap-2">
              <a href={liquidExplorerHref} target="_blank" rel="noreferrer">
                <ExternalLink className="size-4" aria-hidden="true" />
                {t("graph.openLiquidNetwork")}
              </a>
            </Button>
          ) : null}
        </div>
      </div>
      </div>
      {alertWarnings.map((warning) => (
        <div
          key={`${warning.code}-${warning.message}`}
          className="flex gap-2 rounded-md border bg-muted/35 px-3 py-2 text-sm"
        >
          <AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-600" />
          <span>{warning.message}</span>
        </div>
      ))}
    </div>
  );
}

function graphSupportText(
  graph: TransactionGraphPayload,
  t: ReturnType<typeof useTranslation<"transactions">>["t"],
) {
  if (graph.supportLevel !== "partial") return t("graph.fullSupport");
  if (graph.unsupportedReason === "confidential_values_hidden") {
    return t("graph.confidentialSupport");
  }
  if (graph.unsupportedReason === "input_prevout_values_missing") {
    return t("graph.inputPrevoutSupport");
  }
  return t("graph.partialSupport");
}

export function TransactionGraphPanel({
  graph,
  loading,
  error,
  hideSensitive,
}: {
  graph?: TransactionGraphPayload;
  loading?: boolean;
  error?: string | null;
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("transactions");
  const showDiagram =
    graph &&
    (graph.supportLevel === "full" || graph.supportLevel === "partial") &&
    (graph.inputs.length > 0 || graph.outputs.length > 0);
  const alertWarnings = (graph?.warnings ?? []).filter(
    (warning) => warning.level === "warning" || warning.level === "error",
  );

  return (
    <div className="space-y-4">
      {showDiagram ? (
        <>
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <div className="text-sm font-medium">{t("graph.title")}</div>
              <div className="text-xs text-muted-foreground">
                {graphSupportText(graph, t)}
              </div>
            </div>
            <Dialog>
              <DialogTrigger asChild>
                <Button type="button" variant="outline" size="sm" className="gap-2">
                  <Maximize2 className="size-4" aria-hidden="true" />
                  {t("graph.expand")}
                </Button>
              </DialogTrigger>
              <DialogContent className="w-[min(1180px,calc(100vw-2rem))] max-w-none sm:max-w-none">
                <DialogHeader>
                  <DialogTitle className="flex items-center gap-2">
                    <Expand className="size-4" aria-hidden="true" />
                    {t("graph.expandedTitle")}
                  </DialogTitle>
                </DialogHeader>
                <TransactionFlowDiagram graph={graph} hideSensitive={hideSensitive} expanded />
              </DialogContent>
            </Dialog>
          </div>
          <SwapRouteStrip route={graph.swapRoute} hideSensitive={hideSensitive} />
          <AnnotationStrip annotations={graph.annotations} hideSensitive={hideSensitive} />
          <TransactionFlowDiagram graph={graph} hideSensitive={hideSensitive} />
          {alertWarnings.length ? (
            <div className="space-y-2">
              {alertWarnings.map((warning) => (
                <div
                  key={`${warning.code}-${warning.message}`}
                  className="flex gap-2 rounded-md border bg-muted/35 px-3 py-2 text-sm"
                >
                  <AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-600" />
                  <span>{warning.message}</span>
                </div>
              ))}
            </div>
          ) : null}
        </>
      ) : (
        <GraphEmptyState graph={graph} loading={loading} error={error} />
      )}
    </div>
  );
}
