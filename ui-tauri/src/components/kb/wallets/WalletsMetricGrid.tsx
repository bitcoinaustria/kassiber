import { AlertTriangle, CheckCircle2, Wallet } from "lucide-react";
import { useTranslation } from "react-i18next";

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
  const { t } = useTranslation("connections");
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
      ? t("metrics.upToDateRefreshingNow", { count: syncingCount })
      : unsyncedCount === 0
        ? t("metrics.upToDateAllSources")
        : t("metrics.upToDateNotUpToDate", { count: unsyncedCount });

  return (
    <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
      <MetricCard
        label={t("metrics.totalBalance")}
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
        label={t("metrics.upToDate")}
        icon={<CheckCircle2 className="size-4" aria-hidden="true" />}
        value={`${syncedCount.toLocaleString("en-US")} / ${connections.length.toLocaleString("en-US")}`}
        detail={upToDateDetail}
      />
      <MetricCard
        label={t("metrics.needsAttention")}
        icon={<AlertTriangle className="size-4" aria-hidden="true" />}
        value={errorCount.toLocaleString("en-US")}
        detail={
          errorCount > 0
            ? t("metrics.needsAttentionFailed")
            : t("metrics.needsAttentionNoFailed")
        }
      />
    </div>
  );
}
