import { useTranslation } from "react-i18next";
import { ArrowDownLeft, ArrowUpRight, ChevronRight, Minus } from "lucide-react";

import { cn } from "@/lib/utils";
import { openLocalReference } from "../localReference";
import type { TransactionRow, TransactionsVisual as TransactionsVisualModel } from "./model";
import type { VisualFormat } from "./format";
import type { VisualPalette } from "./palette";
import { stagger } from "./motion";
import { GrowBar } from "./primitives";

interface Props {
  visual: TransactionsVisualModel;
  fmt: VisualFormat;
  palette: VisualPalette;
  hideSensitive: boolean;
}

export function TransactionsVisual({ visual, fmt, palette, hideSensitive }: Props) {
  const { t } = useTranslation("assistant");
  const maxAbs = Math.max(1, ...visual.rows.map((row) => Math.abs(row.amountSat)));
  const { summary } = visual;
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <span>{t("visual.transactions.shown", { count: summary.shown, total: summary.total })}</span>
        {summary.inSat ? (
          <span className="inline-flex items-center gap-1">
            <ArrowDownLeft className="size-3" style={{ color: palette.positive }} aria-hidden="true" />
            <span className={cn("tabular-nums", hideSensitive && "sensitive")}>{fmt.btc(summary.inSat / 1e8)}</span>
          </span>
        ) : null}
        {summary.outSat ? (
          <span className="inline-flex items-center gap-1">
            <ArrowUpRight className="size-3" style={{ color: palette.negative }} aria-hidden="true" />
            <span className={cn("tabular-nums", hideSensitive && "sensitive")}>{fmt.btc(summary.outSat / 1e8)}</span>
          </span>
        ) : null}
      </div>
      {visual.rows.length ? (
        <ul className="divide-y divide-border/50 overflow-hidden rounded-md border border-border/60">
          {visual.rows.map((row, index) => (
            <TransactionItem
              key={row.id}
              row={row}
              index={index}
              fraction={Math.abs(row.amountSat) / maxAbs}
              fmt={fmt}
              palette={palette}
              hideSensitive={hideSensitive}
            />
          ))}
        </ul>
      ) : null}
    </div>
  );
}

export function TransactionItem({
  row,
  index,
  fraction,
  fmt,
  palette,
  hideSensitive,
}: {
  row: TransactionRow;
  index: number;
  fraction: number;
  fmt: VisualFormat;
  palette: VisualPalette;
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("assistant");
  const incoming = row.direction === "in";
  const color = row.direction === "none" ? palette.muted : incoming ? palette.positive : palette.negative;
  const Icon = row.direction === "none" ? Minus : incoming ? ArrowDownLeft : ArrowUpRight;
  const title = row.counterparty || row.note || row.type || t(`visual.transactions.direction.${row.direction}`);
  const amount = fmt.signed(fmt.asset(Math.abs(row.amountSat) / 1e8, row.asset), row.amountSat);
  const status = row.status && row.status !== "completed" ? row.status : null;
  return (
    <li className="kb-visual-rise" style={{ animationDelay: `${stagger(index)}ms` }}>
      <button
        type="button"
        onClick={() => openLocalReference({ route: "/transactions", transaction: row.id })}
        className="group/tx relative grid w-full grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-3 px-3 py-2 text-left transition-colors hover:bg-muted/40 focus-visible:bg-muted/40 focus-visible:outline-none"
        aria-label={t("visual.transactions.open", { title })}
      >
        <span
          className="flex size-7 items-center justify-center rounded-full"
          style={{ backgroundColor: `color-mix(in oklab, ${color} 16%, transparent)` }}
          aria-hidden="true"
        >
          <Icon className="size-3.5" style={{ color }} />
        </span>
        <span className="min-w-0">
          <span className="flex items-center gap-1.5">
            <span className="truncate text-xs font-medium">{title}</span>
            {status ? (
              <span className="shrink-0 rounded-full bg-amber-500/10 px-1.5 text-2xs text-amber-700 dark:text-amber-300">
                {t(`visual.transactions.status.${status}`, { defaultValue: status })}
              </span>
            ) : null}
          </span>
          <span className="block truncate text-2xs text-muted-foreground">
            {[row.wallet, row.date ? fmt.date(row.date, "short") : null, ...row.tags.slice(0, 2)].filter(Boolean).join(" · ")}
          </span>
        </span>
        <span className="flex items-center gap-1.5 text-right">
          <span>
            <span className={cn("block text-xs font-medium tabular-nums", hideSensitive && "sensitive")}>{amount}</span>
            {row.fiatValue !== null ? (
              <span className={cn("block text-2xs tabular-nums text-muted-foreground", hideSensitive && "sensitive")}>
                {fmt.fiat(Math.abs(row.fiatValue))}
              </span>
            ) : null}
          </span>
          <ChevronRight className="size-3.5 text-muted-foreground opacity-0 transition-opacity group-hover/tx:opacity-100" aria-hidden="true" />
        </span>
        <span className="pointer-events-none absolute inset-x-3 bottom-0 h-[2px]">
          <GrowBar fraction={fraction} color={color} delayMs={stagger(index) + 120} className="opacity-50" />
        </span>
      </button>
    </li>
  );
}
