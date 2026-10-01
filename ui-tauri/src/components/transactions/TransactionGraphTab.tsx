import {
  AlertTriangle,
  ArrowRight,
  ArrowRightLeft,
  ChevronDown,
  ChevronUp,
  Info,
  Maximize2,
} from "lucide-react";
import { exchangeTransfer } from "./ExchangeTransferModel";
import { ExchangeTransferSummary } from "./ExchangeTransferSummary";
import type { TFunction } from "i18next";
import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
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
import { formatCount } from "@/lib/localeFormat";
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
  classifyRouteKind,
  classifyRouteOutRole,
  looksLightning,
  looksLiquid,
  nodeTooltipTitle,
  sensitiveGraphText,
  MAX_COMPACT_ROWS,
  type GraphRow,
  type TransactionRouteKind,
  type TransactionGraphAnnotation,
  type TransactionGraphIssueTarget,
  type TransactionGraphNode,
  type TransactionGraphPayload,
  type TransactionSwapRoute,
  type TransactionSwapRouteLeg,
  type TransactionSwapRouteLegKey,
} from "./TransactionGraphModel";
import { TransactionGraph3D } from "./graph3d/TransactionGraph3D";
import {
  BOWTIE_LINE_LIMIT,
  bowtieLines,
  bowtieTotal,
  graphIsLiquid,
  graphLayoutRows,
} from "./TransactionGraphGeometry";

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

// The expanded dialog shows far more strands, but still caps so a fan-out
// transaction cannot render thousands of paths. Matches the backend node cap.
const MAX_EXPANDED_ROWS = 250;
const MAX_DETAIL_COLLAPSED_ROWS = 8;

function formatNodeAmount(node: TransactionGraphNode, hidden: boolean, t: TFunction<"transactions">) {
  if (hidden) return t("graph.hidden");
  if (node.valueState === "confidential") return t("graph.confidentialAmount");
  if (node.valueState === "other_asset") {
    // No asset registry means no precision, so naming the asset is as far as we
    // can honestly go — a raw integer could be wrong by orders of magnitude.
    return node.asset
      ? t("graph.otherAssetNamed", { asset: node.asset })
      : t("graph.otherAsset");
  }
  // The daemon always ships valueSats and valueBtc together, or neither.
  if (typeof node.valueBtc === "number") return formatBtc(node.valueBtc);
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
    incoming_payment_candidate: t("graph.roles.incomingPaymentCandidate"),
    owned_return: t("graph.roles.ownedReturn"),
    owned_destination: t("graph.roles.ownedDestination"),
    op_return: t("graph.roles.opReturn"),
    coinbase: t("graph.roles.coinbase"),
    peg_in: t("graph.roles.pegIn"),
    peg_out: t("graph.roles.pegOut"),
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
  if (node.ownership === "peg_out") return t("graph.ownership.pegOut");
  if (node.ownership === "coinbase") return t("graph.ownership.coinbase");
  if (node.ownership === "overflow") return t("graph.ownership.aggregated");
  return t("graph.ownership.unknown");
}

// Two deliberately different orders: a detail row identifies a leg by address,
// while the strand's copy target must be the outpoint its aria-label promises —
// and must never fall back to a synthetic label like "+12 more".
function displayReference(node: TransactionGraphNode) {
  return node.address || node.outpoint || node.txid || node.label || "";
}

function copyReference(node: TransactionGraphNode) {
  return node.outpoint || node.address || node.txid || null;
}

/** One name per leg across the drawing and the list; the fee counts as an output. */
function graphPart(side: GraphRow["side"], id: string) {
  return `${side === "input" ? "input" : "output"}:${id}`;
}

function nodeDetailReference(
  node: TransactionGraphNode,
  hidden: boolean,
  t: TFunction<"transactions">,
) {
  const reference = displayReference(node);
  if (!reference) return t("graph.inputsOutputs.unknownReference");
  return sensitiveGraphText(formatShortTxid(reference), hidden, t("graph.hidden"));
}

function conciseScriptType(scriptType: string | undefined) {
  if (!scriptType) return "";
  const normalized = scriptType.replace(/[_-]/g, " ").replace(/\s+/g, " ").trim();
  const lower = normalized.toLowerCase();
  if (lower === "unknown") return "";
  if (lower.includes("taproot")) return "taproot";
  if (lower.includes("witness v0") && lower.includes("keyhash")) return "segwit v0";
  if (lower.includes("witness v0") && lower.includes("scripthash")) return "segwit script";
  return normalized;
}

