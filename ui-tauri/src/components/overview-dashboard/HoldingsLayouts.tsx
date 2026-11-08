import { Link } from "@tanstack/react-router";
import type { CSSProperties, ReactNode } from "react";
import { Landmark } from "lucide-react";
import { useTranslation } from "react-i18next";

import { RailIcon } from "@/components/kb/BitcoinRail";
import { formatBtc } from "@/lib/currency";
import { cn } from "@/lib/utils";
import { blurClass, type HoldingsItem } from "./model";

export type HoldingsLayoutProps = {
  rows: HoldingsItem[];
  hideSensitive: boolean;
  /** The row's figure in the display currency, or "—" without a rate. */
  valueLabel: (btc: number, value: number) => string;
  /** Fiat value regardless of the display currency, or null without a rate. */
  fiatLabel: (value: number) => string | null;
  isBitcoinMode: boolean;
};

const rowKey = (item: HoldingsItem) => item.connectionId ?? item.nameKey ?? item.name;

const shareLabel = (item: HoldingsItem) =>
  item.empty ? "—" : item.percent === 0 && item.btc > 0 ? "<1%" : `${item.percent}%`;

/** Opens the source's wallet page; the folded "Other sources" row opens none. */
const SourceLink = ({
  item,
  className,
  children,
}: {
  item: HoldingsItem;
  className: string;
  children: ReactNode;
}) => {
  const { t } = useTranslation("overview");
  if (!item.connectionId) {
    return (
      <div className={className}>
        {children}
      </div>
    );
  }
  return (
    <Link
      to="/connections/$connectionId"
      params={{ connectionId: item.connectionId }}
      aria-label={t("holdings.openSource", { name: item.name })}
      className={cn(
        className,
        "transition-colors hover:bg-muted/45 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
      )}
    >
      {children}
    </Link>
  );
};

const SourceIcon = ({ item }: { item: HoldingsItem }) =>
  item.rail ? (
    <RailIcon rail={item.rail} size="compact" />
  ) : item.connectionId ? (
    <span className="flex size-5 items-center justify-center rounded-full bg-muted text-muted-foreground">
      <Landmark className="size-3" aria-hidden="true" />
    </span>
  ) : null;

const SourceName = ({ item }: { item: HoldingsItem }) => {
  const { t } = useTranslation("overview");
  return (
    <span className="flex min-w-0 items-center gap-1.5">
      <span
        className={cn("truncate", (item.nameKey || item.empty) && "text-muted-foreground")}
        title={item.nameKey ? undefined : item.name}
      >
        {item.nameKey ? t(item.nameKey as never) : item.name}
      </span>
      {item.syncState ? (
        <span
          className={cn(
            "size-1.5 shrink-0 rounded-full",
            item.syncState === "error" ? "bg-red-500" : "animate-pulse bg-amber-500",
          )}
          role="img"
          aria-label={t(`holdings.syncState.${item.syncState}`)}
          title={t(`holdings.syncState.${item.syncState}`)}
        />
      ) : null}
    </span>
  );
};

/** Two columns read down, then across; the fixed figure columns keep both in line. */
const TwoColumnList = ({ rows, children }: { rows: HoldingsItem[]; children: (item: HoldingsItem) => ReactNode }) => (
  <ul
    className={cn(
      "grid gap-x-8 gap-y-0.5",
      rows.length > 1 &&
        "@min-[44rem]/holdings:grid-flow-col @min-[44rem]/holdings:grid-cols-2 @min-[44rem]/holdings:grid-rows-[repeat(var(--holding-rows),auto)]",
    )}
    style={{ "--holding-rows": Math.ceil(rows.length / 2) } as CSSProperties}
  >
    {rows.map((item) => (
      <li key={rowKey(item)} className="min-w-0">
        {children(item)}
      </li>
    ))}
  </ul>
);

