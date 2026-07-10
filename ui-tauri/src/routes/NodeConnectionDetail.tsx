/**
 * Lightning node detail surface.
 *
 * Renders channels, capacity, and routing snapshot for `lnd` / `core-ln`
 * / `nwc` connections. The shape lives on `Connection.node` and is mocked
 * in dev mode until the LND/CLN sync daemon kinds are merged; when they
 * land, the daemon should map its status snapshot into the same shape.
 */

import { Fragment, useMemo, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { Link } from "@tanstack/react-router";
import {
  Activity,
  ArrowDownRight,
  ArrowLeft,
  ArrowRight,
  ArrowUpRight,
  Cable,
  ClipboardList,
  Coins,
  ExternalLink,
  Gauge,
  Globe2,
  ReceiptText,
  RefreshCw,
  Repeat,
  Server,
  ShieldCheck,
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
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { connectionKindLabels } from "@/lib/connectionDisplay";
import { formatShortDate } from "@/lib/date";
import { DEFAULT_OPEN_COST_SAT } from "@/lib/lightning";
import {
  pageHeaderActionClassName,
  pageHeaderActionsClassName,
  pageHeaderClassName,
  pageHeaderIconButtonClassName,
  screenShellClassName,
} from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import { MISSING_FIAT_LABEL } from "@/lib/currency";
import { formatCount, formatSats } from "@/lib/localeFormat";
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
  formatSats(value, { unit: "sat" });
const fmtEur = (value: number | null) =>
  value === null
    ? MISSING_FIAT_LABEL
    : "€ " +
      value.toLocaleString("de-AT", {
        minimumFractionDigits: 2,
        maximumFractionDigits: 2,
      });
const fmtSatSigned = (value: number) =>
  `${value >= 0 ? "+ " : "- "}${formatSats(Math.abs(value), { unit: "sat" })}`;
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

type NodeChannelCloseKind = NonNullable<NodeChannel["closeKind"]>;

const channelCloseKindLabelKeys = {
  cooperative: "node.closeKind.cooperative",
  force: "node.closeKind.force",
  breach: "node.closeKind.breach",
} as const satisfies Record<NodeChannelCloseKind, string>;

const channelCloseKindLabel = (
  closeKind: NodeChannel["closeKind"] | undefined,
  t: TFunction<"connections">,
) =>
  closeKind
    ? t(channelCloseKindLabelKeys[closeKind])
    : t("node.channels.closedFallback");

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
  isSnapshotLoading?: boolean;
  snapshotErrorMessage?: string | null;
}

export function NodeConnectionDetail({
  connection,
  priceEur,
  hideSensitive,
  onSync,
  isSyncRunning,
  isSnapshotLoading = false,
  snapshotErrorMessage = null,
}: NodeConnectionDetailProps) {
  const { t } = useTranslation("connections");
  const node = connection.node;
  const isBusy = isSyncRunning || isSnapshotLoading;
  const refreshButtonLabel = isBusy
    ? t("node.header.refreshing")
    : t("node.header.refresh");

  if (!node) {
    return (
      <div className={screenShellClassName}>
        <NodeHeader
          connection={connection}
          isSyncRunning={isBusy}
          refreshButtonLabel={refreshButtonLabel}
          onSync={onSync}
        />
        <Card>
          <CardHeader className="border-b px-4 pb-3">
            <CardTitle className="text-sm sm:text-base">
              {isSnapshotLoading
                ? t("node.loadingSnapshot.title")
                : t("node.noSnapshot.title")}
            </CardTitle>
            <CardDescription>
              {isSnapshotLoading
                ? t("node.loadingSnapshot.description")
                : snapshotErrorMessage
                  ? t("node.noSnapshot.errorDescription", {
                      message: snapshotErrorMessage,
                    })
                  : t("node.noSnapshot.description")}
            </CardDescription>
          </CardHeader>
          <CardContent className="flex items-start gap-2 px-4 pt-4">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={isBusy}
              aria-busy={isBusy}
              onClick={onSync}
            >
              <RefreshCw
                className={cn("size-4", isBusy && "animate-spin")}
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
        isSyncRunning={isBusy}
        refreshButtonLabel={refreshButtonLabel}
        onSync={onSync}
      />

      <NodeOperatorHero
        connection={connection}
        node={node}
        priceEur={priceEur}
        hideSensitive={hideSensitive}
        isSyncRunning={isBusy}
        refreshButtonLabel={refreshButtonLabel}
        onSync={onSync}
      />

      <Tabs defaultValue="overview" className="gap-3">
        <TabsList className="grid h-auto w-full grid-cols-2 gap-1 p-1 sm:grid-cols-5">
          <TabsTrigger value="overview">{t("node.tabs.overview")}</TabsTrigger>
          <TabsTrigger value="channels">{t("node.tabs.channels")}</TabsTrigger>
          <TabsTrigger value="activity">{t("node.tabs.activity")}</TabsTrigger>
          <TabsTrigger value="profitability">
            {t("node.tabs.profitability")}
          </TabsTrigger>
          <TabsTrigger value="accounting">
            {t("node.tabs.accounting")}
          </TabsTrigger>
        </TabsList>

        <TabsContent value="overview" className="space-y-3">
          <NodeMetrics
            node={node}
            priceEur={priceEur}
            hideSensitive={hideSensitive}
          />
          <div className="grid grid-cols-1 gap-3 xl:grid-cols-[minmax(0,1fr)_minmax(360px,0.8fr)]">
            <NodeActivityCard node={node} />
            <AccountingReadinessCard connection={connection} node={node} />
          </div>
        </TabsContent>

        <TabsContent value="channels" className="space-y-3">
          <ChannelsCard
            channels={node.channels}
            closedChannels={node.closedChannels ?? []}
            hideSensitive={hideSensitive}
            totalCapacitySat={node.totalCapacitySat}
          />
        </TabsContent>

        <TabsContent value="activity" className="space-y-3">
          <NodeActivityCard node={node} />
          {node.forwards && node.forwards.length > 0 ? (
            <ForwardsCard
              forwards={node.forwards}
              hideSensitive={hideSensitive}
            />
          ) : null}
        </TabsContent>

        <TabsContent value="profitability" className="space-y-3">
          {node.routing ? (
            <RoutingSummary
              routing={node.routing}
              hideSensitive={hideSensitive}
              priceEur={priceEur}
            />
          ) : null}
          <ChannelEconomicsCard
            channels={node.channels}
            hideSensitive={hideSensitive}
          />
        </TabsContent>

        <TabsContent value="accounting" className="space-y-3">
          <div className="grid grid-cols-1 gap-3 xl:grid-cols-[minmax(0,1fr)_minmax(360px,0.8fr)]">
            <AccountingReadinessCard connection={connection} node={node} />
            <NodeDetailsCard
              connection={connection}
              node={node}
              hideSensitive={hideSensitive}
            />
          </div>
        </TabsContent>
      </Tabs>
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
    <div className={pageHeaderClassName}>
      <div className="flex min-w-0 items-center gap-3">
        <Button
          asChild
          variant="outline"
          size="icon"
          className={cn(pageHeaderIconButtonClassName, "shrink-0")}
        >
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
      <div className={cn(pageHeaderActionsClassName, "shrink-0 self-start sm:self-center")}>
        <Button
          type="button"
          variant="outline"
          size="sm"
          className={pageHeaderActionClassName}
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
          <span className="grid justify-items-center">
            <span
              aria-hidden="true"
              className="invisible col-start-1 row-start-1"
            >
              {t("node.header.refresh")}
            </span>
            <span
              aria-hidden="true"
              className="invisible col-start-1 row-start-1"
            >
              {t("node.header.refreshing")}
            </span>
            <span className="col-start-1 row-start-1">
              {refreshButtonLabel}
            </span>
          </span>
        </Button>
      </div>
    </div>
  );
}

interface NodeOperatorHeroProps {
  connection: Connection;
  node: NodeSnapshot;
  priceEur: number;
  hideSensitive: boolean;
  isSyncRunning: boolean;
  refreshButtonLabel: string;
  onSync: () => void;
}

function NodeOperatorHero({
  connection,
  node,
  priceEur,
  hideSensitive,
  isSyncRunning,
  refreshButtonLabel,
  onSync,
}: NodeOperatorHeroProps) {
  const { t } = useTranslation("connections");
  const activeChannels = node.channels.filter(
    (channel) => channel.state === "active",
  );
  const totalCapacitySat = Math.max(0, node.totalCapacitySat);
  const localPct =
    totalCapacitySat > 0
      ? Math.min(
          100,
          Math.max(0, (node.totalLocalBalanceSat / totalCapacitySat) * 100),
        )
      : 0;
  const remotePct =
    totalCapacitySat > 0
      ? Math.min(
          100,
          Math.max(0, (node.totalRemoteBalanceSat / totalCapacitySat) * 100),
        )
      : 0;
  const inboundRatio =
    totalCapacitySat > 0 ? node.totalRemoteBalanceSat / totalCapacitySat : 0;
  const outboundRatio =
    totalCapacitySat > 0 ? node.totalLocalBalanceSat / totalCapacitySat : 0;
  const signalKey =
    activeChannels.length === 0
      ? "noActiveChannels"
      : inboundRatio < 0.15
        ? "lowReceive"
        : outboundRatio < 0.15
          ? "lowSend"
          : "balanced";
  const netRoutingSat = node.routing?.netProfitSat ?? 0;
  const netRoutingBtc = netRoutingSat / 100_000_000;

  return (
    <Card className="rounded-xl">
      <CardContent className="grid gap-4 px-4 py-4 xl:grid-cols-[minmax(0,1fr)_220px]">
        <div className="min-w-0 space-y-4">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <Badge variant="secondary" className="rounded-md">
              {t("node.hero.operatorView")}
            </Badge>
            <span
              className={cn(
                "inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-xs font-medium ring-1 ring-inset",
                signalKey === "balanced"
                  ? "bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-300 dark:ring-emerald-400/20"
                  : signalKey === "noActiveChannels"
                    ? "bg-red-50 text-red-700 ring-red-600/20 dark:bg-red-900/30 dark:text-red-300 dark:ring-red-400/20"
                    : "bg-amber-50 text-amber-700 ring-amber-600/20 dark:bg-amber-900/30 dark:text-amber-300 dark:ring-amber-400/20",
              )}
            >
              <Gauge className="size-3.5" aria-hidden="true" />
              {t(`node.hero.signal.${signalKey}`)}
            </span>
            <span className="text-xs text-muted-foreground">
              {t("node.hero.snapshotLine", {
                channels: formatCount(activeChannels.length),
                forwards: formatCount(
                  node.routing?.forwardCount ??
                  node.forwards?.length ??
                  0,
                ),
                peers: formatCount(node.peerCount),
              })}
            </span>
          </div>

          <div className="grid gap-3 md:grid-cols-2 2xl:grid-cols-4">
            <HeroStat
              label={t("node.hero.canSend")}
              value={fmtSat(node.totalLocalBalanceSat)}
              detail={fmtEur(
                (node.totalLocalBalanceSat / 100_000_000) * priceEur,
              )}
              icon={<Zap className="size-4" aria-hidden="true" />}
              tone="text-emerald-700 dark:text-emerald-300"
              hideSensitive={hideSensitive}
            />
            <HeroStat
              label={t("node.hero.canReceive")}
              value={fmtSat(node.totalRemoteBalanceSat)}
              detail={t("node.hero.remoteCapacity")}
              icon={<Cable className="size-4" aria-hidden="true" />}
              tone="text-sky-700 dark:text-sky-300"
              hideSensitive={hideSensitive}
            />
            <HeroStat
              label={t("node.hero.onchainReserve")}
              value={fmtSat(node.onchainBalanceSat)}
              detail={fmtEur(
                (node.onchainBalanceSat / 100_000_000) * priceEur,
              )}
              icon={<Coins className="size-4" aria-hidden="true" />}
              tone="text-foreground"
              hideSensitive={hideSensitive}
            />
            <HeroStat
              label={t("node.hero.netRouting")}
              value={fmtSatSigned(netRoutingSat)}
              detail={fmtEur(netRoutingBtc * priceEur)}
              icon={<TrendingUp className="size-4" aria-hidden="true" />}
              tone={
                netRoutingSat > 0
                  ? "text-emerald-700 dark:text-emerald-300"
                  : netRoutingSat < 0
                    ? "text-red-700 dark:text-red-300"
                    : "text-muted-foreground"
              }
              hideSensitive={hideSensitive}
            />
          </div>

          <div className="space-y-2">
            <div
              className="flex h-3 w-full overflow-hidden rounded-full bg-muted"
              role="img"
              aria-label={t("node.hero.liquidityAria", {
                local: formatCount(node.totalLocalBalanceSat),
                remote: formatCount(node.totalRemoteBalanceSat),
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
            <div className="flex flex-wrap items-center justify-between gap-2 text-[11px] text-muted-foreground">
              <span className={cn("tabular-nums", blurClass(hideSensitive))}>
                <span className="text-emerald-700 dark:text-emerald-300">
                  {Math.round(localPct)}%
                </span>{" "}
                {t("node.hero.localShare")}
              </span>
              <span className={cn("tabular-nums", blurClass(hideSensitive))}>
                <span className="text-sky-700 dark:text-sky-300">
                  {Math.round(remotePct)}%
                </span>{" "}
                {t("node.hero.remoteShare")}
              </span>
              <span className={cn("tabular-nums", blurClass(hideSensitive))}>
                {t("node.hero.capacity", { value: fmtSat(totalCapacitySat) })}
              </span>
            </div>
          </div>
        </div>

        <div className="flex flex-col gap-2 xl:items-stretch xl:justify-center">
          <Button
            type="button"
            variant="default"
            size="sm"
            disabled={isSyncRunning}
            aria-busy={isSyncRunning}
            onClick={onSync}
          >
            <RefreshCw
              className={cn("size-4", isSyncRunning && "animate-spin")}
              aria-hidden="true"
            />
            {refreshButtonLabel}
          </Button>
          <Button asChild type="button" variant="outline" size="sm">
            <Link
              to="/transactions"
              search={{ wallet: connection.label }}
              hash="transactions-table"
            >
              <ReceiptText className="size-4" aria-hidden="true" />
              {t("node.hero.viewTransactions")}
            </Link>
          </Button>
          <Button asChild type="button" variant="outline" size="sm">
            <Link to="/reports">
              <ArrowRight className="size-4" aria-hidden="true" />
              {t("node.hero.openReports")}
            </Link>
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

interface HeroStatProps {
  label: string;
  value: string;
  detail: string;
  icon: ReactNode;
  tone: string;
  hideSensitive: boolean;
}

function HeroStat({
  label,
  value,
  detail,
  icon,
  tone,
  hideSensitive,
}: HeroStatProps) {
  return (
    <div className="min-w-0 rounded-md border bg-background px-3 py-2.5">
      <div className="flex items-center gap-2 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
        {icon}
        <span className="truncate">{label}</span>
      </div>
      <p
        className={cn(
          "mt-1 truncate font-mono text-base font-semibold tabular-nums",
          tone,
          blurClass(hideSensitive),
        )}
      >
        {value}
      </p>
      <p
        className={cn(
          "mt-0.5 truncate text-[11px] text-muted-foreground",
          blurClass(hideSensitive),
        )}
      >
        {detail}
      </p>
    </div>
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
          sat: formatCount(node.totalLocalBalanceSat),
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

function NodeActivityCard({ node }: { node: NodeSnapshot }) {
  const { t } = useTranslation("connections");
  const paidInvoices = node.paidInvoiceCount ?? 0;
  const invoiceCount = node.invoiceCount ?? paidInvoices;
  const completedPayments = node.completedPaymentCount ?? 0;
  const paymentCount = node.paymentCount ?? completedPayments;
  const failedOrExpired =
    (node.failedPaymentCount ?? 0) + (node.expiredInvoiceCount ?? 0);
  const forwardCount = node.routing?.forwardCount ?? node.forwards?.length ?? 0;
  return (
    <Card>
      <CardHeader className="border-b px-4 pb-3">
        <CardTitle className="flex items-center gap-2 text-sm sm:text-base">
          <Activity className="size-4" aria-hidden="true" />
          {t("node.activity.title")}
        </CardTitle>
        <CardDescription>{t("node.activity.description")}</CardDescription>
      </CardHeader>
      <CardContent className="grid gap-3 px-4 pt-4 sm:grid-cols-2 xl:grid-cols-4">
        <CompactStat
          label={t("node.activity.invoices")}
          value={`${formatCount(paidInvoices)} / ${formatCount(invoiceCount)}`}
          detail={t("node.activity.paid")}
          icon={<ArrowDownRight className="size-4" aria-hidden="true" />}
        />
        <CompactStat
          label={t("node.activity.payments")}
          value={`${formatCount(completedPayments)} / ${formatCount(paymentCount)}`}
          detail={t("node.activity.completed")}
          icon={<ArrowUpRight className="size-4" aria-hidden="true" />}
        />
        <CompactStat
          label={t("node.activity.forwards")}
          value={formatCount(forwardCount)}
          detail={node.routing?.windowLabel ?? t("node.activity.snapshotWindow")}
          icon={<Repeat className="size-4" aria-hidden="true" />}
        />
        <CompactStat
          label={t("node.activity.exceptions")}
          value={formatCount(failedOrExpired)}
          detail={t("node.activity.failedExpired")}
          icon={<XCircle className="size-4" aria-hidden="true" />}
        />
      </CardContent>
    </Card>
  );
}

interface AccountingReadinessCardProps {
  connection: Connection;
  node: NodeSnapshot;
}

function AccountingReadinessCard({
  connection,
  node,
}: AccountingReadinessCardProps) {
  const { t } = useTranslation("connections");
  const importedTransactions = connection.transactionCount ?? 0;
  const bookedInvoiceCount = node.paidInvoiceCount ?? 0;
  const operationalEvents =
    bookedInvoiceCount +
    (node.completedPaymentCount ?? 0) +
    (node.routing?.forwardCount ?? 0);
  return (
    <Card>
      <CardHeader className="border-b px-4 pb-3">
        <CardTitle className="flex items-center gap-2 text-sm sm:text-base">
          <ClipboardList className="size-4" aria-hidden="true" />
          {t("node.accounting.title")}
        </CardTitle>
        <CardDescription>{t("node.accounting.description")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3 px-4 pt-4">
        <div className="grid gap-3 sm:grid-cols-3">
          <CompactStat
            label={t("node.accounting.imported")}
            value={formatCount(importedTransactions)}
            detail={t("node.accounting.transactions")}
            icon={<ReceiptText className="size-4" aria-hidden="true" />}
          />
          <CompactStat
            label={t("node.accounting.bookedInvoices")}
            value={formatCount(bookedInvoiceCount)}
            detail={t("node.accounting.finalized")}
            icon={<ShieldCheck className="size-4" aria-hidden="true" />}
          />
          <CompactStat
            label={t("node.accounting.nodeEvents")}
            value={formatCount(operationalEvents)}
            detail={t("node.accounting.auditTrail")}
            icon={<Activity className="size-4" aria-hidden="true" />}
          />
        </div>
        <div className="flex flex-wrap gap-2">
          <Button asChild type="button" variant="outline" size="sm">
            <Link
              to="/transactions"
              search={{ wallet: connection.label }}
              hash="transactions-table"
            >
              <ReceiptText className="size-4" aria-hidden="true" />
              {t("node.accounting.viewTransactions")}
            </Link>
          </Button>
          <Button asChild type="button" variant="outline" size="sm">
            <Link to="/journals">
              <ClipboardList className="size-4" aria-hidden="true" />
              {t("node.accounting.openJournals")}
            </Link>
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

function ChannelEconomicsCard({
  channels,
  hideSensitive,
}: {
  channels: NodeChannel[];
  hideSensitive: boolean;
}) {
  const { t } = useTranslation("connections");
  const rows = useMemo(
    () =>
      [...channels].sort(
        (a, b) => (b.earnedRoutingSat ?? 0) - (a.earnedRoutingSat ?? 0),
      ),
    [channels],
  );
  return (
    <Card>
      <CardHeader className="border-b px-4 pb-3">
        <CardTitle className="flex items-center gap-2 text-sm sm:text-base">
          <TrendingUp className="size-4" aria-hidden="true" />
          {t("node.channelEconomics.title")}
        </CardTitle>
        <CardDescription>
          {t("node.channelEconomics.description", {
            cost: formatCount(DEFAULT_OPEN_COST_SAT),
          })}
        </CardDescription>
      </CardHeader>
      <CardContent className="p-0">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{t("node.channelEconomics.peer")}</TableHead>
              <TableHead className="text-right">
                {t("node.channelEconomics.forwards")}
              </TableHead>
              <TableHead className="text-right">
                {t("node.channelEconomics.earned")}
              </TableHead>
              <TableHead className="text-right">
                {t("node.channelEconomics.openCost")}
              </TableHead>
              <TableHead className="text-right">
                {t("node.channelEconomics.state")}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.length === 0 ? (
              <TableRow>
                <TableCell
                  className="text-center text-sm text-muted-foreground"
                  colSpan={5}
                >
                  {t("node.channelEconomics.empty")}
                </TableCell>
              </TableRow>
            ) : (
              rows.map((channel) => {
                const earned = channel.earnedRoutingSat ?? 0;
                const coversOpenCost = earned >= DEFAULT_OPEN_COST_SAT;
                return (
                  <TableRow key={channel.id}>
                    <TableCell className="min-w-0 align-top">
                      <span className="block truncate text-sm font-medium">
                        {channel.peerAlias}
                      </span>
                      <span className="block truncate font-mono text-[10px] text-muted-foreground sm:text-xs">
                        {channel.shortChannelId ?? channel.id}
                      </span>
                    </TableCell>
                    <TableCell className="text-right font-mono text-sm tabular-nums">
                      {formatCount(channel.forwardCount ?? 0)}
                    </TableCell>
                    <TableCell
                      className={cn(
                        "text-right font-mono text-sm tabular-nums",
                        blurClass(hideSensitive),
                      )}
                    >
                      {fmtSat(earned)}
                    </TableCell>
                    <TableCell className="text-right">
                      <span
                        className={cn(
                          "inline-flex items-center rounded-md px-1.5 py-0.5 text-[10px] font-medium ring-1 ring-inset",
                          coversOpenCost
                            ? "bg-emerald-50 text-emerald-700 ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-300 dark:ring-emerald-400/20"
                            : "bg-muted text-muted-foreground ring-border",
                        )}
                      >
                        {coversOpenCost
                          ? t("node.channelEconomics.covered")
                          : t("node.channelEconomics.notYet")}
                      </span>
                    </TableCell>
                    <TableCell className="text-right">
                      <span
                        className={cn(
                          "inline-flex items-center rounded-md px-1.5 py-0.5 text-[10px] font-medium ring-1 ring-inset",
                          channelStateTone[channel.state],
                        )}
                      >
                        {channelStateLabel(channel.state, t)}
                      </span>
                    </TableCell>
                  </TableRow>
                );
              })
            )}
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  );
}

interface CompactStatProps {
  label: string;
  value: string;
  detail: string;
  icon: ReactNode;
}

function CompactStat({ label, value, detail, icon }: CompactStatProps) {
  return (
    <div className="min-w-0 rounded-md border bg-background px-3 py-2.5">
      <div className="flex items-center gap-2 text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
        {icon}
        <span className="truncate">{label}</span>
      </div>
      <p className="mt-1 truncate font-mono text-sm font-semibold tabular-nums">
        {value}
      </p>
      <p className="mt-0.5 truncate text-[11px] text-muted-foreground">
        {detail}
      </p>
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
            forwards: formatCount(routing.forwardCount),
            payments: formatCount(routing.paymentCount),
            rebalances: formatCount(routing.rebalanceCount),
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
                    value: formatCount(sortedClosed.length),
                  })
                : t("node.channels.showClosed", {
                    value: formatCount(sortedClosed.length),
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
              : t("node.channels.unannouncedPeer")}
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
              local: formatCount(channel.localBalanceSat),
              capacity: formatCount(channel.capacitySat),
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
                value:
                  channel.feeRatePpm === null || channel.feeRatePpm === undefined
                    ? "—"
                    : formatCount(channel.feeRatePpm),
              })}
            </span>
            <span className="text-[10px] text-muted-foreground sm:text-xs">
              {t("node.channels.baseMsat", {
                value: formatCount(channel.baseFeeMsat ?? 0),
              })}
            </span>
          </Fragment>
        )}
        {channel.forwardCount ? (
          <span className="mt-0.5 block text-[10px] text-muted-foreground sm:text-xs">
            {t("node.channels.forwardsEarned", {
              value: formatCount(channel.forwardCount),
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
            {channelCloseKindLabel(channel.closeKind, t)} ·{" "}
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
              value={formatCount(node.blockHeight)}
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
              value={formatCount(connection.transactionCount)}
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
              local: formatCount(channel.localBalanceSat),
              capacity: formatCount(channel.capacitySat),
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
          {channel.isPrivate ? (
            <DetailRow
              label={t("node.channelDetail.visibility")}
              value={t("node.channelDetail.privateVisibility")}
            />
          ) : null}
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
                channel.closeKind
                  ? ` · ${channelCloseKindLabel(channel.closeKind, t)}`
                  : ""
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
              value={formatCount(channel.htlcCount)}
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
              value: formatCount(channel.feeRatePpm ?? 0),
            })}
          />
          <DetailRow
            label={t("node.channelDetail.baseFee")}
            value={t("node.channelDetail.baseFeeValue", {
              value: formatCount(channel.baseFeeMsat ?? 0),
            })}
          />
          {typeof channel.forwardCount === "number" ? (
            <DetailRow
              label={t("node.channelDetail.forwardsWindow")}
              value={formatCount(channel.forwardCount)}
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
                  value: formatCount(feeMsat),
                })
              : t("node.forwards.feeSat", {
                  value: formatCount(feeSat),
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
