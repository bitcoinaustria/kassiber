import {
  AlertTriangle,
  ArrowRight,
  ArrowRightLeft,
  ChevronDown,
  ChevronUp,
  Info,
  Maximize2,
} from "lucide-react";
import type { TFunction } from "i18next";
import { useEffect, useId, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";

import bitcoinIcon from "@/assets/integrations/bitcoin.svg";
import liquidIcon from "@/assets/integrations/liquid.svg";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogTrigger,
} from "@/components/ui/dialog";
import { openExternalUrl } from "@/daemon/transport";
import { formatBtc } from "@/lib/currency";
import { formatCount, formatSats } from "@/lib/localeFormat";
import {
  connectionAssetIconKind,
  type ConnectionAssetLabel,
} from "@/lib/connectionDisplay";
import {
  explorerTargetForAddress,
  explorerTargetForTransaction,
  type ExplorerNetwork,
  type ExplorerSettings,
  type ExplorerTarget,
} from "@/lib/explorer";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import { copyText, formatShortTxid } from "./model";
import {
  compactGraphRows,
  nodeTooltipTitle,
  sensitiveGraphText,
  type GraphRow,
  type TransactionGraphAnnotation,
  type TransactionGraphIssueTarget,
  type TransactionGraphNode,
  type TransactionGraphPayload,
  type TransactionSwapRoute,
  type TransactionSwapRouteLeg,
  type TransactionSwapRouteLegKey,
} from "./TransactionGraphModel";

export type {
  TransactionGraphAnnotation,
  TransactionGraphIssueTarget,
  TransactionGraphNode,
  TransactionGraphPayload,
  TransactionSwapRoute,
  TransactionSwapRouteLeg,
  TransactionSwapRouteLegKey,
} from "./TransactionGraphModel";

type TransactionGraphIssueLabelKey =
  | "graph.addBitcoinBackend"
  | "graph.reviewBitcoinBackend"
  | "graph.addLiquidBackend"
  | "graph.reviewLiquidBackend";

type TransactionGraphIssueAction = {
  target: TransactionGraphIssueTarget;
  labelKey: TransactionGraphIssueLabelKey;
};

const MAX_COMPACT_ROWS = 24;
// The expanded dialog shows far more strands, but still caps so a fan-out
// transaction cannot render thousands of paths. Matches the backend node cap.
const MAX_EXPANDED_ROWS = 250;
const MAX_DETAIL_COLLAPSED_ROWS = 8;

function formatNodeAmount(node: TransactionGraphNode, hidden: boolean, t: TFunction<"transactions">) {
  if (hidden) return t("graph.hidden");
  if (node.valueState === "confidential") return t("graph.confidentialAmount");
  if (typeof node.valueBtc === "number") {
    return formatBtc(node.valueBtc);
  }
  if (typeof node.valueSats === "number") {
    return formatSats(node.valueSats);
  }
  return "";
}

function nodeDisplayTitle(node: TransactionGraphNode, t: TFunction<"transactions">) {
  if (node.overflow) {
    return t("graph.overflowMore", { count: node.overflowCount ?? 0 });
  }
  return nodeTooltipTitle(node);
}

function roleLabel(role: string | undefined, t: TFunction<"transactions">) {
  const labels: Record<string, string> = {
    input: t("graph.roles.input"),
    output: t("graph.roles.output"),
    change: t("graph.roles.change"),
    external_recipient: t("graph.roles.externalRecipient"),
    incoming_payment: t("graph.roles.incomingPayment"),
    owned_destination: t("graph.roles.ownedDestination"),
    op_return: t("graph.roles.opReturn"),
    fee: t("graph.roles.fee"),
    overflow: t("graph.roles.overflow"),
    ambiguous_owned_output: t("graph.roles.ambiguousOwnedOutput"),
  };
  return labels[role ?? ""] ?? (role ? role.replace(/_/g, " ") : t("graph.roles.leg"));
}

function ownershipBoundaryLabel(node: TransactionGraphNode, t: TFunction<"transactions">) {
  if (node.role === "fee" || node.ownership === "network_fee") return t("graph.ownership.networkFee");
  if (node.ownership === "owned") {
    return node.role === "change"
      ? t("graph.ownership.internalChange")
      : t("graph.ownership.internalWallet");
  }
  if (node.ownership === "external") return t("graph.ownership.externalWallet");
  if (node.ownership === "ambiguous") return t("graph.ownership.ambiguousWallet");
  if (node.ownership === "unspendable") return t("graph.ownership.unspendable");
  if (node.ownership === "overflow") return t("graph.ownership.aggregated");
  return t("graph.ownership.unknown");
}

function nodeReference(node: TransactionGraphNode) {
  return node.address || node.outpoint || node.txid || node.label || "";
}

function nodeDetailReference(
  node: TransactionGraphNode,
  hidden: boolean,
  t: TFunction<"transactions">,
) {
  const reference = nodeReference(node);
  if (!reference) return t("graph.inputsOutputs.unknownReference");
  return sensitiveGraphText(formatShortTxid(reference), hidden, t("graph.hidden"));
}

function conciseScriptType(scriptType: string | undefined) {
  if (!scriptType) return "";
  const normalized = scriptType.replace(/[_-]/g, " ").replace(/\s+/g, " ").trim();
  const lower = normalized.toLowerCase();
  if (lower.includes("taproot")) return "taproot";
  if (lower.includes("witness v0") && lower.includes("keyhash")) return "segwit v0";
  if (lower.includes("witness v0") && lower.includes("scripthash")) return "segwit script";
  return normalized;
}

function nodeDetailMeta(
  node: TransactionGraphNode,
  side: "input" | "output",
  hidden: boolean,
  t: TFunction<"transactions">,
) {
  const parts = [];
  if (node.role && node.role !== side) {
    parts.push(roleLabel(node.role, t));
  }
  parts.push(ownershipBoundaryLabel(node, t));
  const scriptType = conciseScriptType(node.scriptType);
  if (scriptType) parts.push(scriptType);
  if (typeof node.index === "number") {
    parts.push(t("graph.inputsOutputs.index", { index: node.index }));
  }
  if (!hidden && node.address && node.outpoint) {
    parts.push(formatShortTxid(node.outpoint));
  }
  return parts;
}

function graphExplorerNetwork(graph: TransactionGraphPayload): ExplorerNetwork {
  const text = `${graph.transaction?.network ?? ""} ${graph.transaction?.asset ?? ""}`.toLowerCase();
  return text.includes("liquid") || text.includes("lbtc") || text.includes("l-btc")
    ? "liquid"
    : "bitcoin";
}