/** The sources as a list: icon, name, latest activity, value, share. */
export const HoldingsListLayout = ({ rows, hideSensitive, valueLabel }: HoldingsLayoutProps) => (
  <TwoColumnList rows={rows}>
    {(item) => (
      <SourceLink
        item={item}
        className="grid h-7 min-w-0 grid-cols-[1.5rem_minmax(0,1fr)_6.5rem_2.5rem] items-center gap-2 rounded-md px-1 text-xs @min-[56rem]/holdings:grid-cols-[1.5rem_minmax(0,1fr)_5.5rem_6.5rem_2.5rem]"
      >
        <span className="flex justify-center">
          <SourceIcon item={item} />
        </span>
        <SourceName item={item} />
        <span className="hidden truncate text-right text-muted-foreground tabular-nums @min-[56rem]/holdings:block">
          {item.lastTransactionAt?.slice(0, 10) ?? ""}
        </span>
        <span
          className={cn(
            "truncate text-right tabular-nums",
            item.empty ? "text-muted-foreground" : "font-medium",
            blurClass(hideSensitive),
          )}
        >
          {valueLabel(item.btc, item.value)}
        </span>
        <span className={cn("text-right text-muted-foreground tabular-nums", blurClass(hideSensitive))}>
          {shareLabel(item)}
        </span>
      </SourceLink>
    )}
  </TwoColumnList>
);

/**
 * A tile per source: the figure large, then its bitcoin, share, latest
 * activity and transaction count. A few sources fill the row; many wrap.
 */
export const HoldingsTilesLayout = ({ rows, hideSensitive, valueLabel, fiatLabel, isBitcoinMode }: HoldingsLayoutProps) => {
  const { t } = useTranslation("overview");
  return (
    <ul className="grid grid-cols-[repeat(auto-fit,minmax(13.5rem,1fr))] gap-2">
      {rows.map((item) => {
        // The other unit underneath: bitcoin under fiat, fiat under bitcoin.
        const secondary = isBitcoinMode ? fiatLabel(item.value) ?? "" : formatBtc(item.btc, { precision: 8 });
        return (
          <li key={rowKey(item)} className="min-w-0">
            <SourceLink
              item={item}
              className={cn(
                "flex h-full min-w-0 flex-col gap-1.5 rounded-lg border bg-background/40 p-3 text-xs",
                item.empty && "border-dashed opacity-70",
              )}
            >
              <div className="flex min-w-0 items-center gap-2">
                <SourceIcon item={item} />
                <SourceName item={item} />
                <span className={cn("ml-auto shrink-0 text-muted-foreground tabular-nums", blurClass(hideSensitive))}>
                  {shareLabel(item)}
                </span>
              </div>
              <div className={cn("text-lg font-semibold tabular-nums", blurClass(hideSensitive))}>
                {valueLabel(item.btc, item.value)}
              </div>
              <div className={cn("text-muted-foreground tabular-nums", blurClass(hideSensitive))}>
                {secondary}
              </div>
              {item.connectionId ? (
                <div className="mt-auto flex items-center justify-between gap-2 border-t pt-1.5 text-2xs text-muted-foreground tabular-nums">
                  <span>{item.lastTransactionAt?.slice(0, 10) ?? "—"}</span>
                  <span>{t("holdings.transactions", { count: item.transactionCount ?? 0 })}</span>
                </div>
              ) : null}
            </SourceLink>
          </li>
        );
      })}
    </ul>
  );
};

