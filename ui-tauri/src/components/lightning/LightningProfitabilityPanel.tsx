/**
 * Reports panel for `ui.reports.lightning_profitability`.
 *
 * Self-contained: it reads `ui.overview.snapshot` for the list of
 * Lightning-kind connections, lets the user pick one, and renders the
 * routing summary tiles plus the per-channel covers-open-cost table.
 */

import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { TrendingUp, Zap } from "lucide-react";

import { useDaemon, retryRetryableDaemonError } from "@/daemon/client";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  DEFAULT_OPEN_COST_SAT,
  connectionSupportsLightningCapability,
  LIGHTNING_CONNECTION_KINDS,
} from "@/lib/lightning";
import { cn } from "@/lib/utils";
import { formatCount, formatSats } from "@/lib/localeFormat";
import type { Connection, OverviewSnapshot } from "@/mocks/seed";

interface LightningProfitabilityReport {
  connection: {
    id: string;
    label: string;
    kind: string;
    lightningCapabilities?: unknown;
  };
  windowLabel: string;
  summary: {
    routingRevenueSat: number;
    paymentCostSat: number;
    rebalanceCostSat: number;
    onchainCostSat: number;
    netProfitSat: number;
    forwardCount: number;
    paymentCount: number;
    rebalanceCount: number;
  };
  channels: Array<{
    channelId: string;
    peerAlias: string;
    capacitySat: number;
    earnedRoutingSat: number;
    openCostSat: number;
    coversOpenCost: boolean;
  }>;
}

const fmtSat = (value: number) => formatSats(value, { unit: "sat" });
const fmtSatSigned = (value: number) =>
  `${value >= 0 ? "+ " : "- "}${formatSats(Math.abs(value), { unit: "sat" })}`;

export function LightningProfitabilityPanel() {
  const { t } = useTranslation("connections");
  const overview = useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const lightningConnections = useMemo(
    () =>
      (overview.data?.data?.connections ?? []).filter((connection) =>
        LIGHTNING_CONNECTION_KINDS.has(connection.kind),
      ),
    [overview.data],
  );
  const reportableConnections = useMemo(
    () =>
      lightningConnections.filter((connection) =>
        connectionSupportsLightningCapability(
          connection,
          "routingProfitability",
        ),
      ),
    [lightningConnections],
  );
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const activeConnection: Connection | null =
    reportableConnections.find((connection) => connection.id === selectedId) ??
    reportableConnections[0] ??
    null;

  if (lightningConnections.length === 0) {
    return null;
  }

  return (
    <Card>
      <CardHeader className="border-b px-4 pb-3">
        <CardTitle className="flex items-center gap-2 text-sm sm:text-base">
          <TrendingUp className="size-4" aria-hidden="true" />
          {t("node.profitabilityReport.title")}
        </CardTitle>
        <CardDescription>
          {t("node.profitabilityReport.description", {
            cost: formatCount(DEFAULT_OPEN_COST_SAT),
          })}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4 px-4 pt-4">
        {reportableConnections.length === 0 ? (
          <div className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-700 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-300">
            {t("node.profitabilityReport.unsupported")}
          </div>
        ) : (
          <>
            <div className="flex flex-wrap items-center gap-2">
              <Label
                className="text-xs text-muted-foreground"
                htmlFor="ln-profitability-connection"
              >
                {t("node.profitabilityReport.connection")}
              </Label>
              <select
                id="ln-profitability-connection"
                className="h-8 rounded-md border border-input bg-background px-2 text-sm"
                value={activeConnection?.id ?? ""}
                onChange={(event) => setSelectedId(event.target.value)}
              >
                {reportableConnections.map((connection) => (
                  <option key={connection.id} value={connection.id}>
                    {connection.label}
                  </option>
                ))}
              </select>
            </div>
            {activeConnection ? (
              <LightningProfitabilityBody connectionId={activeConnection.id} />
            ) : null}
          </>
        )}
      </CardContent>
    </Card>
  );
}

