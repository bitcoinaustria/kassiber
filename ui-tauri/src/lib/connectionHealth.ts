import type { Net } from "@/components/kb/settings/SettingsModel";
import type { NetworkStatus } from "./networkStatus";

export type ConnectionProbeKind = "bitcoinrpc" | "electrum" | "http" | "unsupported";
export type ConnectionHealthStatus =
  | "unknown"
  | "checking"
  | "healthy"
  | "unhealthy"
  | "unavailable";
export type ConnectionIndicatorTone = "neutral" | "online" | "warning" | "error";

export const CONNECTION_HEALTH_CHECK_INTERVAL_MS = 60_000;
export const CONNECTION_HEALTH_CHECK_JITTER_MS = 5_000;

export interface ConnectionHealthInput {
  id: string;
  name: string;
  url: string;
  kind?: string | null;
  net: Net;
  allowDisplayHttpProbe?: boolean;
}

export interface ConnectionHealthSnapshot {
  status: ConnectionHealthStatus;
}

export interface ConnectionHealthCheckGateInput {
  checking: boolean;
  checkableConnectionCount: number;
  daemonEnabled: boolean;
  documentVisible: boolean;
  networkStatus: NetworkStatus;
}

export interface ImmediateConnectionHealthCheckInput {
  canCheckConnections: boolean;
  hasUncheckedConnection: boolean;
  lastCheckedAt?: string;
  nowMs?: number;
}

export function endpointWithPort(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed) return "Missing endpoint";
  try {
    const parsed = new URL(trimmed.includes("://") ? trimmed : `https://${trimmed}`);
    const protocol = parsed.protocol.replace(/:$/, "");
    const host = parsed.hostname.includes(":")
      ? `[${parsed.hostname}]`
      : parsed.hostname;
    const port =
      parsed.port ||
      (parsed.protocol === "https:" || parsed.protocol === "wss:"
        ? "443"
        : parsed.protocol === "http:" || parsed.protocol === "ws:"
          ? "80"
          : parsed.protocol === "ssl:"
            ? "50002"
            : parsed.protocol === "tcp:"
              ? "50001"
              : "");
    const pathname = parsed.pathname === "/" ? "" : parsed.pathname;
    const suffix = `${pathname}${parsed.search}`;
    return `${protocol}://${host}${port ? `:${port}` : ""}${suffix}`;
  } catch {
    return trimmed;
  }
}

export function connectionProbeKind(connection: ConnectionHealthInput): ConnectionProbeKind {
  const kind = (connection.kind ?? "").toLowerCase();
  const url = connection.url.trim().toLowerCase();
  const allowDisplayHttpProbe = connection.allowDisplayHttpProbe !== false;
  if (kind === "electrum" || url.startsWith("ssl://") || url.startsWith("tcp://")) {
    return "electrum";
  }
  if (
    kind === "bitcoinrpc" &&
    (url.startsWith("http://") || url.startsWith("https://"))
  ) {
    return "bitcoinrpc";
  }
  if (
    allowDisplayHttpProbe &&
    ["coingecko", "coinbase-exchange", "esplora", "liquid-esplora"].includes(
      kind,
    ) &&
    (url.startsWith("http://") || url.startsWith("https://"))
  ) {
    return "http";
  }
  return "unsupported";
}

export function settingsHashForConnection(connection: ConnectionHealthInput): string {
  switch (connection.net) {
    case "BTC":
      return "bitcoin";
    case "LIQUID":
      return "liquid";
    case "LN":
      return "lightning";
    case "FX":
      return "market";
  }
}

export function connectionHealthTone(
  networkStatus: NetworkStatus,
  snapshots: ConnectionHealthSnapshot[],
): ConnectionIndicatorTone {
  if (networkStatus === "offline") return "error";
  const unhealthy = snapshots.filter(
    (snapshot) => snapshot.status === "unhealthy",
  ).length;
  const healthy = snapshots.some((snapshot) => snapshot.status === "healthy");
  if (unhealthy > 0) return healthy ? "warning" : "error";
  if (snapshots.some((snapshot) => snapshot.status === "checking")) {
    return "warning";
  }
  if (healthy) return "online";
  return "neutral";
}

export function canRunConnectionHealthChecks({
  checking,
  checkableConnectionCount,
  daemonEnabled,
  documentVisible,
  networkStatus,
}: ConnectionHealthCheckGateInput): boolean {
  return (
    daemonEnabled &&
    documentVisible &&
    networkStatus === "online" &&
    !checking &&
    checkableConnectionCount > 0
  );
}

export function nextConnectionHealthCheckDelayMs(
  random: () => number = Math.random,
): number {
  const jitter =
    Math.round(random() * CONNECTION_HEALTH_CHECK_JITTER_MS * 2) -
    CONNECTION_HEALTH_CHECK_JITTER_MS;
  return CONNECTION_HEALTH_CHECK_INTERVAL_MS + jitter;
}

export function isConnectionHealthStale(
  checkedAt: string | undefined,
  nowMs: number = Date.now(),
): boolean {
  if (!checkedAt) return true;
  const checkedAtMs = Date.parse(checkedAt);
  if (Number.isNaN(checkedAtMs)) return true;
  return nowMs - checkedAtMs >= CONNECTION_HEALTH_CHECK_INTERVAL_MS;
}

export function shouldRunImmediateConnectionHealthCheck({
  canCheckConnections,
  hasUncheckedConnection,
  lastCheckedAt,
  nowMs,
}: ImmediateConnectionHealthCheckInput): boolean {
  if (!canCheckConnections) return false;
  return (
    hasUncheckedConnection ||
    isConnectionHealthStale(lastCheckedAt, nowMs)
  );
}
