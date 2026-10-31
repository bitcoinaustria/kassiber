import * as React from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  ArrowDownUp,
  ArrowUpRight,
  Check,
  ChartLine,
  ChartPie,
  Copy,
  LayoutDashboard,
  List,
  ListChecks,
  Maximize2,
  MessageSquarePlus,
  Receipt,
  Table2,
  TriangleAlert,
  Workflow,
} from "lucide-react";

import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { localeForLanguage } from "@/i18n/config";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";
import { AssistantSessionContext } from "../assistantSession";
import { openAppRoute, openLocalReference } from "../localReference";
import { AllocationVisual } from "./AllocationVisual";
import { BalanceVisual } from "./BalanceVisual";
import { CashFlowVisual } from "./CashFlowVisual";
import { FlowVisual } from "./FlowVisual";
import { OverviewVisual } from "./OverviewVisual";
import { ReadinessVisual } from "./ReadinessVisual";
import { TaxYearsVisual } from "./TaxYearsVisual";
import { TransactionsVisual } from "./TransactionsVisual";
import { visualFormat, type VisualFormat } from "./format";
import type { ChatVisual as ChatVisualModel, ChatVisualSource } from "./model";
import { useVisualPalette, type VisualPalette } from "./palette";
import { tableCsv, visualTable } from "./table";

const SOURCE_ICON: Record<ChatVisualSource, typeof ChartLine> = {
  overview: LayoutDashboard,
  balance_history: ChartLine,
  portfolio: ChartPie,
  tax_summary: Receipt,
  cash_flow: ArrowDownUp,
  transactions: List,
  transaction_flow: Workflow,
  readiness: ListChecks,
};

const SOURCE_ROUTE: Record<ChatVisualSource, string> = {
  overview: "/overview",
  balance_history: "/reports",
  portfolio: "/reports",
  tax_summary: "/reports",
  cash_flow: "/transactions",
  transactions: "/transactions",
  transaction_flow: "/transactions",
  readiness: "/reports",
};

type FollowUp =
  | "balanceOverTime"
  | "whatNeedsAttention"
  | "holdings"
  | "cashFlow"
  | "largestTransactions"
  | "disposals"
  | "largestOutgoing"
  | "overview"
  | "recentTransactions";

/** Two follow-up questions per visual, asked in the user's language. */
const FOLLOW_UPS: Record<ChatVisualSource, FollowUp[]> = {
  overview: ["balanceOverTime", "whatNeedsAttention"],
  balance_history: ["holdings", "cashFlow"],
  portfolio: ["balanceOverTime", "largestTransactions"],
  tax_summary: ["whatNeedsAttention", "disposals"],
  cash_flow: ["largestOutgoing", "balanceOverTime"],
  transactions: ["cashFlow", "overview"],
  transaction_flow: ["recentTransactions", "overview"],
  readiness: [],
};

function accent(source: ChatVisualSource, palette: VisualPalette): string {
  switch (source) {
    case "cash_flow":
    case "tax_summary":
      return palette.positive;
    case "readiness":
      return palette.series[2];
    case "transactions":
    case "transaction_flow":
      return palette.series[3];
    default:
      return palette.series[0];
  }
}

function subtitle(visual: ChatVisualModel, fmt: VisualFormat, t: TFunction<"assistant">): string {
  const parts: string[] = [];
  if (visual.source === "cash_flow" && visual.period) parts.push(t(`visual.period.${visual.period}`, { defaultValue: visual.period }));
  if (visual.source === "balance_history") parts.push(t(`visual.interval.${visual.interval}`, { defaultValue: visual.interval }));
  if (visual.source === "transactions") parts.push(t(`visual.sort.${visual.sort}`, { defaultValue: visual.sort }));
  for (const [key, value] of Object.entries(visual.filters)) {
    if (key === "period") parts.push(t(`visual.period.${value}`, { defaultValue: value }));
    else if (key === "direction") parts.push(t(`visual.transactions.direction.${value === "inbound" ? "in" : "out"}`));
    else if (key === "query") parts.push(`“${value}”`);
    else parts.push(value);
  }
  if (visual.asOf) parts.push(t("visual.asOf", { date: fmt.date(visual.asOf, "short") }));
  return parts.join(" · ");
}

function caveatText(caveat: { code: string; count?: number }, t: TFunction<"assistant">): string {
  const fallback = t("visual.caveat.other", { code: caveat.code });
  // Without a count the base key applies, not a plural form of zero.
  return caveat.count === undefined
    ? t(`visual.caveat.${caveat.code}`, { defaultValue: fallback })
    : t(`visual.caveat.${caveat.code}`, { count: caveat.count, defaultValue: fallback });
}

