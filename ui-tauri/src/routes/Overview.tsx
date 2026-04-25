/**
 * Overview screen — empty state + populated dashboard.
 *
 * Visual translation of claude-design/screens/overview.jsx. Layout is
 * "option D" from the variants: split chart card (KPI gutter + chart)
 * paired with the Connections card on top, transactions preview +
 * balances middle, report tiles bottom. Inline styles become Tailwind
 * classes against the theme tokens; the chart and per-row composition
 * math are preserved verbatim.
 *
 * Outstanding before this screen is feature-complete:
 *  - Real `reports.balance-history` series instead of synthesized jitter
 *    (lives in BalanceChart for now)
 *  - Add-connection picker / connection detail navigation
 *  - Currency toggle (₿/€) is currently fixed to BTC; the toggle from
 *    chrome.jsx lands when AppHeader is translated
 */

import { useState } from "react";
import { Plus } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { useDaemon } from "@/daemon/client";
import { useUiStore } from "@/store/ui";
import { useCurrency, type Currency } from "@/lib/currency";
import { cn } from "@/lib/utils";

import { Button } from "@/components/ui/button";
import { KbCard } from "@/components/kb/KbCard";
import { BalanceChart } from "@/components/kb/BalanceChart";
import { ChartFullscreen } from "@/components/kb/ChartFullscreen";
import { AddConnectionFlow } from "@/components/kb/AddConnectionFlow";
import { RangeTabs, type Range } from "@/components/kb/RangeTabs";
import { SyncDot } from "@/components/kb/SyncDot";
import { ProtocolChip } from "@/components/kb/ProtocolChip";
import { GutterStat } from "@/components/kb/Stats";
import type {
  Connection,
  OverviewSnapshot,
  Tx,
} from "@/mocks/seed";

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

