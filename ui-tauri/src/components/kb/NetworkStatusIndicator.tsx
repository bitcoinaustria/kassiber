import { RefreshCw, Wifi, WifiOff } from "lucide-react";
import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import {
  abbreviateEndpointMiddle,
  canRunConnectionHealthChecks,
  connectionHealthTone,
  connectionProbeKind,
  endpointWithPort,
  nextConnectionHealthCheckDelayMs,
  settingsHashForConnection,
  shouldRunImmediateConnectionHealthCheck,
  type ConnectionHealthStatus,
  type ConnectionIndicatorTone,
  type ConnectionProbeKind,
} from "@/lib/connectionHealth";
import { isOnionEndpoint } from "@/lib/backendTrust";
import { cn } from "@/lib/utils";
import {
  networkStatusLabel,
  readNetworkStatus,
  subscribeNetworkStatus,
  type NetworkStatus,
} from "@/lib/networkStatus";
import { PENDING_SETTINGS_BACKEND_EDIT_KEY } from "./settingsSections";
import { visibleConnectionBackends } from "./backendConnectionRows";
import {
  backendProtocolLabel,
  backendRowToSettingsBackend,
  type Backend,
  type BackendSettingsData,
} from "./settings/SettingsModel";

type ConnectionHealthRow = {
  id: string;
  backendId?: string;
  name: string;
  endpoint: string;
  fingerprint: string;
  rawUrl: string;
  protocol: string;
  probeKind: ConnectionProbeKind;
  settingsHash: string;
  proxy?: string;
  trustSelfSigned?: boolean;
};

type ConnectionHealthRecord = {
  status: ConnectionHealthStatus;
  fingerprint?: string;
  message?: string;
  checkedAt?: string;
};

type BackendProbeEnvelope = {
  ok: boolean;
  logs?: string[];
  status?: number;
};

type BitcoinRpcProbeEnvelope = {
  reachable: boolean;
  chain?: string | null;
  network?: string | null;
  blocks?: number | null;
  headers?: number | null;
  peers?: number | null;
  status?: string | null;
  pruned?: boolean | null;
  ibd?: boolean | null;
  wallet_rpc?: {
    available?: boolean;
    error?: {
      message?: string;
      hint?: string;
    };
  } | null;
  block_filters?: {
    available?: boolean;
    error?: {
      message?: string;
      hint?: string;
    };
  } | null;
  warnings?: string[];
  error?: {
    message?: string;
    hint?: string;
  };
};

function bitcoinRpcHealth(
  payload: BitcoinRpcProbeEnvelope | undefined,
  t: TFunction<"chrome">,
) {
  if (!payload?.reachable) {
    return {
      ok: false,
      message: payload?.error?.message ?? t("network.checkFailed"),
    };
  }
  if (payload.wallet_rpc?.available === false) {
    return {
      ok: false,
      message:
        payload.wallet_rpc.error?.hint ??
        payload.wallet_rpc.error?.message ??
        t("network.coreWalletRpcUnavailable"),
    };
  }
  if (payload.ibd) {
    return { ok: true, message: t("network.coreInitialBlockDownload") };
  }
  if (payload.pruned) {
    return { ok: true, message: t("network.corePruned") };
  }
  return { ok: true, message: t("network.coreReachable") };
}

