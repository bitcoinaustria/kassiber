import { normalizeExplorerBaseUrl, type ExplorerSettings } from "@/lib/explorer";
import { backendTrustFromEndpoint, type InfrastructureOwnership } from "@/lib/backendTrust";
import type { AiModelRow } from "@/lib/aiCapabilities";
import {
  CLN_PRESENCE_SENTINEL_COMMANDO_PEER,
  CLN_PRESENCE_SENTINEL_LIGHTNING_DIR,
  CLN_PRESENCE_SENTINEL_RPC_FILE,
} from "@/lib/lightning";
import {
  DEFAULT_BACKEND_NAME,
  DEFAULT_BACKEND_URL,
} from "@/components/kb/Onboarding/constants";

export const PLAINTEXT_DELETE_ACK = "DELETE LOCAL DATA";
export const KRAKEN_OHLCVT_SUPPORT_URL =
  "https://support.kraken.com/hc/articles/360047124832-Downloadable-historical-OHLCVT-Open-High-Low-Close-Volume-Trades-data";
export const KRAKEN_MARKET_DATA_BLOG_URL =
  "https://blog.kraken.com/product/api/unlocked-3-the-market-data-feeds-systematic-traders-use";

export type Net = "BTC" | "LIQUID" | "LN" | "FX";

export interface Backend {
  id: string;
  name: string;
  url: string;
  net: Net;
  kind?: string;
  chain?: string;
  network?: string;
  health: string;
  on: boolean;
  isDefault?: boolean;
  auth: string;
  authHeader?: string;
  token?: string;
  username?: string;
  password?: string;
  commandoPeerId?: string;
  lightningCli?: string;
  lightningDir?: string;
  rpcFile?: string;
  trustSsl?: boolean;
  infrastructureOwner?: InfrastructureOwnership;
  certificate?: string;
  proxy?: {
    host: string;
    port: string;
  } | null;
}

export interface BackendSettingsRow {
  name: string;
  kind?: string;
  chain?: string;
  network?: string;
  url?: string;
  source?: string;
  is_default?: boolean;
  has_url?: boolean;
  has_auth_header?: boolean;
  has_token?: boolean;
  has_certificate?: boolean;
  has_username?: boolean;
  has_password?: boolean;
  has_commando_peer_id?: boolean;
  has_lightning_dir?: boolean;
  has_rpc_file?: boolean;
  insecure?: boolean;
  tor_proxy?: string;
  infrastructure_owner?: string;
}

export interface BackendSettingsData {
  backends: BackendSettingsRow[];
  summary: {
    count: number;
    default_backend: string | null;
  };
}

export interface StatusData {
  data_root: string;
  database: string;
  current_workspace: string | null;
  current_profile: string | null;
  workspaces: number;
  profiles: number;
}

export interface ResetBookData {
  reset: boolean;
  removed: Record<string, number>;
  preserved: Record<string, number>;
  rates_scope: "global" | "preserved";
  shared_rates_cleared: boolean;
}

export type KrakenRatesImportOperation = "full" | "incremental";

export interface KrakenRatesImportSummaryRow {
  pair: string;
  samples: number;
  files: number;
  skipped_rows: number;
  skipped_files: number;
  first_timestamp: string | null;
  last_timestamp: string | null;
  granularity?: string | null;
}

export interface KrakenRatesImportData {
  source: "kraken-csv";
  operation: KrakenRatesImportOperation;
  path: string;
  bundled?: boolean;
  pair: string | null;
  summary: KrakenRatesImportSummaryRow[];
  totals: {
    pairs: number;
    samples: number;
    rows: number;
    files: number;
    skipped_rows: number;
    skipped_files: number;
  };
}

export interface RateRebuildData {
  source: string;
  pair: string | null;
  reprice_transactions: boolean;
  deleted: {
    rates: number;
    checked_minutes: number;
    transaction_prices: number;
    profiles_invalidated: number;
  };
  sync: Array<{
    pair: string;
    samples?: number;
    windows?: number;
    missing_minutes?: number;
    checked_minutes?: number;
  }>;
  reprice?: {
    auto_priced?: number;
  } | null;
  journals?:
    | {
        ok: true;
        result?: RateRebuildJournalResult | null;
      }
    | {
        ok: false;
        error?: DaemonErrorPayload | null;
      }
    | RateRebuildJournalResult
    | null;
}

export interface RateRebuildJournalResult {
  entries_created?: number;
  quarantined?: number;
  auto_priced?: number;
}