function VisualBody({
  visual,
  fmt,
  palette,
  hideSensitive,
  expanded,
  onAsk,
}: {
  visual: ChatVisualModel;
  fmt: VisualFormat;
  palette: VisualPalette;
  hideSensitive: boolean;
  expanded?: boolean;
  onAsk?: (prompt: string) => void;
}) {
  const props = { fmt, palette, hideSensitive, expanded };
  switch (visual.source) {
    case "overview":
      return <OverviewVisual visual={visual} {...props} />;
    case "balance_history":
      return <BalanceVisual visual={visual} {...props} />;
    case "portfolio":
      return <AllocationVisual visual={visual} {...props} />;
    case "tax_summary":
      return <TaxYearsVisual visual={visual} {...props} />;
    case "cash_flow":
      return <CashFlowVisual visual={visual} {...props} />;
    case "transactions":
      return <TransactionsVisual visual={visual} {...props} />;
    case "transaction_flow":
      return <FlowVisual visual={visual} {...props} />;
    case "readiness":
      return <ReadinessVisual visual={visual} fmt={fmt} onAsk={onAsk} />;
  }
}

function ActionButton({
  label,
  onClick,
  pressed,
  children,
}: {
  label: string;
  onClick: () => void;
  pressed?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      aria-pressed={pressed}
      onClick={onClick}
      className={cn(
        "inline-flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
        pressed && "bg-muted text-foreground",
      )}
    >
      {children}
    </button>
  );
}

export function ChatVisual({ visual }: { visual: ChatVisualModel }) {
  const { t, i18n } = useTranslation("assistant");
  const session = React.useContext(AssistantSessionContext);
  const hideSensitive = useUiStore((state) => state.hideSensitive);
  const palette = useVisualPalette();
  const fmt = React.useMemo(
    () => visualFormat(localeForLanguage(i18n.language), visual.fiatCurrency),
    [i18n.language, visual.fiatCurrency],
  );
  const [showTable, setShowTable] = React.useState(false);
  const [expanded, setExpanded] = React.useState(false);
  const [copied, setCopied] = React.useState(false);
  const Icon = SOURCE_ICON[visual.source];
  const title = visual.title ?? t(`visual.source.${visual.source}`);
  const table = React.useMemo(() => visualTable(visual, fmt, t), [visual, fmt, t]);
  const onAsk = session?.sendPrompt ? (prompt: string) => void session.sendPrompt(prompt) : undefined;
  const glow = accent(visual.source, palette);
  const scope = subtitle(visual, fmt, t);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(tableCsv(table));
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      setCopied(false);
    }
  };
  const openInApp = () => {
    if (visual.source === "transaction_flow" && visual.localTransaction) {
      openLocalReference({ route: "/transactions", transaction: visual.localTransaction });
    } else {
      openAppRoute(SOURCE_ROUTE[visual.source]);
    }
  };

  return (
    <figure
      className="kb-surface-inset kb-visual-rise relative my-3 w-full min-w-0 overflow-hidden p-3 sm:p-4"
      data-slot="chat-visual"
      data-visual-source={visual.source}
      aria-label={title}
    >
      <div
        className="pointer-events-none absolute -top-24 -left-16 size-64 rounded-full opacity-[0.10] blur-3xl dark:opacity-[0.16]"
        style={{ backgroundColor: glow }}
        aria-hidden="true"
      />
      <figcaption className="relative mb-3 flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-2.5">
          <span
            className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-md"
            style={{ backgroundColor: `color-mix(in oklab, ${glow} 16%, transparent)` }}
            aria-hidden="true"
          >
            <Icon className="size-4" style={{ color: glow }} />
          </span>
          <div className="min-w-0">
            <div className="truncate text-sm font-semibold">{title}</div>
            {scope ? <div className="truncate text-2xs text-muted-foreground">{scope}</div> : null}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-0.5">
          <ActionButton label={t(showTable ? "visual.actions.hideTable" : "visual.actions.showTable")} pressed={showTable} onClick={() => setShowTable((value) => !value)}>
            <Table2 className="size-3.5" aria-hidden="true" />
          </ActionButton>
          <ActionButton label={t(copied ? "visual.actions.copied" : "visual.actions.copyCsv")} onClick={() => void copy()}>
            {copied ? <Check className="size-3.5" aria-hidden="true" /> : <Copy className="size-3.5" aria-hidden="true" />}
          </ActionButton>
          <ActionButton label={t("visual.actions.expand")} onClick={() => setExpanded(true)}>
            <Maximize2 className="size-3.5" aria-hidden="true" />
          </ActionButton>
          <ActionButton label={t("visual.actions.openInApp")} onClick={openInApp}>
            <ArrowUpRight className="size-3.5" aria-hidden="true" />
          </ActionButton>
        </div>
      </figcaption>
      <div className="relative">
        {showTable ? (
          <VisualTableView table={table} hideSensitive={hideSensitive} />
        ) : (
          <VisualBody visual={visual} fmt={fmt} palette={palette} hideSensitive={hideSensitive} onAsk={onAsk} />
        )}
      </div>
      {visual.caveats.length ? (
        <ul className="relative mt-3 space-y-1" data-visual-caveats>
          {visual.caveats.map((caveat) => (
            <li key={caveat.code} role="note" className="flex items-start gap-1.5 text-xs text-muted-foreground">
              <TriangleAlert className="mt-0.5 size-3.5 shrink-0 text-amber-500" aria-hidden="true" />
              {caveatText(caveat, t)}
            </li>
          ))}
        </ul>
      ) : null}
      <div className="relative mt-3 flex flex-wrap items-center gap-1.5">
        {onAsk
          ? FOLLOW_UPS[visual.source].map((key) => (
              <button
                key={key}
                type="button"
                onClick={() => onAsk(t(`visual.followUp.${key}`))}
                className="inline-flex items-center gap-1 rounded-full border border-border/60 bg-background/50 px-2.5 py-1 text-2xs text-muted-foreground transition-colors hover:border-border hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
              >
                <MessageSquarePlus className="size-3" aria-hidden="true" />
                {t(`visual.followUp.${key}`)}
              </button>
            ))
          : null}
        <span className="ml-auto text-2xs text-muted-foreground/80">{t("visual.provenance")}</span>
      </div>
      <Dialog open={expanded} onOpenChange={setExpanded}>
        <DialogContent className="sm:max-w-4xl">
          <DialogTitle className="flex items-center gap-2 text-base">
            <Icon className="size-4" style={{ color: glow }} aria-hidden="true" />
            {title}
          </DialogTitle>
          <DialogDescription className="text-xs">{scope}</DialogDescription>
          <div className="max-h-[75vh] overflow-y-auto">
            <VisualBody visual={visual} fmt={fmt} palette={palette} hideSensitive={hideSensitive} expanded onAsk={onAsk} />
          </div>
        </DialogContent>
      </Dialog>
    </figure>
  );
}

