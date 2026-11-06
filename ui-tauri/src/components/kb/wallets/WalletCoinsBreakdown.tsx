/**
 * How a wallet's unspent coins are made up: how many sit in each size band
 * and how long they have sat since confirmation. Both are groupings of the
 * watch-only UTXO inventory the daemon already returned — chain facts, not
 * tax lots — so a coin's age here says nothing about its holding period.
 * The size bands answer "how fragmented is this wallet" (many small coins
 * cost more to spend); the age bands answer "how much of it is dormant".
 * With values hidden, the size bands show nothing: how many coins sit in
 * each band bounds the balance.
 */

import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import {
  portfolioChartColors,
  useResolvedColorMode,
} from "@/components/overview-dashboard/model";
import { formatBtc } from "@/lib/currency";
import { formatCount } from "@/lib/localeFormat";
import { cn } from "@/lib/utils";

import type { WalletUtxoRow } from "./UtxosInventoryPanel";

const BTC_ASSETS = new Set(["BTC", "LBTC", "L-BTC"]);
const DAY_MS = 24 * 60 * 60 * 1000;

// Decimal bands, so each edge is a round BTC figure.
const SIZE_EDGES_SAT = [100_000, 1_000_000, 10_000_000, 100_000_000];

type AgeBand = "unconfirmed" | "month" | "year" | "threeYears" | "older";
const AGE_BANDS: AgeBand[] = ["month", "year", "threeYears", "older"];

interface Band {
  key: string;
  label: string;
  count: number;
  sat: number;
}

/** `₿ 0.001` for 100,000 sat: a band edge with only its significant digit. */
function edgeLabel(sat: number) {
  return formatBtc(sat / 1e8, {
    precision: Math.max(0, Math.round(8 - Math.log10(sat))),
  });
}

function ageBand(row: WalletUtxoRow, now: number): AgeBand {
  const confirmedAt = row.block_time ? Date.parse(row.block_time) : Number.NaN;
  if (row.confirmation_status !== "confirmed" || !Number.isFinite(confirmedAt)) {
    return "unconfirmed";
  }
  const days = (now - confirmedAt) / DAY_MS;
  if (days < 30) return "month";
  if (days < 365) return "year";
  if (days < 3 * 365) return "threeYears";
  return "older";
}

function median(values: number[]) {
  if (!values.length) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2
    ? sorted[middle]
    : Math.round((sorted[middle - 1] + sorted[middle]) / 2);
}

