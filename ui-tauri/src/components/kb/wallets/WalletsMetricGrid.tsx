import { AlertTriangle, CheckCircle2, Wallet } from "lucide-react";

import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
import { MetricCard } from "@/components/kb/MetricCard";
import type { Currency } from "@/lib/currency";
import type { Connection } from "@/mocks/seed";

import {
  formatBtc,
  formatEur,
  hiddenSensitiveClassName,
} from "./format";

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
  const totalEur = totalBtc * priceEur;
  const errorCount = connections.filter((c) => c.status === "error").length;
  const snapshotSyncingCount = connections.filter(
    (c) => c.status === "syncing",
  ).length;
  const syncingCount = isSyncing ? connections.length : snapshotSyncingCount;
  const syncedCount = connections.filter((c) => c.status === "synced").length;
  const unsyncedCount = connections.length - syncedCount;
  const upToDateDetail =
    syncingCount > 0
      ? `${syncingCount.toLocaleString("en-US")} refreshing now`
      : unsyncedCount === 0
        ? "All configured sources"
        : `${unsyncedCount.toLocaleString("en-US")} not yet up to date`;

  return (
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
      <MetricCard
        label="Total balance"
        icon={<Wallet className="size-4" aria-hidden="true" />}
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
      <MetricCard
        label="Up to date"
        icon={<CheckCircle2 className="size-4" aria-hidden="true" />}
        value={`${syncedCount.toLocaleString("en-US")} / ${connections.length.toLocaleString("en-US")}`}
        detail={upToDateDetail}
      />
      <MetricCard
        label="Needs attention"
        icon={<AlertTriangle className="size-4" aria-hidden="true" />}
        value={errorCount.toLocaleString("en-US")}
        detail={errorCount > 0 ? "Failed source(s)" : "No failed sources"}
      />
    </div>
  );
}
