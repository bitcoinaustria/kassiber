/**
 * Lightning node detail surface.
 *
 * Renders channels, capacity, and routing snapshot for `lnd` / `core-ln`
 * / `nwc` connections. The shape lives on `Connection.node` and is mocked
 * in dev mode until the LND/CLN sync daemon kinds are merged; when they
 * land, the daemon should map its status snapshot into the same shape.
 */

import { Fragment, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { Link } from "@tanstack/react-router";
import {
  ArrowDownRight,
  ArrowLeft,
  ArrowUpRight,
  Cable,
  Coins,
  ExternalLink,
  Globe2,
  RefreshCw,
  Repeat,
  Server,
  TrendingUp,
  XCircle,
  Zap,
} from "lucide-react";

import { ConnectionAssetBadge } from "@/components/kb/ConnectionAssetBadge";
import { ConnectionStatusPill } from "@/components/kb/ConnectionStatusPill";
import { DetailRow } from "@/components/kb/DetailRow";
import { MetricCard } from "@/components/kb/MetricCard";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { connectionKindLabels } from "@/lib/connectionDisplay";
import { formatShortDate } from "@/lib/date";
import { screenShellClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import { MISSING_FIAT_LABEL } from "@/lib/currency";
import type {
  Connection,
  NodeChannel,
  NodeChannelState,
  NodeForward,
  NodeForwardStatus,
  NodeSnapshot,
} from "@/mocks/seed";

const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

const fmtBtc = (value: number) => `₿ ${value.toFixed(8)}`;
const fmtSat = (value: number) =>
  `${value.toLocaleString("en-US")} sat`;
const fmtEur = (value: number | null) =>
  value === null
    ? MISSING_FIAT_LABEL
    : "€ " +
      value.toLocaleString("de-AT", {
        minimumFractionDigits: 2,
        maximumFractionDigits: 2,
      });
const fmtSatSigned = (value: number) =>
  `${value >= 0 ? "+ " : "- "}${Math.abs(value).toLocaleString("en-US")} sat`;
const fmtPubkey = (value: string) =>
  value.length <= 18 ? value : `${value.slice(0, 8)}…${value.slice(-6)}`;

const channelStateLabelKeys = {
  active: "node.channelState.active",
  inactive: "node.channelState.inactive",
  pending_open: "node.channelState.pendingOpen",
  pending_close: "node.channelState.pendingClose",
  closed: "node.channelState.closed",
  force_closed: "node.channelState.forceClosed",
} as const satisfies Record<NodeChannelState, string>;

const channelStateLabel = (
  state: NodeChannelState,
  t: TFunction<"connections">,
) => t(channelStateLabelKeys[state]);

const forwardStatusLabelKeys = {
  settled: "node.forwardStatus.settled",
  failed: "node.forwardStatus.failed",
  offered: "node.forwardStatus.offered",
} as const satisfies Record<NodeForwardStatus, string>;

const forwardStatusLabel = (
  status: NodeForwardStatus,
  t: TFunction<"connections">,
) => t(forwardStatusLabelKeys[status]);

const forwardStatusTone: Record<NodeForwardStatus, string> = {
  settled:
    "bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-300 dark:ring-emerald-400/20",
  failed:
    "bg-red-50 text-red-700 ring-red-600/20 dark:bg-red-900/30 dark:text-red-300 dark:ring-red-400/20",
  offered:
    "bg-amber-50 text-amber-700 ring-amber-600/20 dark:bg-amber-900/30 dark:text-amber-300 dark:ring-amber-400/20",
};

function explorerHrefForOutpoint(outpoint: string | null | undefined) {
  if (!outpoint) return null;
  const [txid] = outpoint.split(":");
  if (!txid || txid.length < 32) return null;
  return `https://mempool.space/tx/${txid}`;
}

function relativeFrom(value: string | null | undefined) {
  if (!value) return null;
  const ts = Date.parse(value);
  if (Number.isNaN(ts)) return null;
  const diffSec = Math.max(0, (Date.now() - ts) / 1000);
  if (diffSec < 60) return `${Math.round(diffSec)}s ago`;
  if (diffSec < 3600) return `${Math.round(diffSec / 60)}m ago`;
  if (diffSec < 86400) return `${Math.round(diffSec / 3600)}h ago`;
  return `${Math.round(diffSec / 86400)}d ago`;
}

const channelStateTone: Record<NodeChannelState, string> = {
  active:
    "bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-300 dark:ring-emerald-400/20",
  inactive:
    "bg-muted text-muted-foreground ring-border",
  pending_open:
    "bg-amber-50 text-amber-700 ring-amber-600/20 dark:bg-amber-900/30 dark:text-amber-300 dark:ring-amber-400/20",
  pending_close:
    "bg-amber-50 text-amber-700 ring-amber-600/20 dark:bg-amber-900/30 dark:text-amber-300 dark:ring-amber-400/20",
  closed:
    "bg-muted text-muted-foreground ring-border",
  force_closed:
    "bg-red-50 text-red-700 ring-red-600/20 dark:bg-red-900/30 dark:text-red-300 dark:ring-red-400/20",
};

interface NodeConnectionDetailProps {
  connection: Connection;
  priceEur: number;
  hideSensitive: boolean;
  onSync: () => void;
  isSyncRunning: boolean;
}

export function NodeConnectionDetail({
  connection,
  priceEur,
  hideSensitive,
  onSync,
  isSyncRunning,
}: NodeConnectionDetailProps) {
  const { t } = useTranslation("connections");
  const node = connection.node;
  const refreshButtonLabel = isSyncRunning
    ? t("node.header.refreshing")
    : t("node.header.refresh");

  if (!node) {
    return (
      <div className={screenShellClassName}>
        <NodeHeader
          connection={connection}
          isSyncRunning={isSyncRunning}
          refreshButtonLabel={refreshButtonLabel}
          onSync={onSync}
        />
        <Card>
          <CardHeader className="border-b px-4 pb-3">
            <CardTitle className="text-sm sm:text-base">
              {t("node.noSnapshot.title")}
            </CardTitle>
            <CardDescription>
              {t("node.noSnapshot.description")}
            </CardDescription>
          </CardHeader>
          <CardContent className="flex items-start gap-2 px-4 pt-4">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={isSyncRunning}
              onClick={onSync}
            >
              <RefreshCw
                className={cn("size-4", isSyncRunning && "animate-spin")}
                aria-hidden="true"
              />
              {refreshButtonLabel}
            </Button>
          </CardContent>
        </Card>
      </div>
    );
  }

  return (
    <div className={screenShellClassName}>
      <NodeHeader
        connection={connection}
        node={node}
        isSyncRunning={isSyncRunning}
        refreshButtonLabel={refreshButtonLabel}
        onSync={onSync}
      />

      <NodeMetrics
        node={node}
        priceEur={priceEur}
        hideSensitive={hideSensitive}
      />

      {node.routing ? (
        <RoutingSummary
          routing={node.routing}
          hideSensitive={hideSensitive}
          priceEur={priceEur}
        />
      ) : null}

      <div className="grid grid-cols-1 gap-3 xl:grid-cols-[minmax(0,1.45fr)_minmax(360px,0.85fr)]">
        <ChannelsCard
          channels={node.channels}
          closedChannels={node.closedChannels ?? []}
          hideSensitive={hideSensitive}
          totalCapacitySat={node.totalCapacitySat}
        />
        <NodeDetailsCard
          connection={connection}
          node={node}
          hideSensitive={hideSensitive}
        />
      </div>

      {node.forwards && node.forwards.length > 0 ? (
        <ForwardsCard
          forwards={node.forwards}
          hideSensitive={hideSensitive}
        />
      ) : null}
    </div>
  );
}

interface NodeHeaderProps {
  connection: Connection;
  node?: NodeSnapshot;
  isSyncRunning: boolean;
  refreshButtonLabel: string;
  onSync: () => void;
}

function NodeHeader({
  connection,
  node,
  isSyncRunning,
  refreshButtonLabel,
  onSync,
}: NodeHeaderProps) {
  const { t } = useTranslation("connections");
  return (
    <Card className="rounded-xl py-3">
      <CardContent className="flex flex-col gap-3 px-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex min-w-0 items-center gap-3">
          <Button asChild variant="outline" size="icon" className="shrink-0">
            <Link to="/connections" aria-label={t("node.header.backToWallets")}>
              <ArrowLeft className="size-4" aria-hidden="true" />
            </Link>
          </Button>
          <ConnectionAssetBadge
            connection={connection}
            size="md"
            className="hidden sm:flex"
          />
          <div className="min-w-0">
            <h1 className="truncate text-xl font-semibold tracking-tight sm:text-2xl">
              {connection.label}
            </h1>
            <div className="mt-1 flex min-w-0 flex-wrap items-center gap-2 text-xs text-muted-foreground">
              <span className="truncate">
                {connectionKindLabels[connection.kind]}
              </span>
              {node?.alias ? (
                <>
                  <span aria-hidden="true">·</span>
                  <span className="truncate">{node.alias}</span>
                </>
              ) : null}
              {node?.network ? (
                <>
                  <span aria-hidden="true">·</span>
                  <span className="truncate">{node.network}</span>
                </>
              ) : null}
              {connection.status !== "synced" ? (
                <>
                  <span aria-hidden="true">·</span>
                  <ConnectionStatusPill status={connection.status} />
                </>
              ) : null}
            </div>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2 self-start sm:self-center">
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="min-w-[7.5rem]"
            disabled={isSyncRunning}
            aria-busy={isSyncRunning}
            aria-label={t("node.header.refreshAction", {
              action: refreshButtonLabel,
              label: connection.label,
            })}
            onClick={onSync}
          >
            <RefreshCw
              className={cn("size-4", isSyncRunning && "animate-spin")}
              aria-hidden="true"
            />
            <span>{refreshButtonLabel}</span>
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

interface NodeMetricsProps {
  node: NodeSnapshot;
  priceEur: number;
  hideSensitive: boolean;
}

function NodeMetrics({ node, priceEur, hideSensitive }: NodeMetricsProps) {
  const { t } = useTranslation("connections");
  const activeChannels = node.channels.filter(
    (channel) => channel.state === "active",
  );
  const totalLightningSat =
    node.totalLocalBalanceSat + node.totalRemoteBalanceSat;
  const inboundLiquiditySat = node.totalRemoteBalanceSat;
  const localBtc = node.totalLocalBalanceSat / 100_000_000;
  const onchainBtc = node.onchainBalanceSat / 100_000_000;
  return (
    <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
      <MetricCard
        label={t("node.metrics.localBalance")}
        value={
          <span className={blurClass(hideSensitive)}>{fmtBtc(localBtc)}</span>
        }
        detail={t("node.metrics.localBalanceDetail", {
          sat: node.totalLocalBalanceSat.toLocaleString("en-US"),
          eur: fmtEur(localBtc * priceEur),
        })}
        icon={<Zap className="size-4" aria-hidden="true" />}
      />
      <MetricCard
        label={t("node.metrics.inboundLiquidity")}
        value={
          <span className={blurClass(hideSensitive)}>
            {fmtSat(inboundLiquiditySat)}
          </span>
        }
        detail={t("node.metrics.totalCapacity", {
          capacity: fmtSat(node.totalCapacitySat),
        })}
        icon={<Cable className="size-4" aria-hidden="true" />}
      />
      <MetricCard
        label={t("node.metrics.channels")}
        value={`${activeChannels.length} / ${node.channels.length}`}
        detail={t("node.metrics.channelsDetail", {
          peers: node.peerCount,
          closed: node.closedChannels?.length ?? 0,
        })}
        icon={<Server className="size-4" aria-hidden="true" />}
      />
      <MetricCard
        label={t("node.metrics.onChain")}
        value={
          <span className={blurClass(hideSensitive)}>
            {fmtBtc(onchainBtc)}
          </span>
        }
        detail={
          totalLightningSat > 0
            ? t("node.metrics.localInChannels", {
                percent: Math.round(
                  (node.totalLocalBalanceSat / totalLightningSat) * 100,
                ),
              })
            : t("node.metrics.noChannelLiquidity")
        }
        icon={<Coins className="size-4" aria-hidden="true" />}
      />
    </div>
  );
}

interface RoutingSummaryProps {
  routing: NonNullable<NodeSnapshot["routing"]>;
  priceEur: number;
  hideSensitive: boolean;
}

function RoutingSummary({
  routing,
  priceEur,
  hideSensitive,
}: RoutingSummaryProps) {
  const { t } = useTranslation("connections");
  const profitBtc = routing.netProfitSat / 100_000_000;
  const profitTone =
    routing.netProfitSat > 0
      ? "text-emerald-700 dark:text-emerald-300"
      : routing.netProfitSat < 0
        ? "text-red-700 dark:text-red-300"
        : "text-muted-foreground";
  return (
    <Card>
      <CardHeader className="border-b px-4 pb-3">
        <CardTitle className="flex items-center gap-2 text-sm sm:text-base">
          <TrendingUp className="size-4" aria-hidden="true" />
          {t("node.routing.title")}
        </CardTitle>
        <CardDescription>
          {t("node.routing.description", {
            window: routing.windowLabel,
            forwards: routing.forwardCount.toLocaleString("en-US"),
            payments: routing.paymentCount.toLocaleString("en-US"),
            rebalances: routing.rebalanceCount.toLocaleString("en-US"),
          })}
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-3 px-4 pt-4 sm:grid-cols-2 xl:grid-cols-5">
        <RoutingStat
          label={t("node.routing.revenue")}
          value={fmtSatSigned(routing.routingRevenueSat)}
          tone="text-emerald-700 dark:text-emerald-300"
          hideSensitive={hideSensitive}
        />
        <RoutingStat
          label={t("node.routing.paymentFees")}
          value={fmtSatSigned(-routing.paymentCostSat)}
          tone="text-red-700 dark:text-red-300"
          hideSensitive={hideSensitive}
        />
        <RoutingStat
          label={t("node.routing.rebalanceFees")}
          value={fmtSatSigned(-routing.rebalanceCostSat)}
          tone="text-red-700 dark:text-red-300"
          hideSensitive={hideSensitive}
        />
        <RoutingStat
          label={t("node.routing.onChainCosts")}
          value={fmtSatSigned(-routing.onchainCostSat)}
          tone="text-red-700 dark:text-red-300"
          hideSensitive={hideSensitive}
        />
        <RoutingStat
          label={t("node.routing.netProfit")}
          value={fmtSatSigned(routing.netProfitSat)}
          detail={fmtEur(profitBtc * priceEur)}
          tone={profitTone}
          hideSensitive={hideSensitive}
        />
      </CardContent>
    </Card>
  );
}

interface RoutingStatProps {
  label: string;
  value: string;
  detail?: string;
  tone: string;
  hideSensitive: boolean;
}

function RoutingStat({
  label,
  value,
  detail,
  tone,
  hideSensitive,
}: RoutingStatProps) {
  return (
    <div className="space-y-1 rounded-md border bg-background px-3 py-2.5">
      <span className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </span>
      <p
        className={cn(
          "font-mono text-sm font-semibold tabular-nums",
          tone,
          blurClass(hideSensitive),
        )}
      >
        {value}
      </p>
      {detail ? (
        <p
          className={cn(
            "text-[11px] text-muted-foreground tabular-nums",
            blurClass(hideSensitive),
          )}
        >
          {detail}
        </p>
      ) : null}
    </div>
  );
}

interface ChannelsCardProps {
  channels: NodeChannel[];
  closedChannels: NodeChannel[];
  hideSensitive: boolean;
  totalCapacitySat: number;
}

function ChannelsCard({
  channels,
  closedChannels,
  hideSensitive,
  totalCapacitySat,
}: ChannelsCardProps) {
  const { t } = useTranslation("connections");
  // When a node only has closed channels left, default to showing them so
  // the "Show closed" toggle is not the only way to see any channel data.
  const closedOnly = channels.length === 0 && closedChannels.length > 0;
  const [showClosed, setShowClosed] = useState(closedOnly);
  const [openChannelId, setOpenChannelId] = useState<string | null>(null);
  const sorted = useMemo(
    () => [...channels].sort((a, b) => b.capacitySat - a.capacitySat),
    [channels],
  );
  const sortedClosed = useMemo(
    () => [...closedChannels].sort((a, b) => b.capacitySat - a.capacitySat),
    [closedChannels],
  );
  const allChannels = useMemo(
    () => [...sorted, ...sortedClosed],
    [sorted, sortedClosed],
  );
  const openChannel =
    allChannels.find((channel) => channel.id === openChannelId) ?? null;
  const visibleRows = useMemo(
    () => (showClosed ? [...sorted, ...sortedClosed] : sorted),
    [showClosed, sorted, sortedClosed],
  );
  return (
    <Card>
      <CardHeader className="border-b px-4 pb-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <CardTitle className="flex items-center gap-2 text-sm sm:text-base">
            {t("node.channels.title")}
            <span className="inline-flex items-center rounded-md bg-gray-50 px-2 py-1 text-[10px] font-medium text-gray-600 ring-1 ring-gray-500/10 ring-inset sm:text-xs dark:bg-gray-800/50 dark:text-gray-400 dark:ring-gray-400/20">
              {channels.length}
            </span>
          </CardTitle>
          {sortedClosed.length > 0 ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="text-xs"
              onClick={() => setShowClosed((prev) => !prev)}
            >
              {showClosed
                ? t("node.channels.hideClosed", {
                    value: sortedClosed.length.toLocaleString("en-US"),
                  })
                : t("node.channels.showClosed", {
                    value: sortedClosed.length.toLocaleString("en-US"),
                  })}
            </Button>
          ) : null}
        </div>
        <CardDescription>
          {t("node.channels.description")}
        </CardDescription>
      </CardHeader>
      <CardContent className="p-0">
        {visibleRows.length === 0 ? (
          <div className="px-5 py-8 text-sm text-muted-foreground">
            {t("node.channels.empty")}
          </div>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("node.channels.peer")}</TableHead>
                <TableHead className="w-[40%]">
                  {t("node.channels.balance")}
                </TableHead>
                <TableHead className="text-right">
                  {t("node.channels.capacity")}
                </TableHead>
                <TableHead className="text-right">
                  {t("node.channels.feePolicy")}
                </TableHead>
                <TableHead className="text-right">
                  {t("node.channels.state")}
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {visibleRows.map((channel) => (
                <ChannelRow
                  key={channel.id}
                  channel={channel}
                  hideSensitive={hideSensitive}
                  totalCapacitySat={totalCapacitySat}
                  onOpen={() => setOpenChannelId(channel.id)}
                />
              ))}
            </TableBody>
          </Table>
        )}
      </CardContent>
      <ChannelDetailSheet
        channel={openChannel}
        open={openChannel !== null}
        onOpenChange={(open) => {
          if (!open) setOpenChannelId(null);
        }}
        hideSensitive={hideSensitive}
        totalCapacitySat={totalCapacitySat}
      />
    </Card>
  );
}

interface ChannelRowProps {
  channel: NodeChannel;
  hideSensitive: boolean;
  totalCapacitySat: number;
  onOpen: () => void;
}

function ChannelRow({
  channel,
  hideSensitive,
  totalCapacitySat,
  onOpen,
}: ChannelRowProps) {
  const { t } = useTranslation("connections");
  const capacity = Math.max(1, channel.capacitySat);
  const localPct = Math.min(
    100,
    Math.max(0, (channel.localBalanceSat / capacity) * 100),
  );
  const remotePct = Math.min(
    100,
    Math.max(0, (channel.remoteBalanceSat / capacity) * 100),
  );
  const sharePct =
    totalCapacitySat > 0
      ? Math.round((channel.capacitySat / totalCapacitySat) * 100)
      : 0;
  // Only active/inactive channels contribute to totalCapacitySat (closed and
  // pending capacities are not summed), so the share-of-node line only makes
  // sense for those states. Hide it elsewhere to avoid misleading percentages
  // like "1 BTC closed channel = 21% of node".
  const showSharePct =
    channel.state === "active" || channel.state === "inactive";
  return (
    <TableRow
      className="cursor-pointer transition-colors hover:bg-muted/45 focus-within:bg-muted/45"
      onClick={onOpen}
    >
      <TableCell className="min-w-0 align-top">
        {/*
          The row-level onClick gives mouse users a full-row hit target.
          Keyboard / assistive-tech users open the channel-detail Sheet via
          this in-cell button — leaving the <tr>'s implicit "row" role intact
          so cell associations are still announced correctly.
        */}
        <button
          type="button"
          className="flex min-w-0 flex-col gap-1 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring rounded"
          onClick={(event) => {
            event.stopPropagation();
            onOpen();
          }}
          aria-label={t("node.channels.openDetail", {
            peer: channel.peerAlias,
          })}
        >
          <span className="flex items-center gap-1.5 truncate text-sm font-medium">
            {channel.peerAlias}
            {channel.isPrivate ? (
              <Badge variant="outline" className="rounded-md text-[10px]">
                {t("node.channels.private")}
              </Badge>
            ) : null}
            {channel.isInitiator ? null : (
              <Badge variant="outline" className="rounded-md text-[10px]">
                {t("node.channels.remoteOpened")}
              </Badge>
            )}
          </span>
          <span
            className={cn(
              "block truncate font-mono text-[10px] text-muted-foreground sm:text-xs",
              blurClass(hideSensitive),
            )}
          >
            {channel.peerPubkey
              ? fmtPubkey(channel.peerPubkey)
              : t("node.channels.privatePeer")}
            {channel.shortChannelId ? ` · ${channel.shortChannelId}` : ""}
          </span>
        </button>
      </TableCell>
      <TableCell className="align-top">
        <div className="flex flex-col gap-1">
          <div
            className="flex h-2 w-full overflow-hidden rounded-full bg-muted"
            role="img"
            aria-label={t("node.channels.balanceAria", {
              local: channel.localBalanceSat.toLocaleString("en-US"),
              capacity: channel.capacitySat.toLocaleString("en-US"),
            })}
          >
            <div
              className="h-full bg-emerald-500 dark:bg-emerald-400/80"
              style={{ width: `${localPct}%` }}
            />
            <div
              className="h-full bg-sky-500 dark:bg-sky-400/80"
              style={{ width: `${remotePct}%` }}
            />
          </div>
          <div
            className={cn(
              "flex items-center justify-between font-mono text-[10px] text-muted-foreground tabular-nums sm:text-xs",
              blurClass(hideSensitive),
            )}
          >
            <span>
              <span className="text-emerald-700 dark:text-emerald-300">
                {fmtSat(channel.localBalanceSat)}
              </span>{" "}
              {t("node.channels.localLabel")}
            </span>
            <span>
              <span className="text-sky-700 dark:text-sky-300">
                {fmtSat(channel.remoteBalanceSat)}
              </span>{" "}
              {t("node.channels.remoteLabel")}
            </span>
          </div>
        </div>
      </TableCell>
      <TableCell className="align-top text-right">
        <span
          className={cn(
            "block font-mono text-sm tabular-nums",
            blurClass(hideSensitive),
          )}
        >
          {fmtSat(channel.capacitySat)}
        </span>
        {showSharePct ? (
          <span className="text-[10px] text-muted-foreground sm:text-xs">
            {t("node.channels.shareOfNode", { percent: sharePct })}
          </span>
        ) : null}
      </TableCell>
      <TableCell className="align-top text-right">
        {channel.baseFeeMsat == null && channel.feeRatePpm == null ? (
          <span className="text-xs text-muted-foreground">—</span>
        ) : (
          <Fragment>
            <span className="block font-mono text-sm tabular-nums">
              {t("node.channels.ppm", {
                value: channel.feeRatePpm?.toLocaleString("en-US") ?? "—",
              })}
            </span>
            <span className="text-[10px] text-muted-foreground sm:text-xs">
              {t("node.channels.baseMsat", {
                value: (channel.baseFeeMsat ?? 0).toLocaleString("en-US"),
              })}
            </span>
          </Fragment>
        )}
        {channel.forwardCount ? (
          <span className="mt-0.5 block text-[10px] text-muted-foreground sm:text-xs">
            {t("node.channels.forwardsEarned", {
              value: channel.forwardCount.toLocaleString("en-US"),
              earned: fmtSat(channel.earnedRoutingSat ?? 0),
            })}
          </span>
        ) : null}
      </TableCell>
      <TableCell className="align-top text-right">
        <span
          className={cn(
            "inline-flex items-center rounded-md px-1.5 py-0.5 text-[10px] font-medium ring-1 ring-inset",
            channelStateTone[channel.state],
          )}
        >
          {channelStateLabel(channel.state, t)}
        </span>
        {channel.closedAt ? (
          <span className="mt-0.5 block text-[10px] text-muted-foreground sm:text-xs">
            {channel.closeKind ?? t("node.channels.closedFallback")} ·{" "}
            {formatShortDate(channel.closedAt)}
          </span>
        ) : null}
      </TableCell>
    </TableRow>
  );
}

interface NodeDetailsCardProps {
  connection: Connection;
  node: NodeSnapshot;
  hideSensitive: boolean;
}

function NodeDetailsCard({
  connection,
  node,
  hideSensitive,
}: NodeDetailsCardProps) {
  const { t } = useTranslation("connections");
  return (
    <div className="space-y-3">
      <Card>
        <CardHeader className="border-b px-4 pb-3">
          <CardTitle className="flex items-center gap-2 text-sm sm:text-base">
            <Globe2 className="size-4" aria-hidden="true" />
            {t("node.identity.title")}
          </CardTitle>
          <CardDescription>
            {t("node.identity.description")}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3 px-4 pt-4">
          <DetailRow label={t("node.identity.alias")} value={node.alias} />
          <DetailRow
            label={t("node.identity.publicKey")}
            value={
              <span className={blurClass(hideSensitive)}>{node.pubkey}</span>
            }
            mono
            copy
          />
          <DetailRow label={t("node.identity.network")} value={node.network} />
          {node.implementationVersion ? (
            <DetailRow
              label={t("node.identity.implementation")}
              value={node.implementationVersion}
            />
          ) : null}
          {typeof node.blockHeight === "number" ? (
            <DetailRow
              label={t("node.identity.blockHeight")}
              value={node.blockHeight.toLocaleString("en-US")}
              mono
            />
          ) : null}
          <DetailRow
            label={t("node.identity.connectionKind")}
            value={connection.kind}
            mono
          />
          <DetailRow
            label={t("node.identity.kassiberId")}
            value={connection.id}
            mono
            copy
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="border-b px-4 pb-3">
          <CardTitle className="text-sm sm:text-base">
            {t("node.syncStatus.title")}
          </CardTitle>
          <CardDescription>
            {t("node.syncStatus.description")}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3 px-4 pt-4">
          <DetailRow
            label={t("node.syncStatus.status")}
            value={connection.status}
            mono
          />
          <DetailRow
            label={t("node.syncStatus.lastRefresh")}
            value={connection.last}
          />
          {connection.lastSyncAt ? (
            <DetailRow
              label={t("node.syncStatus.lastSyncAt")}
              value={formatShortDate(connection.lastSyncAt)}
              mono
            />
          ) : null}
          {connection.lastTransactionAt ? (
            <DetailRow
              label={t("node.syncStatus.lastTransaction")}
              value={formatShortDate(connection.lastTransactionAt)}
              mono
            />
          ) : null}
          {typeof connection.transactionCount === "number" ? (
            <DetailRow
              label={t("node.syncStatus.importedTransactions")}
              value={connection.transactionCount.toLocaleString("en-US")}
            />
          ) : null}
        </CardContent>
      </Card>
    </div>
  );
}

interface ChannelDetailSheetProps {
  channel: NodeChannel | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  hideSensitive: boolean;
  totalCapacitySat: number;
}

function ChannelDetailSheet({
  channel,
  open,
  onOpenChange,
  hideSensitive,
  totalCapacitySat,
}: ChannelDetailSheetProps) {
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side="right"
        className="w-full sm:max-w-md overflow-y-auto"
      >
        {channel ? (
          <ChannelDetailBody
            channel={channel}
            hideSensitive={hideSensitive}
            totalCapacitySat={totalCapacitySat}
          />
        ) : null}
      </SheetContent>
    </Sheet>
  );
}

interface ChannelDetailBodyProps {
  channel: NodeChannel;
  hideSensitive: boolean;
  totalCapacitySat: number;
}

function ChannelDetailBody({
  channel,
  hideSensitive,
  totalCapacitySat,
}: ChannelDetailBodyProps) {
  const { t } = useTranslation("connections");
  const capacity = Math.max(1, channel.capacitySat);
  const localPct = (channel.localBalanceSat / capacity) * 100;
  const remotePct = (channel.remoteBalanceSat / capacity) * 100;
  // Only show "% of node" for channels included in the totalCapacitySat
  // denominator. The seed contract excludes closed and pending channels
  // from the totals; showing their share against the live denominator
  // would imply they still contribute to current node capacity.
  const showSharePct =
    channel.state === "active" || channel.state === "inactive";
  const sharePct =
    showSharePct && totalCapacitySat > 0
      ? Math.round((channel.capacitySat / totalCapacitySat) * 100)
      : null;
  const explorerHref = explorerHrefForOutpoint(channel.fundingOutpoint);
  const lastActivityRel = relativeFrom(channel.lastActivityAt);
  return (
    <>
      <SheetHeader className="border-b">
        <SheetTitle className="flex items-center gap-2 text-base">
          <span
            className={cn(
              "inline-flex size-7 items-center justify-center rounded-md border",
              channelStateTone[channel.state],
            )}
            aria-hidden="true"
          >
            <Zap className="size-3.5" />
          </span>
          <span className="truncate">{channel.peerAlias}</span>
        </SheetTitle>
        <SheetDescription>
          {channelStateLabel(channel.state, t)} ·{" "}
          {channel.isInitiator
            ? t("node.channelDetail.locallyInitiated")
            : t("node.channelDetail.remoteOpened")}
          {channel.isPrivate ? ` · ${t("node.channelDetail.private")}` : ""}
        </SheetDescription>
      </SheetHeader>
      <div className="space-y-4 px-4 pb-6">
        <div className="space-y-2 rounded-md border bg-background px-3 py-3">
          <div
            className="flex h-2.5 w-full overflow-hidden rounded-full bg-muted"
            role="img"
            aria-label={t("node.channels.balanceAria", {
              local: channel.localBalanceSat.toLocaleString("en-US"),
              capacity: channel.capacitySat.toLocaleString("en-US"),
            })}
          >
            <div
              className="h-full bg-emerald-500 dark:bg-emerald-400/80"
              style={{ width: `${localPct}%` }}
            />
            <div
              className="h-full bg-sky-500 dark:bg-sky-400/80"
              style={{ width: `${remotePct}%` }}
            />
          </div>
          <div
            className={cn(
              "flex items-center justify-between font-mono text-xs tabular-nums",
              blurClass(hideSensitive),
            )}
          >
            <span className="text-emerald-700 dark:text-emerald-300">
              {t("node.channelDetail.localBalance", {
                value: fmtSat(channel.localBalanceSat),
              })}
            </span>
            <span className="text-sky-700 dark:text-sky-300">
              {t("node.channelDetail.remoteBalance", {
                value: fmtSat(channel.remoteBalanceSat),
              })}
            </span>
          </div>
          <div className="flex items-center justify-between text-[11px] text-muted-foreground">
            <span>
              {t("node.channelDetail.capacity", {
                capacity: fmtSat(channel.capacitySat),
              })}
            </span>
            {sharePct !== null ? (
              <span>
                {t("node.channelDetail.shareOfNode", { percent: sharePct })}
              </span>
            ) : null}
          </div>
        </div>

        <div className="space-y-2.5">
          <h4 className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
            {t("node.channelDetail.peer")}
          </h4>
          <DetailRow
            label={t("node.channelDetail.alias")}
            value={channel.peerAlias}
          />
          {channel.peerPubkey ? (
            <DetailRow
              label={t("node.channelDetail.publicKey")}
              value={
                <span className={blurClass(hideSensitive)}>
                  {channel.peerPubkey}
                </span>
              }
              mono
              copy
            />
          ) : (
            <DetailRow
              label={t("node.channelDetail.publicKey")}
              value={
                <span className="text-muted-foreground">
                  {t("node.channelDetail.publicKeyHidden")}
                </span>
              }
            />
          )}
        </div>

        <div className="space-y-2.5">
          <h4 className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
            {t("node.channelDetail.channel")}
          </h4>
          {channel.shortChannelId ? (
            <DetailRow
              label={t("node.channelDetail.shortChannelId")}
              value={channel.shortChannelId}
              mono
              copy
            />
          ) : (
            <DetailRow
              label={t("node.channelDetail.shortChannelId")}
              value={t("node.channelDetail.shortChannelIdPending")}
            />
          )}
          {channel.fundingOutpoint ? (
            <DetailRow
              label={t("node.channelDetail.fundingOutpoint")}
              value={
                <span
                  className={cn(
                    "inline-flex items-center gap-1 font-mono text-xs",
                    blurClass(hideSensitive),
                  )}
                >
                  <span className="truncate">{channel.fundingOutpoint}</span>
                  {explorerHref ? (
                    <a
                      href={explorerHref}
                      target="_blank"
                      rel="noreferrer"
                      className="shrink-0 text-muted-foreground hover:text-foreground"
                      aria-label={t("node.channelDetail.openFundingTx")}
                    >
                      <ExternalLink className="size-3" aria-hidden="true" />
                    </a>
                  ) : null}
                </span>
              }
            />
          ) : null}
          {channel.openedAt ? (
            <DetailRow
              label={t("node.channelDetail.opened")}
              value={`${formatShortDate(channel.openedAt)}${
                relativeFrom(channel.openedAt)
                  ? ` · ${relativeFrom(channel.openedAt)}`
                  : ""
              }`}
              mono
            />
          ) : null}
          {channel.closedAt ? (
            <DetailRow
              label={t("node.channelDetail.closed")}
              value={`${formatShortDate(channel.closedAt)}${
                channel.closeKind ? ` · ${channel.closeKind}` : ""
              }`}
              mono
            />
          ) : null}
          {channel.lastActivityAt ? (
            <DetailRow
              label={t("node.channelDetail.lastActivity")}
              value={`${formatShortDate(channel.lastActivityAt)}${
                lastActivityRel ? ` · ${lastActivityRel}` : ""
              }`}
              mono
            />
          ) : null}
          {typeof channel.htlcCount === "number" ? (
            <DetailRow
              label={t("node.channelDetail.inFlightHtlcs")}
              value={channel.htlcCount.toLocaleString("en-US")}
            />
          ) : null}
        </div>

        <div className="space-y-2.5">
          <h4 className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
            {t("node.channelDetail.routing")}
          </h4>
          <DetailRow
            label={t("node.channelDetail.feeRate")}
            value={t("node.channelDetail.feeRateValue", {
              value: (channel.feeRatePpm ?? 0).toLocaleString("en-US"),
            })}
          />
          <DetailRow
            label={t("node.channelDetail.baseFee")}
            value={t("node.channelDetail.baseFeeValue", {
              value: (channel.baseFeeMsat ?? 0).toLocaleString("en-US"),
            })}
          />
          {typeof channel.forwardCount === "number" ? (
            <DetailRow
              label={t("node.channelDetail.forwardsWindow")}
              value={channel.forwardCount.toLocaleString("en-US")}
            />
          ) : null}
          {typeof channel.earnedRoutingSat === "number" ? (
            <DetailRow
              label={t("node.channelDetail.earnedRouting")}
              value={fmtSat(channel.earnedRoutingSat)}
              mono
            />
          ) : null}
        </div>
      </div>
    </>
  );
}

interface ForwardsCardProps {
  forwards: NodeForward[];
  hideSensitive: boolean;
}

function ForwardsCard({ forwards, hideSensitive }: ForwardsCardProps) {
  const { t } = useTranslation("connections");
  const sorted = useMemo(
    () =>
      [...forwards].sort(
        (a, b) => Date.parse(b.occurredAt) - Date.parse(a.occurredAt),
      ),
    [forwards],
  );
  return (
    <Card>
      <CardHeader className="border-b px-4 pb-3">
        <CardTitle className="flex items-center gap-2 text-sm sm:text-base">
          <Repeat className="size-4" aria-hidden="true" />
          {t("node.forwards.title")}
          <span className="inline-flex items-center rounded-md bg-gray-50 px-2 py-1 text-[10px] font-medium text-gray-600 ring-1 ring-gray-500/10 ring-inset sm:text-xs dark:bg-gray-800/50 dark:text-gray-400 dark:ring-gray-400/20">
            {sorted.length}
          </span>
        </CardTitle>
        <CardDescription>
          {t("node.forwards.description")}
        </CardDescription>
      </CardHeader>
      <CardContent className="p-0">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("node.forwards.time")}</TableHead>
              <TableHead>{t("node.forwards.route")}</TableHead>
              <TableHead className="text-right">
                {t("node.forwards.amount")}
              </TableHead>
              <TableHead className="text-right">
                {t("node.forwards.fee")}
              </TableHead>
              <TableHead className="text-right">
                {t("node.forwards.status")}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {sorted.map((forward) => (
              <ForwardRow
                key={forward.id}
                forward={forward}
                hideSensitive={hideSensitive}
              />
            ))}
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  );
}

