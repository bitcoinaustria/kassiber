import { useState } from "react";
import { useTranslation } from "react-i18next";

import { TransactionFlowDiagram } from "@/components/transactions/TransactionGraphTab";
import { transactionGraphLookupReferenceArgs } from "@/components/transactions/TransactionGraphLookup";
import type { TransactionGraphPayload } from "@/components/transactions/TransactionGraphModel";
import { useDaemon } from "@/daemon/client";
import type { FlowNode, TransactionFlowVisual } from "./model";
import type { VisualFormat } from "./format";
import type { VisualPalette } from "./palette";
import { CountUp, StatTile } from "./primitives";
import { TransactionItem } from "./TransactionsVisual";

// "ambiguous" is a coin of several of the user's own wallets: still theirs.
const OWN = new Set(["owned", "ambiguous"]);

interface Props {
  visual: TransactionFlowVisual;
  fmt: VisualFormat;
  palette: VisualPalette;
  hideSensitive: boolean;
  expanded?: boolean;
}

/**
 * One transaction's flow. The figures above the drawing come from the
 * daemon's redacted summary, which is also all the model saw. The drawing is
 * the transaction detail panel's flat graph, which the desktop reads from the
 * local book itself (`ui.transactions.graph`, never a public lookup), so the
 * chat and the panel draw the same data and its outpoints never reach the model.
 * The chat keeps to the flat drawing: no 3D scene to drag inside a message.
 */
export function FlowVisual(props: Props) {
  const { visual } = props;
  if (!visual.localTransaction) return <TransactionFallback {...props} />;
  return <BookGraph {...props} transactionId={visual.localTransaction} />;
}

function BookGraph({
  visual,
  fmt,
  palette,
  hideSensitive,
  expanded,
  transactionId,
}: Props & { transactionId: string }) {
  const hasLegs = visual.nodes.length > 0;
  const query = useDaemon<TransactionGraphPayload>(
    "ui.transactions.graph",
    transactionGraphLookupReferenceArgs(transactionId),
    { enabled: hasLegs },
  );
  const [activePart, setActivePart] = useState<string | null>(null);
  const graph = query.data?.data;
  const drawable =
    graph &&
    (graph.supportLevel === "full" || graph.supportLevel === "partial") &&
    (graph.inputs.length > 0 || graph.outputs.length > 0);
  if (!hasLegs || (!query.isLoading && !drawable)) {
    return <TransactionFallback visual={visual} fmt={fmt} palette={palette} hideSensitive={hideSensitive} />;
  }
  return (
    <div className="space-y-3">
      <FlowTiles visual={visual} fmt={fmt} hideSensitive={hideSensitive} />
      {drawable ? (
        <div className="kb-visual-rise" data-slot="chat-visual-graph">
          <TransactionFlowDiagram
            graph={graph}
            hideSensitive={hideSensitive}
            expanded={expanded}
            activePart={activePart}
            onHoverPart={setActivePart}
          />
        </div>
      ) : (
        <div className="kb-visual-shimmer h-48 w-full rounded-md bg-muted/40" role="status" />
      )}
      {visual.transaction ? (
        <ul className="overflow-hidden rounded-md border border-border/60">
          <TransactionItem row={visual.transaction} index={0} fraction={0} fmt={fmt} palette={palette} hideSensitive={hideSensitive} />
        </ul>
      ) : null}
    </div>
  );
}

function FlowTiles({
  visual,
  fmt,
  hideSensitive,
}: {
  visual: TransactionFlowVisual;
  fmt: VisualFormat;
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("assistant");
  const inputs = visual.nodes.filter((node) => node.side === "input");
  const outputs = visual.nodes.filter((node) => node.side !== "input");
  const sum = (nodes: FlowNode[]) => nodes.reduce((total, node) => total + (node.valueSats ?? 0), 0);
  const allKnown = visual.nodes.every((node) => node.valueSats !== null);
  const ownedIn = sum(inputs.filter((node) => OWN.has(node.ownership)));
  const ownedOut = sum(outputs.filter((node) => OWN.has(node.ownership)));
  const fee = outputs.find((node) => node.side === "fee")?.valueSats ?? null;
  return (
    <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
      <StatTile
        label={t("visual.flow.yourChange")}
        value={allKnown && (ownedIn || ownedOut) ? ownedOut - ownedIn : null}
        format={(value) => fmt.signed(fmt.btc(Math.abs(value) / 1e8), value)}
        sensitive={hideSensitive}
      />
      <StatTile
        label={t("visual.flow.moved")}
        value={inputs.every((node) => node.valueSats !== null) ? sum(inputs) : null}
        format={(value) => fmt.btc(value / 1e8)}
        sensitive={hideSensitive}
        delayMs={70}
      />
      <StatTile label={t("visual.flow.fee")} value={fee} format={fmt.sats} sensitive={hideSensitive} delayMs={140} />
    </div>
  );
}

/** Without a transaction graph, the transaction itself, as the book records it. */
function TransactionFallback({ visual, fmt, palette, hideSensitive }: Props) {
  const { t } = useTranslation("assistant");
  const tx = visual.transaction;
  if (!tx) return null;
  const incoming = tx.direction === "in";
  return (
    <div className="space-y-3">
      <div className="kb-visual-rise flex flex-wrap items-end justify-between gap-3">
        <div>
          <div className="text-2xs text-muted-foreground">
            {t(`visual.transactions.direction.${tx.direction}`)}
            {tx.date ? ` · ${fmt.date(tx.date, "short")}` : ""}
          </div>
          <CountUp
            value={tx.amountSat / 1e8}
            format={(value) => fmt.signed(fmt.asset(Math.abs(value), tx.asset), value)}
            sensitive={hideSensitive}
            className="text-2xl font-semibold tracking-tight"
          />
        </div>
        <span
          className="rounded-full px-2 py-0.5 text-2xs font-medium"
          style={{
            color: "var(--foreground)",
            backgroundColor: `color-mix(in oklab, ${incoming ? palette.positive : palette.negative} 16%, transparent)`,
          }}
        >
          {tx.wallet}
        </span>
      </div>
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
        <StatTile label={t("visual.flow.fiatValue")} value={tx.fiatValue === null ? null : Math.abs(tx.fiatValue)} format={fmt.fiat} sensitive={hideSensitive} />
        <StatTile label={t("visual.flow.fee")} value={tx.feeSat} format={fmt.sats} sensitive={hideSensitive} delayMs={70} />
        <div className="kb-visual-rise min-w-0 rounded-md border border-border/60 bg-background/40 px-3 py-2" style={{ animationDelay: "140ms" }}>
          <div className="truncate text-2xs text-muted-foreground">{t("visual.flow.counterparty")}</div>
          <div className="mt-0.5 truncate text-sm font-medium">{tx.counterparty || tx.note || "—"}</div>
        </div>
      </div>
    </div>
  );
}