/**
 * "3 blocks earlier" / "12 blocks later" for a leg whose counterpart transaction
 * has a locally known height. Absent heights render nothing rather than a guess.
 */
function blockDistanceLabel(
  node: TransactionGraphNode,
  side: "input" | "output",
  blockHeight: number | null | undefined,
  t: TFunction<"transactions">,
) {
  if (typeof blockHeight !== "number") return null;
  const other = side === "input" ? node.prevoutBlockHeight : node.spentByBlockHeight;
  if (typeof other !== "number") return null;
  const delta = side === "input" ? blockHeight - other : other - blockHeight;
  if (delta < 0) return null;
  if (delta === 0) return t("graph.inputsOutputs.sameBlock");
  return side === "input"
    ? t("graph.inputsOutputs.blocksEarlier", { count: delta })
    : t("graph.inputsOutputs.blocksLater", { count: delta });
}

function nodeDetailMeta(
  node: TransactionGraphNode,
  side: "input" | "output",
  hidden: boolean,
  t: TFunction<"transactions">,
  blockHeight?: number | null,
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
  if (node.spentByTxid) {
    parts.push(
      hidden
        ? t("graph.inputsOutputs.spent")
        : t("graph.inputsOutputs.spentBy", {
            reference: formatShortTxid(node.spentByTxid),
          }),
    );
  }
  const distance = blockDistanceLabel(node, side, blockHeight, t);
  if (distance) parts.push(distance);
  return parts;
}

function graphExplorerNetwork(graph: TransactionGraphPayload): ExplorerNetwork {
  const chain = graph.transaction?.chain?.trim().toLowerCase();
  if (chain === "liquid" || chain === "bitcoin") return chain;
  // Pre-`chain` payloads only: fall back to the old label sniffing.
  return looksLiquid(graph.transaction?.network, graph.transaction?.asset)
    ? "liquid"
    : "bitcoin";
}

// Liquid mainnet pegs out to Bitcoin mainnet; the Liquid testnets peg out to
// Bitcoin testnet.
function bitcoinNetworkForPeg(graph: TransactionGraphPayload) {
  const network = graph.transaction?.network?.trim().toLowerCase();
  if (!network || network === "liquidv1") return "main";
  return network === "elementsregtest" ? "regtest" : "test";
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
  // A peg-out's destination is a Bitcoin address on a Liquid transaction, so it
  // has to resolve against the Bitcoin explorer, not the graph's own chain.
  const network: ExplorerNetwork =
    node.role === "peg_out" ? "bitcoin" : graphExplorerNetwork(graph);
  const networkName =
    node.role === "peg_out" ? bitcoinNetworkForPeg(graph) : graph.transaction?.network;
  if (node.address) {
    return explorerTargetForAddress({
      address: node.address,
      network,
      networkName,
      settings,
    });
  }
  const txid = node.txid || node.outpoint?.split(":")[0];
  return explorerTargetForTransaction({
    txid,
    network,
    networkName,
    settings,
  });
}

function amountSummary(nodes: TransactionGraphNode[]) {
  return nodes.reduce(
    (summary, node) => {
      if (node.valueState === "confidential") {
        summary.confidentialCount += 1;
      } else if (node.valueState === "other_asset") {
        summary.otherAssetCount += 1;
      } else if (typeof node.valueSats === "number") {
        summary.knownCount += 1;
        summary.knownSats += node.valueSats;
      }
      return summary;
    },
    {
      knownSats: 0,
      knownCount: 0,
      confidentialCount: 0,
      otherAssetCount: 0,
    },
  );
}

function hasCompleteTotal(nodes: TransactionGraphNode[]) {
  if (!nodes.length) return true;
  return nodes.every(
    (node) =>
      node.valueState !== "confidential" &&
      node.valueState !== "other_asset" &&
      typeof node.valueSats === "number",
  );
}

