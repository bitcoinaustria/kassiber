import { Link } from "@tanstack/react-router";
import {
  ArrowLeftRight,
  ArrowUpRight,
  MoreHorizontal,
  PieChartIcon,
} from "lucide-react";
import { Cell, Pie, PieChart } from "recharts";

import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
import { Button } from "@/components/ui/button";
import { ChartContainer } from "@/components/ui/chart";
import { type Currency } from "@/lib/currency";
import { cn } from "@/lib/utils";
import type { OverviewSnapshot } from "@/mocks/seed";

import {
  activeMarketFiatCurrency,
  blurClass,
  buildBalanceDrivers,
  buildBalanceRailItems,
  buildHoldingsBySource,
  donutCenterValueClass,
  formatCompactDisplayMoney,
  formatDriverValue,
  formatSignedDisplayMoney,
  holdingsChartConfig,
  transactionsDriverSearch,
  useHoverHighlight,
} from "./model";

export const BalanceDriversCard = ({
  snapshot,
  hideSensitive,
  currency,
}: {
  snapshot: OverviewSnapshot;
  hideSensitive: boolean;
  currency: Currency;
}) => {
  const { rows, maxValueBtc, netBtc, transactionCount } =
    buildBalanceDrivers(snapshot);
  const fiatCurrency = activeMarketFiatCurrency(snapshot);
  const netEur = netBtc * snapshot.priceEur;
  const netTone =
    netBtc > 0
      ? "text-emerald-700 dark:text-emerald-300"
      : netBtc < 0
        ? "text-red-700 dark:text-red-300"
        : "text-muted-foreground";

  return (
    <div className="flex flex-col gap-3 rounded-xl border bg-card p-3 sm:p-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 sm:gap-2.5">
          <Button
            variant="outline"
            size="icon"
            className="size-7 sm:size-8"
            aria-label="Balance drivers"
          >
            <ArrowLeftRight className="size-4 text-muted-foreground sm:size-[18px]" />
          </Button>
          <div>
            <span className="text-sm font-medium">Balance Drivers</span>
            <p
              className={cn(
                "text-[10px] text-muted-foreground sm:text-xs",
                blurClass(hideSensitive),
              )}
            >
              Latest {transactionCount.toLocaleString("en-US")} transactions
            </p>
          </div>
        </div>
        <CurrencyToggleText
          className={cn(
            "text-right text-sm font-semibold tabular-nums",
            netTone,
            blurClass(hideSensitive),
          )}
        >
          {formatSignedDisplayMoney(
            netEur,
            snapshot.priceEur,
            currency,
            fiatCurrency,
          )}
        </CurrencyToggleText>
      </div>

      <div className="space-y-2.5">
        {rows.map((item) => {
          const Icon = item.icon;
          const width =
            maxValueBtc > 0 ? Math.max((item.valueBtc / maxValueBtc) * 100, 4) : 0;
          return (
            <Link
              key={item.key}
              to="/transactions"
              search={transactionsDriverSearch(item.key)}
              className="-mx-1 grid gap-1.5 rounded-md px-1 py-1 transition-colors hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              aria-label={`View ${item.label.toLowerCase()} transactions`}
            >
              <div className="flex items-center justify-between gap-3">
                <div className="flex min-w-0 items-center gap-2">
                  <Icon className={cn("size-3.5 shrink-0", item.toneClassName)} />
                  <span className="truncate text-xs text-muted-foreground">
                    {item.label}
                  </span>
                  <span className="text-[10px] text-muted-foreground">
                    {item.count}
                  </span>
                </div>
                <span
                  className={cn(
                    "shrink-0 text-xs font-medium tabular-nums",
                    item.toneClassName,
                    blurClass(hideSensitive),
                  )}
                >
                  {formatDriverValue(
                    item.valueBtc,
                    snapshot.priceEur,
                    currency,
                    fiatCurrency,
                  )}
                </span>
              </div>
              <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                <div
                  className={cn(
                    "h-full rounded-full",
                    item.key === "incoming"
                      ? "bg-emerald-500"
                      : item.key === "outgoing"
                        ? "bg-red-500"
                        : item.key === "swap"
                          ? "bg-sky-500"
                          : "bg-muted-foreground/60",
                  )}
                  style={{ width: `${width}%` }}
                />
              </div>
            </Link>
          );
        })}
      </div>
    </div>
  );
};