export interface DaemonErrorPayload {
  code?: string;
  message?: string;
  hint?: string | null;
  retryable?: boolean;
}

export type AiSecretStoreId = "macos_keychain" | "windows_dpapi" | "linux_secret_service" | "sqlcipher_inline";
export type AiSecretState = "ok" | "missing" | "needs_reauth" | "unavailable";

export interface AiSecretStorePolicy {
  platform?: "macos" | "windows" | "linux" | "unsupported";
  default?: {
    store_id: AiSecretStoreId;
    native_store_id?: AiSecretStoreId | null;
    native_available: boolean;
    warning?: string | null;
  };
  availability?: {
    state: "available" | "locked_needs_unlock" | "unavailable";
    reason?: string;
  };
}

export interface AiProviderRow {
  name: string;
  base_url: string;
  kind: "local" | "remote" | "tee";
  default_model?: string | null;
  notes?: string | null;
  has_api_key: boolean;
  secret_ref?: {
    store_id: AiSecretStoreId;
    state: AiSecretState;
  };
  is_default: boolean;
  acknowledged_at?: string | null;
}

export interface AiProvidersListData {
  providers: AiProviderRow[];
  default: string | null;
  secret_store_policy?: AiSecretStorePolicy;
}

export const AI_KIND_BADGE: Record<AiProviderRow["kind"], string> = {
  local:
    "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  remote:
    "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  tee: "border-sky-500/25 bg-sky-500/10 text-sky-700 dark:text-sky-300",
};

export function isCliAiProvider(row: AiProviderRow): boolean {
  return (
    row.base_url === "claude-cli://default" ||
    row.base_url === "codex-cli://default"
  );
}

export function aiSecretStoreLabel(storeId: AiSecretStoreId | undefined): string {
  switch (storeId) {
    case "macos_keychain":
      return "macOS Keychain";
    case "windows_dpapi":
      return "Windows Credential Manager";
    case "linux_secret_service":
      return "Secret Service";
    case "sqlcipher_inline":
    default:
      return "SQLCipher inline";
  }
}

export function aiSecretStateLabel(state: AiSecretState | undefined): string {
  switch (state) {
    case "ok":
      return "ok";
    case "needs_reauth":
      return "needs re-entry";
    case "unavailable":
      return "unavailable";
    case "missing":
    default:
      return "missing";
  }
}

export function formatModelSummary(models: AiModelRow[]): string {
  const ids = models
    .map((model) => model.id)
    .filter((id, index, values) => id && values.indexOf(id) === index);
  if (ids.length <= 3) return ids.join(", ");
  return `${ids.slice(0, 3).join(", ")} +${ids.length - 3}`;
}

export const DEFAULT_BACKENDS: Backend[] = [
  {
    id: "b1",
    name: DEFAULT_BACKEND_NAME,
    url: DEFAULT_BACKEND_URL,
    net: "BTC",
    health: "#893,014 - 2m",
    on: true,
    auth: "none",
  },
  {
    id: "b2",
    name: "Liquid Network",
    url: "https://liquid.network/api",
    net: "LIQUID",
    kind: "liquid-esplora",
    health: "Explorer API",
    on: true,
    auth: "none",
  },
  {
    id: "b3",
    name: "Coinbase Exchange",
    url: "https://api.exchange.coinbase.com",
    net: "FX",
    health: "BTC/EUR 1m live",
    on: true,
    auth: "none",
  },
  {
    id: "b4",
    name: "CoinGecko",
    url: "https://api.coingecko.com/api/v3",
    net: "FX",
    health: "Daily fallback",
    on: false,
    auth: "none",
  },
];

export const DEFAULT_RATE_BACKENDS: Backend[] = DEFAULT_BACKENDS.filter(
  (backend) => backend.net === "FX",
);

export function backendNetFromRow(row: BackendSettingsRow): Net {
  const chain = (row.chain ?? "").toLowerCase();
  const kind = (row.kind ?? "").toLowerCase();
  if (kind === "lnd" || kind === "coreln") return "LN";
  if (chain === "liquid" || kind === "liquid-esplora") return "LIQUID";
  return "BTC";
}

export function backendAuthLabel(row: BackendSettingsRow): string {
  if (row.has_auth_header) return "bearer";
  if (row.has_token) return "apikey";
  if (row.has_username || row.has_password) return "basic";
  return "none";
}

