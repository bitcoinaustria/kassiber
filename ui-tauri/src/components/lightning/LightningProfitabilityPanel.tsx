/**
 * Reports panel for `ui.reports.lightning_profitability`.
 *
 * Self-contained: it reads `ui.overview.snapshot` for the list of
 * Lightning-kind connections, lets the user pick one, and renders the
 * routing summary tiles plus the per-channel covers-open-cost table.
 */

import { useMemo, useState } from "react";
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
  LIGHTNING_CONNECTION_KINDS,
} from "@/lib/lightning";
import { cn } from "@/lib/utils";
import type { Connection, OverviewSnapshot } from "@/mocks/seed";

interface LightningProfitabilityReport {
  connection: {
    id: string;
    label: string;
    kind: string;
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

const fmtSat = (value: number) => `${value.toLocaleString("en-US")} sat`;
const fmtSatSigned = (value: number) =>
  `${value >= 0 ? "+ " : "- "}${Math.abs(value).toLocaleString("en-US")} sat`;

export function LightningProfitabilityPanel() {
  const overview = useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const lightningConnections = useMemo(
    () =>
      (overview.data?.data?.connections ?? []).filter((connection) =>
        LIGHTNING_CONNECTION_KINDS.has(connection.kind),
      ),
    [overview.data],
  );
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const activeConnection: Connection | null =
    lightningConnections.find((connection) => connection.id === selectedId) ??
    lightningConnections[0] ??
    null;

  if (lightningConnections.length === 0) {
    return null;
  }

  return (
    <Card>
      <CardHeader className="border-b px-4 pb-3">
        <CardTitle className="flex items-center gap-2 text-sm sm:text-base">
          <TrendingUp className="size-4" aria-hidden="true" />
          Lightning routing profitability
        </CardTitle>
        <CardDescription>
          Routing revenue minus payment, rebalance, and on-chain costs per
          Lightning connection. The per-channel column reports whether earned
          routing covers a coarse {DEFAULT_OPEN_COST_SAT.toLocaleString("en-US")} sat
          open-cost default; adapters that know each channel's exact funding
          fee can refine the threshold.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4 px-4 pt-4">
        <div className="flex flex-wrap items-center gap-2">
          <Label className="text-xs text-muted-foreground" htmlFor="ln-profitability-connection">
            Connection
          </Label>
          <select
            id="ln-profitability-connection"
            className="h-8 rounded-md border border-input bg-background px-2 text-sm"
            value={activeConnection?.id ?? ""}
            onChange={(event) => setSelectedId(event.target.value)}
          >
            {lightningConnections.map((connection) => (
              <option key={connection.id} value={connection.id}>
                {connection.label}
              </option>
            ))}
          </select>
        </div>
        {activeConnection ? (
          <LightningProfitabilityBody connectionId={activeConnection.id} />
        ) : null}
      </CardContent>
    </Card>
  );
}

function LightningProfitabilityBody({
  connectionId,
}: {
  connectionId: string;
}) {
  const profitability = useDaemon<LightningProfitabilityReport>(
    "ui.reports.lightning_profitability",
    { connection: connectionId },
    { retry: retryRetryableDaemonError },
  );
  if (profitability.isLoading) {
    return (
      <div className="text-sm text-muted-foreground">
        Reading routing profitability…
      </div>
    );
  }
  if (profitability.isError || profitability.data?.error) {
    const message =
      profitability.error instanceof Error
        ? profitability.error.message
        : profitability.data?.error?.message ??
          "Routing profitability is not available for this connection yet.";
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
        No routing data available.
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
        {windowLabel} ·{" "}
        {summary.forwardCount.toLocaleString("en-US")} forwards ·{" "}
        {summary.paymentCount.toLocaleString("en-US")} payments ·{" "}
        {summary.rebalanceCount.toLocaleString("en-US")} rebalances
      </div>
      <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-5">
        <Stat
          label="Revenue"
          value={fmtSatSigned(summary.routingRevenueSat)}
          tone="text-emerald-700 dark:text-emerald-300"
        />
        <Stat
          label="Payment fees"
          value={fmtSatSigned(-summary.paymentCostSat)}
          tone="text-red-700 dark:text-red-300"
        />
        <Stat
          label="Rebalance fees"
          value={fmtSatSigned(-summary.rebalanceCostSat)}
          tone="text-red-700 dark:text-red-300"
        />
        <Stat
          label="On-chain costs"
          value={fmtSatSigned(-summary.onchainCostSat)}
          tone="text-red-700 dark:text-red-300"
        />
        <Stat
          label="Net profit"
          value={fmtSatSigned(summary.netProfitSat)}
          tone={profitTone}
        />
      </div>
      <div className="overflow-hidden rounded-md border">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Peer</TableHead>
              <TableHead className="text-right">Capacity</TableHead>
              <TableHead className="text-right">Earned routing</TableHead>
              <TableHead className="text-right">Open cost</TableHead>
              <TableHead className="text-right">Covers open cost</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {channels.length === 0 ? (
              <TableRow>
                <TableCell
                  className="text-center text-sm text-muted-foreground"
                  colSpan={5}
                >
                  No channels reported.
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
                      {channel.coversOpenCost ? "Yes" : "Not yet"}
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