interface ForwardRowProps {
  forward: NodeForward;
  hideSensitive: boolean;
}

function ForwardRow({ forward, hideSensitive }: ForwardRowProps) {
  const { t } = useTranslation("connections");
  const amountSat = Math.round(forward.amountInMsat / 1_000);
  // Sub-1000 msat fees would round down to "0.5 sat" etc, which reads wrong.
  // Render those in msat so the value stays informative; otherwise show
  // whole sats.
  const feeMsat = forward.feeMsat;
  const feeRendersAsMsat = feeMsat > 0 && feeMsat < 1_000;
  const feeSat = Math.round(feeMsat / 1_000);
  const relative = relativeFrom(forward.occurredAt);
  const statusIcon =
    forward.status === "settled" ? (
      <ArrowDownRight className="size-3.5" aria-hidden="true" />
    ) : forward.status === "failed" ? (
      <XCircle className="size-3.5" aria-hidden="true" />
    ) : (
      <ArrowUpRight className="size-3.5" aria-hidden="true" />
    );
  return (
    <TableRow>
      <TableCell className="align-top">
        <span className="block text-sm">{formatShortDate(forward.occurredAt)}</span>
        {relative ? (
          <span className="text-[10px] text-muted-foreground sm:text-xs">
            {relative}
          </span>
        ) : null}
      </TableCell>
      <TableCell className="min-w-0 align-top">
        <div className="flex min-w-0 flex-col gap-0.5 text-xs">
          <span className="flex min-w-0 items-center gap-1.5">
            <ArrowDownRight
              className="size-3 text-emerald-600 dark:text-emerald-400"
              aria-hidden="true"
            />
            <span className="truncate font-medium">{forward.inPeerAlias}</span>
            {forward.inShortChannelId ? (
              <span
                className={cn(
                  "shrink-0 font-mono text-[10px] text-muted-foreground",
                  blurClass(hideSensitive),
                )}
              >
                {forward.inShortChannelId}
              </span>
            ) : null}
          </span>
          <span className="flex min-w-0 items-center gap-1.5">
            <ArrowUpRight
              className="size-3 text-sky-600 dark:text-sky-400"
              aria-hidden="true"
            />
            <span className="truncate font-medium">{forward.outPeerAlias}</span>
            {forward.outShortChannelId ? (
              <span
                className={cn(
                  "shrink-0 font-mono text-[10px] text-muted-foreground",
                  blurClass(hideSensitive),
                )}
              >
                {forward.outShortChannelId}
              </span>
            ) : null}
          </span>
        </div>
      </TableCell>
      <TableCell className="align-top text-right">
        <span
          className={cn(
            "block font-mono text-sm tabular-nums",
            blurClass(hideSensitive),
          )}
        >
          {fmtSat(amountSat)}
        </span>
      </TableCell>
      <TableCell className="align-top text-right">
        <span
          className={cn(
            "block font-mono text-sm tabular-nums",
            forward.status === "settled"
              ? "text-emerald-700 dark:text-emerald-300"
              : "text-muted-foreground",
            blurClass(hideSensitive),
          )}
        >
          {forward.status === "settled" && feeMsat > 0
            ? feeRendersAsMsat
              ? t("node.forwards.feeMsat", {
                  value: feeMsat.toLocaleString("en-US"),
                })
              : t("node.forwards.feeSat", {
                  value: feeSat.toLocaleString("en-US"),
                })
            : "—"}
        </span>
      </TableCell>
      <TableCell className="align-top text-right">
        <span
          className={cn(
            "inline-flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[10px] font-medium ring-1 ring-inset",
            forwardStatusTone[forward.status],
          )}
        >
          {statusIcon}
          {forwardStatusLabel(forward.status, t)}
        </span>
        {forward.failureReason ? (
          <span className="mt-0.5 block text-[10px] text-muted-foreground sm:text-xs">
            {forward.failureReason}
          </span>
        ) : null}
      </TableCell>
    </TableRow>
  );
}
