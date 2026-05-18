/**
 * Lightning node detail surface.
 *
 * Renders channels, capacity, and routing snapshot for `lnd` / `core-ln`
 * / `nwc` connections. The shape lives on `Connection.node` and is mocked
 * in dev mode until the LND/CLN sync daemon kinds are merged; when they
 * land, the daemon should map its status snapshot into the same shape.
 */

import { Fragment, useMemo, useState } from "react";
import { Link } from "@tanstack/react-router";
import {
  ArrowDownRight,
  ArrowLeft,
  ArrowUpRight,
  Cable,
  Coins,
  ExternalLink,
  Globe2,
  MoreHorizontal,
  Pencil,
  RefreshCw,
  Repeat,
  Server,
  TrendingUp,
  Trash2,
  XCircle,
  Zap,
} from "lucide-react";

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
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
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
import {
  connectionKindLabels,
  connectionKindTone,
} from "@/lib/connectionDisplay";
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

const channelStateLabels: Record<NodeChannelState, string> = {
  active: "Active",
  inactive: "Inactive",
  pending_open: "Pending open",
  pending_close: "Pending close",
  closed: "Closed",
  force_closed: "Force-closed",
};

const forwardStatusLabels: Record<NodeForwardStatus, string> = {
  settled: "Settled",
  failed: "Failed",
  offered: "In flight",
};

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

function shortDate(value: string | null | undefined) {
  if (!value) return "—";
  const normalized = value.replace("T", " ").replace(/Z$/, "");
  return normalized.length > 16 ? normalized.slice(0, 16) : normalized;
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
  onEdit?: () => void;
  onDelete?: () => void;
}