function connectionRowsFromBackends(
  savedBackends: Backend[],
): ConnectionHealthRow[] {
  const rows = visibleConnectionBackends(savedBackends)
    .filter((backend) => backend.on && backend.url.trim())
    .map((backend) =>
      connectionRowFromBackend(backend, `backend:${backend.id}`, backend.id),
    );
  const seen = new Set<string>();
  return rows.filter((row) => {
    const key = `${row.rawUrl}|${row.name}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function connectionRowFromBackend(
  backend: Backend,
  id: string,
  backendId?: string,
): ConnectionHealthRow {
  return {
    id,
    backendId,
    name: backend.name,
    endpoint: endpointWithPort(backend.url),
    fingerprint: [
      backend.url,
      backend.proxy ? `${backend.proxy.host}:${backend.proxy.port}` : "",
      backend.trustSsl ? "trust-self-signed" : "",
      backend.kind,
    ].join("|"),
    rawUrl: backend.url,
    protocol: backendProtocolLabel(backend),
    probeKind: connectionProbeKind({
      ...backend,
      allowDisplayHttpProbe:
        backendId === undefined || backend.urlSafeForHttpProbe === true,
    }),
    settingsHash: settingsHashForConnection(backend),
    proxy: backend.proxy
      ? `${backend.proxy.host}:${backend.proxy.port}`
      : undefined,
    trustSelfSigned: Boolean(backend.trustSsl),
  };
}

function rowHealthStatus(
  row: ConnectionHealthRow,
  records: Record<string, ConnectionHealthRecord>,
): ConnectionHealthStatus {
  if (!row.rawUrl.trim()) {
    return "unhealthy";
  }
  if (row.probeKind === "unsupported") {
    return "unavailable";
  }
  return records[row.id]?.status ?? "unknown";
}

type TFn = TFunction<"chrome">;

function connectionStatusLabel(status: ConnectionHealthStatus, t: TFn) {
  switch (status) {
    case "healthy":
      return t("network.health.healthy");
    case "unhealthy":
      return t("network.health.failed");
    case "checking":
      return t("network.health.checking");
    case "unavailable":
      return t("network.health.notCheckable");
    case "unknown":
    default:
      return t("network.health.notChecked");
  }
}

function connectionStatusText(
  row: ConnectionHealthRow,
  status: ConnectionHealthStatus,
  t: TFn,
) {
  if (status === "unavailable" && row.probeKind === "unsupported") {
    return t("network.health.skipped");
  }
  return connectionStatusLabel(status, t);
}

function connectionStatusTitle(
  row: ConnectionHealthRow,
  status: ConnectionHealthStatus,
  t: TFn,
  record?: ConnectionHealthRecord,
) {
  if (record?.message) return record.message;
  if (status === "unavailable" && row.probeKind === "unsupported") {
    return t("network.unsupportedTitle");
  }
  return connectionStatusLabel(status, t);
}

function connectionRouteLabel(row: ConnectionHealthRow, t: TFn) {
  const routeKind = connectionRouteKind(row);
  if (routeKind === "tor") {
    return t("network.route.tor");
  }
  if (routeKind === "proxy") {
    return t("network.route.proxy");
  }
  return null;
}

function connectionRouteKind(row: ConnectionHealthRow) {
  if (isOnionEndpoint(row.rawUrl)) return "tor";
  if (row.proxy) return "proxy";
  return null;
}

function connectionRouteTitle(row: ConnectionHealthRow, t: TFn) {
  if (isOnionEndpoint(row.rawUrl)) {
    return row.proxy
      ? t("network.route.torVia", { proxy: row.proxy })
      : t("network.route.torOnion");
  }
  if (row.proxy) {
    return t("network.route.proxyVia", { proxy: row.proxy });
  }
  return "";
}

function connectionDotClassName(
  status: ConnectionHealthStatus,
  routeKind?: "tor" | "proxy" | null,
) {
  if (routeKind === "tor") {
    return "bg-violet-500";
  }
  switch (status) {
    case "healthy":
      return "bg-emerald-500";
    case "unhealthy":
      return "bg-red-500";
    case "checking":
      return "bg-amber-500";
    case "unavailable":
      return "bg-slate-400";
    case "unknown":
    default:
      return "bg-muted-foreground/35";
  }
}

function connectionIndicatorClassName(tone: ConnectionIndicatorTone) {
  switch (tone) {
    case "error":
      return "border-red-500/35 bg-red-500/10 text-red-700 hover:bg-red-500/15 hover:text-red-700 dark:text-red-300 dark:hover:text-red-300";
    case "warning":
      return "border-amber-500/35 bg-amber-500/10 text-amber-700 hover:bg-amber-500/15 hover:text-amber-700 dark:text-amber-300 dark:hover:text-amber-300";
    case "online":
      return "border-emerald-500/20 bg-sidebar-accent/30 text-emerald-700 hover:bg-sidebar-accent/60 hover:text-emerald-700 dark:text-emerald-300 dark:hover:text-emerald-300";
    case "neutral":
    default:
      return "border-border bg-sidebar-accent/20 text-muted-foreground hover:bg-sidebar-accent/50 hover:text-foreground";
  }
}

function connectionIndicatorLabel(tone: ConnectionIndicatorTone, t: TFn) {
  switch (tone) {
    case "error":
      return t("network.indicator.errors");
    case "warning":
      return t("network.indicator.warning");
    case "online":
      return t("network.indicator.online");
    case "neutral":
    default:
      return t("network.indicator.neutral");
  }
}

function readDocumentVisible() {
  return typeof document === "undefined" || document.visibilityState === "visible";
}

export function NetworkStatusIndicator({
  daemonEnabled,
}: {
  daemonEnabled: boolean;
}) {
  const { t } = useTranslation("chrome");
  const navigate = useNavigate();
  const [status, setStatus] = React.useState<NetworkStatus>(() =>
    readNetworkStatus(),
  );
  const [open, setOpen] = React.useState(false);
  const [checking, setChecking] = React.useState(false);
  const [documentVisible, setDocumentVisible] = React.useState(() =>
    readDocumentVisible(),
  );
  const [healthRecords, setHealthRecords] = React.useState<
    Record<string, ConnectionHealthRecord>
  >({});
  const backendSettingsQuery = useDaemon<BackendSettingsData>(
    "ui.backends.settings.list",
    undefined,
    { enabled: daemonEnabled },
  );
  const testElectrum = useDaemonMutation<BackendProbeEnvelope>(
    "ui.backends.electrum.test",
  );
  const testHttp = useDaemonMutation<BackendProbeEnvelope>(
    "ui.backends.http.test",
  );
  const testBitcoinRpc = useDaemonMutation<BitcoinRpcProbeEnvelope>(
    "ui.backends.bitcoinrpc.test",
  );
  const savedBackends = React.useMemo(
    () =>
      (backendSettingsQuery.data?.data?.backends ?? []).map(
        backendRowToSettingsBackend,
      ),
    [backendSettingsQuery.data?.data?.backends],
  );
  const connectionRows = React.useMemo(
    () => connectionRowsFromBackends(savedBackends),
    [savedBackends],
  );
  const checkableRows = React.useMemo(
    () =>
      connectionRows.filter(
        (row) => row.rawUrl.trim() && row.probeKind !== "unsupported",
      ),
    [connectionRows],
  );
  const healthSnapshots = React.useMemo(
    () =>
      connectionRows.map((row) => ({
        status: rowHealthStatus(row, healthRecords),
      })),
    [connectionRows, healthRecords],
  );
  const indicatorTone = connectionHealthTone(status, healthSnapshots);
  const label =
    status === "offline"
      ? networkStatusLabel(status)
      : connectionIndicatorLabel(indicatorTone, t);
  const Icon = status === "offline" ? WifiOff : Wifi;
  const lastCheckedAt = Object.values(healthRecords)
    .map((record) => record.checkedAt)
    .filter((value): value is string => Boolean(value))
    .sort()
    .at(-1);
  const hasUncheckedConnection = checkableRows.some(
    (row) => healthRecords[row.id]?.fingerprint !== row.fingerprint,
  );
  const canCheckConnections = canRunConnectionHealthChecks({
    checking,
    checkableConnectionCount: checkableRows.length,
    daemonEnabled,
    documentVisible,
    networkStatus: status,
  });
  const shouldRunImmediateCheck = shouldRunImmediateConnectionHealthCheck({
    canCheckConnections,
    hasUncheckedConnection,
    lastCheckedAt,
  });

  React.useEffect(() => {
    return subscribeNetworkStatus(setStatus);
  }, []);

  React.useEffect(() => {
    const syncVisibility = () => setDocumentVisible(readDocumentVisible());

    syncVisibility();
    if (typeof document === "undefined") return () => {};

    document.addEventListener("visibilitychange", syncVisibility);
    return () => {
      document.removeEventListener("visibilitychange", syncVisibility);
    };
  }, []);

  const runConnectionChecks = React.useCallback(async () => {
    const now = new Date().toISOString();
    if (!canCheckConnections) return;
    setChecking(true);
    const results = await Promise.all(
      checkableRows.map(
        async (row): Promise<[string, ConnectionHealthRecord]> => {
          try {
            const envelope =
              row.probeKind === "electrum"
                ? await testElectrum.mutateAsync({
                    url: row.rawUrl,
                    trust_self_signed: row.trustSelfSigned,
                    proxy: row.proxy,
                    timeout: 5,
                  })
                : row.probeKind === "bitcoinrpc"
                  ? await testBitcoinRpc.mutateAsync({
                      backend: row.backendId,
                      url: row.backendId ? undefined : row.rawUrl,
                      timeout: 5,
                    })
                : await testHttp.mutateAsync({
                    url: row.rawUrl,
                    proxy: row.proxy,
                    timeout: 5,
                  });
            const payload = envelope.data;
            const coreHealth =
              row.probeKind === "bitcoinrpc"
                ? bitcoinRpcHealth(payload as BitcoinRpcProbeEnvelope | undefined, t)
                : undefined;
            const ok =
              coreHealth?.ok ??
              Boolean((payload as BackendProbeEnvelope | undefined)?.ok);
            return [
              row.id,
              {
                fingerprint: row.fingerprint,
                status: ok ? "healthy" : "unhealthy",
                message:
                  coreHealth?.message ??
                  (payload as BackendProbeEnvelope | undefined)?.logs?.at(-1) ??
                  (ok
                    ? t("network.checkPassed")
                    : t("network.checkFailed")),
                checkedAt: now,
              },
            ];
          } catch (error) {
            return [
              row.id,
              {
                fingerprint: row.fingerprint,
                status: "unhealthy",
                message:
                  error instanceof Error
                    ? error.message
                    : t("network.checkFailed"),
                checkedAt: now,
              },
            ];
          }
        },
      ),
    );
    setHealthRecords((current) => {
      const next = { ...current };
      for (const [id, record] of results) {
        next[id] = record;
      }
      return next;
    });
    setChecking(false);
  }, [
    canCheckConnections,
    checkableRows,
    t,
    testBitcoinRpc,
    testElectrum,
    testHttp,
  ]);

  React.useEffect(() => {
    if (!shouldRunImmediateCheck) return;
    void runConnectionChecks();
  }, [runConnectionChecks, shouldRunImmediateCheck]);

  React.useEffect(() => {
    if (!canCheckConnections || !lastCheckedAt) return undefined;
    const timeout = window.setTimeout(() => {
      void runConnectionChecks();
    }, nextConnectionHealthCheckDelayMs());

    return () => {
      window.clearTimeout(timeout);
    };
  }, [canCheckConnections, lastCheckedAt, runConnectionChecks]);

  const openSettingsConnection = React.useCallback(
    (row: ConnectionHealthRow) => {
      if (row.backendId) {
        window.sessionStorage.setItem(
          PENDING_SETTINGS_BACKEND_EDIT_KEY,
          row.backendId,
        );
      }
      void navigate({
        to: "/settings",
        hash: row.settingsHash,
      });
      window.dispatchEvent(
        new CustomEvent("kassiber:settings-section", {
          detail: {
            section: row.settingsHash,
            backendId: row.backendId ?? null,
          },
        }),
      );
      setOpen(false);
    },
    [navigate],
  );

  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className={cn(
            "relative size-8 shrink-0 border",
            connectionIndicatorClassName(indicatorTone),
          )}
          aria-label={label}
          title={label}
        >
          <Icon className="size-4" aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="w-[42rem] max-w-[calc(100vw-1rem)] sm:w-[46rem]"
      >
        <div className="flex items-center justify-between gap-2 px-2 py-1.5">
          <DropdownMenuLabel className="p-0">
            {t("network.outboundConnections")}
          </DropdownMenuLabel>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="size-7"
            disabled={!canCheckConnections}
            aria-label={t("network.checkConnections")}
            title={t("network.checkConnections")}
            onClick={(event) => {
              event.preventDefault();
              void runConnectionChecks();
            }}
          >
            <RefreshCw
              className={cn("size-3.5", checking && "animate-spin")}
              aria-hidden="true"
            />
          </Button>
        </div>
        <DropdownMenuSeparator />
        <div className="px-1 py-1">
          {backendSettingsQuery.isLoading ? (
            <div className="px-2 py-4 text-sm text-muted-foreground">
              {t("network.loading")}
            </div>
          ) : backendSettingsQuery.error ? (
            <div className="px-2 py-4 text-sm text-red-700 dark:text-red-300">
              {t("network.loadError")}
            </div>
          ) : connectionRows.length === 0 ? (
            <div className="px-2 py-4 text-sm text-muted-foreground">
              {t("network.noneConfigured")}
            </div>
          ) : (
            <Table className="table-fixed">
              <TableHeader>
                <TableRow>
                  <TableHead className="w-16">
                    {t("network.columnState")}
                  </TableHead>
                  <TableHead className="w-[38%]">
                    {t("network.columnConnection")}
                  </TableHead>
                  <TableHead className="hidden sm:table-cell">{t("network.columnEndpoint")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {connectionRows.map((row) => {
                  const rowStatus = rowHealthStatus(row, healthRecords);
                  const record = healthRecords[row.id];
                  const rowStatusText = connectionStatusText(row, rowStatus, t);
                  const rowStatusTitle = connectionStatusTitle(
                    row,
                    rowStatus,
                    t,
                    record,
                  );
                  const routeLabel = connectionRouteLabel(row, t);
                  const routeTitle = connectionRouteTitle(row, t);
                  const routeKind = connectionRouteKind(row);
                  const dotTitle =
                    routeKind === "tor" && routeTitle
                      ? `${rowStatusTitle} · ${routeTitle}`
                      : rowStatusTitle;
                  return (
                    <TableRow key={row.id}>
                      <TableCell>
                        <span
                          className={cn(
                            "block size-2.5 rounded-full",
                            connectionDotClassName(rowStatus, routeKind),
                          )}
                          aria-label={rowStatusText}
                          title={dotTitle}
                        />
                      </TableCell>
                      <TableCell className="min-w-0">
                        <button
                          type="button"
                          className="block min-w-0 text-left hover:text-primary focus-visible:rounded-sm focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                          onClick={() => openSettingsConnection(row)}
                        >
                          <span className="block truncate font-medium">
                            {row.name}
                          </span>
                          <span
                            className="block truncate text-xs text-muted-foreground"
                            title={rowStatusTitle}
                          >
                            {row.protocol} · {rowStatusText}
                            {routeLabel ? ` · ${routeLabel}` : ""}
                          </span>
                        </button>
                      </TableCell>
                      <TableCell className="hidden min-w-0 sm:table-cell">
                        <button
                          type="button"
                          className="flex max-w-full items-center gap-2 text-left font-mono text-xs text-muted-foreground hover:text-primary focus-visible:rounded-sm focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                          title={row.endpoint}
                          onClick={() => openSettingsConnection(row)}
                        >
                          <span className="min-w-0 truncate">
                            {abbreviateEndpointMiddle(row.endpoint)}
                          </span>
                        </button>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
          )}
        </div>
        {lastCheckedAt ? (
          <div className="border-t px-3 py-2 text-[11px] text-muted-foreground">
            {t("network.lastChecked", {
              time: new Date(lastCheckedAt).toLocaleTimeString(),
            })}
          </div>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