function explorerTargetForGraphNode({
  graph,
  node,
  settings,
}: {
  graph: TransactionGraphPayload;
  node: TransactionGraphNode;
  settings: ExplorerSettings;
}): ExplorerTarget | null {
  const network = graphExplorerNetwork(graph);
  if (node.address) {
    return explorerTargetForAddress({
      address: node.address,
      network,
      settings,
    });
  }
  const txid = node.txid || node.outpoint?.split(":")[0];
  return explorerTargetForTransaction({
    txid,
    network,
    settings,
  });
}

function amountSummary(nodes: TransactionGraphNode[]) {
  return nodes.reduce(
    (summary, node) => {
      if (node.valueState === "confidential") {
        summary.confidentialCount += 1;
      } else if (typeof node.valueSats === "number") {
        summary.knownCount += 1;
        summary.knownSats += node.valueSats;
      } else {
        summary.unknownCount += 1;
      }
      return summary;
    },
    {
      knownSats: 0,
      knownCount: 0,
      unknownCount: 0,
      confidentialCount: 0,
    },
  );
}

function hasCompleteTotal(nodes: TransactionGraphNode[]) {
  if (!nodes.length) return true;
  return nodes.every(
    (node) =>
      node.valueState !== "confidential" &&
      typeof node.valueSats === "number",
  );
}

function formatTotal(nodes: TransactionGraphNode[], t: TFunction<"transactions">) {
  const summary = amountSummary(nodes);
  if (summary.knownCount > 0) return formatBtc(summary.knownSats / 100_000_000);
  if (summary.confidentialCount > 0) {
    return t("graph.confidentialAmount");
  }
  return t("graph.inputsOutputs.unknownAmount");
}

function TransactionIoMarker({
  side,
}: {
  side: "input" | "output";
}) {
  const { t } = useTranslation("transactions");
  const isInput = side === "input";
  return (
    <span
      role="img"
      aria-label={
        isInput
          ? t("graph.inputsOutputs.spentInput")
          : t("graph.inputsOutputs.createdOutput")
      }
      className={cn(
        "mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-full border",
        isInput
          ? "border-red-500/30 bg-red-500/10 text-red-600 dark:text-red-400"
          : "border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
      )}
    >
      <ArrowRight className="size-3" aria-hidden="true" />
    </span>
  );
}

function TransactionIoRow({
  node,
  side,
  hideSensitive,
  explorerTarget,
  onOpenExplorer,
}: {
  node: TransactionGraphNode;
  side: "input" | "output";
  hideSensitive: boolean;
  explorerTarget: ExplorerTarget | null;
  onOpenExplorer: (target: ExplorerTarget) => void;
}) {
  const { t } = useTranslation("transactions");
  const amount =
    formatNodeAmount(node, hideSensitive, t) ||
    t("graph.inputsOutputs.unknownAmount");
  const canOpenExplorer = Boolean(explorerTarget && !hideSensitive && !node.overflow);
  const content = (
    <>
      <TransactionIoMarker side={side} />
      <div className="min-w-0">
        <div
          className={cn(
            "truncate font-mono text-xs font-medium",
            hideSensitive && "sensitive",
          )}
        >
          {nodeDetailReference(node, hideSensitive, t)}
        </div>
        <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] text-muted-foreground">
          {nodeDetailMeta(node, side, hideSensitive, t).map((part, index) => (
            <span
              key={`${node.id}-${part}-${index}`}
              className={cn(index > 3 && "hidden sm:inline")}
            >
              {part}
            </span>
          ))}
        </div>
      </div>
      <div
        className={cn(
          "flex shrink-0 items-start gap-1 self-start pt-0.5 text-right text-xs font-medium tabular-nums",
          hideSensitive && "sensitive",
        )}
      >
        <span>{amount}</span>
      </div>
    </>
  );
  if (canOpenExplorer && explorerTarget) {
    const openLabel = t("graph.inputsOutputs.openExplorer", {
      explorer: explorerTarget.label,
      reference: nodeDetailReference(node, false, t),
    });
    return (
      <button
        type="button"
        className="grid w-full min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] gap-2 border-t py-2 text-left first:border-t-0 hover:bg-muted/35 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        aria-label={openLabel}
        title={openLabel}
        onClick={() => onOpenExplorer(explorerTarget)}
      >
        {content}
      </button>
    );
  }
  return (
    <div className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] gap-2 border-t py-2 first:border-t-0">
      {content}
    </div>
  );
}

function TransactionIoColumn({
  title,
  nodes,
  side,
  hideSensitive,
  expanded,
  onToggleExpanded,
  explorerSettings,
  graph,
  onOpenExplorer,
}: {
  title: string;
  nodes: TransactionGraphNode[];
  side: "input" | "output";
  hideSensitive: boolean;
  expanded: boolean;
  onToggleExpanded: () => void;
  explorerSettings: ExplorerSettings;
  graph: TransactionGraphPayload;
  onOpenExplorer: (target: ExplorerTarget) => void;
}) {
  const { t } = useTranslation("transactions");
  const visibleNodes = expanded ? nodes : nodes.slice(0, MAX_DETAIL_COLLAPSED_ROWS);
  const hiddenCount = Math.max(0, nodes.length - visibleNodes.length);
  return (
    <div className="min-w-0">
      <div className="flex items-center justify-between gap-2 border-b pb-1.5">
        <div className="text-[10px] font-semibold uppercase tracking-wide text-muted-foreground">
          {title}
        </div>
        <div className="text-[10px] text-muted-foreground">
          {formatCount(nodes.length)}
        </div>
      </div>
      <div className={cn("overflow-auto pr-1", expanded ? "max-h-[520px]" : "max-h-[360px]")}>
        {visibleNodes.map((node) => (
          <TransactionIoRow
            key={`${side}-${node.id}`}
            node={node}
            side={side}
            hideSensitive={hideSensitive}
            explorerTarget={
              hideSensitive
                ? null
                : explorerTargetForGraphNode({ graph, node, settings: explorerSettings })
            }
            onOpenExplorer={onOpenExplorer}
          />
        ))}
        {nodes.length > MAX_DETAIL_COLLAPSED_ROWS ? (
          <button
            type="button"
            className="flex w-full items-center gap-1 border-t py-2 text-left text-xs text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            onClick={onToggleExpanded}
          >
            {expanded ? (
              <ChevronUp className="size-3.5" aria-hidden="true" />
            ) : (
              <ChevronDown className="size-3.5" aria-hidden="true" />
            )}
            {expanded
              ? t("graph.inputsOutputs.showFewer")
              : t("graph.inputsOutputs.showAll", { count: hiddenCount })}
          </button>
        ) : null}
      </div>
    </div>
  );
}