const fmtBtc = (v: number) => v.toFixed(8);
const fmtEur = (v: number) =>
  "€ " +
  v.toLocaleString("de-AT", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
const fmtEurThousands = (v: number) =>
  "€ " + Math.round(v / 1000).toLocaleString("de-AT") + " k";

export function Overview() {
  const { data, isLoading } = useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const currency = useCurrency();
  const navigate = useNavigate();
  const [addOpen, setAddOpen] = useState(false);

  if (isLoading || !data?.data) {
    return (
      <div className="flex flex-1 items-center justify-center font-mono text-xs text-ink-3">
        loading…
      </div>
    );
  }

  const snapshot = data.data;
  const isEmpty = snapshot.connections.length === 0;
  const onSelectConnection = (id: string) =>
    void navigate({
      to: "/connections/$connectionId",
      params: { connectionId: id },
    });

  return (
    <>
      {isEmpty ? (
        <EmptyOverview onAdd={() => setAddOpen(true)} />
      ) : (
        <PopulatedOverview
          snapshot={snapshot}
          hideSensitive={hideSensitive}
          currency={currency}
          onAddConnection={() => setAddOpen(true)}
          onSelectConnection={onSelectConnection}
        />
      )}
      <AddConnectionFlow open={addOpen} onClose={() => setAddOpen(false)} />
    </>
  );
}

function EmptyOverview({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="flex flex-1 items-center justify-center p-10">
      <div className="flex max-w-[520px] flex-col items-center gap-5 text-center">
        <div className="mb-3 grid grid-cols-4 gap-1.5 opacity-35">
          {Array.from({ length: 12 }).map((_, i) => (
            <div
              key={i}
              className="h-7 w-10 border border-dashed border-line-2"
            />
          ))}
        </div>
        <h2 className="m-0 font-sans text-4xl font-semibold leading-[1.1] tracking-[-0.01em] text-ink">
          No connections yet.
        </h2>
        <p className="m-0 font-sans text-sm leading-[1.55] text-ink-2">
          Add a watch-only connection — XPub, descriptor, Lightning node, or
          CSV — to import transactions.
        </p>
        <Button size="lg" onClick={onAdd} className="rounded-none">
          <Plus className="size-3" />
          Add connection
        </Button>
      </div>
    </div>
  );
}

interface PopulatedOverviewProps {
  snapshot: OverviewSnapshot;
  hideSensitive: boolean;
  currency: Currency;
  onAddConnection: () => void;
  onSelectConnection: (id: string) => void;
}

function PopulatedOverview({
  snapshot,
  hideSensitive,
  currency,
  onAddConnection,
  onSelectConnection,
}: PopulatedOverviewProps) {
  const [chartRange, setChartRange] = useState<Range>("ytd");
  const [chartExpanded, setChartExpanded] = useState(false);
  const totalBtc = snapshot.connections.reduce((s, c) => s + c.balance, 0);
  const totalEur = totalBtc * snapshot.priceEur;

  return (
    <div className="flex-1 overflow-auto p-3">
      {/* Top row: chart card | connections — stacks below lg */}
      <div className="mb-2.5 grid grid-cols-1 gap-2.5 lg:grid-cols-[minmax(0,2.4fr)_minmax(0,1fr)]">
        <KbCard
          title="Balance & performance"
          action={
            <div className="flex items-center gap-2">
              <RangeTabs value={chartRange} onChange={setChartRange} />
              <button
                type="button"
                onClick={() => setChartExpanded(true)}
                title="Expand chart"
                className="flex size-5 cursor-pointer items-center justify-center border border-line bg-transparent p-0 hover:border-ink"
              >
                <svg width="10" height="10" viewBox="0 0 10 10" fill="none">
                  <path
                    d="M1 4 V1 H4 M9 4 V1 H6 M1 6 V9 H4 M9 6 V9 H6"
                    stroke="var(--color-ink-2)"
                    strokeWidth="1.2"
                    strokeLinecap="square"
                    fill="none"
                  />
                </svg>
                <span className="sr-only">Expand chart</span>
              </button>
            </div>
          }
          pad={false}
          className="@container flex flex-col"
        >
          <div className="flex min-h-0 flex-1 flex-col @3xl:grid @3xl:grid-cols-[240px_minmax(0,1fr)]">
            <ChartGutter
              snapshot={snapshot}
              totalBtc={totalBtc}
              totalEur={totalEur}
              hideSensitive={hideSensitive}
              chartRange={chartRange}
              currency={currency}
            />
            <div className="flex min-h-[260px] flex-col p-3.5">
              <div className="min-h-0 flex-1">
                <BalanceChart
                  series={snapshot.balanceSeries}
                  ccy={currency}
                  priceEur={snapshot.priceEur}
                  range={chartRange}
                />
              </div>
            </div>
          </div>
        </KbCard>{/* end balance card */}

        <ConnectionsCard
          connections={snapshot.connections}
          hideSensitive={hideSensitive}
          onAddConnection={onAddConnection}
          onSelectConnection={onSelectConnection}
        />
      </div>

      {/* Middle row: transactions preview | balances — stacks below md */}
      <div className="mb-2.5 grid grid-cols-1 items-stretch gap-2.5 md:grid-cols-[minmax(0,1.6fr)_minmax(0,1.5fr)]">
        <KbCard
          title="Transactions"
          pad={false}
          action={
            <div className="flex items-center gap-3">
              <span className="font-mono text-[10px] text-ink-3">
                {snapshot.txs.length} entries
              </span>
              <button className="cursor-pointer border-none bg-transparent font-mono text-[10px] uppercase tracking-[0.1em] text-ink">
                open all →
              </button>
            </div>
          }
        >
          <TransactionsPreview
            txs={snapshot.txs.slice(0, 6)}
            hideSensitive={hideSensitive}
            currency={currency}
          />
        </KbCard>
        <KbCard title="Balances">
          <BalanceRows hideSensitive={hideSensitive} />
        </KbCard>
      </div>

      {/* Bottom row: report tiles — 1 → 2 → 3 columns */}
      <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2 xl:grid-cols-3">
        <ReportTile
          title="Capital gains"
          sub="FIFO · EUR · jurisdiction preset"
          detail="YTD realized: + € 42,118.92"
          icon="↗"
        />
        <ReportTile
          title="Journal entries"
          sub="Debit / credit · double-entry"
          detail={`${snapshot.txs.length * 2} entries · YTD`}
          icon="≡"
        />
        <ReportTile
          title="Balance sheet"
          sub="Assets · Liabilities · Equity"
          detail="As of 2026-04-18"
          icon="▭"
        />
      </div>

      <ChartFullscreen
        open={chartExpanded}
        onClose={() => setChartExpanded(false)}
        totalBtc={totalBtc}
      />
    </div>
  );
}

interface ChartGutterProps {
  snapshot: OverviewSnapshot;
  totalBtc: number;
  totalEur: number;
  hideSensitive: boolean;
  chartRange: Range;
  currency: Currency;
}

function ChartGutter({
  snapshot,
  totalBtc,
  totalEur,
  hideSensitive,
  chartRange,
  currency,
}: ChartGutterProps) {
  const isEur = currency === "eur";
  return (
    <div className="flex flex-col gap-3 border-b border-line bg-paper-2 px-4 py-3.5 @3xl:border-b-0 @3xl:border-r">
      <div>
        <div className="font-mono text-[9px] uppercase tracking-[0.14em] text-ink-3">
          Total
        </div>
        <div
          className={cn(
            "font-sans text-[26px] font-medium leading-[1.05] tracking-[-0.015em] text-ink",
            blurClass(hideSensitive),
          )}
        >
          {isEur ? fmtEur(totalEur) : "₿ " + fmtBtc(totalBtc)}
        </div>
        <div
          className={cn(
            "mt-0.5 font-mono text-[10px] text-ink-2",
            blurClass(hideSensitive),
          )}
        >
          {isEur ? "₿ " + fmtBtc(totalBtc) : fmtEur(totalEur)}
        </div>
      </div>

      <div className="border-t border-line pt-2.5">
        <div className="font-mono text-[9px] uppercase tracking-[0.14em] text-ink-3">
          {chartRange.toUpperCase()} · change
        </div>
        <RangeDelta
          range={chartRange}
          ccy={currency}
          priceEur={snapshot.priceEur}
          hideSensitive={hideSensitive}
        />
      </div>

      <div className="grid grid-cols-2 gap-2.5 border-t border-line pt-2.5">
        <GutterStat
          label="Cost basis"
          value={
            <span className={blurClass(hideSensitive)}>
              {fmtEurThousands(snapshot.fiat.eurCostBasis)}
            </span>
          }
        />
        <GutterStat
          label="Market"
          value={
            <span className={blurClass(hideSensitive)}>
              {fmtEurThousands(snapshot.fiat.eurBalance)}
            </span>
          }
        />
        <GutterStat
          label="Unrealized"
          value={
            <span className={blurClass(hideSensitive)}>
              + {fmtEurThousands(snapshot.fiat.eurUnrealized)}
            </span>
          }
          color="text-[#3fa66a]"
        />
        <GutterStat
          label="Realized YTD"
          value={
            <span className={blurClass(hideSensitive)}>
              + {fmtEurThousands(snapshot.fiat.eurRealizedYTD)}
            </span>
          }
          color="text-[#3fa66a]"
        />
      </div>

      <div className="mt-auto flex items-baseline justify-between gap-2 border-t border-line pt-2.5">
        <div className="min-w-0">
          <div className="font-mono text-[9px] uppercase tracking-[0.14em] text-ink-3">
            BTC / EUR · spot
          </div>
          <div className="font-sans text-sm font-medium leading-[1.1] tracking-[-0.005em] text-ink">
            {fmtEur(snapshot.priceEur)}
          </div>
        </div>
        <span className="whitespace-nowrap font-mono text-[10px] text-[#3fa66a]">
          + 1.42 % · 24h
        </span>
      </div>
    </div>
  );
}

interface RangeDeltaProps {
  range: Range;
  ccy: "btc" | "eur";
  priceEur: number;
  hideSensitive: boolean;
}

const RANGE_DELTAS: Record<Range, { btc: number; pct: number }> = {
  d:   { btc: -0.00184221, pct: -0.04 },
  w:   { btc: 0.0241, pct: 0.55 },
  m:   { btc: -0.03, pct: -0.68 },
  ytd: { btc: 1.88, pct: 75.12 },
  "1y":{ btc: 2.14, pct: 95.7 },
  "5y":{ btc: 4.15, pct: 1810.0 },
  all: { btc: 4.38008004, pct: 999 },
};

function RangeDelta({ range, ccy, priceEur, hideSensitive }: RangeDeltaProps) {
  const d = RANGE_DELTAS[range];
  const up = d.btc >= 0;
  const colorClass = up ? "text-[#3fa66a]" : "text-accent";
  const sign = up ? "+" : "−";
  const abs = Math.abs(d.btc);
  const absEur = abs * priceEur;
  const fmtEurInt = (v: number) =>
    "€ " +
    v.toLocaleString("de-AT", {
      minimumFractionDigits: 0,
      maximumFractionDigits: 0,
    });

  return (
    <>
      <div
        className={cn(
          "mt-0.5 whitespace-nowrap font-sans text-[22px] font-medium leading-[1.05] tracking-[-0.01em]",
          colorClass,
          blurClass(hideSensitive),
        )}
      >
        {sign} {ccy === "btc" ? abs.toFixed(abs < 0.01 ? 8 : 4) + " ₿" : fmtEurInt(absEur)}
      </div>
      <div className={cn("mt-0.5 font-mono text-[10px]", colorClass)}>
        {sign} {Math.abs(d.pct).toFixed(2)} %
        <span className={cn("ml-1 text-ink-3", blurClass(hideSensitive))}>
          · ≈ {sign}{" "}
          {ccy === "btc"
            ? fmtEurInt(absEur)
            : abs.toFixed(abs < 0.01 ? 8 : 4) + " ₿"}
        </span>
      </div>
    </>
  );
}

interface ConnectionsCardProps {
  connections: Connection[];
  hideSensitive: boolean;
  onAddConnection: () => void;
  onSelectConnection: (id: string) => void;
}

function ConnectionsCard({
  connections,
  hideSensitive,
  onAddConnection,
  onSelectConnection,
}: ConnectionsCardProps) {
  const [excluded, setExcluded] = useState<Set<string>>(() => new Set());

  const toggleExclude = (id: string) => {
    setExcluded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const includedConns = connections.filter((c) => !excluded.has(c.id));
  const includedBtc = includedConns.reduce((s, c) => s + c.balance, 0);
  const includedSats = Math.round(includedBtc * 1e8);
  const includedCount = connections.length - excluded.size;

  const syncingN = connections.filter(
    (c) => c.status === "syncing" && !excluded.has(c.id),
  ).length;
  const errorN = connections.filter(
    (c) => c.status === "error" && !excluded.has(c.id),
  ).length;
  const headDot = errorN
    ? { bg: "bg-accent", pulse: false }
    : syncingN
      ? { bg: "bg-[#c9a43a]", pulse: true }
      : { bg: "bg-[#3fa66a]", pulse: false };

  const headerAction = (
    <span className="inline-flex items-center gap-1.5 font-mono text-[10px] tracking-[0.04em] text-ink-3 tabular-nums">
      <span
        className={cn(
          "inline-block size-1.5 shrink-0 rounded-full",
          headDot.bg,
          headDot.pulse && "animate-pulse",
        )}
      />
      {includedCount} of {connections.length}
    </span>
  );

  return (
    <KbCard title="Connections" action={headerAction} pad={false}>
      <div className="flex items-baseline justify-between gap-2 border-b border-line bg-paper-2 px-3 py-1.5">
        <div className="min-w-0">
          <div className="font-mono text-[8px] uppercase tracking-[0.14em] text-ink-3">
            {excluded.size === 0 ? "Composition" : "Composition · filtered"}
          </div>
          <div
            className={cn(
              "mt-px font-mono text-xs tabular-nums text-ink",
              blurClass(hideSensitive),
            )}
          >
            {includedSats.toLocaleString("en-US")}{" "}
            <span className="text-[9px] tracking-[0.1em] text-ink-3">SAT</span>
          </div>
        </div>
        {excluded.size > 0 && (
          <button
            onClick={() => setExcluded(new Set())}
            className="cursor-pointer border-none bg-transparent p-0 font-mono text-[9px] uppercase tracking-[0.1em] text-ink-2 underline underline-offset-2"
          >
            reset
          </button>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {connections.map((c, i) => {
          const sats = Math.round(c.balance * 1e8);
          const isExcluded = excluded.has(c.id);
          const pct =
            !isExcluded && includedBtc > 0
              ? (c.balance / includedBtc) * 100
              : 0;
          return (
            <div
              key={c.id}
              className={cn(
                "grid grid-cols-[20px_1fr_auto] items-center gap-x-2.5 px-3 py-2.5 transition-colors hover:bg-paper-2",
                i > 0 && "border-t border-line",
                isExcluded && "bg-paper-2 opacity-55",
              )}
            >
              <button
                onClick={() => toggleExclude(c.id)}
                title={
                  isExcluded ? "Include in totals" : "Exclude from totals"
                }
                className={cn(
                  "flex size-3.5 cursor-pointer items-center justify-center border border-ink",
                  isExcluded ? "bg-transparent" : "bg-ink",
                )}
              >
                {!isExcluded && (
                  <svg width="9" height="9" viewBox="0 0 9 9" fill="none">
                    <path
                      d="M1.5 4.5 L3.5 6.5 L7.5 2.5"
                      stroke="var(--color-paper)"
                      strokeWidth="1.4"
                      strokeLinecap="square"
                    />
                  </svg>
                )}
              </button>

              <button
                onClick={() => onSelectConnection(c.id)}
                className="flex min-w-0 cursor-pointer flex-col gap-1 border-none bg-transparent p-0 text-left"
              >
                <div className="flex min-w-0 items-center gap-2">
                  <SyncDot status={c.status} />
                  <span className="overflow-hidden truncate whitespace-nowrap font-sans text-[13px] font-semibold tracking-[-0.005em] text-ink">
                    {c.label}
                  </span>
                  <ProtocolChip kind={c.kind} />
                </div>
                <div className="relative h-0.5 w-full max-w-[180px] bg-line">
                  <div
                    className={cn(
                      "absolute inset-y-0 left-0 bg-ink transition-[width] duration-200",
                      blurClass(hideSensitive),
                    )}
                    style={{
                      width: isExcluded ? "0%" : `${Math.max(1.5, pct)}%`,
                    }}
                  />
                </div>
              </button>

              <div className="min-w-[56px] text-right">
                <div
                  className={cn(
                    "font-sans text-base font-medium leading-[1.05] tracking-[-0.01em] tabular-nums",
                    isExcluded ? "text-ink-3" : "text-ink",
                    blurClass(hideSensitive),
                  )}
                >
                  {isExcluded
                    ? "—"
                    : pct < 0.1
                      ? "<0.1"
                      : pct.toFixed(pct < 10 ? 1 : 0)}
                  <span className="ml-px text-[10px] text-ink-3">%</span>
                </div>
                <div
                  className={cn(
                    "mt-px font-mono text-[9px] tabular-nums text-ink-3",
                    isExcluded && "line-through",
                    blurClass(hideSensitive),
                  )}
                >
                  {sats.toLocaleString("en-US")}
                </div>
              </div>
            </div>
          );
        })}
        <button
          onClick={onAddConnection}
          className="sticky bottom-0 flex w-full cursor-pointer items-center justify-center gap-2 border-none border-t border-line bg-paper px-3 py-2.5"
        >
          <Plus className="size-2.5 text-ink-2" />
          <span className="font-sans text-xs font-medium text-ink">
            Add connection
          </span>
        </button>
      </div>
    </KbCard>
  );
}

interface TransactionsPreviewProps {
  txs: Tx[];
  hideSensitive: boolean;
  currency: Currency;
}

const TX_TYPE_COLOR: Record<Tx["type"], string> = {
  Income: "text-[#3fa66a]",
  Expense: "text-accent",
  Transfer: "text-ink-2",
  Fee: "text-ink-2",
  Swap: "text-[#8b6f3c]",
  Mint: "text-[#3f7aa6]",
  Melt: "text-[#a66a3f]",
  Consolidation: "text-ink-2",
  Rebalance: "text-ink-2",
};

function TransactionsPreview({
  txs,
  hideSensitive,
  currency,
}: TransactionsPreviewProps) {
  const isEur = currency === "eur";
  return (
    <table className="w-full border-collapse font-mono text-[11px]">
      <thead>
        <tr className="border-b border-line">
          <th className="px-3.5 py-2 text-left font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3">
            Date
          </th>
          <th className="px-3.5 py-2 text-left font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3">
            Type
          </th>
          <th className="px-3.5 py-2 text-left font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3">
            Counterparty
          </th>
          <th className="px-3.5 py-2 text-right font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3">
            {isEur ? "€" : "₿"}
          </th>
          <th className="px-3.5 py-2 text-right font-sans text-[9px] font-semibold uppercase tracking-[0.12em] text-ink-3">
            {isEur ? "sats" : "€"}
          </th>
        </tr>
      </thead>
      <tbody>
        {txs.map((tx) => {
          const btc = tx.amountSat / 1e8;
          const sign = tx.amountSat > 0 ? "+" : tx.amountSat < 0 ? "−" : "";
          const primary = isEur
            ? (tx.eur > 0 ? "+ €" : "− €") + Math.abs(tx.eur).toFixed(2)
            : sign + " ₿ " + Math.abs(btc).toFixed(8);
          const secondary = isEur
            ? (tx.amountSat > 0 ? "+" : "") +
              tx.amountSat.toLocaleString("en-US")
            : (tx.eur > 0 ? "+ €" : "− €") + Math.abs(tx.eur).toFixed(2);
          return (
            <tr key={tx.id} className="border-b border-line">
              <td className="px-3.5 py-2.5 text-ink-2">{tx.date.slice(5, 10)}</td>
              <td className="px-3.5 py-2.5">
                <span
                  className={cn(
                    "text-[9px] uppercase tracking-[0.1em]",
                    TX_TYPE_COLOR[tx.type],
                  )}
                >
                  {tx.type}
                </span>
              </td>
              <td className="px-3.5 py-2.5 font-sans text-xs text-ink">
                {tx.counter}
              </td>
              <td
                className={cn(
                  "px-3.5 py-2.5 text-right",
                  tx.amountSat > 0 ? "text-[#3fa66a]" : "text-ink",
                  blurClass(hideSensitive),
                )}
              >
                {primary}
              </td>
              <td
                className={cn(
                  "px-3.5 py-2.5 text-right",
                  blurClass(hideSensitive),
                )}
              >
                {secondary}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

interface BalanceRow {
  k: string;
  sub: string;
  sat: number;
  open?: boolean;
  children?: Array<{ k: string; sat: number }>;
}

const BALANCE_ROWS: BalanceRow[] = [
  {
    k: "Assets",
    sub: "Resources owned",
    sat: 438_007_404,
    open: true,
    children: [
      { k: "On-chain holdings", sat: 432_953_372 },
      { k: "Lightning channels", sat: 4_821_309 },
      { k: "Cashu (ecash)", sat: 19_823 },
      { k: "NWC balances", sat: 213_500 },
    ],
  },
  { k: "Income", sub: "Money earned", sat: 75_520_000 },
  { k: "Expenses", sub: "Money spent", sat: -2_812_410 },
  { k: "Liabilities", sub: "Debts and obligations", sat: 0 },
  { k: "Equity", sub: "Owner contributions", sat: 360_000_000 },
];

function formatSat(n: number): string {
  const sign = n < 0 ? "− " : "";
  const abs = Math.abs(n).toString().padStart(9, "0");
  const btc = abs.slice(0, -8) || "0";
  const rest = abs.slice(-8);
  return sign + btc + "." + rest.slice(0, 2) + " " + rest.slice(2, 5) + " " + rest.slice(5);
}

function BalanceRows({ hideSensitive }: { hideSensitive: boolean }) {
  return (
    <div className="flex flex-col">
      {BALANCE_ROWS.map((r, i) => (
        <div key={r.k}>
          <div
            className={cn(
              "flex items-center justify-between px-0.5 py-2",
              i < BALANCE_ROWS.length - 1 && "border-b border-line",
            )}
          >
            <div className="flex items-baseline gap-2.5">
              <span className="font-sans text-sm text-ink">{r.k}</span>
              <span className="font-sans text-[11px] text-ink-3">{r.sub}</span>
            </div>
            <span
              className={cn(
                "font-mono text-xs tracking-[-0.01em]",
                r.sat < 0 ? "text-accent" : "text-ink",
                blurClass(hideSensitive),
              )}
            >
              ₿ {formatSat(r.sat)}{" "}
              <span className="text-ink-3">sat</span>
            </span>
          </div>
          {r.open &&
            r.children?.map((c) => (
              <div
                key={c.k}
                className="flex justify-between border-b border-dotted border-line px-0.5 py-1 pl-4"
              >
                <span className="font-sans text-[11px] text-ink-2">
                  ↳ {c.k}
                </span>
                <span
                  className={cn(
                    "font-mono text-[11px] text-ink-2",
                    blurClass(hideSensitive),
                  )}
                >
                  ₿ {formatSat(c.sat)}
                </span>
              </div>
            ))}
        </div>
      ))}
    </div>
  );
}

interface ReportTileProps {
  title: string;
  sub: string;
  detail: string;
  icon: string;
  onClick?: () => void;
}

function ReportTile({ title, sub, detail, icon, onClick }: ReportTileProps) {
  return (
    <button
      onClick={onClick}
      className="flex cursor-pointer items-start gap-3.5 border border-line bg-paper-2 p-4 text-left"
    >
      <div className="flex size-8.5 flex-shrink-0 items-center justify-center border border-ink font-sans text-lg text-ink">
        {icon}
      </div>
      <div className="min-w-0 flex-1">
        <div className="font-sans text-base text-ink">{title}</div>
        <div className="mt-0.5 font-sans text-[11px] text-ink-3">{sub}</div>
        <div className="mt-2.5 font-mono text-[11px] text-ink">{detail}</div>
      </div>
      <span className="font-mono text-xs text-ink-3">↗</span>
    </button>
  );
}
