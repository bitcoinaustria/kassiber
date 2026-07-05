import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
import type { Currency } from "@/lib/currency";
import type { Connection } from "@/mocks/seed";

import {
  formatBtc,
  formatEur,
  hiddenSensitiveClassName,
} from "./format";

function WalletsOverviewStat({
  label,
  value,
  detail,
  link,
}: {
  label: string;
  value: ReactNode;
  detail?: ReactNode;
  link?: {
    to: "/transactions";
    search?: {
      quick?: "review_queue";
    };
    hash?: string;
    ariaLabel: string;
  };
}) {
  const content = (
    <div className="pointer-events-none relative z-20 space-y-1.5">
      <div className="text-muted-foreground">
        <span className="text-xs font-medium">{label}</span>
      </div>
      <p className="text-lg font-semibold tracking-tight tabular-nums sm:text-xl">
        {value}
      </p>
      {detail != null ? (
        <p className="truncate text-[10px] font-medium leading-tight text-muted-foreground sm:text-xs">
          {detail}
        </p>
      ) : null}
    </div>
  );

  return link ? (
    <Link
      to={link.to}
      search={link.search}
      hash={link.hash}
      aria-label={link.ariaLabel}
      className="group relative isolate block overflow-hidden p-3 transition-colors before:absolute before:inset-0 before:z-0 before:origin-left before:scale-x-0 before:bg-muted/45 before:content-[''] before:transition-transform before:duration-200 before:ease-out hover:before:scale-x-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-within:before:scale-x-100"
    >
      {content}
    </Link>
  ) : (
    <div className="group relative isolate overflow-hidden p-3 transition-colors before:absolute before:inset-0 before:z-0 before:origin-left before:scale-x-0 before:bg-muted/45 before:content-[''] before:transition-transform before:duration-200 before:ease-out hover:before:scale-x-100 focus-within:before:scale-x-100">
      {content}
    </div>
  );
}

interface WalletsMetricGridProps {
  connections: Connection[];
  currency: Currency;
  hideSensitive: boolean;
  isSyncing: boolean;
  priceEur: number;
  totalBtc: number;
}

export function WalletsMetricGrid({
  connections,
  currency,
  hideSensitive,
  isSyncing,
  priceEur,
  totalBtc,
}: WalletsMetricGridProps) {
  const { t } = useTranslation("connections");
  const totalEur = totalBtc * priceEur;
  const totalTransactions = connections.reduce(
    (sum, connection) => sum + (connection.transactionCount ?? 0),
    0,
  );
  const errorCount = connections.filter((c) => c.status === "error").length;
  const snapshotSyncingCount = connections.filter(
    (c) => c.status === "syncing",
  ).length;
  const syncingCount = isSyncing ? connections.length : snapshotSyncingCount;
  const syncedCount = connections.filter((c) => c.status === "synced").length;
  const unsyncedCount = connections.length - syncedCount;
  const upToDateDetail =
    syncingCount > 0
      ? t("metrics.upToDateRefreshingNow", { count: syncingCount })
      : unsyncedCount === 0
        ? t("metrics.upToDateAllSources")
        : t("metrics.upToDateNotUpToDate", { count: unsyncedCount });

  return (
    <div className="overflow-hidden rounded-lg border bg-card">
      <div className="grid grid-cols-1 divide-x-0 divide-y divide-border sm:grid-cols-2 sm:divide-y-0 xl:grid-cols-4 xl:divide-x">
        <WalletsOverviewStat
          label={t("metrics.totalBalance")}
          value={
            <span className={hiddenSensitiveClassName(hideSensitive)}>
              <CurrencyToggleText>
                {currency === "eur"
                  ? formatEur(totalEur)
                  : `₿ ${formatBtc(totalBtc)}`}
              </CurrencyToggleText>
            </span>
          }
          detail={
            <CurrencyToggleText>
              {currency === "eur"
                ? `₿ ${formatBtc(totalBtc)}`
                : formatEur(totalEur)}
            </CurrencyToggleText>
          }
        />
        <WalletsOverviewStat
          label={t("metrics.totalTransactions")}
          value={totalTransactions.toLocaleString("en-US")}
          detail={t("metrics.totalTransactionsDetail")}
          link={{
            to: "/transactions",
            hash: "transactions-table",
            ariaLabel: t("metrics.openTransactions"),
          }}
        />
        <WalletsOverviewStat
          label={t("metrics.upToDate")}
          value={`${syncedCount.toLocaleString("en-US")} / ${connections.length.toLocaleString("en-US")}`}
          detail={upToDateDetail}
        />
        <WalletsOverviewStat
          label={t("metrics.needsAttention")}
          value={errorCount.toLocaleString("en-US")}
          detail={
            errorCount > 0
              ? t("metrics.needsAttentionFailed")
              : t("metrics.needsAttentionNoFailed")
          }
          link={{
            to: "/transactions",
            search: { quick: "review_queue" },
            hash: "transactions-table",
            ariaLabel: t("metrics.openReview"),
          }}
        />
      </div>
    </div>
  );
}