function BandList({
  title,
  bands,
  total,
  color,
  hideSensitive,
  maskCounts = false,
}: {
  title: string;
  bands: Band[];
  total: number;
  color: string;
  hideSensitive: boolean;
  /** Hide the counts and bars too, for bands that reveal amounts. */
  maskCounts?: boolean;
}) {
  const { t } = useTranslation("connections");
  return (
    <div className="min-w-0">
      <h3 className="mb-1 text-xs font-medium text-muted-foreground">{title}</h3>
      <ul className="divide-y divide-border/60">
        {bands.map((band) => {
          const share = total > 0 ? band.count / total : 0;
          const amount = formatBtc(band.sat / 1e8);
          return (
            <li
              key={band.key}
              className="grid grid-cols-[minmax(7.5rem,auto)_minmax(0,1fr)_minmax(9rem,auto)] items-center gap-3 py-2"
              title={
                maskCounts
                  ? undefined
                  : t("detail.coins.bandTitle", {
                      count: band.count,
                      amount: hideSensitive ? "—" : amount,
                      share: Math.round(share * 100),
                    })
              }
            >
              <span className="truncate text-xs tabular-nums">{band.label}</span>
              <span className="h-2 overflow-hidden rounded-full bg-muted" aria-hidden="true">
                {maskCounts ? null : (
                  <span
                    className="block h-full rounded-full"
                    style={{
                      width: `${share * 100}%`,
                      minWidth: band.count > 0 ? 4 : 0,
                      backgroundColor: color,
                    }}
                  />
                )}
              </span>
              <span className="text-right text-xs tabular-nums">
                {maskCounts ? (
                  <span className="sensitive text-muted-foreground">
                    {t("detail.coins.count", { count: 0 })}
                  </span>
                ) : band.count > 0 ? (
                  <>
                    <span className="font-medium text-foreground">
                      {t("detail.coins.count", { count: band.count })}
                    </span>
                    <span
                      className={cn(
                        "ml-1.5 text-muted-foreground",
                        hideSensitive && "sensitive",
                      )}
                    >
                      {amount}
                    </span>
                  </>
                ) : (
                  <span className="text-muted-foreground">—</span>
                )}
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

export function WalletCoinsBreakdown({
  rows,
  totalCount,
  hideSensitive,
}: {
  rows: WalletUtxoRow[];
  /** Every coin in the wallet, when the daemon returned only some of them. */
  totalCount?: number;
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("connections");
  const colorMode = useResolvedColorMode();
  const color = portfolioChartColors[colorMode].value;

  const summary = useMemo(() => {
    const coins = rows.filter((row) => BTC_ASSETS.has(row.asset.toUpperCase()));
    const sizes: Band[] = [
      ...SIZE_EDGES_SAT.map((edge, index) => ({
        key: `size-${edge}`,
        label:
          index === 0
            ? t("detail.coins.size.below", { max: edgeLabel(edge) })
            : t("detail.coins.size.between", {
                min: edgeLabel(SIZE_EDGES_SAT[index - 1]),
                max: edgeLabel(edge),
              }),
        count: 0,
        sat: 0,
      })),
      {
        key: "size-top",
        label: t("detail.coins.size.atLeast", {
          min: edgeLabel(SIZE_EDGES_SAT[SIZE_EDGES_SAT.length - 1]),
        }),
        count: 0,
        sat: 0,
      },
    ];
    const ages = new Map<AgeBand, Band>(
      (["unconfirmed", ...AGE_BANDS] as AgeBand[]).map((band) => [
        band,
        // dynamic key
        { key: band, label: t(`detail.coins.age.${band}` as never), count: 0, sat: 0 },
      ]),
    );
    const now = Date.now();
    for (const coin of coins) {
      const sat = coin.amount_sat;
      const sizeIndex = SIZE_EDGES_SAT.findIndex((edge) => sat < edge);
      const size = sizes[sizeIndex === -1 ? sizes.length - 1 : sizeIndex];
      size.count += 1;
      size.sat += sat;
      const age = ages.get(ageBand(coin, now));
      if (age) {
        age.count += 1;
        age.sat += sat;
      }
    }
    const amounts = coins.map((coin) => coin.amount_sat);
    const unconfirmed = ages.get("unconfirmed");
    return {
      count: coins.length,
      sizes,
      // Unconfirmed coins get a band only when there are some.
      ages: [
        ...(unconfirmed && unconfirmed.count > 0 ? [unconfirmed] : []),
        ...AGE_BANDS.map((band) => ages.get(band)!),
      ],
      largest: amounts.length ? Math.max(...amounts) : 0,
      smallest: amounts.length ? Math.min(...amounts) : 0,
      median: median(amounts),
    };
  }, [rows, t]);

  if (summary.count === 0) return null;

  return (
    <section className="kb-surface overflow-hidden">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 border-b px-3 py-2.5 sm:px-4">
        <span className="flex flex-wrap items-baseline gap-x-2">
          <span className="text-sm font-medium">{t("detail.coins.title")}</span>
          {totalCount !== undefined && totalCount > rows.length ? (
            // The daemon returns a capped inventory, oldest coins first.
            <span className="text-xs text-muted-foreground tabular-nums">
              {t("detail.coins.partial", {
                loaded: formatCount(rows.length),
                total: formatCount(totalCount),
              })}
            </span>
          ) : null}
        </span>
        <span
          className={cn(
            "text-xs text-muted-foreground tabular-nums",
            hideSensitive && "sensitive",
          )}
        >
          {t("detail.coins.extremes", {
            largest: formatBtc(summary.largest / 1e8),
            median: formatBtc(summary.median / 1e8),
            smallest: formatBtc(summary.smallest / 1e8),
          })}
        </span>
      </div>
      <div className="grid gap-x-8 gap-y-4 px-3 py-3 sm:px-4 @4xl/wallet:grid-cols-2">
        <BandList
          title={t("detail.coins.bySize")}
          bands={summary.sizes}
          total={summary.count}
          color={color}
          hideSensitive={hideSensitive}
          maskCounts={hideSensitive}
        />
        <BandList
          title={t("detail.coins.byAge")}
          bands={summary.ages}
          total={summary.count}
          color={color}
          hideSensitive={hideSensitive}
        />
      </div>
    </section>
  );
}