function formatTotal(nodes: TransactionGraphNode[], t: TFunction<"transactions">) {
  const summary = amountSummary(nodes);
  if (summary.knownCount > 0) return formatBtc(summary.knownSats / 100_000_000);
  if (summary.confidentialCount > 0) {
    return t("graph.confidentialAmount");
  }
  // Nothing on this side is measured in bitcoin, which is not the same as hidden.
  if (summary.otherAssetCount > 0) return t("graph.otherAsset");
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
  onOpenTransaction,
  blockHeight,
  active = false,
  onHoverPart,
}: {
  node: TransactionGraphNode;
  side: "input" | "output";
  hideSensitive: boolean;
  explorerTarget: ExplorerTarget | null;
  onOpenExplorer: (target: ExplorerTarget) => void;
  onOpenTransaction?: (transactionId: string) => void;
  blockHeight?: number | null;
  /** Lit with its leg in the drawing above. */
  active?: boolean;
  onHoverPart?: (part: string | null) => void;
}) {
  const { t } = useTranslation("transactions");
  const part = graphPart(side, node.id);
  // The row and its leg in the drawing light up together, from either side.
  const linked = {
    "data-graph-part": part,
    "data-active": active || undefined,
    onPointerEnter: () => onHoverPart?.(part),
    onPointerLeave: () => onHoverPart?.(null),
    onFocus: () => onHoverPart?.(part),
    onBlur: () => onHoverPart?.(null),
  };
  const amount =
    formatNodeAmount(node, hideSensitive, t) ||
    t("graph.inputsOutputs.unknownAmount");
  const canOpenExplorer = Boolean(explorerTarget && !hideSensitive && !node.overflow);
  // Following the money inside the book, backwards through an input's own
  // funding row or forwards through an output's spend, as an explorer would.
  const bookTarget = !onOpenTransaction || hideSensitive
    ? null
    : side === "input"
      ? node.fundedByTransactionId ?? null
      : node.spentByTransactionId ?? null;
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
        <div className="mt-0.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-muted-foreground">
          {nodeDetailMeta(node, side, hideSensitive, t, blockHeight).map((part, index) => (
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
  if (bookTarget) {
    // Following the money inside the book beats leaving for an explorer.
    const openLabel =
      side === "input"
        ? t("graph.inputsOutputs.openFundingTransaction", {
            reference: formatShortTxid(node.txid ?? ""),
          })
        : t("graph.inputsOutputs.openSpendingTransaction", {
            reference: formatShortTxid(node.spentByTxid ?? ""),
          });
    return (
      <button
        type="button"
        className={ioRowClassName}
        aria-label={openLabel}
        title={openLabel}
        onClick={() => onOpenTransaction?.(bookTarget)}
        {...linked}
      >
        {content}
      </button>
    );
  }
  if (canOpenExplorer && explorerTarget) {
    const openLabel = t("graph.inputsOutputs.openExplorer", {
      explorer: explorerTarget.label,
      reference: nodeDetailReference(node, false, t),
    });
    return (
      <button
        type="button"
        className={ioRowClassName}
        aria-label={openLabel}
        title={openLabel}
        onClick={() => onOpenExplorer(explorerTarget)}
        {...linked}
      >
        {content}
      </button>
    );
  }
  return (
    <div
      className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] gap-2 border-t py-2 first:border-t-0 data-[active]:bg-muted/45"
      {...linked}
    >
      {content}
    </div>
  );
}

const ioRowClassName =
  "grid w-full min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] gap-2 border-t py-2 text-left first:border-t-0 hover:bg-muted/35 data-[active]:bg-muted/45 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

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
  onOpenTransaction,
  activePart,
  onHoverPart,
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
  onOpenTransaction?: (transactionId: string) => void;
  activePart?: string | null;
  onHoverPart?: (part: string | null) => void;
}) {
  const { t } = useTranslation("transactions");
  const visibleNodes = expanded ? nodes : nodes.slice(0, MAX_DETAIL_COLLAPSED_ROWS);
  const hiddenCount = Math.max(0, nodes.length - visibleNodes.length);
  return (
    <div className="min-w-0">
      <div className="flex items-center justify-between gap-2 border-b pb-1.5">
        <div className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
          {title}
        </div>
        <div className="text-2xs text-muted-foreground">
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
            onOpenTransaction={onOpenTransaction}
            blockHeight={graph.transaction?.blockHeight}
            active={activePart === graphPart(side, node.id)}
            onHoverPart={onHoverPart}
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
  onOpenTransaction,
  activePart = null,
  onHoverPart,
}: {
  graph: TransactionGraphPayload;
  hideSensitive: boolean;
  onOpenTransaction?: (transactionId: string) => void;
  activePart?: string | null;
  onHoverPart?: (part: string | null) => void;
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
          onOpenTransaction={onOpenTransaction}
          activePart={activePart}
          onHoverPart={onHoverPart}
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
          onOpenTransaction={onOpenTransaction}
          activePart={activePart}
          onHoverPart={onHoverPart}
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
  curveBase: number;
  estimatedVisualValue: boolean;
  zeroValue: boolean;
};

const ZERO_VALUE_STRAND_THICKNESS = 3;
const GRAPH_ROW_HEIGHT = 29;

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function buildDrawableRows(
  rows: GraphRow[],
  total: number,
  height: number,
  combinedWeight: number,
  curveWidth: number,
): DrawableGraphRow[] {
  // The outer ends keep this view's 40px top margin and scrolling height.
  const lines = bowtieLines(rows, total, {
    height,
    combinedWeight,
    curveWidth,
    outerTop: 40,
    outerSpan: Math.max(120, height - 80),
    zeroThickness: ZERO_VALUE_STRAND_THICKNESS,
  });
  return rows.map((node, index) => {
    const line = lines[index];
    return {
      ...node,
      outerY: line.outerY,
      innerY: line.innerY,
      thickness: line.thickness,
      weight: line.weight,
      offset: line.offset,
      curveBase: line.curveBase,
      estimatedVisualValue: line.estimated,
      zeroValue: line.zeroValue,
    };
  });
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
  // mempool's makePath: both curve ends move out by the offset, from a start the
  // side's widest strand and largest offset set.
  const curveStart = Math.min(
    Math.max(start + 5, edgePadding + node.curveBase - node.offset),
    end - 28,
  );
  const curveEnd = clamp(end - node.offset - 10, curveStart + 18, end - 4);
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
        d={path}
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

const MAX_ANNOTATION_BADGES = 8;

// The daemon's `message`/`label` strings are English by construction (they are
// built in Python, which stays machine-deterministic). Codes are stable, so the
// UI translates by code and only falls back to the English text for codes it has
// no key for — quarantine reasons and journal blockers, mostly.
function graphCodeText(
  t: TFunction<"transactions">,
  namespace: "warnings" | "annotations",
  code: string | undefined,
  fallback: string | undefined,
) {
  const key = `graph.${namespace}.${String(code || "").trim()}`;
  return t(key, { defaultValue: fallback || code || "" });
}

function AnnotationStrip({
  annotations,
}: {
  annotations?: TransactionGraphAnnotation[];
}) {
  const { t } = useTranslation("transactions");
  // Group ids used to be concatenated into the badge text, which put a raw
  // internal identifier on screen. Collapse repeats of the same annotation into
  // one badge with a count instead. This does drop the ability to tell two
  // custody decisions apart from the strip; nothing else in the UI renders
  // `groupId`, so if that correlation is wanted it needs a real surface rather
  // than a uuid in a badge.
  const badges = new Map<string, { label: string; severity?: string; count: number }>();
  for (const annotation of annotations ?? []) {
    const label = graphCodeText(t, "annotations", annotation.code, annotation.label);
    if (!label) continue;
    const existing = badges.get(label);
    if (existing) {
      existing.count += 1;
    } else if (badges.size < MAX_ANNOTATION_BADGES) {
      badges.set(label, { label, severity: annotation.severity, count: 1 });
    }
  }
  if (!badges.size) return null;
  return (
    <div className="flex flex-wrap gap-1.5">
      {[...badges.values()].map((badge) => (
        <Badge
          key={badge.label}
          variant={badge.severity === "warning" ? "destructive" : "secondary"}
          className="rounded-md"
        >
          {badge.count > 1 ? `${badge.label} · ${formatCount(badge.count)}` : badge.label}
        </Badge>
      ))}
    </div>
  );
}

function formatRouteAmount(
  amountBtc: number | null | undefined,
  asset: string | null | undefined,
  hidden: boolean,
  hiddenLabel: string,
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
  hiddenLabel: string,
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
  const parts = [leg.asset, leg.network, leg.wallet?.kind, leg.wallet?.label];
  const text = parts.filter(Boolean).join(" ").toLowerCase();
  if (!text) return null;
  if (looksLiquid(...parts)) return "LBTC";
  if (looksLightning(...parts)) return "LN-BTC";
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

function pairedRouteKind(route: TransactionSwapRoute): TransactionRouteKind {
  // Prefer the daemon-computed routeKind; the shared classifier only covers
  // payloads that predate it (older snapshots, mocks).
  const server = String(route.routeKind || "").toLowerCase();
  if (server === "swap" || server === "coinjoin" || server === "transfer" || server === "pair") {
    return server;
  }
  return classifyRouteKind({
    kind: route.kind,
    policy: route.policy,
    outAsset: route.out.asset,
    inAsset: route.in.asset,
  });
}

function swapRouteOutLooksLikeConsolidation(route: TransactionSwapRoute) {
  if (pairedRouteKind(route) !== "swap") return false;
  // Trust the per-leg role the daemon (or the client fallback route) assigns;
  // only re-derive it for payloads that predate per-leg roles.
  if (route.out.role === "consolidation") return true;
  if (route.out.role === "spend") return false;
  return (
    classifyRouteOutRole({
      kind: route.kind,
      description: `${route.out.kind || ""} ${route.out.description || ""}`,
      outNetwork: route.out.network || route.out.asset,
      inNetwork: route.in.network || route.in.asset,
    }) === "consolidation"
  );
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
          <div className="text-2xs font-medium uppercase text-muted-foreground">
            {label}
          </div>
          <div className="mt-1 flex min-w-0 items-center gap-1.5 text-sm font-medium">
            <span>{leg.network || leg.asset || unknownLabel}</span>
            <RouteLegAssetIcon leg={leg} />
          </div>
        </div>
        {active ? (
          <Badge variant="outline" className="shrink-0 rounded-md px-1.5 py-0 text-2xs">
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
              <div className="mt-1 truncate text-xs text-muted-foreground">
                {t(`graph.policy.${route.policy}`, { defaultValue: route.policy })}
              </div>
            ) : null}
            {fee ? (
              <button
                type="button"
                className={cn(
                  "mt-2 inline-flex max-w-full items-center justify-center rounded-md border border-border/70 bg-muted/35 px-1.5 py-0.5 text-2xs font-medium tabular-nums text-muted-foreground transition-colors hover:border-primary/40 hover:bg-primary/10 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
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
  activePart = null,
  onHoverPart,
}: {
  graph: TransactionGraphPayload;
  hideSensitive: boolean;
  expanded?: boolean;
  /** A leg lit from outside the drawing, e.g. its row in the list below. */
  activePart?: string | null;
  onHoverPart?: (part: string | null) => void;
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
  const [hoverDetail, setHoverDetail] = useState<DrawableGraphRow | null>(null);
  const [measuredCanvasWidth, setMeasuredCanvasWidth] = useState<number | null>(null);
  const {
    inputRows,
    destinationRows,
    layoutInputRows,
    layoutDestinationRows,
    totalInputRows,
    totalDestinationRows,
  } = graphLayoutRows(graph, hideSensitive, expanded ? MAX_EXPANDED_ROWS : MAX_COMPACT_ROWS);
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
  const total = bowtieTotal(totalInputRows, totalDestinationRows, graphIsLiquid(graph));
  const combinedWeight = Math.min(expanded ? 96 : 82, Math.max(26, Math.floor((canvasWidth - 2 * edgePadding) / 9)));
  const inputDrawRows = buildDrawableRows(
    layoutInputRows,
    total,
    height,
    combinedWeight,
    curveWidth,
  );
  const outputDrawRows = buildDrawableRows(
    layoutDestinationRows,
    total,
    height,
    combinedWeight,
    curveWidth,
  );
  // The visible strand and its wide invisible hit target follow the same path.
  const pathFor = (node: DrawableGraphRow) => {
    const side = node.side === "input" ? "input" : "output";
    const make = node.zeroValue ? makeZeroValuePath : makeBowtiePath;
    return make(node, side, canvasWidth, edgePadding, centerX);
  };
  const isActive = (node: DrawableGraphRow) =>
    hoverDetail
      ? hoverDetail.id === node.id && hoverDetail.side === node.side
      : activePart === graphPart(node.side, node.id);
  const hover = (node: DrawableGraphRow | null) => {
    setHoverDetail(node);
    onHoverPart?.(node ? graphPart(node.side, node.id) : null);
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
              testId="transaction-input-strand"
              active={isActive(node)}
              gradientIds={gradientIds}
              markerIds={markerIds}
              hideSensitive={hideSensitive}
              onHover={hover}
              onLeave={() => hover(null)}
            />
          ))}
          {outputDrawRows.map((node) => (
            <GraphStrand
              key={`curve-${node.id}`}
              node={node}
              path={pathFor(node)}
              testId={node.side === "fee" ? "transaction-fee-strand" : "transaction-output-strand"}
              active={isActive(node)}
              gradientIds={gradientIds}
              markerIds={markerIds}
              hideSensitive={hideSensitive}
              onHover={hover}
              onLeave={() => hover(null)}
            />
          ))}
        </svg>
        </div>
      </div>
      {hoverDetail ? <GraphLegCard node={hoverDetail} hideSensitive={hideSensitive} /> : null}
    </div>
  );
}

/** The card for the leg under the pointer, over the drawing's lower left. */
function GraphLegCard({
  node,
  hideSensitive,
}: {
  node: TransactionGraphNode;
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("transactions");
  const amount = formatNodeAmount(node, hideSensitive, t);
  const reference = copyReference(node);
  return (
    <div
      data-testid="transaction-graph-hover-detail"
      className="pointer-events-none absolute bottom-2 left-3 max-w-[min(520px,calc(100%-1.5rem))] rounded-md border border-white/10 bg-[#101114]/95 px-3 py-2 text-xs text-white shadow-lg"
    >
      <div className="grid min-w-0 gap-1">
        <div className="truncate font-medium">
          {sensitiveGraphText(nodeDisplayTitle(node, t), hideSensitive, t("graph.hidden"))}
        </div>
        <div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-white/70">
          {amount ? <span>{amount}</span> : null}
          <span>{roleLabel(node.role, t)}</span>
          <span>{ownershipBoundaryLabel(node, t)}</span>
          {reference ? (
            <span className={cn("truncate font-mono", hideSensitive && "sensitive")}>
              {hideSensitive ? t("graph.hidden") : formatShortTxid(reference)}
            </span>
          ) : null}
        </div>
      </div>
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
      <GraphWarnings graph={graph} />
    </div>
  );
}

function GraphWarnings({
  graph,
  onResolveIssue,
}: {
  graph?: TransactionGraphPayload;
  onResolveIssue?: (target: TransactionGraphIssueTarget) => void;
}) {
  const { t } = useTranslation("transactions");
  const alerts = (graph?.warnings ?? []).filter(
    (warning) => warning.level === "warning" || warning.level === "error",
  );
  const action = graphDiagnosticAction(graph);
  if (!alerts.length) return null;
  return (
    <div className="space-y-2">
      {alerts.map((warning) => (
        <div
          key={`${warning.code}-${warning.message}`}
          className="flex gap-2 rounded-md border bg-muted/35 px-3 py-2 text-sm"
        >
          <AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-600" />
          <span>{graphCodeText(t, "warnings", warning.code, warning.message)}</span>
        </div>
      ))}
      {action && onResolveIssue ? (
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="gap-2"
          onClick={() => onResolveIssue(action.target)}
        >
          <ArrowRight className="size-4" aria-hidden="true" />
          {t(action.labelKey)}
        </Button>
      ) : null}
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
  t: TFunction<"transactions">,
) {
  // A complete graph needs no caption; only a partial one explains itself.
  if (graph.supportLevel !== "partial") return null;
  if (graph.unsupportedReason === "confidential_values_hidden") {
    return t("graph.confidentialSupport");
  }
  if (graph.unsupportedReason === "input_prevout_values_missing") {
    return t("graph.inputPrevoutSupport");
  }
  return t("graph.partialSupport");
}

/**
 * The transaction graph in 3D, after the artwork lab's ribbon pieces. The flat
 * bowtie is only the fallback where WebGL is missing or fails.
 */
function TransactionGraphView({
  graph,
  hideSensitive,
  expanded = false,
  activePart,
  onHoverPart,
  onSelectPart,
}: {
  graph: TransactionGraphPayload;
  hideSensitive: boolean;
  expanded?: boolean;
  activePart: string | null;
  onHoverPart: (part: string | null) => void;
  onSelectPart: (part: string) => void;
}) {
  return (
    <TransactionGraph3D
      graph={graph}
      hideSensitive={hideSensitive}
      maxRows={BOWTIE_LINE_LIMIT}
      size={expanded ? "expanded" : "compact"}
      activePart={activePart}
      onHoverPart={onHoverPart}
      onSelectPart={onSelectPart}
      renderLeg={(row) => <GraphLegCard node={row} hideSensitive={hideSensitive} />}
      fallback={
        <TransactionFlowDiagram
          graph={graph}
          hideSensitive={hideSensitive}
          expanded={expanded}
          activePart={activePart}
          onHoverPart={onHoverPart}
        />
      }
    />
  );
}

/**
 * The graph with its inputs and outputs listed below it, as mempool shows a
 * transaction: inputs on the left, outputs on the right, under the drawing.
 */
function TransactionFlowLayout({
  graph,
  hideSensitive,
  expanded = false,
  onOpenTransaction,
}: {
  graph: TransactionGraphPayload;
  hideSensitive: boolean;
  expanded?: boolean;
  onOpenTransaction?: (transactionId: string) => void;
}) {
  // One lit leg for the drawing and the list: pointing at either lights both.
  const [activePart, setActivePart] = useState<string | null>(null);
  const sectionRef = useRef<HTMLElement | null>(null);
  // A click on a leg brings its row into view instead of leaving the sheet.
  const revealRow = (part: string) => {
    const row = [...(sectionRef.current?.querySelectorAll<HTMLElement>("[data-graph-part]") ?? [])]
      .find((element) => element.dataset.graphPart === part);
    row?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  };
  return (
    <section ref={sectionRef} className="space-y-3" data-testid="transaction-flow-layout">
      <TransactionGraphView
        graph={graph}
        hideSensitive={hideSensitive}
        expanded={expanded}
        activePart={activePart}
        onHoverPart={setActivePart}
        onSelectPart={revealRow}
      />
      <TransactionInputsOutputsPanel
        graph={graph}
        hideSensitive={hideSensitive}
        onOpenTransaction={onOpenTransaction}
        activePart={activePart}
        onHoverPart={setActivePart}
      />
    </section>
  );
}

export function TransactionGraphPanel({
  graph,
  loading,
  error,
  hideSensitive,
  selectedSwapLeg,
  onSelectSwapLeg,
  onResolveIssue,
  onOpenTransaction,
  graphlessContent,
  headerAction,
}: {
  graph?: TransactionGraphPayload;
  graphlessContent?: ReactNode;
  /** Shown beside the title, e.g. the explicit on-chain lookup. */
  headerAction?: ReactNode;
  loading?: boolean;
  error?: string | null;
  hideSensitive: boolean;
  selectedSwapLeg?: TransactionSwapRouteLegKey | null;
  onSelectSwapLeg?: (leg: TransactionSwapRouteLegKey) => void;
  onResolveIssue?: (target: TransactionGraphIssueTarget) => void;
  onOpenTransaction?: (transactionId: string) => void;
}) {
  const { t } = useTranslation("transactions");
  const showDiagram =
    graph &&
    (graph.supportLevel === "full" || graph.supportLevel === "partial") &&
    (graph.inputs.length > 0 || graph.outputs.length > 0);

  return (
    <div className="space-y-4">
      {graph?.swapRoute && exchangeTransfer(graph.swapRoute) ? (
        <ExchangeTransferSummary route={graph.swapRoute} hideSensitive={hideSensitive} />
      ) : <SwapRouteStrip
        route={graph?.swapRoute}
        hideSensitive={hideSensitive}
        selectedLeg={selectedSwapLeg}
        onSelectLeg={onSelectSwapLeg}
      />}
      {showDiagram ? (
        <>
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <div className="text-sm font-medium">{t("graph.title")}</div>
              <div className="text-xs text-muted-foreground">
                {graphSupportText(graph, t)}
              </div>
            </div>
            <div className="flex items-center gap-2">
            {headerAction}
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
              <DialogContent className="w-[min(1680px,calc(100vw-2rem))] max-w-none sm:max-w-none">
                <DialogTitle className="sr-only">{t("graph.expandedTitle")}</DialogTitle>
                <TransactionFlowLayout
                  graph={graph}
                  hideSensitive={hideSensitive}
                  expanded
                  onOpenTransaction={onOpenTransaction}
                />
              </DialogContent>
            </Dialog>
            </div>
          </div>
          <AnnotationStrip annotations={graph.annotations} />
          <TransactionFlowLayout
            graph={graph}
            hideSensitive={hideSensitive}
            onOpenTransaction={onOpenTransaction}
          />
          <GraphWarnings graph={graph} onResolveIssue={onResolveIssue} />
        </>
      ) : !loading && !error && graph?.supportLevel === "graphless" && graphlessContent ? (
        <>
          {graphlessContent}
          <GraphWarnings graph={graph} onResolveIssue={onResolveIssue} />
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