export function normalizeInfrastructureOwnership(
  value: string | null | undefined,
): InfrastructureOwnership | undefined {
  if (value === "self" || value === "third_party") return value;
  return undefined;
}

export function parseProxyEndpoint(
  value: string | null | undefined,
): { host: string; port: string } | undefined {
  const trimmed = value?.trim();
  if (!trimmed) return undefined;
  if (trimmed.includes("://")) {
    try {
      const parsed = new URL(trimmed);
      return parsed.hostname && parsed.port
        ? { host: parsed.hostname, port: parsed.port }
        : undefined;
    } catch {
      return undefined;
    }
  }
  const match = trimmed.match(/^(.*):(\d+)$/);
  if (!match) return undefined;
  return { host: match[1].replace(/^\[|\]$/g, ""), port: match[2] };
}

export function backendRowToSettingsBackend(row: BackendSettingsRow): Backend {
  const net = backendNetFromRow(row);
  const name = row.name || "backend";
  return {
    id: name,
    name,
    url: row.url || (row.has_url ? "Configured endpoint" : "Missing endpoint"),
    net,
    kind: row.kind,
    chain: row.chain,
    network: row.network,
    health: row.is_default ? "default" : row.source || row.kind || "configured",
    on: row.has_url !== false,
    isDefault: row.is_default === true,
    auth: backendAuthLabel(row),
    commandoPeerId: row.has_commando_peer_id
      ? CLN_PRESENCE_SENTINEL_COMMANDO_PEER
      : undefined,
    lightningDir: row.has_lightning_dir
      ? CLN_PRESENCE_SENTINEL_LIGHTNING_DIR
      : undefined,
    rpcFile: row.has_rpc_file ? CLN_PRESENCE_SENTINEL_RPC_FILE : undefined,
    trustSsl: row.insecure,
    proxy: parseProxyEndpoint(row.tor_proxy),
    infrastructureOwner: normalizeInfrastructureOwnership(
      row.infrastructure_owner,
    ),
  };
}

export function backendPayload(backend: Backend): Record<string, unknown> {
  const chain = backend.chain ?? (backend.net === "LIQUID" ? "liquid" : "bitcoin");
  const network =
    backend.network ?? (backend.net === "LIQUID" ? "liquidv1" : "main");
  const auth = backend.auth ?? "none";
  const payload: Record<string, unknown> = {
    name: backend.name,
    kind: backend.kind ?? (backend.net === "LIQUID" ? "electrum" : "esplora"),
    url: backend.url,
    chain,
    network,
  };
  const config: Record<string, unknown> = {};
  if (typeof backend.trustSsl === "boolean") {
    config.insecure = backend.trustSsl;
  }
  if (backend.infrastructureOwner) {
    config.infrastructure_owner = backend.infrastructureOwner;
  }
  if (backend.certificate) {
    config.certificate = backend.certificate;
  }
  if (backend.commandoPeerId) {
    config.commando_peer_id = backend.commandoPeerId;
  }
  if (backend.lightningCli) {
    config.lightning_cli = backend.lightningCli;
  }
  if (backend.lightningDir) {
    config.lightning_dir = backend.lightningDir;
  }
  if (backend.rpcFile) {
    config.rpc_file = backend.rpcFile;
  }
  const clear = new Set<string>();
  if (auth === "none") {
    clear.add("auth_header");
    clear.add("token");
    clear.add("username");
    clear.add("password");
  } else if (auth === "bearer") {
    clear.add("token");
    clear.add("username");
    clear.add("password");
  } else if (auth === "apikey") {
    clear.add("auth_header");
    clear.add("username");
    clear.add("password");
  } else if (auth === "basic") {
    clear.add("auth_header");
    clear.add("token");
  }
  if (auth === "bearer" && backend.authHeader) {
    payload.auth_header = backend.authHeader;
  }
  if (auth === "apikey" && backend.token) {
    payload.token = backend.token;
  }
  if (auth === "basic" && backend.username) {
    config.username = backend.username;
  }
  if (auth === "basic" && backend.password) {
    config.password = backend.password;
  }
  if (backend.proxy?.host && backend.proxy.port) {
    payload.tor_proxy = `${backend.proxy.host}:${backend.proxy.port}`;
  } else if (backend.proxy === null) {
    clear.add("tor_proxy");
  }
  if (Object.keys(config).length > 0) {
    payload.config = config;
  }
  if (clear.size > 0) {
    payload.clear = Array.from(clear);
  }
  return payload;
}