function VisualTableView({ table, hideSensitive }: { table: ReturnType<typeof visualTable>; hideSensitive: boolean }) {
  return (
    <div className="max-h-72 overflow-auto rounded-md border border-border/60">
      <table className="w-full text-xs">
        <thead className="sticky top-0 bg-card">
          <tr className="border-b">
            {table.columns.map((column, index) => (
              <th key={`${column}-${index}`} scope="col" className={cn("px-2 py-1.5 text-left font-medium text-muted-foreground", index > 0 && "text-right")}>
                {column}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {table.rows.map((row, rowIndex) => (
            <tr key={rowIndex} className="border-b last:border-b-0">
              {row.map((cell, index) => (
                <td key={index} className={cn("px-2 py-1", cell.numeric ? "text-right tabular-nums" : index > 0 && "text-right", cell.numeric && hideSensitive && "sensitive")}>
                  {cell.display}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Shown while the daemon draws a visual the model asked for. */
export function ChatVisualSkeleton({ source }: { source: string | null }) {
  const { t } = useTranslation("assistant");
  // The model's raw argument: only an own key of the map, never a prototype name.
  const known = source && Object.hasOwn(SOURCE_ICON, source) ? (source as ChatVisualSource) : null;
  const Icon = known ? SOURCE_ICON[known] : ChartLine;
  return (
    <div className="kb-surface-inset kb-visual-rise my-3 w-full overflow-hidden p-3 sm:p-4" role="status" data-slot="chat-visual-pending">
      <div className="mb-3 flex items-center gap-2.5">
        <span className="flex size-7 items-center justify-center rounded-md bg-muted" aria-hidden="true">
          <Icon className="size-4 text-muted-foreground" />
        </span>
        <span className="text-xs text-muted-foreground">
          {t("visual.drawing", { source: known ? t(`visual.source.${known}`) : t("visual.source.generic") })}
        </span>
      </div>
      <div className="space-y-2">
        <div className="kb-visual-shimmer h-6 w-1/3 rounded-md bg-muted/50" />
        <div className="kb-visual-shimmer h-32 w-full rounded-md bg-muted/40" />
      </div>
    </div>
  );
}