export function NodeConnectionDetail({
  connection,
  priceEur,
  hideSensitive,
  onSync,
  isSyncRunning,
  onEdit,
  onDelete,
}: NodeConnectionDetailProps) {
  const node = connection.node;
  const refreshButtonLabel = isSyncRunning ? "Refreshing" : "Refresh";

  if (!node) {
    return (
      <div className={screenShellClassName}>
        <NodeHeader
          connection={connection}
          isSyncRunning={isSyncRunning}
          refreshButtonLabel={refreshButtonLabel}
          onSync={onSync}
          onEdit={onEdit}
          onDelete={onDelete}
        />
        <Card>
          <CardHeader className="border-b px-4 pb-3">
            <CardTitle className="text-sm sm:text-base">No node snapshot yet</CardTitle>
            <CardDescription>
              Lightning node sync has not produced a snapshot for this
              connection yet. Run a refresh to fetch channels, balances, and the
              routing summary.
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
        onEdit={onEdit}
        onDelete={onDelete}
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
  onEdit?: () => void;
  onDelete?: () => void;
}

function NodeHeader({
  connection,
  node,
  isSyncRunning,
  refreshButtonLabel,
  onSync,
  onEdit,
  onDelete,
}: NodeHeaderProps) {
  return (
    <Card className="rounded-xl py-3">
      <CardContent className="flex flex-col gap-3 px-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex min-w-0 items-center gap-3">
          <Button asChild variant="outline" size="icon" className="shrink-0">
            <Link to="/connections" aria-label="Back to wallets">
              <ArrowLeft className="size-4" aria-hidden="true" />
            </Link>
          </Button>
          <span
            className={cn(
              "hidden size-9 shrink-0 items-center justify-center rounded-md border sm:flex",
              connectionKindTone(connection.kind),
            )}
            aria-hidden="true"
          >
            <Zap className="size-4" />
          </span>
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
            aria-label={`${refreshButtonLabel} ${connection.label}`}
            onClick={onSync}
          >
            <RefreshCw
              className={cn("size-4", isSyncRunning && "animate-spin")}
              aria-hidden="true"
            />
            <span>{refreshButtonLabel}</span>
          </Button>
          {onEdit || onDelete ? (
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  type="button"
                  variant="outline"
                  size="icon"
                  aria-label="More actions"
                >
                  <MoreHorizontal className="size-4" aria-hidden="true" />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                {onEdit ? (
                  <DropdownMenuItem onClick={onEdit}>
                    <Pencil className="size-4" aria-hidden="true" />
                    Edit
                  </DropdownMenuItem>
                ) : null}
                {onEdit && onDelete ? <DropdownMenuSeparator /> : null}
                {onDelete ? (
                  <DropdownMenuItem
                    className="text-destructive focus:text-destructive"
                    onClick={onDelete}
                  >
                    <Trash2 className="size-4" aria-hidden="true" />
                    Remove
                  </DropdownMenuItem>
                ) : null}
              </DropdownMenuContent>
            </DropdownMenu>
          ) : null}
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
        label="Local balance"
        value={
          <span className={blurClass(hideSensitive)}>{fmtBtc(localBtc)}</span>
        }
        detail={`${node.totalLocalBalanceSat.toLocaleString(
          "en-US",
        )} sat · ${fmtEur(localBtc * priceEur)}`}
        icon={<Zap className="size-4" aria-hidden="true" />}
      />
      <MetricCard
        label="Inbound liquidity"
        value={
          <span className={blurClass(hideSensitive)}>
            {fmtSat(inboundLiquiditySat)}
          </span>
        }
        detail={`Total capacity ${fmtSat(node.totalCapacitySat)}`}
        icon={<Cable className="size-4" aria-hidden="true" />}
      />
      <MetricCard
        label="Channels"
        value={`${activeChannels.length} / ${node.channels.length}`}
        detail={`${node.peerCount} peers · ${
          (node.closedChannels?.length ?? 0)
        } closed`}
        icon={<Server className="size-4" aria-hidden="true" />}
      />
      <MetricCard
        label="On-chain"
        value={
          <span className={blurClass(hideSensitive)}>
            {fmtBtc(onchainBtc)}
          </span>
        }
        detail={
          totalLightningSat > 0
            ? `${Math.round(
                (node.totalLocalBalanceSat / totalLightningSat) * 100,
              )}% local in channels`
            : "No channel liquidity"
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
          Routing summary
        </CardTitle>
        <CardDescription>
          {routing.windowLabel} · {routing.forwardCount.toLocaleString("en-US")}{" "}
          forwards · {routing.paymentCount.toLocaleString("en-US")} payments ·{" "}
          {routing.rebalanceCount.toLocaleString("en-US")} rebalances
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-3 px-4 pt-4 sm:grid-cols-2 xl:grid-cols-5">
        <RoutingStat
          label="Revenue"
          value={fmtSatSigned(routing.routingRevenueSat)}
          tone="text-emerald-700 dark:text-emerald-300"
          hideSensitive={hideSensitive}
        />
        <RoutingStat
          label="Payment fees"
          value={fmtSatSigned(-routing.paymentCostSat)}
          tone="text-red-700 dark:text-red-300"
          hideSensitive={hideSensitive}
        />
        <RoutingStat
          label="Rebalance fees"
          value={fmtSatSigned(-routing.rebalanceCostSat)}
          tone="text-red-700 dark:text-red-300"
          hideSensitive={hideSensitive}
        />
        <RoutingStat
          label="On-chain costs"
          value={fmtSatSigned(-routing.onchainCostSat)}
          tone="text-red-700 dark:text-red-300"
          hideSensitive={hideSensitive}
        />
        <RoutingStat
          label="Net profit"
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
  const [showClosed, setShowClosed] = useState(false);
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
  return (
    <Card>
      <CardHeader className="border-b px-4 pb-3">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <CardTitle className="flex items-center gap-2 text-sm sm:text-base">
            Channels
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
              {showClosed ? "Hide" : "Show"} closed (
              {sortedClosed.length.toLocaleString("en-US")})
            </Button>
          ) : null}
        </div>
        <CardDescription>
          Capacity, local / remote balance, and routing fee policy per channel.
          Click a row for full channel details.
        </CardDescription>
      </CardHeader>
      <CardContent className="p-0">
        {sorted.length === 0 ? (
          <div className="px-5 py-8 text-sm text-muted-foreground">
            No channels reported by the node.
          </div>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Peer</TableHead>
                <TableHead className="w-[40%]">Balance</TableHead>
                <TableHead className="text-right">Capacity</TableHead>
                <TableHead className="text-right">Fee policy</TableHead>
                <TableHead className="text-right">State</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {sorted.map((channel) => (
                <ChannelRow
                  key={channel.id}
                  channel={channel}
                  hideSensitive={hideSensitive}
                  totalCapacitySat={totalCapacitySat}
                  onOpen={() => setOpenChannelId(channel.id)}
                />
              ))}
              {showClosed
                ? sortedClosed.map((channel) => (
                    <ChannelRow
                      key={channel.id}
                      channel={channel}
                      hideSensitive={hideSensitive}
                      totalCapacitySat={totalCapacitySat}
                      onOpen={() => setOpenChannelId(channel.id)}
                    />
                  ))
                : null}
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
  return (
    <TableRow
      className="cursor-pointer transition-colors hover:bg-muted/45 focus-within:bg-muted/45"
      onClick={onOpen}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onOpen();
        }
      }}
      tabIndex={0}
      role="button"
      aria-label={`Channel detail for ${channel.peerAlias}`}
    >
      <TableCell className="min-w-0 align-top">
        <div className="flex min-w-0 flex-col gap-1">
          <span className="flex items-center gap-1.5 truncate text-sm font-medium">
            {channel.peerAlias}
            {channel.isPrivate ? (
              <Badge variant="outline" className="rounded-md text-[10px]">
                Private
              </Badge>
            ) : null}
            {channel.isInitiator ? null : (
              <Badge variant="outline" className="rounded-md text-[10px]">
                Remote-opened
              </Badge>
            )}
          </span>
          <span
            className={cn(
              "block truncate font-mono text-[10px] text-muted-foreground sm:text-xs",
              blurClass(hideSensitive),
            )}
          >
            {fmtPubkey(channel.peerPubkey)}
            {channel.shortChannelId ? ` · ${channel.shortChannelId}` : ""}
          </span>
        </div>
      </TableCell>
      <TableCell className="align-top">
        <div className="flex flex-col gap-1">
          <div
            className="flex h-2 w-full overflow-hidden rounded-full bg-muted"
            role="img"
            aria-label={`${channel.localBalanceSat.toLocaleString(
              "en-US",
            )} sat local of ${channel.capacitySat.toLocaleString("en-US")} sat capacity`}
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
              local
            </span>
            <span>
              <span className="text-sky-700 dark:text-sky-300">
                {fmtSat(channel.remoteBalanceSat)}
              </span>{" "}
              remote
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
        <span className="text-[10px] text-muted-foreground sm:text-xs">
          {sharePct}% of node
        </span>
      </TableCell>
      <TableCell className="align-top text-right">
        {channel.baseFeeMsat == null && channel.feeRatePpm == null ? (
          <span className="text-xs text-muted-foreground">—</span>
        ) : (
          <Fragment>
            <span className="block font-mono text-sm tabular-nums">
              {channel.feeRatePpm?.toLocaleString("en-US") ?? "—"} ppm
            </span>
            <span className="text-[10px] text-muted-foreground sm:text-xs">
              base {(channel.baseFeeMsat ?? 0).toLocaleString("en-US")} msat
            </span>
          </Fragment>
        )}
        {channel.forwardCount ? (
          <span className="mt-0.5 block text-[10px] text-muted-foreground sm:text-xs">
            {channel.forwardCount.toLocaleString("en-US")} forwards ·{" "}
            {fmtSat(channel.earnedRoutingSat ?? 0)}
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
          {channelStateLabels[channel.state]}
        </span>
        {channel.closedAt ? (
          <span className="mt-0.5 block text-[10px] text-muted-foreground sm:text-xs">
            {channel.closeKind ?? "closed"} ·{" "}
            {formatNodeDate(channel.closedAt)}
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
  return (
    <div className="space-y-3">
      <Card>
        <CardHeader className="border-b px-4 pb-3">
          <CardTitle className="flex items-center gap-2 text-sm sm:text-base">
            <Globe2 className="size-4" aria-hidden="true" />
            Node identity
          </CardTitle>
          <CardDescription>
            Read-only metadata from the Lightning node.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3 px-4 pt-4">
          <DetailRow label="Alias" value={node.alias} />
          <DetailRow
            label="Public key"
            value={
              <span className={blurClass(hideSensitive)}>{node.pubkey}</span>
            }
            mono
            copy
          />
          <DetailRow label="Network" value={node.network} />
          {node.implementationVersion ? (
            <DetailRow
              label="Implementation"
              value={node.implementationVersion}
            />
          ) : null}
          {typeof node.blockHeight === "number" ? (
            <DetailRow
              label="Block height"
              value={node.blockHeight.toLocaleString("en-US")}
              mono
            />
          ) : null}
          <DetailRow label="Connection kind" value={connection.kind} mono />
          <DetailRow label="Kassiber ID" value={connection.id} mono copy />
        </CardContent>
      </Card>

      <Card>
        <CardHeader className="border-b px-4 pb-3">
          <CardTitle className="text-sm sm:text-base">Sync status</CardTitle>
          <CardDescription>
            Read-only sync state for this Lightning connection.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3 px-4 pt-4">
          <DetailRow label="Status" value={connection.status} mono />
          <DetailRow label="Last refresh" value={connection.last} />
          {connection.lastSyncAt ? (
            <DetailRow
              label="Last sync at"
              value={formatNodeDate(connection.lastSyncAt)}
              mono
            />
          ) : null}
          {connection.lastTransactionAt ? (
            <DetailRow
              label="Last transaction"
              value={formatNodeDate(connection.lastTransactionAt)}
              mono
            />
          ) : null}
          {typeof connection.transactionCount === "number" ? (
            <DetailRow
              label="Imported transactions"
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
  const capacity = Math.max(1, channel.capacitySat);
  const localPct = (channel.localBalanceSat / capacity) * 100;
  const remotePct = (channel.remoteBalanceSat / capacity) * 100;
  const sharePct =
    totalCapacitySat > 0
      ? Math.round((channel.capacitySat / totalCapacitySat) * 100)
      : 0;
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
          {channelStateLabels[channel.state]} ·{" "}
          {channel.isInitiator ? "Locally initiated" : "Remote-opened"}
          {channel.isPrivate ? " · Private" : ""}
        </SheetDescription>
      </SheetHeader>
      <div className="space-y-4 px-4 pb-6">
        <div className="space-y-2 rounded-md border bg-background px-3 py-3">
          <div
            className="flex h-2.5 w-full overflow-hidden rounded-full bg-muted"
            role="img"
            aria-label={`${channel.localBalanceSat.toLocaleString(
              "en-US",
            )} sat local of ${channel.capacitySat.toLocaleString("en-US")} sat capacity`}
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
              {fmtSat(channel.localBalanceSat)} local
            </span>
            <span className="text-sky-700 dark:text-sky-300">
              {fmtSat(channel.remoteBalanceSat)} remote
            </span>
          </div>
          <div className="flex items-center justify-between text-[11px] text-muted-foreground">
            <span>Capacity {fmtSat(channel.capacitySat)}</span>
            <span>{sharePct}% of node</span>
          </div>
        </div>

        <div className="space-y-2.5">
          <h4 className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
            Peer
          </h4>
          <DetailRow label="Alias" value={channel.peerAlias} />
          <DetailRow
            label="Public key"
            value={
              <span className={blurClass(hideSensitive)}>
                {channel.peerPubkey}
              </span>
            }
            mono
            copy
          />
        </div>

        <div className="space-y-2.5">
          <h4 className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
            Channel
          </h4>
          {channel.shortChannelId ? (
            <DetailRow
              label="Short channel id"
              value={channel.shortChannelId}
              mono
              copy
            />
          ) : (
            <DetailRow
              label="Short channel id"
              value="Pending confirmation"
            />
          )}
          {channel.fundingOutpoint ? (
            <DetailRow
              label="Funding outpoint"
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
                      aria-label="Open funding transaction on mempool.space"
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
              label="Opened"
              value={`${shortDate(channel.openedAt)}${
                relativeFrom(channel.openedAt)
                  ? ` · ${relativeFrom(channel.openedAt)}`
                  : ""
              }`}
              mono
            />
          ) : null}
          {channel.closedAt ? (
            <DetailRow
              label="Closed"
              value={`${shortDate(channel.closedAt)}${
                channel.closeKind ? ` · ${channel.closeKind}` : ""
              }`}
              mono
            />
          ) : null}
          {channel.lastActivityAt ? (
            <DetailRow
              label="Last activity"
              value={`${shortDate(channel.lastActivityAt)}${
                lastActivityRel ? ` · ${lastActivityRel}` : ""
              }`}
              mono
            />
          ) : null}
          {typeof channel.htlcCount === "number" ? (
            <DetailRow
              label="In-flight HTLCs"
              value={channel.htlcCount.toLocaleString("en-US")}
            />
          ) : null}
        </div>

        <div className="space-y-2.5">
          <h4 className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
            Routing
          </h4>
          <DetailRow
            label="Fee rate"
            value={`${(channel.feeRatePpm ?? 0).toLocaleString("en-US")} ppm`}
          />
          <DetailRow
            label="Base fee"
            value={`${(channel.baseFeeMsat ?? 0).toLocaleString("en-US")} msat`}
          />
          {typeof channel.forwardCount === "number" ? (
            <DetailRow
              label="Forwards (window)"
              value={channel.forwardCount.toLocaleString("en-US")}
            />
          ) : null}
          {typeof channel.earnedRoutingSat === "number" ? (
            <DetailRow
              label="Earned routing"
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
          Recent forwards
          <span className="inline-flex items-center rounded-md bg-gray-50 px-2 py-1 text-[10px] font-medium text-gray-600 ring-1 ring-gray-500/10 ring-inset sm:text-xs dark:bg-gray-800/50 dark:text-gray-400 dark:ring-gray-400/20">
            {sorted.length}
          </span>
        </CardTitle>
        <CardDescription>
          HTLCs routed through this node, newest first. Failed and in-flight
          forwards stay listed so you can see liquidity friction.
        </CardDescription>
      </CardHeader>
      <CardContent className="p-0">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Time</TableHead>
              <TableHead>Route</TableHead>
              <TableHead className="text-right">Amount</TableHead>
              <TableHead className="text-right">Fee</TableHead>
              <TableHead className="text-right">Status</TableHead>
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
  const amountSat = Math.round(forward.amountInMsat / 1_000);
  const feeSat = forward.feeMsat / 1_000;
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
        <span className="block text-sm">{shortDate(forward.occurredAt)}</span>
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
          {forward.status === "settled" && feeSat > 0
            ? `+ ${feeSat.toLocaleString("en-US")} sat`
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
          {forwardStatusLabels[forward.status]}
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

function formatNodeDate(value: string | null | undefined) {
  if (!value) return "—";
  const normalized = value.replace("T", " ").replace(/Z$/, "");
  return normalized.length > 16 ? normalized.slice(0, 16) : normalized;
}