function LightningProfitabilityBody({
  connectionId,
}: {
  connectionId: string;
}) {
  const { t } = useTranslation("connections");
  const profitability = useDaemon<LightningProfitabilityReport>(
    "ui.reports.lightning_profitability",
    { connection: connectionId },
    { retry: retryRetryableDaemonError },
  );
  if (profitability.isLoading) {
    return (
      <div className="text-sm text-muted-foreground">
        {t("node.profitabilityReport.loading")}
      </div>
    );
  }
  if (profitability.isError || profitability.data?.error) {
    const message =
      profitability.error instanceof Error
        ? profitability.error.message
        : profitability.data?.error?.message ??
          t("node.profitabilityReport.errorFallback");
    return (
      <div className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-700 dark:border-amber-900/60 dark:bg-amber-950/40 dark:text-amber-300">
        {message}
      </div>
    );
  }
  const report = profitability.data?.data;
  if (!report) {
    return (
      <div className="text-sm text-muted-foreground">
        {t("node.profitabilityReport.empty")}
      </div>
    );
  }
  const { summary, channels, windowLabel } = report;
  const profitTone =
    summary.netProfitSat > 0
      ? "text-emerald-700 dark:text-emerald-300"
      : summary.netProfitSat < 0
        ? "text-red-700 dark:text-red-300"
        : "text-muted-foreground";
  return (
    <div className="space-y-4">
      <div className="text-xs text-muted-foreground">
        {t("node.profitabilityReport.windowSummary", {
          window: windowLabel,
          forwards: formatCount(summary.forwardCount),
          payments: formatCount(summary.paymentCount),
          rebalances: formatCount(summary.rebalanceCount),
        })}
      </div>
      <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-5">
        <Stat
          label={t("node.profitabilityReport.revenue")}
          value={fmtSatSigned(summary.routingRevenueSat)}
          tone="text-emerald-700 dark:text-emerald-300"
        />
        <Stat
          label={t("node.profitabilityReport.paymentFees")}
          value={fmtSatSigned(-summary.paymentCostSat)}
          tone="text-red-700 dark:text-red-300"
        />
        <Stat
          label={t("node.profitabilityReport.rebalanceFees")}
          value={fmtSatSigned(-summary.rebalanceCostSat)}
          tone="text-red-700 dark:text-red-300"
        />
        <Stat
          label={t("node.profitabilityReport.onChainCosts")}
          value={fmtSatSigned(-summary.onchainCostSat)}
          tone="text-red-700 dark:text-red-300"
        />
        <Stat
          label={t("node.profitabilityReport.netProfit")}
          value={fmtSatSigned(summary.netProfitSat)}
          tone={profitTone}
        />
      </div>
      <div className="overflow-hidden rounded-md border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("node.profitabilityReport.peer")}</TableHead>
              <TableHead className="text-right">
                {t("node.profitabilityReport.capacity")}
              </TableHead>
              <TableHead className="text-right">
                {t("node.profitabilityReport.earnedRouting")}
              </TableHead>
              <TableHead className="text-right">
                {t("node.profitabilityReport.openCost")}
              </TableHead>
              <TableHead className="text-right">
                {t("node.profitabilityReport.coversOpenCost")}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {channels.length === 0 ? (
              <TableRow>
                <TableCell
                  className="text-center text-sm text-muted-foreground"
                  colSpan={5}
                >
                  {t("node.profitabilityReport.noChannels")}
                </TableCell>
              </TableRow>
            ) : (
              channels.map((channel) => (
                <TableRow key={channel.channelId}>
                  <TableCell className="font-medium">
                    <span className="flex items-center gap-1.5">
                      <Zap className="size-3 text-amber-500" aria-hidden="true" />
                      {channel.peerAlias}
                    </span>
                  </TableCell>
                  <TableCell className="text-right font-mono text-sm tabular-nums">
                    {fmtSat(channel.capacitySat)}
                  </TableCell>
                  <TableCell className="text-right font-mono text-sm tabular-nums">
                    {fmtSat(channel.earnedRoutingSat)}
                  </TableCell>
                  <TableCell className="text-right font-mono text-sm tabular-nums">
                    {fmtSat(channel.openCostSat)}
                  </TableCell>
                  <TableCell className="text-right">
                    <span
                      className={cn(
                        "inline-flex items-center rounded-md px-1.5 py-0.5 text-[10px] font-medium ring-1 ring-inset",
                        channel.coversOpenCost
                          ? "bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-300 dark:ring-emerald-400/20"
                          : "bg-muted text-muted-foreground ring-border",
                      )}
                    >
                      {channel.coversOpenCost
                        ? t("node.profitabilityReport.yes")
                        : t("node.profitabilityReport.notYet")}
                    </span>
                  </TableCell>
                </TableRow>
              ))
            )}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}

function Stat({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone: string;
}) {
  return (
    <div className="space-y-1 rounded-md border bg-background px-3 py-2.5">
      <span className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </span>
      <p
        className={cn(
          "font-mono text-sm font-semibold tabular-nums",
          tone,
        )}
      >
        {value}
      </p>
    </div>
  );
}