export const brandLogoFrame =
  "border-neutral-200 bg-white text-neutral-950 dark:border-neutral-700 dark:bg-white dark:text-neutral-950";
export const compactNumberFormatter = new Intl.NumberFormat(undefined, {
  maximumFractionDigits: 0,
});

export function formatCount(value: number): string {
  return compactNumberFormatter.format(value);
}

export function formatKrakenRange(row: KrakenRatesImportSummaryRow): string {
  if (row.first_timestamp && row.last_timestamp) {
    return `${row.first_timestamp} to ${row.last_timestamp}`;
  }
  return "No imported rows";
}

export function rateRebuildTransactionProgress(data: RateRebuildData | null) {
  if (!data) return null;
  const journalResult = rateRebuildJournalResult(data);
  const refreshedSource =
    data.reprice?.auto_priced ?? journalResult?.auto_priced;
  const refreshed =
    Number(refreshedSource ?? 0) ||
    Number(data.deleted.transaction_prices ?? 0);
  const total = Math.max(refreshed, Number(data.deleted.transaction_prices ?? 0));
  return { refreshed, total };
}

export function rateRebuildJournalResult(
  data: RateRebuildData | null,
): RateRebuildJournalResult | null {
  if (!data?.journals) return null;
  if ("ok" in data.journals) {
    return data.journals.ok ? data.journals.result ?? null : null;
  }
  return data.journals;
}

export function rateRebuildJournalError(data: RateRebuildData | null): string | null {
  if (!data?.journals || !("ok" in data.journals) || data.journals.ok) {
    return null;
  }
  return data.journals.error?.message ?? "Journal processing is still blocked.";
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

export type NetworkLayer = "bitcoin" | "lightning" | "liquid";

export const NETWORK_LAYER_FOR_NET: Record<Net, NetworkLayer | "market"> = {
  BTC: "bitcoin",
  LN: "lightning",
  LIQUID: "liquid",
  FX: "market",
};

export function backendsForLayer(backends: Backend[], layer: NetworkLayer): Backend[] {
  return backends.filter(
    (backend) => NETWORK_LAYER_FOR_NET[backend.net] === layer,
  );
}

// Resolve the privacy posture for a configured backend. The pure logic lives
// in `@/lib/backendTrust` so it can be unit-tested without this component.
export function backendTrust(backend: Backend) {
  return backendTrustFromEndpoint(
    backend.url || "",
    Boolean(backend.proxy),
    backend.infrastructureOwner,
  );
}

export function backendProtocolLabel(backend: Backend): string {
  switch ((backend.kind ?? "").toLowerCase()) {
    case "esplora":
      return "Explorer API";
    case "electrum":
      return "Electrum / Fulcrum";
    case "bitcoinrpc":
      return "Bitcoin Core RPC";
    case "liquid-esplora":
      return "Explorer API";
    case "lnd":
      return "LND REST";
    case "coreln":
      return "Core Lightning";
    default:
      return backend.net === "FX" ? "Rate provider" : "Endpoint";
  }
}

export function backendExplorerBaseUrl(backend: Backend): string | null {
  const kind = (backend.kind ?? "").toLowerCase();
  if (kind !== "esplora" && kind !== "liquid-esplora") return null;
  try {
    const parsed = new URL(backend.url);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      return null;
    }
    return normalizeExplorerBaseUrl(parsed.toString());
  } catch {
    return null;
  }
}

// Transaction-explorer links are derived from the configured Explorer-API
// backends rather than stored separately, so this stays the single source of
// truth: recompute it from the full backend list after any add/edit/delete.
// An empty base falls back to the public default (see `@/lib/explorer`).
export function deriveExplorerSettings(backends: Backend[]): ExplorerSettings {
  const baseForNet = (net: Net) =>
    backends
      .filter((backend) => backend.net === net)
      .map(backendExplorerBaseUrl)
      .find((value): value is string => Boolean(value)) ?? "";
  return {
    bitcoinBaseUrl: baseForNet("BTC"),
    liquidBaseUrl: baseForNet("LIQUID"),
  };
}

export function explorerHostLabel(baseUrl: string): string {
  try {
    return new URL(baseUrl).host || baseUrl;
  } catch {
    return baseUrl;
  }
}

export function endpointHostLabel(endpoint: string): string {
  if (!endpoint) return "No endpoint";
  try {
    return new URL(endpoint).host || endpoint;
  } catch {
    return endpoint;
  }
}