/** Every figure in columns, for the reader who wants exact numbers. */
export const HoldingsTableLayout = ({ rows, hideSensitive, fiatLabel }: HoldingsLayoutProps) => {
  const { t } = useTranslation("overview");
  const head = "px-2 py-1.5 font-medium text-muted-foreground";
  const cell = "px-2 py-1.5 tabular-nums";
  return (
    <div className="overflow-x-auto">
      <table className="w-full min-w-[44rem] text-xs">
        <thead>
          <tr className="border-b text-left">
            <th className={cn(head, "w-[24%]")}>{t("holdings.table.source")}</th>
            <th className={head}>{t("holdings.table.network")}</th>
            <th className={cn(head, "text-right")}>{t("holdings.table.bitcoin")}</th>
            <th className={cn(head, "text-right")}>{t("holdings.table.value")}</th>
            <th className={cn(head, "text-right")}>{t("holdings.table.share")}</th>
            <th className={cn(head, "text-right")}>{t("holdings.table.transactions")}</th>
            <th className={cn(head, "text-right")}>{t("holdings.table.lastActivity")}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((item) => (
            <tr key={rowKey(item)} className="border-b border-border/50 last:border-0 hover:bg-muted/30">
              <td className="max-w-0 px-1 py-0.5">
                <SourceLink item={item} className="flex min-w-0 items-center gap-2 rounded-md px-1 py-1">
                  <span className="flex w-5 justify-center">
                    <SourceIcon item={item} />
                  </span>
                  <SourceName item={item} />
                </SourceLink>
              </td>
              <td className="px-2 py-1.5 text-muted-foreground">
                {item.connectionId ? t(`holdings.rail.${item.group ?? "other"}`) : ""}
              </td>
              <td className={cn(cell, "text-right", item.empty && "text-muted-foreground", blurClass(hideSensitive))}>
                {formatBtc(item.btc, { precision: 8 })}
              </td>
              <td className={cn(cell, "text-right font-medium", item.empty && "font-normal text-muted-foreground", blurClass(hideSensitive))}>
                {fiatLabel(item.value) ?? "—"}
              </td>
              <td className={cn(cell, "text-right text-muted-foreground", blurClass(hideSensitive))}>
                {shareLabel(item)}
              </td>
              <td className={cn(cell, "text-right text-muted-foreground")}>
                {item.connectionId ? item.transactionCount ?? 0 : ""}
              </td>
              <td className={cn(cell, "text-right text-muted-foreground")}>
                {item.lastTransactionAt?.slice(0, 10) ?? ""}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
};

/**
 * The list with each source's last twelve months beside it: which wallets
 * fill up, which drain, and by how much.
 */
export const HoldingsTrendLayout = ({ rows, hideSensitive, valueLabel }: HoldingsLayoutProps) => {
  const { t } = useTranslation("overview");
  return (
    <TwoColumnList rows={rows}>
      {(item) => {
        const trend = item.trend ?? [];
        const change = trend.length > 1 ? trend[trend.length - 1] - trend[0] : 0;
        const direction = Math.abs(change) < 1e-8 ? "flat" : change > 0 ? "up" : "down";
        return (
          <SourceLink
            item={item}
            className="grid h-8 min-w-0 grid-cols-[1.5rem_minmax(0,1fr)_6rem_5rem_6.5rem_2.5rem] items-center gap-2 rounded-md px-1 text-xs"
          >
            <span className="flex justify-center">
              <SourceIcon item={item} />
            </span>
            <SourceName item={item} />
            <span className={blurClass(hideSensitive)}>
              {trend.length > 1 ? <Sparkline values={trend} direction={direction} /> : null}
            </span>
            <span
              className={cn(
                "truncate text-right tabular-nums",
                direction === "up"
                  ? "text-emerald-600 dark:text-emerald-400"
                  : direction === "down"
                    ? "text-red-600 dark:text-red-400"
                    : "text-muted-foreground",
                blurClass(hideSensitive),
              )}
              title={trend.length > 1 ? t("holdings.trendChange", { change: formatBtc(change, { precision: 4, sign: true }) }) : undefined}
            >
              {trend.length < 2 ? "" : direction === "flat" ? "—" : formatBtc(change, { precision: 3, sign: true })}
            </span>
            <span
              className={cn(
                "truncate text-right tabular-nums",
                item.empty ? "text-muted-foreground" : "font-medium",
                blurClass(hideSensitive),
              )}
            >
              {valueLabel(item.btc, item.value)}
            </span>
            <span className={cn("text-right text-muted-foreground tabular-nums", blurClass(hideSensitive))}>
              {shareLabel(item)}
            </span>
          </SourceLink>
        );
      }}
    </TwoColumnList>
  );
};

/** A line of month-end balances scaled to its own range: the shape, not the magnitude. */
const Sparkline = ({ values, direction }: { values: number[]; direction: "up" | "down" | "flat" }) => {
  const width = 96;
  const height = 20;
  const min = Math.min(...values);
  const max = Math.max(...values);
  const span = max - min || 1;
  const points = values
    .map((value, index) => {
      const x = (index / (values.length - 1)) * width;
      const y = height - 1 - ((value - min) / span) * (height - 2);
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
  const stroke =
    direction === "up" ? "rgb(16 185 129)" : direction === "down" ? "rgb(239 68 68)" : "var(--muted-foreground)";
  return (
    <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} aria-hidden="true" className="block">
      <polyline points={points} fill="none" stroke={stroke} strokeWidth="1.5" strokeLinejoin="round" />
    </svg>
  );
};