export const HoldingsBySourceChart = ({
  snapshot,
  hideSensitive,
  currency,
}: {
  snapshot: OverviewSnapshot;
  hideSensitive: boolean;
  currency: Currency;
}) => {
  const isBitcoinMode = currency === "btc";
  const fiatCurrency = activeMarketFiatCurrency(snapshot);
  const { active: activeSlice, handleHover: setHoveredSlice } =
    useHoverHighlight<number>();
  const holdingsData = buildHoldingsBySource(snapshot);
  const { items: balanceRailItems } = buildBalanceRailItems(snapshot);
  const unrealizedPercent = snapshot.fiat.eurCostBasis
    ? (snapshot.fiat.eurUnrealized / snapshot.fiat.eurCostBasis) * 100
    : 0;
  const totalHoldings = holdingsData.reduce(
    (acc, item) => acc + item.value,
    0,
  );
  const totalHoldingsLabel = formatCompactDisplayMoney(
    totalHoldings,
    snapshot.priceEur,
    currency,
    fiatCurrency,
  );
  const singleHolding = holdingsData.length === 1 ? holdingsData[0] : null;

  return (
    <div className="flex flex-1 flex-col gap-3 rounded-xl border bg-card p-3 sm:p-4">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2 sm:gap-2.5">
          <Button
            variant="outline"
            size="icon"
            className="size-7 sm:size-8"
            aria-label="Holdings by source"
          >
            <PieChartIcon className="size-4 text-muted-foreground sm:size-[18px]" />
          </Button>
          <div className="min-w-0">
            <span className="text-sm font-medium">
              Holdings by Source
            </span>
            {isBitcoinMode ? (
              <p className="text-[10px] text-muted-foreground sm:text-xs">
                BTC allocation
              </p>
            ) : (
              <p className="flex items-center gap-1 text-[10px] text-muted-foreground sm:text-xs">
                <ArrowUpRight
                  className={cn(
                    "size-3",
                    unrealizedPercent >= 0
                      ? "text-emerald-600"
                      : "text-red-600",
                  )}
                  aria-hidden="true"
                />
                <span
                  className={cn(
                    unrealizedPercent >= 0
                      ? "text-emerald-600"
                      : "text-red-600",
                    blurClass(hideSensitive),
                  )}
                >
                  {unrealizedPercent >= 0 ? "+" : ""}
                  {unrealizedPercent.toFixed(1)}%
                </span>
                <span>vs cost basis</span>
              </p>
            )}
          </div>
        </div>
        <Button
          variant="ghost"
          size="icon"
          className="size-7 sm:size-8"
          aria-label="More options"
        >
          <MoreHorizontal className="size-4 text-muted-foreground" />
        </Button>
      </div>

      {balanceRailItems.length > 1 ? (
        <div className="flex flex-wrap gap-1.5">
          {balanceRailItems.map((item) => (
            <span
              key={item.key}
              className="inline-flex items-center gap-1 rounded-md border bg-background/55 px-1.5 py-0.5 text-[10px] text-muted-foreground"
            >
              <span
                className="size-1.5 rounded-full"
                style={{ backgroundColor: item.color }}
              />
              {item.label}
              <span
                className={cn("tabular-nums", blurClass(hideSensitive))}
              >
                {item.percent}%
              </span>
            </span>
          ))}
        </div>
      ) : null}

      {singleHolding ? (
        <div className="flex flex-1 items-center rounded-md bg-muted/25 px-3 py-3">
          <div
            className="flex min-w-0 flex-1 items-start gap-2"
            onMouseEnter={() => setHoveredSlice(0)}
            onMouseLeave={() => setHoveredSlice(null)}
          >
            <span
              className="mt-1 size-2.5 shrink-0 rounded-full"
              style={{ backgroundColor: singleHolding.color }}
            />
            <div className="min-w-0">
              <p className="break-words text-sm font-medium leading-5">
                {singleHolding.name}
              </p>
              <p className="text-[10px] text-muted-foreground sm:text-xs">
                Only active source
              </p>
            </div>
          </div>
          <div className="ml-3 shrink-0 text-right">
            <p
              className={cn(
                "text-sm font-semibold tabular-nums",
                blurClass(hideSensitive),
              )}
            >
              {formatCompactDisplayMoney(
                singleHolding.value,
                snapshot.priceEur,
                currency,
                fiatCurrency,
              )}
            </p>
            <p
              className={cn(
                "text-[10px] text-muted-foreground tabular-nums sm:text-xs",
                blurClass(hideSensitive),
              )}
            >
              {singleHolding.percent}%
            </p>
          </div>
        </div>
      ) : (
      <div className="grid flex-1 items-center gap-3 sm:grid-cols-[minmax(104px,0.85fr)_minmax(0,1.15fr)]">
        <div className="relative mx-auto size-[116px] shrink-0 sm:size-[128px] xl:size-[136px]">
          <ChartContainer
            config={holdingsChartConfig}
            className="h-full w-full"
          >
            <PieChart>
              <Pie
                data={holdingsData}
                cx="50%"
                cy="50%"
                innerRadius="55%"
                outerRadius="90%"
                paddingAngle={2}
                dataKey="value"
                strokeWidth={0}
                onMouseEnter={(_: unknown, index: number) =>
                  setHoveredSlice(index)
                }
                onMouseLeave={() => setHoveredSlice(null)}
              >
                {holdingsData.map((entry) => (
                  <Cell key={entry.name} fill={entry.color} />
                ))}
              </Pie>
            </PieChart>
          </ChartContainer>
          <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center">
            <span
              className={cn(
                "max-w-[96px] whitespace-nowrap text-center leading-tight font-semibold tabular-nums sm:max-w-[116px]",
                donutCenterValueClass(totalHoldingsLabel),
                blurClass(hideSensitive),
              )}
            >
              {totalHoldingsLabel}
            </span>
            <span className="text-[8px] text-muted-foreground sm:text-[10px]">
              Total
            </span>
          </div>
        </div>

        <div className="flex min-w-0 flex-col gap-2 sm:gap-3">
          {holdingsData.map((item, index) => (
            <div
              key={item.name}
              className={cn(
                "flex items-start justify-between gap-2 transition-opacity duration-200 motion-reduce:transition-none",
                activeSlice !== null && activeSlice !== index && "opacity-50",
              )}
              onMouseEnter={() => setHoveredSlice(index)}
              onMouseLeave={() => setHoveredSlice(null)}
            >
              <div className="flex min-w-0 flex-1 items-start gap-2">
                <div
                  className="mt-1 size-2 shrink-0 rounded-full sm:size-2.5"
                  style={{ backgroundColor: item.color }}
                />
                <span className="min-w-0 break-words text-[10px] leading-4 text-muted-foreground sm:text-xs">
                  {item.name}
                </span>
              </div>
              <div className="flex shrink-0 flex-wrap justify-end gap-x-1.5 text-[10px] sm:text-xs">
                <span
                  className={cn(
                    "font-medium tabular-nums",
                    blurClass(hideSensitive),
                  )}
                >
                  {formatCompactDisplayMoney(
                    item.value,
                    snapshot.priceEur,
                    currency,
                    fiatCurrency,
                  )}
                </span>
                <span
                  className={cn(
                    "text-muted-foreground tabular-nums",
                    blurClass(hideSensitive),
                  )}
                >
                  {item.percent}%
                </span>
              </div>
            </div>
          ))}
        </div>
      </div>
      )}
    </div>
  );
};

export const OverviewSidePanel = ({
  className,
  snapshot,
  hideSensitive,
  currency,
}: {
  className?: string;
  snapshot: OverviewSnapshot;
  hideSensitive: boolean;
  currency: Currency;
}) => {
  return (
    <div
      className={cn(
        "grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-1",
        className,
      )}
    >
      <BalanceDriversCard
        snapshot={snapshot}
        hideSensitive={hideSensitive}
        currency={currency}
      />
      <HoldingsBySourceChart
        snapshot={snapshot}
        hideSensitive={hideSensitive}
        currency={currency}
      />
    </div>
  );
};