function TransactionIoTotalsPane({
  graph,
  hideSensitive,
}: {
  graph: TransactionGraphPayload;
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("transactions");
  const rows: Array<{
    id: "input" | "output";
    label: string;
    nodes: TransactionGraphNode[];
  }> = [
    { id: "input", label: t("graph.inputsOutputs.inputs"), nodes: graph.inputs },
    { id: "output", label: t("graph.inputsOutputs.outputs"), nodes: graph.outputs },
  ];
  return (
    <div
      className="mt-2 grid gap-4 border-t pt-2 md:grid-cols-2"
      data-testid="transaction-inputs-outputs-totals"
    >
      {rows.map((row) => (
        <div
          key={row.id}
          className="flex min-w-0 items-center justify-between gap-3 text-xs"
        >
          <div className="min-w-0">
            <div className="text-muted-foreground">
              {t(
                hasCompleteTotal(row.nodes)
                  ? "graph.inputsOutputs.total"
                  : "graph.inputsOutputs.knownTotal",
              )}
            </div>
          </div>
          <div
            className={cn(
              "min-w-0 break-words text-right font-medium tabular-nums",
              hideSensitive && "sensitive",
            )}
          >
            {hideSensitive ? t("graph.hidden") : formatTotal(row.nodes, t)}
          </div>
        </div>
      ))}
    </div>
  );
}

export function TransactionInputsOutputsPanel({
  graph,
  hideSensitive,
}: {
  graph: TransactionGraphPayload;
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("transactions");
  const explorerSettings = useUiStore((state) => state.explorerSettings);
  const [expandedColumns, setExpandedColumns] = useState({
    input: false,
    output: false,
  });
  const handleOpenExplorer = (target: ExplorerTarget) => {
    void openExternalUrl(target.url).catch((error) => {
      console.warn("Failed to open explorer URL", error);
    });
  };
  if (!graph.inputs.length && !graph.outputs.length) return null;
  return (
    <section className="border-t pt-3" data-testid="transaction-inputs-outputs-panel">
      <div className="grid gap-4 md:grid-cols-2">
        <TransactionIoColumn
          title={t("graph.inputsOutputs.inputs")}
          nodes={graph.inputs}
          side="input"
          hideSensitive={hideSensitive}
          expanded={expandedColumns.input}
          onToggleExpanded={() =>
            setExpandedColumns((current) => ({
              ...current,
              input: !current.input,
            }))
          }
          explorerSettings={explorerSettings}
          graph={graph}
          onOpenExplorer={handleOpenExplorer}
        />
        <TransactionIoColumn
          title={t("graph.inputsOutputs.outputs")}
          nodes={graph.outputs}
          side="output"
          hideSensitive={hideSensitive}
          expanded={expandedColumns.output}
          onToggleExpanded={() =>
            setExpandedColumns((current) => ({
              ...current,
              output: !current.output,
            }))
          }
          explorerSettings={explorerSettings}
          graph={graph}
          onOpenExplorer={handleOpenExplorer}
        />
      </div>
      <TransactionIoTotalsPane graph={graph} hideSensitive={hideSensitive} />
    </section>
  );
}

type DrawableGraphRow = GraphRow & {
  outerY: number;
  innerY: number;
  thickness: number;
  weight: number;
  offset: number;
  visualValueSats: number;
  estimatedVisualValue: boolean;
  zeroValue: boolean;
};

type GraphHoverDetail = {
  node: DrawableGraphRow;
};

const AMOUNTLESS_FEE_STRAND_THICKNESS = 0.5;
const GRAPH_ROW_HEIGHT = 29;
const GRAPH_MULTI_LEG_GAP = 4;

function positiveKnownSats(node: TransactionGraphNode) {
  return typeof node.valueSats === "number" && node.valueSats > 0
    ? node.valueSats
    : 0;
}

function visualTotalSatsForSides(inputRows: GraphRow[], destinationRows: GraphRow[]) {
  // Drawing only: confidential and missing values still render as confidential
  // text, but the bowtie needs a stable visual total to size unknown legs.
  const inputKnownTotal = inputRows.reduce((sum, node) => sum + positiveKnownSats(node), 0);
  const outputKnownTotal = destinationRows.reduce((sum, node) => sum + positiveKnownSats(node), 0);
  return Math.max(inputKnownTotal, outputKnownTotal, 1);
}

function fallbackVisualSats(visualTotalSats: number, rowCount: number) {
  return Math.max(1, visualTotalSats / Math.max(1, rowCount));
}

function hasAmountlessGeometryValue(node: TransactionGraphNode) {
  return typeof node.valueSats !== "number" || node.valueState === "confidential";
}

function hasKnownZeroGeometryValue(node: TransactionGraphNode) {
  return !hasAmountlessGeometryValue(node) && typeof node.valueSats === "number" && node.valueSats <= 0;
}

function visualSatsForNode(node: TransactionGraphNode, fallbackSats: number) {
  if (hasAmountlessGeometryValue(node)) {
    return Math.max(1, fallbackSats);
  }
  if (typeof node.valueSats === "number") {
    return Math.max(0, node.valueSats);
  }
  return Math.max(1, fallbackSats);
}

function visualKnownSatsForNode(
  node: GraphRow,
  hasAmountlessNonFeeRows: boolean,
) {
  const valueSats = node.valueSats;
  if (typeof valueSats !== "number" || node.valueState === "confidential") return null;
  const value = Math.max(0, valueSats);
  if (node.side === "fee" && hasAmountlessNonFeeRows && value > 0) return 1;
  return value;
}

function redactRowsForGeometry(rows: GraphRow[]): GraphRow[] {
  return rows.map((node) => ({
    ...node,
    valueSats: null,
    valueBtc: null,
    valueState: node.valueState === "confidential" ? "confidential" : "missing",
  }));
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
  const hasAmountlessNonFeeRows = rows.some(
    (node) => node.side !== "fee" && hasAmountlessGeometryValue(node),
  );
  const knownVisualValues = rows.map((node) =>
    visualKnownSatsForNode(node, hasAmountlessNonFeeRows),
  );
  const visualValues = rows.map((node, index) =>
    knownVisualValues[index] ?? visualSatsForNode(node, fallbackSats),
  );
  const unknownCount = rows.filter(hasAmountlessGeometryValue).length;
  const knownTotal = knownVisualValues.reduce(
    (sum: number, value) => sum + (value ?? 0),
    0,
  );
  const unknownShare =
    unknownCount > 0
      ? Math.max(1, (Math.max(totalSats, knownTotal) - knownTotal) / unknownCount)
      : 0;
  const weights = rows.map((_node, index) => {
    if (!totalSats) return combinedWeight / rows.length;
    const value = knownVisualValues[index] ?? (unknownShare || visualValues[index]);
    return (combinedWeight * value) / Math.max(1, totalSats);
  });
  const lines = rows.map((node, index) => {
    const knownZero = hasKnownZeroGeometryValue(node);
    const weight = weights[index] ?? 0;
    const amountlessPeerFee =
      node.side === "fee" && hasAmountlessNonFeeRows && weight > 0;
    return {
      ...node,
      outerY: centerY,
      innerY: centerY,
      thickness: amountlessPeerFee
        ? AMOUNTLESS_FEE_STRAND_THICKNESS
        : knownZero
        ? 3
        : Math.min(combinedWeight + 0.5, Math.max(2, weight) + 1),
      weight,
      offset: 0,
      visualValueSats: visualValues[index] ?? 0,
      estimatedVisualValue: hasAmountlessGeometryValue(node),
      zeroValue: knownZero,
    };
  });
  const visibleWeight = lines.reduce((sum, line) => sum + line.thickness, 0);
  const spacing =
    lines.length <= 1
      ? 0
      : Math.max(
          GRAPH_MULTI_LEG_GAP,
          (Math.max(120, height - 80) - visibleWeight) / Math.max(1, lines.length - 1),
        );
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

type StrandGradientIds = {
  input: string;
  inputHover: string;
  output: string;
  outputHover: string;
  fee: string;
  feeHover: string;
};

type StrandMarkerIds = {
  input: string;
  inputHover: string;
  output: string;
  outputHover: string;
};

function gradientUrl(id: string) {
  return `url(#${id})`;
}

function strandStroke(node: GraphRow, gradientIds: StrandGradientIds, active = false) {
  if (node.role === "fee") {
    return gradientUrl(active ? gradientIds.feeHover : gradientIds.fee);
  }
  if (node.side === "input") {
    return gradientUrl(active ? gradientIds.inputHover : gradientIds.input);
  }
  return gradientUrl(active ? gradientIds.outputHover : gradientIds.output);
}

function visibleStrandStrokeWidth(node: DrawableGraphRow) {
  return node.thickness + 1;
}

const STRAND_MARKER_WIDTH = 1.5;
const STRAND_MARKER_LEAD_RATIO = 0.5;

function hasStrandTip(node: DrawableGraphRow) {
  return node.side !== "fee" && !node.zeroValue;
}

function strandMarkerLead(node: DrawableGraphRow) {
  if (!hasStrandTip(node)) return 0;
  return visibleStrandStrokeWidth(node) * STRAND_MARKER_LEAD_RATIO;
}

function makeBowtiePath(
  node: DrawableGraphRow,
  side: "input" | "output",
  canvasWidth: number,
  edgePadding: number,
  centerX: number,
) {
  const start = edgePadding + strandMarkerLead(node);
  const end = centerX + 1;
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

function makeZeroValuePath(
  node: DrawableGraphRow,
  side: "input" | "output",
  canvasWidth: number,
  edgePadding: number,
  centerX: number,
) {
  const halfWidth = Math.max(1.5, visibleStrandStrokeWidth(node) / 2);
  const start = edgePadding + halfWidth;
  const length = Math.min(60, Math.max(20, centerX - edgePadding - 110));
  const y = node.outerY;
  if (side === "input") {
    return `M ${start} ${y} L ${start + length} ${y}`;
  }
  return `M ${canvasWidth - start} ${y} L ${canvasWidth - start - length} ${y}`;
}

function copyReference(node: GraphRow) {
  return node.outpoint || node.address || node.txid || null;
}

function copyAriaLabel(side: GraphRow["side"], t: TFunction<"transactions">) {
  if (side === "input") return t("graph.copyInput");
  if (side === "fee") return t("graph.copyFee");
  return t("graph.copyOutput");
}

function strandMarkerId(
  node: DrawableGraphRow,
  markerIds: StrandMarkerIds,
  active = false,
) {
  if (!hasStrandTip(node)) return undefined;
  if (node.side === "input") {
    return active ? markerIds.inputHover : markerIds.input;
  }
  return active ? markerIds.outputHover : markerIds.output;
}

function GraphStrand({
  node,
  path,
  markerPath,
  testId,
  active,
  gradientIds,
  markerIds,
  hideSensitive,
  onHover,
  onLeave,
}: {
  node: DrawableGraphRow;
  path: string;
  markerPath: string;
  testId?: string;
  active?: boolean;
  gradientIds: StrandGradientIds;
  markerIds: StrandMarkerIds;
  hideSensitive: boolean;
  onHover: (node: DrawableGraphRow) => void;
  onLeave: () => void;
}) {
  const { t } = useTranslation("transactions");
  // In hidden-sensitive mode the diagram is screenshot-safe; never let a stray
  // click or keypress copy the real outpoint/address/txid to the clipboard.
  const reference = hideSensitive ? null : copyReference(node);
  const ariaLabel = reference
    ? copyAriaLabel(node.side, t)
    : t("graph.legAria", { role: roleLabel(node.role, t) });
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
  const markerId = strandMarkerId(node, markerIds, active);

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
        onPointerLeave={onLeave}
        onPointerCancel={onLeave}
        onFocus={() => onHover(node)}
        onBlur={onLeave}
      />
      <path
        d={path}
        data-testid={testId}
        aria-hidden="true"
        className="pointer-events-none fill-none opacity-100 transition-opacity"
        stroke={strandStroke(node, gradientIds, active)}
        strokeWidth={visibleStrandStrokeWidth(node)}
        strokeLinecap={node.zeroValue ? "round" : "butt"}
        markerStart={markerId ? `url(#${markerId})` : undefined}
      />
    </>
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
  hiddenLabel = "Hidden",
  showAsset = true,
) {
  if (hidden) return hiddenLabel;
  if (typeof amountBtc !== "number") return "";
  const assetText = showAsset && asset ? ` ${asset}` : "";
  return `${formatBtc(amountBtc)}${assetText}`;
}

function formatRouteFeePercent(
  feeBtc: number | null | undefined,
  baseBtc: number | null | undefined,
  hidden: boolean,
  hiddenLabel = "Hidden",
) {
  if (hidden) return hiddenLabel;
  if (
    typeof feeBtc !== "number" ||
    typeof baseBtc !== "number" ||
    !Number.isFinite(feeBtc) ||
    !Number.isFinite(baseBtc) ||
    baseBtc === 0
  ) {
    return "";
  }
  const percent = (Math.abs(feeBtc) / Math.abs(baseBtc)) * 100;
  const precision = percent < 0.1 ? 3 : percent < 1 ? 2 : 1;
  return `${percent.toFixed(precision)}%`;
}

function routeLegAssetLabel(leg: TransactionSwapRouteLeg): ConnectionAssetLabel | null {
  const text = [
    leg.asset,
    leg.network,
    leg.wallet?.kind,
    leg.wallet?.label,
  ]
    .filter(Boolean)
    .join(" ")
    .toLowerCase();
  if (!text) return null;
  if (text.includes("liquid") || text.includes("lbtc") || text.includes("l-btc")) {
    return "LBTC";
  }
  if (text.includes("lightning") || text.includes("ln-btc")) {
    return "LN-BTC";
  }
  if (text.includes("btc") || text.includes("bitcoin") || text.includes("descriptor")) {
    return "BTC";
  }
  return null;
}

function RouteLegAssetIcon({ leg }: { leg: TransactionSwapRouteLeg }) {
  const asset = routeLegAssetLabel(leg);
  if (!asset) return null;
  const iconKind = connectionAssetIconKind(asset);
  const icon = iconKind === "liquid" ? liquidIcon : bitcoinIcon;
  return (
    <span
      className={cn(
        "inline-flex size-4 shrink-0 items-center justify-center rounded-sm border border-border/60 bg-muted/40",
        iconKind === "bitcoin" && "p-0.5",
      )}
      data-testid="swap-route-leg-asset-icon"
      data-asset={asset}
      aria-label={asset}
      title={asset}
    >
      <img src={icon} alt="" className="max-h-full max-w-full object-contain" />
    </span>
  );
}

function legGraphReference(leg: TransactionSwapRouteLeg) {
  return leg.id || leg.txid || leg.externalId || "";
}

function isSyncProvenanceLabel(value: string) {
  return /^synced from\b/i.test(value.trim());
}

function routeCounterparty(
  route: TransactionSwapRoute,
  kind: ReturnType<typeof pairedRouteKind>,
) {
  const candidates = [
    route.out.counterparty,
    route.in.counterparty,
    route.out.description,
    route.in.description,
    route.kind,
  ];
  for (const candidate of candidates) {
    const value = candidate?.trim();
    if (!value) continue;
    if (["swap", "coinjoin", "transfer", "pair"].includes(value.toLowerCase())) continue;
    if (kind === "swap" && isSyncProvenanceLabel(value)) continue;
    return value;
  }
  return "";
}

function pairedRouteKind(route: TransactionSwapRoute): "swap" | "coinjoin" | "transfer" | "pair" {
  // Prefer the daemon-computed routeKind; the heuristic below only covers
  // payloads that predate it (older snapshots, mocks).
  const server = String(route.routeKind || "").toLowerCase();
  if (server === "swap" || server === "coinjoin" || server === "transfer" || server === "pair") {
    return server;
  }
  const explicit = String(route.kind || "").toLowerCase();
  if (explicit.includes("coinjoin") || explicit.includes("whirlpool")) return "coinjoin";
  if (
    explicit.includes("swap") ||
    explicit.startsWith("peg-") ||
    route.out.asset?.toUpperCase() !== route.in.asset?.toUpperCase()
  ) {
    return "swap";
  }
  if (route.policy === "carrying-value") return "transfer";
  return "pair";
}

function swapRouteOutLooksLikeConsolidation(route: TransactionSwapRoute) {
  if (pairedRouteKind(route) !== "swap") return false;
  // Trust the per-leg role the daemon (or the client fallback route) assigns;
  // only re-derive it for payloads that predate per-leg roles.
  if (route.out.role === "consolidation") return true;
  if (route.out.role === "spend") return false;
  const text = `${route.out.kind || ""} ${route.out.description || ""}`.toLowerCase();
  if (text.includes("consolidat")) return true;
  const outNetwork = String(route.out.network || route.out.asset || "").toLowerCase();
  const inNetwork = String(route.in.network || route.in.asset || "").toLowerCase();
  return outNetwork.includes("liquid") && outNetwork !== inNetwork;
}

function SwapRouteLeg({
  label,
  leg,
  active,
  onSelect,
  selectedLabel,
  unknownLabel,
  hideSensitive,
}: {
  label: string;
  leg: TransactionSwapRouteLeg;
  active: boolean;
  onSelect?: () => void;
  selectedLabel: string;
  unknownLabel: string;
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("transactions");
  const hiddenLabel = t("graph.hidden");
  const amount = formatRouteAmount(
    leg.amountBtc,
    leg.asset,
    hideSensitive,
    hiddenLabel,
    false,
  );
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onSelect}
      className={cn(
        "min-w-0 rounded-md border bg-background px-3 py-2 text-left transition-colors",
        onSelect && "cursor-pointer hover:border-primary/40 hover:bg-primary/5",
        active && "border-primary/60 bg-primary/10",
      )}
    >
      <div className="flex min-w-0 items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="text-[10px] font-medium uppercase text-muted-foreground">
            {label}
          </div>
          <div className="mt-1 flex min-w-0 items-center gap-1.5 text-sm font-medium">
            <span>{leg.network || leg.asset || unknownLabel}</span>
            <RouteLegAssetIcon leg={leg} />
          </div>
        </div>
        {active ? (
          <Badge variant="outline" className="shrink-0 rounded-md px-1.5 py-0 text-[10px]">
            {selectedLabel}
          </Badge>
        ) : null}
      </div>
      {amount ? (
        <div className={cn("mt-1 text-sm font-medium tabular-nums", hideSensitive && "sensitive")}>
          {amount}
        </div>
      ) : null}
    </button>
  );
}

function SwapRouteStrip({
  route,
  hideSensitive,
  selectedLeg,
  onSelectLeg,
}: {
  route?: TransactionSwapRoute | null;
  hideSensitive: boolean;
  selectedLeg?: TransactionSwapRouteLegKey | null;
  onSelectLeg?: (leg: TransactionSwapRouteLegKey) => void;
}) {
  const { t } = useTranslation("transactions");
  const [feeMode, setFeeMode] = useState<"relative" | "absolute">("relative");
  if (!route) return null;
  const activeLeg = selectedLeg ?? route.currentLeg ?? "out";
  const canSelectOut = Boolean(onSelectLeg && legGraphReference(route.out));
  const canSelectIn = Boolean(onSelectLeg && legGraphReference(route.in));
  const hiddenLabel = t("graph.hidden");
  const kind = pairedRouteKind(route);
  const counterparty = sensitiveGraphText(routeCounterparty(route, kind), hideSensitive, hiddenLabel);
  const feeBaseBtc =
    route.outFullAmountBtc ?? route.outAmountBtc ?? route.out.amountBtc ?? route.in.amountBtc;
  const feeAbsolute = formatRouteAmount(
    route.swapFeeBtc,
    route.out.asset,
    hideSensitive,
    hiddenLabel,
    false,
  );
  const feeRelative = formatRouteFeePercent(
    route.swapFeeBtc,
    feeBaseBtc,
    hideSensitive,
    hiddenLabel,
  );
  const fee = feeMode === "absolute" ? feeAbsolute : feeRelative || feeAbsolute;
  const hasFeeToggle = Boolean(feeAbsolute && feeRelative);
  const title =
    kind === "coinjoin"
      ? t("graph.coinjoinRouteTitle")
      : kind === "transfer"
        ? t("graph.transferRouteTitle")
        : kind === "pair"
          ? t("graph.pairRouteTitle")
          : t("graph.swapRouteTitle");
  const pairFallback =
    kind === "coinjoin"
      ? t("graph.coinjoinRoutePair")
      : kind === "transfer"
        ? t("graph.transferRoutePair")
        : kind === "pair"
          ? t("graph.pairRoutePair")
          : t("graph.swapRoutePair");
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
            <div className="text-sm font-medium">{title}</div>
          </div>
        </div>
      </div>
      <div className="mt-3 grid items-stretch gap-2 md:grid-cols-[minmax(0,1fr)_minmax(190px,0.9fr)_minmax(0,1fr)]">
        <SwapRouteLeg
          label={outLabel}
          leg={route.out}
          active={activeLeg === "out"}
          onSelect={canSelectOut ? () => onSelectLeg?.("out") : undefined}
          selectedLabel={t("graph.swapRouteSelected")}
          unknownLabel={t("graph.swapRouteUnknownLeg")}
          hideSensitive={hideSensitive}
        />
        <div className="flex min-w-0 items-center justify-center gap-2 px-3 py-2 text-center">
          <ArrowRight className="hidden size-4 shrink-0 text-muted-foreground md:block" aria-hidden="true" />
          <div className="min-w-0">
            <div className={cn("text-sm font-medium leading-snug", hideSensitive && "sensitive")}>
              {counterparty || pairFallback}
            </div>
            {route.policy ? (
              <div className="mt-1 truncate text-[11px] text-muted-foreground">
                {route.policy}
              </div>
            ) : null}
            {fee ? (
              <button
                type="button"
                className={cn(
                  "mt-2 inline-flex max-w-full items-center justify-center rounded border border-border/70 bg-muted/35 px-1.5 py-0.5 text-[10px] font-medium tabular-nums text-muted-foreground transition-colors hover:border-primary/40 hover:bg-primary/10 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                  hideSensitive && "sensitive",
                )}
                aria-pressed={feeMode === "absolute"}
                title={feeMode === "absolute" ? feeRelative : feeAbsolute}
                onClick={() =>
                  hasFeeToggle &&
                  setFeeMode((current) =>
                    current === "relative" ? "absolute" : "relative",
                  )
                }
              >
                {t("graph.swapRouteFee", { fee })}
              </button>
            ) : null}
          </div>
          <ArrowRight className="hidden size-4 shrink-0 text-muted-foreground md:block" aria-hidden="true" />
        </div>
        <SwapRouteLeg
          label={inLabel}
          leg={route.in}
          active={activeLeg === "in"}
          onSelect={canSelectIn ? () => onSelectLeg?.("in") : undefined}
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
  const { t } = useTranslation("transactions");
  const graphInstanceId = useId().replace(/[^a-zA-Z0-9_-]/g, "");
  const gradientIds: StrandGradientIds = {
    input: `transaction-flow-${graphInstanceId}-input-gradient`,
    inputHover: `transaction-flow-${graphInstanceId}-input-hover-gradient`,
    output: `transaction-flow-${graphInstanceId}-output-gradient`,
    outputHover: `transaction-flow-${graphInstanceId}-output-hover-gradient`,
    fee: `transaction-flow-${graphInstanceId}-fee-gradient`,
    feeHover: `transaction-flow-${graphInstanceId}-fee-hover-gradient`,
  };
  const markerIds: StrandMarkerIds = {
    input: `transaction-flow-${graphInstanceId}-input-marker`,
    inputHover: `transaction-flow-${graphInstanceId}-input-hover-marker`,
    output: `transaction-flow-${graphInstanceId}-output-marker`,
    outputHover: `transaction-flow-${graphInstanceId}-output-hover-marker`,
  };
  const preferredCanvasWidth = expanded ? 1120 : 960;
  const shellRef = useRef<HTMLDivElement | null>(null);
  const [hoverDetail, setHoverDetail] = useState<GraphHoverDetail | null>(null);
  const [measuredCanvasWidth, setMeasuredCanvasWidth] = useState<number | null>(null);
  const inputRows = compactGraphRows(
    graph.inputs,
    "input",
    expanded ? MAX_EXPANDED_ROWS : MAX_COMPACT_ROWS,
  );
  const outputRowsBase = compactGraphRows(
    graph.outputs,
    "output",
    expanded ? MAX_EXPANDED_ROWS : MAX_COMPACT_ROWS,
  );
  const outputRows: GraphRow[] = outputRowsBase;
  const feeRow: GraphRow | null = graph.fee ? { ...graph.fee, side: "fee" } : null;
  const destinationRows = feeRow ? [feeRow, ...outputRows] : outputRows;
  const layoutInputRows = hideSensitive ? redactRowsForGeometry(inputRows) : inputRows;
  const layoutDestinationRows = hideSensitive
    ? redactRowsForGeometry(destinationRows)
    : destinationRows;
  const rowCount = Math.max(inputRows.length, destinationRows.length, 2);
  const height = Math.max(280, rowCount * GRAPH_ROW_HEIGHT + 90);
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
  const centerX = canvasWidth / 2;
  const edgePadding = expanded ? 84 : 64;
  const curveWidth = centerX - edgePadding - 12;
  const visualTotal = visualTotalSatsForSides(layoutInputRows, layoutDestinationRows);
  const fallbackSats = fallbackVisualSats(visualTotal, rowCount);
  const combinedWeight = Math.min(expanded ? 96 : 82, Math.max(26, Math.floor((canvasWidth - 2 * edgePadding) / 9)));
  const inputDrawRows = buildDrawableRows(
    layoutInputRows,
    visualTotal,
    height,
    combinedWeight,
    curveWidth,
    fallbackSats,
  );
  const outputDrawRows = buildDrawableRows(
    layoutDestinationRows,
    visualTotal,
    height,
    combinedWeight,
    curveWidth,
    fallbackSats,
  );
  const pathFor = (node: DrawableGraphRow) =>
    node.zeroValue
      ? makeZeroValuePath(
          node,
          node.side === "input" ? "input" : "output",
          canvasWidth,
          edgePadding,
          centerX,
        )
      : makeBowtiePath(node, node.side === "input" ? "input" : "output", canvasWidth, edgePadding, centerX);
  const markerPathFor = (node: DrawableGraphRow) =>
    node.zeroValue
      ? makeZeroValuePath(
          node,
          node.side === "input" ? "input" : "output",
          canvasWidth,
          edgePadding,
          centerX,
        )
      : makeBowtiePath(
          node,
          node.side === "input" ? "input" : "output",
          canvasWidth,
          edgePadding,
          centerX,
        );
  const showHoverDetail = (node: DrawableGraphRow) => {
    setHoverDetail({ node });
  };
  return (
    <div
      ref={shellRef}
      className="relative min-w-0 overflow-hidden bg-transparent"
      style={{ height: viewportHeight }}
      data-testid="transaction-flow-diagram"
    >
      <span className={cn("sr-only", hideSensitive && "sensitive")}>
        {hideSensitive
          ? t("graph.hiddenReferences")
          : t("graph.availableReferences")}
      </span>
      <div className="h-full min-w-0 overflow-y-auto overflow-x-hidden">
        <div
          className="relative min-w-0"
          data-testid="transaction-flow-canvas"
          data-transaction-flow-canvas
          style={{ width: "100%", height }}
        >
          <svg
            className="absolute inset-0 h-full w-full"
            width={canvasWidth}
            height={height}
            viewBox={`0 0 ${canvasWidth} ${height}`}
            role="img"
            aria-label={t("graph.diagramAria")}
          >
            <defs>
              <marker
                id={markerIds.input}
                viewBox="-5 -5 10 10"
                refX="0"
                refY="0"
                markerUnits="strokeWidth"
                markerWidth={STRAND_MARKER_WIDTH}
                markerHeight="1"
                orient="auto"
              >
                <path
                  d="M -5 -5 L 0 0 L -5 5 L 1 5 L 1 -5 Z"
                  strokeWidth="0"
                  fill="rgb(59 130 246)"
                />
              </marker>
              <marker
                id={markerIds.inputHover}
                viewBox="-5 -5 10 10"
                refX="0"
                refY="0"
                markerUnits="strokeWidth"
                markerWidth={STRAND_MARKER_WIDTH}
                markerHeight="1"
                orient="auto"
              >
                <path
                  d="M -5 -5 L 0 0 L -5 5 L 1 5 L 1 -5 Z"
                  strokeWidth="0"
                  fill="rgb(96 165 250)"
                />
              </marker>
              <marker
                id={markerIds.output}
                viewBox="-5 -5 10 10"
                refX="0"
                refY="0"
                markerUnits="strokeWidth"
                markerWidth={STRAND_MARKER_WIDTH}
                markerHeight="1"
                orient="auto"
              >
                <path
                  d="M 1 -5 L 0 -5 L -5 0 L 0 5 L 1 5 Z"
                  strokeWidth="0"
                  fill="rgb(59 130 246)"
                />
              </marker>
              <marker
                id={markerIds.outputHover}
                viewBox="-5 -5 10 10"
                refX="0"
                refY="0"
                markerUnits="strokeWidth"
                markerWidth={STRAND_MARKER_WIDTH}
                markerHeight="1"
                orient="auto"
              >
                <path
                  d="M 1 -5 L 0 -5 L -5 0 L 0 5 L 1 5 Z"
                  strokeWidth="0"
                  fill="rgb(96 165 250)"
                />
              </marker>
              <linearGradient id={gradientIds.input} x1="0%" y1="0%" x2="100%" y2="0%">
                <stop offset="0%" stopColor="rgb(59 130 246)" />
                <stop offset="100%" stopColor="rgb(14 165 233)" />
              </linearGradient>
              <linearGradient id={gradientIds.inputHover} x1="0%" y1="0%" x2="100%" y2="0%">
                <stop offset="0%" stopColor="rgb(96 165 250)" />
                <stop offset="100%" stopColor="rgb(34 211 238)" />
              </linearGradient>
              <linearGradient id={gradientIds.output} x1="0%" y1="0%" x2="100%" y2="0%">
                <stop offset="0%" stopColor="rgb(14 165 233)" />
                <stop offset="100%" stopColor="rgb(59 130 246)" />
              </linearGradient>
              <linearGradient id={gradientIds.outputHover} x1="0%" y1="0%" x2="100%" y2="0%">
                <stop offset="0%" stopColor="rgb(34 211 238)" />
                <stop offset="100%" stopColor="rgb(96 165 250)" />
              </linearGradient>
              <linearGradient id={gradientIds.fee} x1="0%" y1="0%" x2="100%" y2="0%">
                <stop offset="0%" stopColor="rgb(14 165 233)" />
                <stop offset="52%" stopColor="rgb(245 158 11)" />
                <stop offset="100%" stopColor="transparent" />
              </linearGradient>
              <linearGradient id={gradientIds.feeHover} x1="0%" y1="0%" x2="100%" y2="0%">
                <stop offset="0%" stopColor="rgb(34 211 238)" />
                <stop offset="58%" stopColor="rgb(251 191 36)" />
                <stop offset="100%" stopColor="transparent" />
              </linearGradient>
            </defs>
          {inputDrawRows.map((node) => (
            <GraphStrand
              key={`curve-${node.id}`}
              node={node}
              path={pathFor(node)}
              markerPath={markerPathFor(node)}
              testId="transaction-input-strand"
              active={hoverDetail?.node.id === node.id && hoverDetail.node.side === node.side}
              gradientIds={gradientIds}
              markerIds={markerIds}
              hideSensitive={hideSensitive}
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
              gradientIds={gradientIds}
              markerIds={markerIds}
              hideSensitive={hideSensitive}
              onHover={showHoverDetail}
              onLeave={() => setHoverDetail(null)}
            />
          ))}
        </svg>
        </div>
      </div>
      {hoverDetail ? (
        <div
          data-testid="transaction-graph-hover-detail"
          className="pointer-events-none absolute bottom-2 left-3 max-w-[min(520px,calc(100%-1.5rem))] rounded-md border border-white/10 bg-[#101114]/95 px-3 py-2 text-xs text-white shadow-lg"
        >
          <div className="grid min-w-0 gap-1">
            <div className="truncate font-medium">
              {sensitiveGraphText(
                nodeDisplayTitle(hoverDetail.node, t),
                hideSensitive,
                t("graph.hidden"),
              )}
            </div>
            <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-white/70">
              {formatNodeAmount(hoverDetail.node, hideSensitive, t) ? (
                <span>{formatNodeAmount(hoverDetail.node, hideSensitive, t)}</span>
              ) : null}
              <span>{roleLabel(hoverDetail.node.role, t)}</span>
              <span>{ownershipBoundaryLabel(hoverDetail.node, t)}</span>
              {copyReference(hoverDetail.node) ? (
                <span className={cn("truncate font-mono", hideSensitive && "sensitive")}>
                  {hideSensitive
                    ? t("graph.hidden")
                    : formatShortTxid(copyReference(hoverDetail.node) ?? "")}
                </span>
              ) : null}
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function GraphEmptyState({
  graph,
  loading,
  error,
  onResolveIssue,
}: {
  graph?: TransactionGraphPayload;
  loading?: boolean;
  error?: string | null;
  onResolveIssue?: (target: TransactionGraphIssueTarget) => void;
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
  const alertWarnings = (graph?.warnings ?? []).filter(
    (warning) => warning.level === "warning" || warning.level === "error",
  );
  const diagnosticAction = graphDiagnosticAction(graph);
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
            {diagnosticAction && onResolveIssue ? (
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="mt-3 gap-2"
                onClick={() => onResolveIssue(diagnosticAction.target)}
              >
                <ArrowRight className="size-4" aria-hidden="true" />
                {t(diagnosticAction.labelKey)}
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

function graphDiagnosticAction(
  graph?: TransactionGraphPayload,
): TransactionGraphIssueAction | null {
  if (!graph) return null;
  const codes = new Set((graph.warnings ?? []).map((warning) => warning.code));
  const hasLiquidLookupIssue =
    graph.unsupportedReason === "liquid_reference_graph_not_local" ||
    [...codes].some((code) => code.startsWith("liquid_reference_lookup_"));
  if (hasLiquidLookupIssue) {
    return {
      target: "liquid",
      labelKey: codes.has("liquid_reference_lookup_unavailable")
        ? "graph.addLiquidBackend"
        : "graph.reviewLiquidBackend",
    };
  }
  const hasBitcoinLookupIssue = [...codes].some((code) =>
    code.startsWith("bitcoin_reference_lookup_"),
  );
  if (hasBitcoinLookupIssue) {
    return {
      target: "bitcoin",
      labelKey: codes.has("bitcoin_reference_lookup_unavailable")
        ? "graph.addBitcoinBackend"
        : "graph.reviewBitcoinBackend",
    };
  }
  return null;
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
  selectedSwapLeg,
  onSelectSwapLeg,
  onResolveIssue,
}: {
  graph?: TransactionGraphPayload;
  loading?: boolean;
  error?: string | null;
  hideSensitive: boolean;
  selectedSwapLeg?: TransactionSwapRouteLegKey | null;
  onSelectSwapLeg?: (leg: TransactionSwapRouteLegKey) => void;
  onResolveIssue?: (target: TransactionGraphIssueTarget) => void;
}) {
  const { t } = useTranslation("transactions");
  const showDiagram =
    graph &&
    (graph.supportLevel === "full" || graph.supportLevel === "partial") &&
    (graph.inputs.length > 0 || graph.outputs.length > 0);
  const alertWarnings = (graph?.warnings ?? []).filter(
    (warning) => warning.level === "warning" || warning.level === "error",
  );
  const diagnosticAction = graphDiagnosticAction(graph);

  return (
    <div className="space-y-4">
      <SwapRouteStrip
        route={graph?.swapRoute}
        hideSensitive={hideSensitive}
        selectedLeg={selectedSwapLeg}
        onSelectLeg={onSelectSwapLeg}
      />
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
                <Button
                  type="button"
                  variant="outline"
                  size="icon"
                  className="size-8 shrink-0"
                  aria-label={t("graph.expand")}
                  title={t("graph.expand")}
                >
                  <Maximize2 className="size-4" aria-hidden="true" />
                </Button>
              </DialogTrigger>
              <DialogContent className="w-[min(1180px,calc(100vw-2rem))] max-w-none sm:max-w-none">
                <DialogTitle className="sr-only">{t("graph.expandedTitle")}</DialogTitle>
                <TransactionFlowDiagram graph={graph} hideSensitive={hideSensitive} expanded />
              </DialogContent>
            </Dialog>
          </div>
          <AnnotationStrip annotations={graph.annotations} hideSensitive={hideSensitive} />
          <TransactionFlowDiagram graph={graph} hideSensitive={hideSensitive} />
          <TransactionInputsOutputsPanel graph={graph} hideSensitive={hideSensitive} />
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
              {diagnosticAction && onResolveIssue ? (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="gap-2"
                  onClick={() => onResolveIssue(diagnosticAction.target)}
                >
                  <ArrowRight className="size-4" aria-hidden="true" />
                  {t(diagnosticAction.labelKey)}
                </Button>
              ) : null}
            </div>
          ) : null}
        </>
      ) : (
        <GraphEmptyState
          graph={graph}
          loading={loading}
          error={error}
          onResolveIssue={onResolveIssue}
        />
      )}
    </div>
  );
}
