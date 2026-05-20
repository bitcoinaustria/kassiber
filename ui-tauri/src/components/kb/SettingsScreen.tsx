/**
 * SettingsScreen - workspace-wide preferences.
 *
 * Most controls are local UI state until the daemon-backed settings surface
 * lands. Hide-sensitive data is wired to the shared UI store.
 */
import * as React from "react";
import {
  AlertTriangle,
  Archive,
  Bitcoin,
  Bot,
  CheckCircle2,
  ChevronDown,
  Database,
  Download,
  Droplets,
  ExternalLink,
  Eye,
  FileInput,
  Fingerprint,
  HardDrive,
  KeyRound,
  LineChart,
  Lock,
  Minus,
  Monitor,
  Moon,
  Network,
  Palette,
  Pencil,
  Plus,
  RefreshCw,
  Server,
  ShieldCheck,
  ShieldOff,
  Sun,
  Terminal,
  Trash2,
  Upload,
  Wrench,
  XCircle,
  Zap,
  type LucideIcon,
} from "lucide-react";
import { useNavigate, useRouterState } from "@tanstack/react-router";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import bitcoinIcon from "@/assets/integrations/bitcoin.svg";
import coreLightningIcon from "@/assets/integrations/core-lightning.svg";
import lightningLabsIcon from "@/assets/integrations/lightning-labs.png";
import liquidIcon from "@/assets/integrations/liquid.svg";
import mempoolIcon from "@/assets/integrations/mempool-space.svg";
import {
  AiProviderForm,
  type ExistingAiProvider,
} from "@/components/kb/AiProviderForm";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import {
  canUseTouchIdPassphraseUnlock,
  clearImportProject,
  forgetTouchIdPassphrase,
  getTransport,
  installTerminalCommand,
  removeTerminalCommand,
  storeTouchIdPassphrase,
  touchIdPassphraseStatus,
  terminalCommandStatus,
  type TerminalCommandStatus,
  type TouchIdPassphraseStatus,
  openExternalUrl,
} from "@/daemon/transport";
import { normalizeExplorerBaseUrl, type ExplorerSettings } from "@/lib/explorer";
import { isFilePickerAvailable, pickFile } from "@/lib/filePicker";
import { setSessionUnlockPassphrase } from "@/store/sessionLock";
import {
  useUiStore,
  MAX_APP_SCALE,
  MIN_APP_SCALE,
  type AppLockPolicy,
  type DeferredConnectionSetup,
  type ThemePreference,
} from "@/store/ui";
import type { AiModelsListData, AiModelRow } from "@/lib/aiCapabilities";
import {
  CLN_PRESENCE_SENTINEL_COMMANDO_PEER,
  CLN_PRESENCE_SENTINEL_LIGHTNING_DIR,
  CLN_PRESENCE_SENTINEL_RPC_FILE,
  coreLightningBackendModeValid,
} from "@/lib/lightning";
import { screenPanelClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import {
  APP_LOG_MAX_BYTES,
  APP_LOG_MAX_RECORDS,
  getAppLogBufferSize,
  subscribeAppLogRecords,
} from "@/lib/appLogs";
import {
  DEFAULT_BACKEND_NAME,
  DEFAULT_BACKEND_URL,
  databasePassphraseHint,
} from "@/components/kb/Onboarding/constants";

const PLAINTEXT_DELETE_ACK = "DELETE LOCAL DATA";
const KRAKEN_OHLCVT_SUPPORT_URL =
  "https://support.kraken.com/hc/articles/360047124832-Downloadable-historical-OHLCVT-Open-High-Low-Close-Volume-Trades-data";
const KRAKEN_MARKET_DATA_BLOG_URL =
  "https://blog.kraken.com/product/api/unlocked-3-the-market-data-feeds-systematic-traders-use";

type Net = "BTC" | "LIQUID" | "LN" | "FX";
type InfrastructureOwnership = "self" | "third_party";

interface Backend {
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

interface BackendSettingsRow {
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
  infrastructure_owner?: string;
}

interface BackendSettingsData {
  backends: BackendSettingsRow[];
  summary: {
    count: number;
    default_backend: string | null;
  };
}

interface StatusData {
  data_root: string;
  database: string;
  current_workspace: string | null;
  current_profile: string | null;
  workspaces: number;
  profiles: number;
}

interface ResetBookData {
  reset: boolean;
  removed: Record<string, number>;
  preserved: Record<string, number>;
  rates_scope: "global" | "preserved";
  shared_rates_cleared: boolean;
}

type KrakenRatesImportOperation = "full" | "incremental";

interface KrakenRatesImportSummaryRow {
  pair: string;
  samples: number;
  files: number;
  skipped_rows: number;
  skipped_files: number;
  first_timestamp: string | null;
  last_timestamp: string | null;
}

interface KrakenRatesImportData {
  source: "kraken-csv";
  operation: KrakenRatesImportOperation;
  path: string;
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

interface RateRebuildData {
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

interface RateRebuildJournalResult {
  entries_created?: number;
  quarantined?: number;
  auto_priced?: number;
}

interface DaemonErrorPayload {
  code?: string;
  message?: string;
  hint?: string | null;
  retryable?: boolean;
}

type AiSecretStoreId = "macos_keychain" | "windows_dpapi" | "linux_secret_service" | "sqlcipher_inline";
type AiSecretState = "ok" | "missing" | "needs_reauth" | "unavailable";

interface AiSecretStorePolicy {
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

interface AiProviderRow {
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

interface AiProvidersListData {
  providers: AiProviderRow[];
  default: string | null;
  secret_store_policy?: AiSecretStorePolicy;
}

const AI_KIND_BADGE: Record<AiProviderRow["kind"], string> = {
  local:
    "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  remote:
    "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  tee: "border-sky-500/25 bg-sky-500/10 text-sky-700 dark:text-sky-300",
};

import {
  settingsSectionForHash,
  type SettingsSectionId,
} from "./settingsSections";

function isCliAiProvider(row: AiProviderRow): boolean {
  return (
    row.base_url === "claude-cli://default" ||
    row.base_url === "codex-cli://default"
  );
}

function aiSecretStoreLabel(storeId: AiSecretStoreId | undefined): string {
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

function aiSecretStateLabel(state: AiSecretState | undefined): string {
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

function formatModelSummary(models: AiModelRow[]): string {
  const ids = models
    .map((model) => model.id)
    .filter((id, index, values) => id && values.indexOf(id) === index);
  if (ids.length <= 3) return ids.join(", ");
  return `${ids.slice(0, 3).join(", ")} +${ids.length - 3}`;
}

function AiProviderModelSummary({ row }: { row: AiProviderRow }) {
  const isCli = isCliAiProvider(row);
  const modelsQuery = useDaemon<AiModelsListData>(
    "ai.list_models",
    { provider: row.name },
    {
      enabled: isCli,
      refetchOnMount: "always",
      staleTime: 5 * 60 * 1000,
    },
  );

  if (!isCli) return <>{row.default_model ?? "-"}</>;

  const models =
    modelsQuery.data?.kind === "ai.list_models" && modelsQuery.data.data
      ? modelsQuery.data.data.models
      : [];
  const summary = formatModelSummary(models);
  if (summary) return <>{summary}</>;
  if (modelsQuery.isFetching) {
    return <span className="text-muted-foreground">Loading...</span>;
  }
  return <>{row.default_model ?? "-"}</>;
}

const DEFAULT_BACKENDS: Backend[] = [
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

const DEFAULT_RATE_BACKENDS: Backend[] = DEFAULT_BACKENDS.filter(
  (backend) => backend.net === "FX",
);

function backendNetFromRow(row: BackendSettingsRow): Net {
  const chain = (row.chain ?? "").toLowerCase();
  const kind = (row.kind ?? "").toLowerCase();
  if (kind === "lnd" || kind === "coreln") return "LN";
  if (chain === "liquid" || kind === "liquid-esplora") return "LIQUID";
  return "BTC";
}

function backendAuthLabel(row: BackendSettingsRow): string {
  if (row.has_auth_header) return "bearer";
  if (row.has_token) return "apikey";
  if (row.has_username || row.has_password) return "basic";
  return "none";
}

function normalizeInfrastructureOwnership(
  value: string | null | undefined,
): InfrastructureOwnership | undefined {
  if (value === "self" || value === "third_party") return value;
  return undefined;
}

function inferredInfrastructureOwnership(url: string): InfrastructureOwnership {
  const trust = backendTrustFromEndpoint(url);
  return trust.posture === "on-device" ? "self" : "third_party";
}

function backendRowToSettingsBackend(row: BackendSettingsRow): Backend {
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
    infrastructureOwner: normalizeInfrastructureOwnership(
      row.infrastructure_owner,
    ),
  };
}

function backendPayload(backend: Backend): Record<string, unknown> {
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
  }
  if (Object.keys(config).length > 0) {
    payload.config = config;
  }
  if (clear.size > 0) {
    payload.clear = Array.from(clear);
  }
  return payload;
}

const brandLogoFrame =
  "border-neutral-200 bg-white text-neutral-950 dark:border-neutral-700 dark:bg-white dark:text-neutral-950";
const compactNumberFormatter = new Intl.NumberFormat(undefined, {
  maximumFractionDigits: 0,
});

function formatCount(value: number): string {
  return compactNumberFormatter.format(value);
}

function formatKrakenRange(row: KrakenRatesImportSummaryRow): string {
  if (row.first_timestamp && row.last_timestamp) {
    return `${row.first_timestamp} to ${row.last_timestamp}`;
  }
  return "No imported rows";
}

function rateRebuildTransactionProgress(data: RateRebuildData | null) {
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

function rateRebuildJournalResult(
  data: RateRebuildData | null,
): RateRebuildJournalResult | null {
  if (!data?.journals) return null;
  if ("ok" in data.journals) {
    return data.journals.ok ? data.journals.result ?? null : null;
  }
  return data.journals;
}

function rateRebuildJournalError(data: RateRebuildData | null): string | null {
  if (!data?.journals || !("ok" in data.journals) || data.journals.ok) {
    return null;
  }
  return data.journals.error?.message ?? "Journal processing is still blocked.";
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

type NetworkLayer = "bitcoin" | "lightning" | "liquid";

const NETWORK_LAYER_FOR_NET: Record<Net, NetworkLayer | "market"> = {
  BTC: "bitcoin",
  LN: "lightning",
  LIQUID: "liquid",
  FX: "market",
};

function backendsForLayer(backends: Backend[], layer: NetworkLayer): Backend[] {
  return backends.filter(
    (backend) => NETWORK_LAYER_FOR_NET[backend.net] === layer,
  );
}

type TrustPosture = "on-device" | "shielded" | "remote";

interface TrustInfo {
  posture: TrustPosture;
  label: string;
  note: string;
  icon: LucideIcon;
  className: string;
}

function backendTrustFromEndpoint(
  url: string,
  hasProxy = false,
  ownership?: InfrastructureOwnership,
): TrustInfo {
  const normalizedUrl = url.toLowerCase();
  if (ownership === "self") {
    return {
      posture: "on-device",
      label: "Your infrastructure",
      note: "Marked as infrastructure you operate. Address queries still go to this endpoint, so keep its hosting and logs in your trust boundary.",
      icon: ShieldCheck,
      className:
        "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
    };
  }
  const isOnion = normalizedUrl.includes(".onion");
  const isLocal =
    /(?:\/\/|@)(?:127\.0\.0\.1|0\.0\.0\.0|localhost|\[::1\])(?::\d+)?(?:\/|$)/.test(
      normalizedUrl,
    ) ||
    normalizedUrl.includes("://localhost") ||
    normalizedUrl.includes(".local:") ||
    normalizedUrl.endsWith(".local");
  if (isLocal) {
    return {
      posture: "on-device",
      label: "On device",
      note: "Runs on this machine — address queries never leave your device.",
      icon: ShieldCheck,
      className:
        "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
    };
  }
  if (isOnion || hasProxy) {
    return {
      posture: "shielded",
      label:
        ownership === "third_party"
          ? isOnion
            ? "Third-party via Tor"
            : "Third-party via proxy"
          : isOnion
            ? "Tor"
            : "Via proxy",
      note: isOnion
        ? "Reached over Tor — this server cannot tie your queries to your IP address."
        : "Routed through a proxy — your IP address stays hidden from this server.",
      icon: Network,
      className: "border-sky-500/25 bg-sky-500/10 text-sky-700 dark:text-sky-300",
    };
  }
  return {
    posture: "remote",
    label: "Third-party server",
    note: "This provider can observe the addresses you look up. Use your own infrastructure or a proxy if that is not acceptable.",
    icon: ShieldOff,
    className:
      "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  };
}

// Heuristic trust read used to communicate the privacy posture of a backend.
// It is intentionally conservative: anything that is not clearly on-device or
// Tor/proxy-shielded is treated as a third party that can observe queried
// addresses. No hardcoded service allowlists — the URL shape tells the story.
function backendTrust(backend: Backend): TrustInfo {
  return backendTrustFromEndpoint(
    backend.url || "",
    Boolean(backend.proxy?.host),
    backend.infrastructureOwner,
  );
}

function backendProtocolLabel(backend: Backend): string {
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

function backendExplorerBaseUrl(backend: Backend): string | null {
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

function explorerSettingsPatchForBackend(
  backend: Backend,
): Partial<ExplorerSettings> | null {
  const baseUrl = backendExplorerBaseUrl(backend);
  if (!baseUrl) return null;
  if (backend.net === "BTC") return { bitcoinBaseUrl: baseUrl };
  if (backend.net === "LIQUID") return { liquidBaseUrl: baseUrl };
  return null;
}

function explorerHostLabel(baseUrl: string): string {
  try {
    return new URL(baseUrl).host || baseUrl;
  } catch {
    return baseUrl;
  }
}

function endpointHostLabel(endpoint: string): string {
  if (!endpoint) return "No endpoint";
  try {
    return new URL(endpoint).host || endpoint;
  } catch {
    return endpoint;
  }
}

type SettingsGroup =
  | "General"
  | "On-chain & off-chain data"
  | "Privacy & security"
  | "Assistant"
  | "Data"
  | "Desktop";

interface SettingsSectionMeta {
  id: SettingsSectionId;
  slug: string;
  group: SettingsGroup;
  label: string;
  description: string;
  icon: LucideIcon;
}

const SETTINGS_SECTIONS: SettingsSectionMeta[] = [
  {
    id: "general-appearance",
    slug: "appearance",
    group: "General",
    label: "Appearance",
    description: "Theme, denomination, and interface scale.",
    icon: Palette,
  },
  {
    id: "network-market",
    slug: "market",
    group: "General",
    label: "Market data",
    description: "Fiat reference-rate sources and the local pricing cache.",
    icon: LineChart,
  },
  {
    id: "network-bitcoin",
    slug: "bitcoin",
    group: "On-chain & off-chain data",
    label: "Bitcoin",
    description:
      "Base-layer indexers and nodes used to refresh on-chain wallets.",
    icon: Bitcoin,
  },
  {
    id: "network-lightning",
    slug: "lightning",
    group: "On-chain & off-chain data",
    label: "Lightning",
    description:
      "Read-only Lightning node connections for accounting and profitability.",
    icon: Zap,
  },
  {
    id: "network-liquid",
    slug: "liquid",
    group: "On-chain & off-chain data",
    label: "Liquid",
    description: "Sidechain indexers used to refresh Liquid (L-BTC) wallets.",
    icon: Droplets,
  },
  {
    id: "security-privacy",
    slug: "privacy",
    group: "Privacy & security",
    label: "Privacy",
    description: "Control what is shown on screen and what leaves your machine.",
    icon: Eye,
  },
  {
    id: "security-lock",
    slug: "security",
    group: "Privacy & security",
    label: "Lock & encryption",
    description: "App lock, biometric unlock, and the database passphrase.",
    icon: Lock,
  },
  {
    id: "assistant-ai",
    slug: "ai",
    group: "Assistant",
    label: "AI providers",
    description: "Local and remote assistant endpoints and their data posture.",
    icon: Bot,
  },
  {
    id: "data-storage",
    slug: "data",
    group: "Data",
    label: "Data & storage",
    description: "Backups, label imports, the local database, and reset tools.",
    icon: HardDrive,
  },
  {
    id: "desktop-terminal",
    slug: "terminal",
    group: "Desktop",
    label: "Terminal integration",
    description: "Install the kassiber CLI launcher for your shell.",
    icon: Terminal,
  },
  {
    id: "desktop-developer",
    slug: "developer",
    group: "Desktop",
    label: "Developer tools",
    description: "The in-app Logs view and its in-memory buffer.",
    icon: Wrench,
  },
];

const SETTINGS_GROUP_ORDER: SettingsGroup[] = [
  "General",
  "On-chain & off-chain data",
  "Privacy & security",
  "Assistant",
  "Data",
  "Desktop",
];

const DEFAULT_SETTINGS_SECTION: SettingsSectionId = "general-appearance";

function sectionMeta(id: SettingsSectionId): SettingsSectionMeta {
  return (
    SETTINGS_SECTIONS.find((section) => section.id === id) ??
    SETTINGS_SECTIONS[0]
  );
}

function SettingsRail({
  activeId,
  onSelect,
  counts,
}: {
  activeId: SettingsSectionId;
  onSelect: (id: SettingsSectionId) => void;
  counts: Partial<Record<SettingsSectionId, number>>;
}) {
  return (
    <nav
      aria-label="Settings sections"
      className="lg:sticky lg:top-4 lg:w-[236px] lg:shrink-0 lg:self-start"
    >
      <div className="flex flex-col gap-5">
        {SETTINGS_GROUP_ORDER.map((group) => {
          const items = SETTINGS_SECTIONS.filter(
            (section) => section.group === group,
          );
          if (items.length === 0) return null;
          return (
            <div key={group} className="space-y-1.5">
              <p className="kb-mono-caption px-2.5">{group}</p>
              <div className="flex flex-wrap gap-1 lg:flex-col">
                {items.map((section) => {
                  const Icon = section.icon;
                  const active = section.id === activeId;
                  const count = counts[section.id];
                  return (
                    <button
                      key={section.id}
                      type="button"
                      aria-current={active ? "page" : undefined}
                      onClick={() => onSelect(section.id)}
                      className={cn(
                        "flex items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-sm transition-colors",
                        active
                          ? "bg-muted font-medium text-foreground"
                          : "text-muted-foreground hover:bg-muted/60 hover:text-foreground",
                      )}
                    >
                      <Icon className="size-4 shrink-0" aria-hidden="true" />
                      <span className="min-w-0 flex-1 truncate">
                        {section.label}
                      </span>
                      {typeof count === "number" && count > 0 ? (
                        <span className="text-xs tabular-nums text-muted-foreground">
                          {count}
                        </span>
                      ) : null}
                    </button>
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>
    </nav>
  );
}

interface SettingsScreenProps {
  onLock?: () => void;
}

export function SettingsScreen({ onLock }: SettingsScreenProps) {
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const setHideSensitive = useUiStore((s) => s.setHideSensitive);
  const currency = useUiStore((s) => s.currency);
  const setCurrency = useUiStore((s) => s.setCurrency);
  const theme = useUiStore((s) => s.theme);
  const setTheme = useUiStore((s) => s.setTheme);
  const appScale = useUiStore((s) => s.appScale);
  const increaseAppScale = useUiStore((s) => s.increaseAppScale);
  const decreaseAppScale = useUiStore((s) => s.decreaseAppScale);
  const resetAppScale = useUiStore((s) => s.resetAppScale);
  const setExplorerSettings = useUiStore((s) => s.setExplorerSettings);
  const appLockPolicy = useUiStore((s) => s.appLockPolicy);
  const setAppLockPolicy = useUiStore((s) => s.setAppLockPolicy);
  const aiFeaturesEnabled = useUiStore((s) => s.aiFeaturesEnabled);
  const setAiFeaturesEnabled = useUiStore((s) => s.setAiFeaturesEnabled);
  const developerToolsEnabled = useUiStore((s) => s.developerToolsEnabled);
  const setDeveloperToolsEnabled = useUiStore(
    (s) => s.setDeveloperToolsEnabled,
  );
  const identity = useUiStore((s) => s.identity);
  const setIdentity = useUiStore((s) => s.setIdentity);
  const addNotification = useUiStore((s) => s.addNotification);
  const deferredConnectionSetup = useUiStore(
    (s) => s.deferredConnectionSetup,
  );
  const clearDeferredConnectionSetup = useUiStore(
    (s) => s.clearDeferredConnectionSetup,
  );
  const navigate = useNavigate();
  const settingsHash = useRouterState({ select: (s) => s.location.hash });
  const routeSectionId = React.useMemo(
    () => settingsSectionForHash(settingsHash),
    [settingsHash],
  );
  const statusQuery = useDaemon<StatusData>("status", undefined, {
    enabled: true,
  });
  const status =
    statusQuery.data?.kind === "status" ? statusQuery.data.data : null;
  const statusLoaded = statusQuery.data?.kind === "status";
  const touchIdDataRoot = identity?.importedProject?.dataRoot ?? null;
  const touchIdPlatformSupported = canUseTouchIdPassphraseUnlock();
  const encryptedWorkspace =
    Boolean(identity?.encrypted) || identity?.databaseMode === "sqlcipher";
  const [touchIdStatus, setTouchIdStatus] =
    React.useState<TouchIdPassphraseStatus | null>(null);
  const [touchIdStatusPending, setTouchIdStatusPending] =
    React.useState(false);
  const touchIdConfigured = touchIdStatus?.configured === true;
  const touchIdStatusReason = touchIdStatus?.reason ?? null;
  const deleteWorkspace = useDaemonMutation("ui.workspace.delete", {
    dataMode: "real",
  });
  const resetBookData = useDaemonMutation<ResetBookData>("ui.profiles.reset_data", {
    dataMode: "real",
  });
  const changePassphrase = useDaemonMutation("ui.secrets.change_passphrase", {
    dataMode: "real",
  });
  const backendSettingsQuery = useDaemon<BackendSettingsData>(
    "ui.backends.settings.list",
    undefined,
    { refetchOnMount: "always" },
  );
  const createBackend = useDaemonMutation<BackendSettingsRow>("ui.backends.create");
  const updateBackend = useDaemonMutation<BackendSettingsRow>("ui.backends.update");
  const createWallet = useDaemonMutation("ui.wallets.create");
  const deleteBackend = useDaemonMutation<{ name: string; deleted: boolean }>(
    "ui.backends.delete",
  );
  const [clearClipboard, setClearClipboard] = React.useState(true);
  const [backendDialogOpen, setBackendDialogOpen] = React.useState(false);
  const [editingBackendId, setEditingBackendId] = React.useState<string | null>(
    null,
  );
  const [initialBackendTypeId, setInitialBackendTypeId] =
    React.useState<SyncBackendNetwork["id"] | null>(null);
  const deferredBackendDialogKeyRef = React.useRef<string | null>(null);
  const [deleteOpen, setDeleteOpen] = React.useState(false);
  const [deletePassphrase, setDeletePassphrase] = React.useState("");
  const [deleteConfirm, setDeleteConfirm] = React.useState("");
  const [deletePlaintextAck, setDeletePlaintextAck] = React.useState("");
  const [deleteError, setDeleteError] = React.useState<string | null>(null);
  const [resetDataOpen, setResetDataOpen] = React.useState(false);
  const [resetDataPassphrase, setResetDataPassphrase] = React.useState("");
  const [resetDataConfirm, setResetDataConfirm] = React.useState("");
  const [resetDataClearSharedRates, setResetDataClearSharedRates] =
    React.useState(false);
  const [resetDataPlaintextAck, setResetDataPlaintextAck] =
    React.useState("");
  const [resetDataError, setResetDataError] = React.useState<string | null>(
    null,
  );
  const [passphraseOpen, setPassphraseOpen] = React.useState(false);
  const [currentPassphrase, setCurrentPassphrase] = React.useState("");
  const [newPassphrase, setNewPassphrase] = React.useState("");
  const [newPassphraseConfirm, setNewPassphraseConfirm] = React.useState("");
  const [passphraseError, setPassphraseError] = React.useState<string | null>(
    null,
  );
  const [touchIdEnrollOpen, setTouchIdEnrollOpen] = React.useState(false);
  const [touchIdEnrollPassphrase, setTouchIdEnrollPassphrase] =
    React.useState("");
  const [touchIdEnrollError, setTouchIdEnrollError] = React.useState<
    string | null
  >(null);
  const [touchIdEnrollPending, setTouchIdEnrollPending] =
    React.useState(false);
  const [terminalStatus, setTerminalStatus] =
    React.useState<TerminalCommandStatus | null>(null);
  const [terminalStatusError, setTerminalStatusError] = React.useState<
    string | null
  >(null);
  const [terminalCommandPending, setTerminalCommandPending] =
    React.useState(false);
  const [activeSectionId, setActiveSectionId] = React.useState<SettingsSectionId>(
    () => routeSectionId ?? DEFAULT_SETTINGS_SECTION,
  );

  const openTouchIdEnrollment = React.useCallback(() => {
    setTouchIdEnrollError(null);
    setTouchIdEnrollPassphrase("");
    setTouchIdEnrollOpen(true);
  }, []);

  const refreshTouchIdStatus = React.useCallback(async () => {
    if (!encryptedWorkspace || !touchIdPlatformSupported) {
      setTouchIdStatus(null);
      return null;
    }
    setTouchIdStatusPending(true);
    try {
      const status = await touchIdPassphraseStatus(touchIdDataRoot);
      setTouchIdStatus(status);
      return status;
    } catch (error) {
      const status: TouchIdPassphraseStatus = {
        platform: "macos",
        available: false,
        configured: false,
        reason: error instanceof Error ? error.message : String(error),
      };
      setTouchIdStatus(status);
      return status;
    } finally {
      setTouchIdStatusPending(false);
    }
  }, [encryptedWorkspace, touchIdDataRoot, touchIdPlatformSupported]);

  const forgetTouchIdUnlock = React.useCallback(
    async () => {
      try {
        const status = await forgetTouchIdPassphrase(touchIdDataRoot);
        setTouchIdStatus(status);
        setAppLockPolicy({ touchIdUnlock: false });
      } catch (error: unknown) {
        addNotification({
          title: "Touch ID passphrase was not removed",
          body:
            error instanceof Error
              ? error.message
              : "macOS Keychain did not remove the saved passphrase.",
          tone: "warning",
        });
      }
    },
    [addNotification, setAppLockPolicy, touchIdDataRoot],
  );

  React.useEffect(() => {
    void refreshTouchIdStatus();
  }, [refreshTouchIdStatus]);

  React.useEffect(() => {
    if (!appLockPolicy.touchIdUnlock) return;
    if (touchIdStatus?.configured !== false) return;
    setAppLockPolicy({ touchIdUnlock: false });
  }, [appLockPolicy.touchIdUnlock, setAppLockPolicy, touchIdStatus?.configured]);

  React.useEffect(() => {
    setActiveSectionId(routeSectionId ?? DEFAULT_SETTINGS_SECTION);
  }, [routeSectionId]);

  const refreshTerminalCommandStatus = React.useCallback(async () => {
    try {
      const next = await terminalCommandStatus();
      setTerminalStatus(next);
      setTerminalStatusError(null);
      return next;
    } catch (error) {
      const message =
        error instanceof Error
          ? error.message
          : "Could not inspect terminal command.";
      setTerminalStatusError(message);
      return null;
    }
  }, []);

  React.useEffect(() => {
    void refreshTerminalCommandStatus();
  }, [refreshTerminalCommandStatus]);

  // Native menu may re-fire for the same section while the URL hash is
  // unchanged (user already on /settings#privacy, clicks Privacy again after
  // closing the panel). The hash effect won't see a diff, so listen for an
  // explicit `kassiber:settings-section` event and force re-selection.
  React.useEffect(() => {
    const handler = (event: Event) => {
      const detail = (event as CustomEvent<{ section?: string | null }>).detail;
      const next = settingsSectionForHash(detail?.section ?? "");
      if (next) setActiveSectionId(next);
    };
    window.addEventListener("kassiber:settings-section", handler);
    return () => {
      window.removeEventListener("kassiber:settings-section", handler);
    };
  }, []);

  const backends = React.useMemo<Backend[]>(() => {
    const syncRows = backendSettingsQuery.data?.data?.backends ?? [];
    return [
      ...syncRows.map(backendRowToSettingsBackend),
      ...DEFAULT_RATE_BACKENDS,
    ];
  }, [backendSettingsQuery.data]);

  const editingBackend = React.useMemo(
    () => backends.find((backend) => backend.id === editingBackendId) ?? null,
    [backends, editingBackendId],
  );

  const onResetWorkspace = () => {
    const ok = window.confirm(
      "Reset Welcome state?\n\nThis clears your local identity and returns you to the Welcome screen. Encrypted data on disk is not touched.",
    );
    if (!ok) return;
    void (async () => {
      if (identity?.importedProject) {
        await clearImportProject();
      }
      setIdentity(null);
      void navigate({ to: "/", replace: true });
    })().catch(() => {
      setIdentity(null);
      void navigate({ to: "/", replace: true });
    });
  };

  const lockNow = () => {
    window.requestAnimationFrame(() => {
      if (onLock) {
        onLock();
        return;
      }
      window.dispatchEvent(new CustomEvent("kassiber:lock-app"));
    });
  };

  const workspaceLabel =
    status?.current_workspace || identity?.workspace || "current books set";
  const currentBookLabel =
    statusLoaded
      ? status?.current_profile ?? null
      : identity?.profile || identity?.name || null;
  const bookLabel = currentBookLabel || "current book";
  const resetBookAvailable = Boolean(currentBookLabel);

  const openResetBookData = () => {
    setResetDataPassphrase("");
    setResetDataConfirm("");
    setResetDataClearSharedRates(false);
    setResetDataPlaintextAck("");
    setResetDataError(null);
    setResetDataOpen(true);
  };

  const openDeleteWorkspace = () => {
    setDeletePassphrase("");
    setDeleteConfirm("");
    setDeletePlaintextAck("");
    setDeleteError(null);
    setDeleteOpen(true);
  };

  const openChangePassphrase = () => {
    setCurrentPassphrase("");
    setNewPassphrase("");
    setNewPassphraseConfirm("");
    setPassphraseError(null);
    setPassphraseOpen(true);
  };

  const openAddBackend = React.useCallback((typeId?: SyncBackendNetwork["id"]) => {
    setEditingBackendId(null);
    setInitialBackendTypeId(typeId ?? null);
    setBackendDialogOpen(true);
  }, []);

  const openEditBackend = (backend: Backend) => {
    setEditingBackendId(backend.id);
    setInitialBackendTypeId(null);
    setBackendDialogOpen(true);
  };

  React.useEffect(() => {
    const typeId = backendTypeIdForConnectionSetup(deferredConnectionSetup);
    if (!typeId) return;
    const key = `${deferredConnectionSetup?.sourceId ?? ""}:${
      deferredConnectionSetup?.backendKind ?? ""
    }`;
    if (deferredBackendDialogKeyRef.current === key) return;
    deferredBackendDialogKeyRef.current = key;
    setActiveSectionId(
      typeId === "liquid"
        ? "network-liquid"
        : typeId === "coreln" || typeId === "lnd"
          ? "network-lightning"
          : "network-bitcoin",
    );
    openAddBackend(typeId);
  }, [deferredConnectionSetup, openAddBackend]);

  const onSaveBackend = async (backend: Backend) => {
    const payload = backendPayload(backend);
    if (editingBackend) {
      await updateBackend.mutateAsync({ ...payload, name: editingBackend.id });
    } else {
      await createBackend.mutateAsync(payload);
      if (backend.kind === "coreln") {
        await createWallet.mutateAsync({
          label: backend.name,
          kind: "coreln",
          backend: backend.name,
          chain: "bitcoin",
          network: "main",
        });
      }
    }
    const explorerPatch = explorerSettingsPatchForBackend(backend);
    if (explorerPatch) {
      setExplorerSettings(explorerPatch);
    }
    await backendSettingsQuery.refetch();
    setBackendDialogOpen(false);
    setEditingBackendId(null);
  };

  const onDeleteBackend = async (backend: Backend) => {
    const ok = window.confirm(
      `Delete backend '${backend.name}'?\n\nWallets using this endpoint may need another backend before they can sync.`,
    );
    if (!ok) return;
    await deleteBackend.mutateAsync({ name: backend.id });
    await backendSettingsQuery.refetch();
  };

  const onInstallTerminalCommand = async () => {
    const wasRepair = Boolean(terminalStatus?.needsRepair);
    setTerminalCommandPending(true);
    try {
      const next = await installTerminalCommand();
      setTerminalStatus(next);
      setTerminalStatusError(null);
      addNotification({
        title: wasRepair
          ? "Terminal command repaired"
          : "Terminal command installed",
        body: next.pathOnPath
          ? "Open a new terminal and run kassiber status."
          : `Add ${next.binDir} to PATH, then open a new terminal.`,
        tone: next.pathOnPath ? "success" : "warning",
      });
    } catch (error) {
      setTerminalStatusError(
        error instanceof Error
          ? error.message
          : "Could not install terminal command.",
      );
    } finally {
      setTerminalCommandPending(false);
    }
  };

  const onRemoveTerminalCommand = async () => {
    setTerminalCommandPending(true);
    try {
      const next = await removeTerminalCommand();
      setTerminalStatus(next);
      setTerminalStatusError(null);
      addNotification({
        title: "Terminal command removed",
        body: `${next.commandPath || "The command"} was removed.`,
        tone: "success",
      });
    } catch (error) {
      setTerminalStatusError(
        error instanceof Error
          ? error.message
          : "Could not remove terminal command.",
      );
    } finally {
      setTerminalCommandPending(false);
    }
  };

  const goToSection = React.useCallback(
    (id: SettingsSectionId) => {
      setActiveSectionId(id);
      void navigate({
        to: "/settings",
        hash: sectionMeta(id).slug,
        replace: true,
      });
    },
    [navigate],
  );

  const sectionCounts = React.useMemo<
    Partial<Record<SettingsSectionId, number>>
  >(
    () => ({
      "network-bitcoin": backendsForLayer(backends, "bitcoin").length,
      "network-lightning": backendsForLayer(backends, "lightning").length,
      "network-liquid": backendsForLayer(backends, "liquid").length,
      "network-market": backends.filter((backend) => backend.net === "FX")
        .length,
    }),
    [backends],
  );

  const onDeleteWorkspace = async () => {
    setDeleteError(null);
    if (encryptedWorkspace && !deletePassphrase) {
      setDeleteError("Enter the database passphrase.");
      return;
    }
    if (
      !encryptedWorkspace &&
      deletePlaintextAck.trim() !== PLAINTEXT_DELETE_ACK
    ) {
      setDeleteError(`Type ${PLAINTEXT_DELETE_ACK} to confirm local deletion.`);
      return;
    }
    if (deleteConfirm.trim() !== workspaceLabel) {
      setDeleteError(`Type ${workspaceLabel} to confirm deletion.`);
      return;
    }
    try {
      await deleteWorkspace.mutateAsync({
        confirm: "DELETE",
        confirm_workspace: workspaceLabel,
        auth_response: encryptedWorkspace
          ? { passphrase_secret: deletePassphrase }
          : { plaintext_delete_ack: PLAINTEXT_DELETE_ACK },
      });
      if (identity?.importedProject) {
        await clearImportProject().catch(() => {});
      }
      setIdentity(null);
      void navigate({ to: "/", replace: true });
    } catch (error) {
      window.alert(
        error instanceof Error
          ? error.message
          : "Books delete failed.",
      );
    }
  };

  const onResetBookData = async () => {
    setResetDataError(null);
    if (encryptedWorkspace && !resetDataPassphrase) {
      setResetDataError("Enter the database passphrase.");
      return;
    }
    if (
      !encryptedWorkspace &&
      resetDataPlaintextAck.trim() !== PLAINTEXT_DELETE_ACK
    ) {
      setResetDataError(`Type ${PLAINTEXT_DELETE_ACK} to confirm local reset.`);
      return;
    }
    if (resetDataConfirm.trim() !== bookLabel) {
      setResetDataError(`Type ${bookLabel} to confirm reset.`);
      return;
    }
    try {
      const envelope = await resetBookData.mutateAsync({
        confirm: "RESET",
        confirm_profile: bookLabel,
        clear_shared_rates: resetDataClearSharedRates,
        auth_response: encryptedWorkspace
          ? { passphrase_secret: resetDataPassphrase }
          : { plaintext_delete_ack: PLAINTEXT_DELETE_ACK },
      });
      const removed = envelope.data?.removed ?? {};
      const optionalSummary: string[] = [];
      const attachmentsRemoved =
        Number(removed.attachments ?? 0) +
        Number(removed.attachment_files ?? 0);
      const sourceFundsRemoved =
        Number(removed.source_funds_sources ?? 0) +
        Number(removed.source_funds_links ?? 0) +
        Number(removed.source_funds_cases ?? 0) +
        Number(removed.source_funds_snapshots ?? 0);
      const summary = [
        `${formatCount(removed.transactions ?? 0)} transactions`,
        `${formatCount(removed.journal_entries ?? 0)} journal rows`,
        `${formatCount(removed.transaction_pairs ?? 0)} swap pairs`,
        `${formatCount(removed.bip329_labels ?? 0)} BIP329 labels`,
      ];
      if (attachmentsRemoved > 0) {
        optionalSummary.push(`${formatCount(attachmentsRemoved)} attachment records/files`);
      }
      if (sourceFundsRemoved > 0) {
        optionalSummary.push(`${formatCount(sourceFundsRemoved)} source-funds records`);
      }
      if (envelope.data?.shared_rates_cleared) {
        optionalSummary.push(`${formatCount(removed.rates_cache ?? 0)} rate rows`);
      }
      summary.push(...optionalSummary);
      addNotification({
        title: "Book data reset",
        body: `Cleared ${summary.join(", ")}. Wallet and backend connections were kept.`,
        tone: "success",
      });
      setResetDataOpen(false);
      setResetDataPassphrase("");
      setResetDataConfirm("");
      setResetDataClearSharedRates(false);
      setResetDataPlaintextAck("");
    } catch (error) {
      setResetDataError(
        error instanceof Error ? error.message : "Book reset failed.",
      );
    }
  };

  const onChangePassphrase = async () => {
    setPassphraseError(null);
    if (!currentPassphrase) {
      setPassphraseError("Enter the current database passphrase.");
      return;
    }
    const hint = databasePassphraseHint(newPassphrase, newPassphraseConfirm);
    if (hint) {
      setPassphraseError(hint);
      return;
    }

    try {
      await changePassphrase.mutateAsync({
        auth_response: { passphrase_secret: currentPassphrase },
        new_passphrase_secret: newPassphrase,
      });
      await setSessionUnlockPassphrase(newPassphrase);
      if (appLockPolicy.touchIdUnlock && touchIdPlatformSupported) {
        try {
          const status = await storeTouchIdPassphrase(
            newPassphrase,
            touchIdDataRoot,
          );
          setTouchIdStatus(status);
          if (!status.configured) {
            throw new Error(
              status.reason
                ? `Touch ID unlock is not set up: ${status.reason}`
                : "macOS Keychain did not report the saved Touch ID passphrase.",
            );
          }
        } catch (error) {
          setAppLockPolicy({ touchIdUnlock: false });
          await forgetTouchIdPassphrase(touchIdDataRoot).catch(() => {});
          await refreshTouchIdStatus();
          addNotification({
            title: "Touch ID unlock was disabled",
            body:
              error instanceof Error
                ? error.message
                : "The database passphrase changed, but macOS Keychain did not accept the updated Touch ID passphrase.",
            tone: "warning",
          });
        }
      }
      setPassphraseOpen(false);
      setCurrentPassphrase("");
      setNewPassphrase("");
      setNewPassphraseConfirm("");
    } catch (error) {
      setPassphraseError(
        error instanceof Error
          ? error.message
          : "Could not change database passphrase.",
      );
    }
  };

  const onEnrollTouchId = async () => {
    setTouchIdEnrollError(null);
    if (!touchIdEnrollPassphrase) {
      setTouchIdEnrollError("Enter the database passphrase.");
      return;
    }

    setTouchIdEnrollPending(true);
    try {
      const envelope = await getTransport("real").invoke({
        kind: "daemon.unlock",
        args: {
          ...(identity?.importedProject
            ? { require_existing_project: true }
            : {}),
          auth_response: { passphrase_secret: touchIdEnrollPassphrase },
        },
      });
      if (envelope.kind !== "daemon.unlock") {
        throw new Error("Database passphrase did not unlock these books.");
      }
      const status = await storeTouchIdPassphrase(
        touchIdEnrollPassphrase,
        touchIdDataRoot,
      );
      setTouchIdStatus(status);
      if (!status.configured) {
        throw new Error(
          status.reason
            ? `Touch ID unlock is not set up: ${status.reason}`
            : "macOS Keychain did not report the saved Touch ID passphrase.",
        );
      }
      await setSessionUnlockPassphrase(touchIdEnrollPassphrase);
      setAppLockPolicy({ touchIdUnlock: true });
      setTouchIdEnrollOpen(false);
      setTouchIdEnrollPassphrase("");
      addNotification({
        title: "Touch ID unlock enabled",
        body: "The database passphrase was saved in macOS Keychain behind local user presence.",
        tone: "success",
      });
    } catch (error) {
      setAppLockPolicy({ touchIdUnlock: false });
      await forgetTouchIdPassphrase(touchIdDataRoot).catch(() => {});
      await refreshTouchIdStatus();
      setTouchIdEnrollError(
        error instanceof Error
          ? error.message
          : "Could not save the database passphrase for Touch ID unlock.",
      );
    } finally {
      setTouchIdEnrollPending(false);
    }
  };

  const activeMeta = sectionMeta(activeSectionId);
  const sectionContent = (() => {
    switch (activeSectionId) {
      case "general-appearance":
        return (
          <AppearanceSettingsPanel
            theme={theme}
            setTheme={setTheme}
            appScale={appScale}
            increaseAppScale={increaseAppScale}
            decreaseAppScale={decreaseAppScale}
            resetAppScale={resetAppScale}
            currency={currency}
            setCurrency={setCurrency}
          />
        );
      case "network-bitcoin":
        return (
          <NetworkLayerPanel
            layer="bitcoin"
            backends={backends}
            onAdd={() => openAddBackend("bitcoin")}
            onEdit={openEditBackend}
            onDelete={onDeleteBackend}
          />
        );
      case "network-lightning":
        return (
          <NetworkLayerPanel
            layer="lightning"
            backends={backends}
            onAdd={() => openAddBackend("lnd")}
            onEdit={openEditBackend}
            onDelete={onDeleteBackend}
          />
        );
      case "network-liquid":
        return (
          <NetworkLayerPanel
            layer="liquid"
            backends={backends}
            onAdd={() => openAddBackend("liquid")}
            onEdit={openEditBackend}
            onDelete={onDeleteBackend}
          />
        );
      case "network-market":
        return <MarketDataPanel backends={backends} />;
      case "security-privacy":
        return (
          <PrivacySettingsPanel
            hideSensitive={hideSensitive}
            setHideSensitive={setHideSensitive}
            clearClipboard={clearClipboard}
            setClearClipboard={setClearClipboard}
            backends={backends}
            aiFeaturesEnabled={aiFeaturesEnabled}
            onManageConnections={() => goToSection("network-bitcoin")}
            onManageAi={() => goToSection("assistant-ai")}
          />
        );
      case "security-lock":
        return (
          <SecuritySettingsPanel
            appLockPolicy={appLockPolicy}
            setAppLockPolicy={setAppLockPolicy}
            onEnrollTouchId={openTouchIdEnrollment}
            onForgetTouchId={forgetTouchIdUnlock}
            encryptedWorkspace={encryptedWorkspace}
            touchIdPlatformSupported={touchIdPlatformSupported}
            touchIdConfigured={touchIdConfigured}
            touchIdStatusPending={touchIdStatusPending}
            touchIdStatusReason={touchIdStatusReason}
            onRefreshTouchId={() => void refreshTouchIdStatus()}
            onLockNow={lockNow}
            onChangePassphrase={openChangePassphrase}
          />
        );
      case "assistant-ai":
        return (
          <AiProvidersPanel
            aiFeaturesEnabled={aiFeaturesEnabled}
            setAiFeaturesEnabled={setAiFeaturesEnabled}
          />
        );
      case "data-storage":
        return (
          <DataAndStoragePanel
            status={status ?? null}
            onResetWelcome={onResetWorkspace}
            onResetBook={openResetBookData}
            resetBookDisabled={resetBookData.isPending || !resetBookAvailable}
            onDeleteBooks={openDeleteWorkspace}
            deleteBooksDisabled={deleteWorkspace.isPending}
          />
        );
      case "desktop-terminal":
        return (
          <TerminalCommandSettingsPanel
            status={terminalStatus}
            error={terminalStatusError}
            pending={terminalCommandPending}
            onRefresh={() => void refreshTerminalCommandStatus()}
            onInstall={() => void onInstallTerminalCommand()}
            onRemove={() => void onRemoveTerminalCommand()}
          />
        );
      case "desktop-developer":
        return (
          <DeveloperToolsSettingsPanel
            enabled={developerToolsEnabled}
            setEnabled={setDeveloperToolsEnabled}
          />
        );
      default:
        return null;
    }
  })();

  return (
    <>
      <div className={screenPanelClassName}>
        <div className="mx-auto flex w-full max-w-[1500px] min-w-0 flex-col gap-5">
          <div className="space-y-1">
            <h1 className="text-2xl font-semibold tracking-tight">Settings</h1>
            <p className="text-sm text-muted-foreground">
              Configure how Kassiber reaches the Bitcoin network, what stays on
              this machine, and how your books are stored.
            </p>
          </div>

          {deferredConnectionSetup ? (
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-md border border-primary/30 bg-primary/5 p-3 text-sm">
              <span>
                You came here from connection setup
                {deferredConnectionSetup.reason
                  ? ` (${deferredConnectionSetup.reason})`
                  : ""}
                . Configure the backend below, then resume.
              </span>
              <div className="flex gap-2">
                <Button
                  type="button"
                  size="sm"
                  onClick={() => {
                    void navigate({ to: "/connections" });
                  }}
                >
                  Resume connection setup
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  onClick={clearDeferredConnectionSetup}
                >
                  Dismiss
                </Button>
              </div>
            </div>
          ) : null}

          <div className="flex min-w-0 flex-col gap-6 lg:flex-row lg:gap-8">
            <SettingsRail
              activeId={activeSectionId}
              onSelect={goToSection}
              counts={sectionCounts}
            />
            <div className="min-w-0 flex-1">
              <div className="mb-5 space-y-1 border-b pb-4">
                <p className="kb-mono-caption">{activeMeta.group}</p>
                <h2 className="text-lg font-semibold tracking-tight">
                  {activeMeta.label}
                </h2>
                <p className="max-w-2xl text-sm text-muted-foreground">
                  {activeMeta.description}
                </p>
              </div>
              {sectionContent}
            </div>
          </div>
        </div>
      </div>

        <BackendModal
          open={backendDialogOpen}
          initial={editingBackend}
          initialTypeId={initialBackendTypeId ?? undefined}
          onClose={() => {
            setBackendDialogOpen(false);
            setEditingBackendId(null);
            setInitialBackendTypeId(null);
          }}
          onSave={onSaveBackend}
        />
        <Dialog
          open={resetDataOpen}
          onOpenChange={(next) => {
            if (!next) {
              setResetDataOpen(false);
            }
          }}
        >
          <DialogContent className="max-w-md">
            <DialogHeader>
              <DialogTitle>Reset book data</DialogTitle>
              <DialogDescription>
                This keeps wallet and backend connections for {bookLabel}, then
                clears imported/synced rows, journals, swap review state, labels,
                attachments, and source-funds work. The shared fiat-rate cache
                is kept unless you explicitly include it below.
                {encryptedWorkspace
                  ? " Enter the database passphrase and the book name to continue."
                  : " These plaintext books have no database passphrase; type the explicit local-reset challenge and book name to continue."}
              </DialogDescription>
            </DialogHeader>
            <form
              className="space-y-4"
              onSubmit={(event) => {
                event.preventDefault();
                void onResetBookData();
              }}
            >
              {encryptedWorkspace ? (
                <div className="space-y-2">
                  <Label htmlFor="reset-data-passphrase">Passphrase</Label>
                  <Input
                    id="reset-data-passphrase"
                    type="password"
                    autoComplete="current-password"
                    value={resetDataPassphrase}
                    onChange={(event) =>
                      setResetDataPassphrase(event.target.value)
                    }
                  />
                </div>
              ) : (
                <div className="space-y-2">
                  <Label htmlFor="reset-data-plaintext-ack">
                    Plaintext reset challenge
                  </Label>
                  <Input
                    id="reset-data-plaintext-ack"
                    value={resetDataPlaintextAck}
                    placeholder={PLAINTEXT_DELETE_ACK}
                    onChange={(event) =>
                      setResetDataPlaintextAck(event.target.value)
                    }
                  />
                </div>
              )}
              <div className="space-y-2">
                <Label htmlFor="reset-data-confirm">Book name</Label>
                <Input
                  id="reset-data-confirm"
                  value={resetDataConfirm}
                  placeholder={bookLabel}
                  onChange={(event) => setResetDataConfirm(event.target.value)}
                />
              </div>
              <div className="flex items-start gap-3 rounded-md border border-amber-500/30 bg-amber-500/5 p-3">
                <Checkbox
                  id="reset-data-clear-shared-rates"
                  checked={resetDataClearSharedRates}
                  onCheckedChange={(checked) =>
                    setResetDataClearSharedRates(checked === true)
                  }
                />
                <Label
                  htmlFor="reset-data-clear-shared-rates"
                  className="grid gap-1 text-sm leading-relaxed"
                >
                  <span>Also clear shared fiat-rate cache</span>
                  <span className="font-normal text-muted-foreground">
                    This cache is shared across every book in this local data
                    root. Leave it off to keep existing BTC-EUR/BTC-USD history.
                  </span>
                </Label>
              </div>
              {resetDataError && (
                <p className="m-0 text-sm text-destructive">
                  {resetDataError}
                </p>
              )}
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setResetDataOpen(false)}
                >
                  Cancel
                </Button>
                <Button
                  type="submit"
                  variant="destructive"
                  disabled={resetBookData.isPending}
                >
                  {resetBookData.isPending ? "Resetting..." : "Reset"}
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
        <Dialog
          open={deleteOpen}
          onOpenChange={(next) => {
            if (!next) {
              setDeleteOpen(false);
            }
          }}
        >
          <DialogContent className="max-w-md">
            <DialogHeader>
              <DialogTitle>Delete books set</DialogTitle>
              <DialogDescription>
                This removes {workspaceLabel} from the local Kassiber database.
                {encryptedWorkspace
                  ? " Enter the database passphrase and the books set name to continue."
                  : " These plaintext books have no database passphrase; type the explicit local-delete challenge and books set name to continue."}
              </DialogDescription>
            </DialogHeader>
            <form
              className="space-y-4"
              onSubmit={(event) => {
                event.preventDefault();
                void onDeleteWorkspace();
              }}
            >
              {encryptedWorkspace ? (
                <div className="space-y-2">
                  <Label htmlFor="delete-passphrase">Passphrase</Label>
                  <Input
                    id="delete-passphrase"
                    type="password"
                    autoComplete="current-password"
                    value={deletePassphrase}
                    onChange={(event) =>
                      setDeletePassphrase(event.target.value)
                    }
                  />
                </div>
              ) : (
                <div className="space-y-2">
                  <Label htmlFor="delete-plaintext-ack">
                    Plaintext delete challenge
                  </Label>
                  <Input
                    id="delete-plaintext-ack"
                    value={deletePlaintextAck}
                    placeholder={PLAINTEXT_DELETE_ACK}
                    onChange={(event) =>
                      setDeletePlaintextAck(event.target.value)
                    }
                  />
                </div>
              )}
              <div className="space-y-2">
                <Label htmlFor="delete-confirm">Books set name</Label>
                <Input
                  id="delete-confirm"
                  value={deleteConfirm}
                  placeholder={workspaceLabel}
                  onChange={(event) => setDeleteConfirm(event.target.value)}
                />
              </div>
              {deleteError && (
                <p className="m-0 text-sm text-destructive">{deleteError}</p>
              )}
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setDeleteOpen(false)}
                >
                  Cancel
                </Button>
                <Button
                  type="submit"
                  variant="destructive"
                  disabled={deleteWorkspace.isPending}
                >
                  {deleteWorkspace.isPending ? "Deleting..." : "Delete"}
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
        <Dialog
          open={passphraseOpen}
          onOpenChange={(next) => {
            if (!next) {
              setPassphraseOpen(false);
            }
          }}
        >
          <DialogContent className="max-w-md">
            <DialogHeader>
              <DialogTitle>Change database passphrase</DialogTitle>
              <DialogDescription>
                Rekey the local SQLCipher database. The current daemon session
                is reopened with the new passphrase after rotation.
              </DialogDescription>
            </DialogHeader>
            <form
              className="space-y-4"
              onSubmit={(event) => {
                event.preventDefault();
                void onChangePassphrase();
              }}
            >
              <div className="space-y-2">
                <Label htmlFor="current-passphrase">Current passphrase</Label>
                <Input
                  id="current-passphrase"
                  type="password"
                  autoComplete="current-password"
                  value={currentPassphrase}
                  onChange={(event) =>
                    setCurrentPassphrase(event.target.value)
                  }
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="new-passphrase">New passphrase</Label>
                <Input
                  id="new-passphrase"
                  type="password"
                  autoComplete="new-password"
                  value={newPassphrase}
                  onChange={(event) => setNewPassphrase(event.target.value)}
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="new-passphrase-confirm">
                  Confirm new passphrase
                </Label>
                <Input
                  id="new-passphrase-confirm"
                  type="password"
                  autoComplete="new-password"
                  value={newPassphraseConfirm}
                  onChange={(event) =>
                    setNewPassphraseConfirm(event.target.value)
                  }
                />
              </div>
              {passphraseError && (
                <p className="m-0 text-sm text-destructive">
                  {passphraseError}
                </p>
              )}
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setPassphraseOpen(false)}
                >
                  Cancel
                </Button>
                <Button type="submit" disabled={changePassphrase.isPending}>
                  {changePassphrase.isPending ? "Changing..." : "Change"}
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
        <Dialog
          open={touchIdEnrollOpen}
          onOpenChange={(next) => {
            if (!next && !touchIdEnrollPending) {
              setTouchIdEnrollOpen(false);
              setTouchIdEnrollPassphrase("");
              setTouchIdEnrollError(null);
            }
          }}
        >
          <DialogContent className="max-w-md">
            <DialogHeader>
              <DialogTitle>Enable Touch ID unlock</DialogTitle>
              <DialogDescription>
                Enter the database passphrase once. Kassiber will verify it
                locally, then save it in macOS Keychain behind local user
                presence for these books.
              </DialogDescription>
            </DialogHeader>
            <form
              className="space-y-4"
              onSubmit={(event) => {
                event.preventDefault();
                void onEnrollTouchId();
              }}
            >
              <div className="space-y-2">
                <Label htmlFor="touch-id-enroll-passphrase">
                  Database passphrase
                </Label>
                <Input
                  id="touch-id-enroll-passphrase"
                  type="password"
                  autoComplete="current-password"
                  value={touchIdEnrollPassphrase}
                  disabled={touchIdEnrollPending}
                  onChange={(event) =>
                    setTouchIdEnrollPassphrase(event.target.value)
                  }
                />
              </div>
              {touchIdEnrollError && (
                <p className="m-0 text-sm text-destructive">
                  {touchIdEnrollError}
                </p>
              )}
              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  disabled={touchIdEnrollPending}
                  onClick={() => {
                    setTouchIdEnrollOpen(false);
                    setTouchIdEnrollPassphrase("");
                    setTouchIdEnrollError(null);
                  }}
                >
                  Cancel
                </Button>
                <Button type="submit" disabled={touchIdEnrollPending}>
                  {touchIdEnrollPending ? "Saving..." : "Enable Touch ID"}
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
    </>
  );
}

interface SettingsSwitchRowProps {
  label: string;
  description: string;
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  disabled?: boolean;
}

function SettingsSwitchRow({
  label,
  description,
  checked,
  onCheckedChange,
  disabled = false,
}: SettingsSwitchRowProps) {
  return (
    <div
      className={cn(
        "flex items-start justify-between gap-4 rounded-md border bg-background p-3",
        disabled && "opacity-60",
      )}
    >
      <div className="min-w-0 space-y-1">
        <Label className="text-sm font-medium">{label}</Label>
        <p className="text-sm text-muted-foreground">{description}</p>
      </div>
      <Switch
        checked={checked}
        onCheckedChange={onCheckedChange}
        disabled={disabled}
      />
    </div>
  );
}

type CurrencyMode = "btc" | "eur";

function AppearanceSettingsPanel({
  theme,
  setTheme,
  appScale,
  increaseAppScale,
  decreaseAppScale,
  resetAppScale,
  currency,
  setCurrency,
}: {
  theme: ThemePreference;
  setTheme: (theme: ThemePreference) => void;
  appScale: number;
  increaseAppScale: () => void;
  decreaseAppScale: () => void;
  resetAppScale: () => void;
  currency: CurrencyMode;
  setCurrency: (currency: CurrencyMode) => void;
}) {
  const scalePercent = Math.round(appScale * 100);
  return (
    <div className="space-y-6">
      <section className="space-y-2">
        <div>
          <h3 className="text-sm font-semibold">Theme</h3>
          <p className="text-sm text-muted-foreground">
            Follow the system setting or pin a light or dark appearance.
          </p>
        </div>
        <Tabs
          value={theme}
          onValueChange={(value) => setTheme(value as ThemePreference)}
        >
          <TabsList>
            <TabsTrigger value="system">
              <Monitor className="size-4" aria-hidden="true" />
              System
            </TabsTrigger>
            <TabsTrigger value="light">
              <Sun className="size-4" aria-hidden="true" />
              Light
            </TabsTrigger>
            <TabsTrigger value="dark">
              <Moon className="size-4" aria-hidden="true" />
              Dark
            </TabsTrigger>
          </TabsList>
        </Tabs>
      </section>

      <section className="space-y-2">
        <div>
          <h3 className="text-sm font-semibold">Denomination</h3>
          <p className="text-sm text-muted-foreground">
            Choose how balances and reports are shown across the app.
          </p>
        </div>
        <Tabs
          value={currency}
          onValueChange={(value) => setCurrency(value as CurrencyMode)}
        >
          <TabsList>
            <TabsTrigger value="eur">
              <span aria-hidden="true">€</span>
              Euro
            </TabsTrigger>
            <TabsTrigger value="btc">
              <span aria-hidden="true">₿</span>
              Bitcoin
            </TabsTrigger>
          </TabsList>
        </Tabs>
      </section>

      <section className="space-y-2">
        <div>
          <h3 className="text-sm font-semibold">Interface scale</h3>
          <p className="text-sm text-muted-foreground">
            Make every screen denser or larger. Applies across the whole app.
          </p>
        </div>
        <div className="flex max-w-md items-center gap-2 rounded-md border bg-background p-2">
          <Button
            type="button"
            variant="outline"
            size="icon-sm"
            aria-label="Decrease interface scale"
            disabled={appScale <= MIN_APP_SCALE}
            onClick={decreaseAppScale}
          >
            <Minus className="size-4" aria-hidden="true" />
          </Button>
          <div className="flex-1 text-center font-mono text-sm tabular-nums">
            {scalePercent}%
          </div>
          <Button
            type="button"
            variant="outline"
            size="icon-sm"
            aria-label="Increase interface scale"
            disabled={appScale >= MAX_APP_SCALE}
            onClick={increaseAppScale}
          >
            <Plus className="size-4" aria-hidden="true" />
          </Button>
          <Button type="button" variant="ghost" size="sm" onClick={resetAppScale}>
            Reset
          </Button>
        </div>
      </section>
    </div>
  );
}

function DeveloperToolsSettingsPanel({
  enabled,
  setEnabled,
}: {
  enabled: boolean;
  setEnabled: (enabled: boolean) => void;
}) {
  const bytes = useAppLogBufferSize();
  return (
    <section className="space-y-3">
      <p className="max-w-2xl text-sm text-muted-foreground">
        Show the typed Logs view after the local books are unlocked. Logs are
        local-only, kept in RAM, and written to disk only when you export them.
      </p>
      <SettingsSwitchRow
        label="Enable Logs page"
        description={
          enabled
            ? "Logs is visible in Support and route navigation."
            : "Logs is hidden and direct navigation redirects to Overview."
        }
        checked={enabled}
        onCheckedChange={setEnabled}
      />
      <div className="rounded-md border bg-background p-3 text-sm">
        <p className="font-medium">In-memory log buffer</p>
        <p className="text-muted-foreground">
          {formatBytes(bytes)} retained in this GUI session. Kassiber keeps at most{" "}
          {APP_LOG_MAX_RECORDS.toLocaleString()} records or{" "}
          {formatBytes(APP_LOG_MAX_BYTES)}, whichever is reached first. Refreshing
          or closing the app clears the buffer unless you export it first.
        </p>
      </div>
    </section>
  );
}

function useAppLogBufferSize(): number {
  return React.useSyncExternalStore(
    subscribeAppLogRecords,
    getAppLogBufferSize,
    getAppLogBufferSize,
  );
}

function TerminalCommandSettingsPanel({
  status,
  error,
  pending,
  onRefresh,
  onInstall,
  onRemove,
}: {
  status: TerminalCommandStatus | null;
  error: string | null;
  pending: boolean;
  onRefresh: () => void;
  onInstall: () => void;
  onRemove: () => void;
}) {
  const actionLabel = status?.needsRepair
    ? "Repair command"
    : status?.installed
      ? "Reinstall command"
      : "Install command";
  return (
    <section className="space-y-4">
      <p className="max-w-2xl text-sm text-muted-foreground">
        Installs a user-local launcher for the bundled desktop CLI so you can run{" "}
        <span className="font-mono">kassiber</span> from your shell. No
        administrator privileges are required.
      </p>

      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          onClick={onInstall}
          disabled={pending || status?.conflict || status?.available === false}
        >
          {pending ? (
            <RefreshCw className="size-4 animate-spin" aria-hidden="true" />
          ) : (
            <Terminal className="size-4" aria-hidden="true" />
          )}
          {actionLabel}
        </Button>
        <Button
          type="button"
          variant="outline"
          onClick={onRefresh}
          disabled={pending}
        >
          <RefreshCw className="size-4" aria-hidden="true" />
          Refresh
        </Button>
        {status?.managed ? (
          <Button
            type="button"
            variant="ghost"
            onClick={onRemove}
            disabled={pending}
          >
            Remove
          </Button>
        ) : null}
      </div>

      {error ? (
        <div className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
          <XCircle className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <span>{error}</span>
        </div>
      ) : null}

      {status ? (
        <div className="space-y-3">
          <div
            className={cn(
              "flex items-start gap-2 rounded-md border p-3 text-sm",
              status.installed && status.pathOnPath
                ? "border-emerald-500/30 bg-emerald-500/5 text-emerald-700 dark:text-emerald-300"
                : "border-amber-500/30 bg-amber-500/10 text-amber-800 dark:text-amber-200",
            )}
          >
            {status.installed && status.pathOnPath ? (
              <CheckCircle2 className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            ) : (
              <AlertTriangle className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            )}
            <span>{status.message}</span>
          </div>

          <div className="grid gap-3 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label htmlFor="settings-terminal-command">Command</Label>
              <Input
                id="settings-terminal-command"
                readOnly
                value={status.commandPath || "loading..."}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="settings-terminal-target">Desktop executable</Label>
              <Input
                id="settings-terminal-target"
                readOnly
                value={status.targetPath || "loading..."}
              />
            </div>
          </div>

          <div className="rounded-md border bg-background p-3 text-sm">
            <p className="font-mono">kassiber status</p>
          </div>

          {!status.pathOnPath ? (
            <div className="space-y-1.5">
              <Label htmlFor="settings-terminal-path">PATH update</Label>
              <Input
                id="settings-terminal-path"
                readOnly
                value={status.pathHint}
              />
            </div>
          ) : null}
        </div>
      ) : (
        <div className="rounded-md border bg-muted/30 p-3 text-sm text-muted-foreground">
          Inspecting desktop command status...
        </div>
      )}
    </section>
  );
}

function SecuritySettingsPanel({
  appLockPolicy,
  setAppLockPolicy,
  onEnrollTouchId,
  onForgetTouchId,
  encryptedWorkspace,
  touchIdPlatformSupported,
  touchIdConfigured,
  touchIdStatusPending,
  touchIdStatusReason,
  onRefreshTouchId,
  onLockNow,
  onChangePassphrase,
}: {
  appLockPolicy: AppLockPolicy;
  setAppLockPolicy: (policy: Partial<AppLockPolicy>) => void;
  onEnrollTouchId: () => void;
  onForgetTouchId: () => void;
  encryptedWorkspace: boolean;
  touchIdPlatformSupported: boolean;
  touchIdConfigured: boolean;
  touchIdStatusPending: boolean;
  touchIdStatusReason: string | null;
  onRefreshTouchId: () => void;
  onLockNow: () => void;
  onChangePassphrase: () => void;
}) {
  return (
    <section className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(280px,360px)]">
      <div className="space-y-3">
        <div className="flex items-center gap-2">
          <Lock className="size-4" aria-hidden="true" />
          <h3 className="text-sm font-semibold">App lock</h3>
        </div>
        <SettingsSwitchRow
          label="Auto-lock when idle"
          description="Require passphrase after inactivity."
          checked={appLockPolicy.autoLockWhenIdle}
          onCheckedChange={(checked) =>
            setAppLockPolicy({ autoLockWhenIdle: checked })
          }
        />
        <div
          className={cn(
            "space-y-2 rounded-md border bg-background p-3",
            !appLockPolicy.autoLockWhenIdle && "pointer-events-none opacity-50",
          )}
        >
          <Label>Idle timeout</Label>
          <div className="flex flex-wrap gap-2">
            {[1, 5, 15, 30, 60].map((minutes) => (
              <Button
                key={minutes}
                type="button"
                variant={
                  appLockPolicy.idleMinutes === minutes ? "default" : "outline"
                }
                size="sm"
                onClick={() => setAppLockPolicy({ idleMinutes: minutes })}
              >
                {minutes}m
              </Button>
            ))}
          </div>
        </div>
        <SettingsSwitchRow
          label="Require passphrase on launch"
          description={
            encryptedWorkspace
              ? "Prompt immediately when Kassiber opens; cold starts still need the database passphrase when the daemon is locked."
              : "Prompt every time Kassiber opens."
          }
          checked={appLockPolicy.requirePassphraseOnLaunch}
          onCheckedChange={(checked) =>
            setAppLockPolicy({ requirePassphraseOnLaunch: checked })
          }
        />
        <SettingsSwitchRow
          label="Lock on window close"
          description="Clear in-memory decrypted state when the app window closes."
          checked={appLockPolicy.lockOnWindowClose}
          onCheckedChange={(checked) =>
            setAppLockPolicy({ lockOnWindowClose: checked })
          }
        />
        <div className="space-y-3 rounded-md border bg-background p-3">
          <div className="flex items-center gap-2">
            <Fingerprint className="size-4" aria-hidden="true" />
            <h3 className="text-sm font-semibold">Biometric unlock</h3>
          </div>
          <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
            <div className="min-w-0 space-y-1">
              <p className="text-sm font-medium">
                {touchIdStatusPending
                  ? "Checking Touch ID enrollment"
                  : touchIdConfigured
                    ? "Touch ID unlock enrolled"
                    : "Touch ID unlock not enrolled"}
              </p>
              <p className="text-sm text-muted-foreground">
                {encryptedWorkspace
                  ? touchIdPlatformSupported
                    ? touchIdConfigured
                      ? "Saved for these books in this macOS user account."
                      : touchIdStatusReason
                        ? `Not set up: ${touchIdStatusReason}`
                        : "Verify the database passphrase once and save it in macOS Keychain."
                    : "Touch ID unlock is available in the macOS desktop app."
                  : "Available after these books use SQLCipher encryption."}
              </p>
            </div>
            <div className="flex flex-wrap gap-2">
              <Button
                type="button"
                size="sm"
                variant="ghost"
                disabled={!encryptedWorkspace || !touchIdPlatformSupported}
                onClick={onRefreshTouchId}
              >
                <RefreshCw className="size-4" aria-hidden="true" />
                Refresh
              </Button>
              {touchIdConfigured ? (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={!encryptedWorkspace || !touchIdPlatformSupported}
                  onClick={onForgetTouchId}
                >
                  Forget
                </Button>
              ) : (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={!encryptedWorkspace || !touchIdPlatformSupported}
                  onClick={onEnrollTouchId}
                >
                  Set up
                </Button>
              )}
            </div>
          </div>
        </div>
        <div className="flex flex-col gap-3 rounded-md border bg-background p-3 sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0 space-y-1">
            <p className="text-sm font-medium">
              Session unlock preference
            </p>
            <p className="text-sm text-muted-foreground">
              Remember whether this device should offer biometric unlock on the lock screen.
            </p>
          </div>
          <Switch
            checked={appLockPolicy.touchIdUnlock && touchIdConfigured}
            disabled={!encryptedWorkspace || !touchIdConfigured}
            onCheckedChange={(checked) =>
              setAppLockPolicy({ touchIdUnlock: checked })
            }
          />
        </div>
      </div>
      <div className="space-y-4 rounded-md border border-primary/15 bg-background p-4">
        <div className="space-y-1">
          <h3 className="flex items-center gap-2 text-sm font-semibold">
            {appLockPolicy.touchIdUnlock ? (
              <Fingerprint className="size-4" aria-hidden="true" />
            ) : (
              <KeyRound className="size-4" aria-hidden="true" />
            )}
            Security boundary
          </h3>
          <p className="m-0 text-sm leading-6 text-muted-foreground">
            Lock closes the daemon database handle for encrypted books.
            Unlocking reopens the local SQLCipher database with the passphrase.
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button type="button" size="sm" variant="outline" onClick={onLockNow}>
            <Lock className="size-4" aria-hidden="true" />
            Lock now
          </Button>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={onChangePassphrase}
            disabled={!encryptedWorkspace}
          >
            <KeyRound className="size-4" aria-hidden="true" />
            Change passphrase
          </Button>
        </div>
      </div>
    </section>
  );
}

function MarketDataPanel({ backends }: { backends: Backend[] }) {
  const rateBackends = backends.filter((backend) => backend.net === "FX");
  const importKrakenRates = useDaemonMutation<KrakenRatesImportData>(
    "ui.rates.kraken_csv.import",
  );
  const rebuildRates = useDaemonMutation<RateRebuildData>("ui.rates.rebuild");
  const addNotification = useUiStore((state) => state.addNotification);
  const updateNotification = useUiStore((state) => state.updateNotification);
  const rebuildNoticeRef = React.useRef<string | null>(null);
  const [krakenArchivePath, setKrakenArchivePath] = React.useState("");
  const [krakenImportResult, setKrakenImportResult] =
    React.useState<KrakenRatesImportData | null>(null);
  const [krakenImportError, setKrakenImportError] = React.useState<string | null>(
    null,
  );
  const [pendingKrakenOperation, setPendingKrakenOperation] =
    React.useState<KrakenRatesImportOperation | null>(null);
  const [rateRebuildOpen, setRateRebuildOpen] = React.useState(false);
  const [rateRebuildResult, setRateRebuildResult] =
    React.useState<RateRebuildData | null>(null);
  const [rateRebuildError, setRateRebuildError] = React.useState<string | null>(
    null,
  );
  const openMarketDataUrl = React.useCallback(
    (event: React.MouseEvent<HTMLAnchorElement>, url: string) => {
      event.preventDefault();
      void openExternalUrl(url).catch((error) => {
        addNotification({
          title: "Could not open link",
          body:
            error instanceof Error
              ? error.message
              : "Could not open the link in the default browser.",
          tone: "warning",
        });
      });
    },
    [addNotification],
  );

  const chooseKrakenArchive = async () => {
    setKrakenImportError(null);
    const selected = await pickFile({
      title: "Choose Kraken OHLCVT CSV or ZIP",
      filters: [
        {
          name: "Kraken OHLCVT",
          extensions: ["zip", "csv"],
        },
      ],
    });
    if (selected) {
      setKrakenArchivePath(selected);
    }
  };

  const chooseKrakenDirectory = async () => {
    setKrakenImportError(null);
    const selected = await pickFile({
      title: "Choose extracted Kraken OHLCVT folder",
      directory: true,
    });
    if (selected) {
      setKrakenArchivePath(selected);
    }
  };

  const startKrakenImport = async (operation: KrakenRatesImportOperation) => {
    let archivePath = krakenArchivePath.trim();
    setKrakenImportError(null);
    setKrakenImportResult(null);

    if (!archivePath && isFilePickerAvailable) {
      const selected = await pickFile({
        title:
          operation === "full"
            ? "Choose extracted Kraken OHLCVT folder"
            : "Choose Kraken update OHLCVT CSV or ZIP",
        directory: operation === "full",
        filters:
          operation === "full"
            ? undefined
            : [
                {
                  name: "Kraken OHLCVT",
                  extensions: ["zip", "csv"],
                },
              ],
      });
      if (!selected) return;
      archivePath = selected;
      setKrakenArchivePath(selected);
    }

    if (!archivePath) {
      setKrakenImportError("Enter a local Kraken CSV or ZIP path.");
      return;
    }

    setPendingKrakenOperation(operation);
    try {
      const envelope = await importKrakenRates.mutateAsync({
        path: archivePath,
        operation,
      });
      setKrakenImportResult(envelope.data ?? null);
    } catch (error) {
      setKrakenImportError(
        error instanceof Error ? error.message : "Kraken import failed.",
      );
    } finally {
      setPendingKrakenOperation(null);
    }
  };

  const isImportingKraken = importKrakenRates.isPending;
  const isRebuildingRates = rebuildRates.isPending;
  const rateRebuildProgress = rateRebuildTransactionProgress(rateRebuildResult);
  const rateRebuildSamples =
    rateRebuildResult?.sync.reduce(
      (total, row) => total + Number(row.samples ?? 0),
      0,
    ) ?? 0;
  const rateRebuildJournalBlocker = rateRebuildJournalError(rateRebuildResult);
  const startRateRebuild = async () => {
    setRateRebuildError(null);
    setRateRebuildResult(null);
    rebuildNoticeRef.current = addNotification({
      title: "Pricing cache rebuild started",
      body: "Kassiber is clearing provider-derived prices, fetching fresh Coinbase one-minute windows, and reprocessing journals.",
      tone: "warning",
      progress: {
        indeterminate: true,
        label: "Rebuilding",
      },
    });
    try {
      const envelope = await rebuildRates.mutateAsync({
        source: "coinbase-exchange",
        reprice_transactions: true,
      });
      const payload = envelope.data ?? null;
      setRateRebuildResult(payload);
      setRateRebuildOpen(false);
      const journalBlocker = rateRebuildJournalError(payload);
      const fetchedRows =
        payload?.sync.reduce(
          (total, row) => total + Number(row.samples ?? 0),
          0,
        ) ?? 0;
      const notification = {
        title: journalBlocker
          ? "Pricing cache rebuilt with journal blocker"
          : "Pricing cache rebuilt",
        body: payload
          ? `${formatCount(payload.deleted.transaction_prices)} cached transaction prices cleared; ${formatCount(
              fetchedRows,
            )} rate rows fetched.${journalBlocker ? ` ${journalBlocker}` : ""}`
          : "Coinbase pricing cache was rebuilt.",
        tone: journalBlocker ? "warning" : "success",
        progress: undefined,
      } as const;
      if (rebuildNoticeRef.current) {
        updateNotification(rebuildNoticeRef.current, notification);
        rebuildNoticeRef.current = null;
      } else {
        addNotification(notification);
      }
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "Could not rebuild pricing cache.";
      setRateRebuildError(message);
      const notification = {
        title: "Pricing cache rebuild failed",
        body: message,
        tone: "error",
        progress: undefined,
      } as const;
      if (rebuildNoticeRef.current) {
        updateNotification(rebuildNoticeRef.current, notification);
        rebuildNoticeRef.current = null;
      } else {
        addNotification(notification);
      }
    }
  };
  const importedPairs = krakenImportResult?.summary ?? [];
  const importedTotals = krakenImportResult?.totals;
  return (
    <section className="space-y-4">
      <p className="max-w-2xl text-sm text-muted-foreground">
        Fiat reference rates are sourced independently of wallet sync. Kassiber
        keeps a local price cache so reports never have to query an exchange for
        every transaction. These lookups reveal pricing interest, not your
        wallet addresses.
      </p>

      <div className="space-y-2">
        <p className="text-sm font-medium">Rate providers</p>
        <BackendTable backends={rateBackends} />
      </div>

      <div className="rounded-md border bg-background p-3">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0">
            <p className="text-sm font-medium">Rebuild pricing cache</p>
            <p className="text-xs text-muted-foreground">
              Clear Coinbase provider samples, checked-empty minutes, and
              cached provider-generated transaction prices, then fetch fresh
              one-minute rates for the active books.
            </p>
          </div>
          <Button
            type="button"
            variant="outline"
            className="shrink-0"
            onClick={() => {
              setRateRebuildError(null);
              setRateRebuildOpen(true);
            }}
            disabled={isRebuildingRates || isImportingKraken}
          >
            {isRebuildingRates ? (
              <RefreshCw className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <Database className="size-4" aria-hidden="true" />
            )}
            Rebuild cache
          </Button>
        </div>
        <p className="mt-2 text-xs text-muted-foreground">
          Manual overrides and imported exchange execution prices are kept. Large
          wallets can take a while because Kassiber refetches missing windows and
          reprocesses journals afterward.
        </p>
        {isRebuildingRates ? (
          <div className="mt-3 rounded-md border border-primary/25 bg-primary/5 p-3">
            <div className="flex items-center justify-between gap-3 text-xs">
              <span className="font-medium text-foreground">
                Rebuilding provider rates
              </span>
              <span className="text-muted-foreground">
                Counting transaction rates…
              </span>
            </div>
            <div
              className="mt-2 h-2 overflow-hidden rounded-full bg-muted"
              role="progressbar"
              aria-label="Pricing cache rebuild progress"
              aria-valuetext="Rebuilding pricing cache"
            >
              <div className="h-full w-1/2 animate-pulse rounded-full bg-primary" />
            </div>
            <p className="mt-2 text-xs text-muted-foreground">
              Kassiber is fetching missing one-minute rates and will report how
              many transactions have provider rates when journals finish.
            </p>
          </div>
        ) : null}
        {rateRebuildResult ? (
          <div
            className={cn(
              "mt-3 rounded-md border p-3 text-sm",
              rateRebuildJournalBlocker
                ? "border-amber-500/40 bg-amber-500/10 text-amber-800 dark:text-amber-200"
                : "border-emerald-500/30 bg-emerald-500/5 text-emerald-700 dark:text-emerald-300",
            )}
          >
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span className="font-medium">
                {rateRebuildJournalBlocker
                  ? "Pricing refreshed; journals still blocked"
                  : rateRebuildProgress?.total
                  ? `${formatCount(rateRebuildProgress.refreshed)} / ${formatCount(
                      rateRebuildProgress.total,
                    )} transaction rates refreshed`
                  : "Pricing cache rebuilt"}
              </span>
              <span
                className={cn(
                  "text-xs",
                  rateRebuildJournalBlocker
                    ? "text-amber-800/80 dark:text-amber-200/80"
                    : "text-emerald-700/80 dark:text-emerald-300/80",
                )}
              >
                {formatCount(rateRebuildSamples)} rate rows fetched
              </span>
            </div>
            <div
              className={cn(
                "mt-2 h-2 overflow-hidden rounded-full",
                rateRebuildJournalBlocker
                  ? "bg-amber-950/10 dark:bg-amber-100/15"
                  : "bg-emerald-950/10 dark:bg-emerald-100/15",
              )}
              role="progressbar"
              aria-label="Transaction rate refresh progress"
              aria-valuemin={0}
              aria-valuemax={rateRebuildProgress?.total ?? 1}
              aria-valuenow={rateRebuildProgress?.refreshed ?? 1}
            >
              <div
                className={cn(
                  "h-full w-full rounded-full",
                  rateRebuildJournalBlocker ? "bg-amber-500" : "bg-emerald-500",
                )}
              />
            </div>
            {rateRebuildJournalBlocker ? (
              <p className="mt-2 text-xs text-amber-800/80 dark:text-amber-200/80">
                {rateRebuildJournalBlocker}
              </p>
            ) : null}
            <p
              className={cn(
                "mt-2 text-xs",
                rateRebuildJournalBlocker
                  ? "text-amber-800/80 dark:text-amber-200/80"
                  : "text-emerald-700/80 dark:text-emerald-300/80",
              )}
            >
              Removed {formatCount(rateRebuildResult.deleted.rates)} rate rows,{" "}
              {formatCount(rateRebuildResult.deleted.checked_minutes)} checked
              minutes, and{" "}
              {formatCount(rateRebuildResult.deleted.transaction_prices)} cached
              transaction prices.
            </p>
          </div>
        ) : null}
      </div>

      <div className="rounded-md border bg-background p-3">
        <div className="flex flex-col gap-2 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0">
            <p className="text-sm font-medium">Kraken offline history</p>
            <p className="text-xs text-muted-foreground">
              One-minute Bitcoin candles from a local Kraken CSV or ZIP archive.
            </p>
            <div className="mt-2 flex flex-wrap gap-x-3 gap-y-1 text-xs">
              <a
                href={KRAKEN_OHLCVT_SUPPORT_URL}
                onClick={(event) =>
                  openMarketDataUrl(event, KRAKEN_OHLCVT_SUPPORT_URL)
                }
                className="inline-flex items-center gap-1 text-primary underline-offset-4 hover:underline"
              >
                Get Kraken archive
                <ExternalLink className="size-3" aria-hidden="true" />
              </a>
              <a
                href={KRAKEN_MARKET_DATA_BLOG_URL}
                onClick={(event) =>
                  openMarketDataUrl(event, KRAKEN_MARKET_DATA_BLOG_URL)
                }
                className="inline-flex items-center gap-1 text-primary underline-offset-4 hover:underline"
              >
                Kraken market data blog
                <ExternalLink className="size-3" aria-hidden="true" />
              </a>
            </div>
          </div>
          <span className="inline-flex w-fit items-center rounded-md border bg-muted px-2 py-1 text-xs text-muted-foreground">
            kraken-csv
          </span>
        </div>

        <div className="mt-3 grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto]">
          <Input
            value={krakenArchivePath}
            onChange={(event) => setKrakenArchivePath(event.target.value)}
            placeholder="~/Downloads/Kraken_OHLCVT.zip or extracted folder"
            aria-label="Kraken CSV, ZIP, or folder path"
            disabled={isImportingKraken}
          />
          <div className="flex gap-2">
            <Button
              type="button"
              variant="outline"
              className="flex-1 sm:flex-none"
              onClick={() => void chooseKrakenArchive()}
              disabled={!isFilePickerAvailable || isImportingKraken}
              title={
                isFilePickerAvailable
                  ? "Choose CSV or ZIP"
                  : "Use the path field in browser mode"
              }
            >
              <Upload className="size-4" aria-hidden="true" />
              File
            </Button>
            <Button
              type="button"
              variant="outline"
              className="flex-1 sm:flex-none"
              onClick={() => void chooseKrakenDirectory()}
              disabled={!isFilePickerAvailable || isImportingKraken}
              title={
                isFilePickerAvailable
                  ? "Choose extracted folder"
                  : "Use the path field in browser mode"
              }
            >
              <FileInput className="size-4" aria-hidden="true" />
              Folder
            </Button>
          </div>
        </div>

        <div className="mt-3 flex flex-col gap-2 sm:flex-row">
          <Button
            type="button"
            onClick={() => void startKrakenImport("full")}
            disabled={isImportingKraken}
          >
            {pendingKrakenOperation === "full" ? (
              <RefreshCw className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <Database className="size-4" aria-hidden="true" />
            )}
            Full history
          </Button>
          <Button
            type="button"
            variant="outline"
            onClick={() => void startKrakenImport("incremental")}
            disabled={isImportingKraken}
          >
            {pendingKrakenOperation === "incremental" ? (
              <RefreshCw className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <RefreshCw className="size-4" aria-hidden="true" />
            )}
            Incremental update
          </Button>
        </div>

        {krakenImportError ? (
          <div className="mt-3 flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">
            <XCircle className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            <span>{krakenImportError}</span>
          </div>
        ) : null}

        {krakenImportResult ? (
          <div className="mt-3 rounded-md border border-emerald-500/30 bg-emerald-500/5 p-3">
            <div className="flex items-start gap-2 text-sm text-emerald-700 dark:text-emerald-300">
              <CheckCircle2 className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
              <span>
                {importedTotals?.pairs
                  ? `${formatCount(importedTotals.samples)} rows across ${formatCount(
                      importedTotals.pairs,
                    )} pair${importedTotals.pairs === 1 ? "" : "s"}`
                  : "No Bitcoin minute rows imported"}
              </span>
            </div>
            {importedPairs.length ? (
              <div className="mt-2 divide-y rounded-md border bg-background text-xs">
                {importedPairs.map((row) => (
                  <div
                    key={row.pair}
                    className="grid gap-1 px-3 py-2 sm:grid-cols-[120px_minmax(0,1fr)_120px]"
                  >
                    <span className="font-medium">{row.pair}</span>
                    <span className="truncate text-muted-foreground">
                      {formatKrakenRange(row)}
                    </span>
                    <span className="text-muted-foreground sm:text-right">
                      {formatCount(row.samples)} rows
                    </span>
                  </div>
                ))}
              </div>
            ) : null}
            {importedTotals?.skipped_rows || importedTotals?.skipped_files ? (
              <p className="mt-2 text-xs text-muted-foreground">
                Skipped {formatCount(importedTotals.skipped_rows)} rows and{" "}
                {formatCount(importedTotals.skipped_files)} files.
              </p>
            ) : null}
          </div>
        ) : null}
      </div>
      <Dialog open={rateRebuildOpen} onOpenChange={setRateRebuildOpen}>
        <DialogContent className="max-w-md">
          <DialogHeader>
            <DialogTitle>Rebuild pricing cache?</DialogTitle>
            <DialogDescription>
              Kassiber will delete Coinbase provider cache rows and refetch
              one-minute rates for missing transaction windows in the active
              books.
            </DialogDescription>
          </DialogHeader>
          <div className="rounded-md border border-amber-500/30 bg-amber-500/10 p-3 text-sm text-amber-800 dark:text-amber-200">
            <div className="flex items-start gap-2">
              <AlertTriangle
                className="mt-0.5 size-4 shrink-0"
                aria-hidden="true"
              />
              <div className="space-y-1">
                <p className="font-medium">Large wallets can take a while.</p>
                <p>
                  The rebuild also clears provider-generated transaction prices
                  and reprocesses journals. Manual overrides and imported
                  execution prices are preserved.
                </p>
              </div>
            </div>
          </div>
          {rateRebuildError ? (
            <p className="text-sm text-destructive">{rateRebuildError}</p>
          ) : null}
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setRateRebuildOpen(false)}
              disabled={isRebuildingRates}
            >
              Cancel
            </Button>
            <Button
              type="button"
              onClick={() => void startRateRebuild()}
              disabled={isRebuildingRates}
            >
              {isRebuildingRates ? (
                <RefreshCw className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <Database className="size-4" aria-hidden="true" />
              )}
              {isRebuildingRates ? "Rebuilding..." : "Rebuild"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}

function BackendTable({
  backends,
  actions = false,
  onEdit,
  onDelete,
}: {
  backends: Backend[];
  actions?: boolean;
  onEdit?: (backend: Backend) => void;
  onDelete?: (backend: Backend) => void;
}) {
  return (
    <div className="overflow-x-auto rounded-md border bg-background">
      <Table>
        <TableHeader>
          <TableRow className="bg-muted/50 hover:bg-muted/50">
            <TableHead>Backend</TableHead>
            <TableHead>Network</TableHead>
            <TableHead>Health</TableHead>
            <TableHead>Auth</TableHead>
            <TableHead className="text-right">Status</TableHead>
            {actions ? <TableHead className="text-right">Actions</TableHead> : null}
          </TableRow>
        </TableHeader>
        <TableBody>
          {backends.map((backend) => (
            <TableRow key={backend.id}>
              <TableCell className="min-w-[240px]">
                <div className="font-medium">{backend.name}</div>
                <div className="max-w-[360px] truncate text-xs text-muted-foreground">
                  {backend.url}
                </div>
              </TableCell>
              <TableCell>
                <NetworkBadge net={backend.net} />
              </TableCell>
              <TableCell className="text-muted-foreground">
                {backend.health}
              </TableCell>
              <TableCell className="text-muted-foreground">
                {backend.auth}
              </TableCell>
              <TableCell className="text-right">
                <StatusBadge active={backend.on} />
              </TableCell>
              {actions ? (
                <TableCell className="text-right">
                  <div className="flex justify-end gap-1">
                    <Button
                      type="button"
                      size="icon-sm"
                      variant="ghost"
                      aria-label={`Edit ${backend.name}`}
                      onClick={() => onEdit?.(backend)}
                    >
                      <Pencil className="size-3.5" aria-hidden="true" />
                    </Button>
                    <Button
                      type="button"
                      size="icon-sm"
                      variant="ghost"
                      aria-label={`Delete ${backend.name}`}
                      onClick={() => onDelete?.(backend)}
                    >
                      <Trash2 className="size-3.5" aria-hidden="true" />
                    </Button>
                  </div>
                </TableCell>
              ) : null}
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </div>
  );
}

function DataSettingsPanel({ status }: { status: StatusData | null }) {
  return (
    <section className="space-y-4">
      <div className="flex items-center gap-2">
        <Archive className="size-4 text-muted-foreground" aria-hidden="true" />
        <h3 className="text-sm font-semibold">Backups, labels &amp; imports</h3>
      </div>
      <div className="grid gap-2 sm:grid-cols-3">
        <Button type="button" variant="outline" className="justify-start">
          <Download className="size-4" aria-hidden="true" />
          Backup
        </Button>
        <Button type="button" variant="outline" className="justify-start">
          <Upload className="size-4" aria-hidden="true" />
          Restore
        </Button>
        <Button type="button" variant="outline" className="justify-start">
          <FileInput className="size-4" aria-hidden="true" />
          Logs
        </Button>
      </div>
      <Separator />
      <div className="grid gap-2 sm:grid-cols-3">
        <Button type="button" variant="secondary" className="justify-start">
          Import BIP-329
        </Button>
        <Button type="button" variant="secondary" className="justify-start">
          Export BIP-329
        </Button>
        <Button type="button" variant="secondary" className="justify-start">
          Import CSV
        </Button>
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="space-y-1.5">
          <Label htmlFor="settings-data-root">Data root</Label>
          <Input
            id="settings-data-root"
            readOnly
            value={status?.data_root ?? "loading..."}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="settings-db-path">Database</Label>
          <Input
            id="settings-db-path"
            readOnly
            value={status?.database ?? "loading..."}
          />
        </div>
      </div>
    </section>
  );
}

function AiProvidersPanel({
  aiFeaturesEnabled,
  setAiFeaturesEnabled,
}: {
  aiFeaturesEnabled: boolean;
  setAiFeaturesEnabled: (enabled: boolean) => void;
}) {
  const providersQuery = useDaemon<AiProvidersListData>("ai.providers.list");
  const data = React.useMemo<AiProvidersListData>(
    () =>
      providersQuery.data?.kind === "ai.providers.list" &&
      providersQuery.data.data
        ? providersQuery.data.data
        : { providers: [], default: null },
    [providersQuery.data],
  );
  const setDefault = useDaemonMutation("ai.providers.set_default");
  const deleteProvider = useDaemonMutation("ai.providers.delete");
  const moveProviderKey = useDaemonMutation("ai.providers.move_api_key");
  const [editingName, setEditingName] = React.useState<string | null>(null);
  const [addOpen, setAddOpen] = React.useState(false);
  const nativeStoreId = data.secret_store_policy?.default?.native_store_id ?? null;
  const nativeAvailable = data.secret_store_policy?.default?.native_available === true;
  const policyWarning = data.secret_store_policy?.default?.warning;

  const editingProvider = React.useMemo<ExistingAiProvider | null>(() => {
    if (!editingName) return null;
    const row = data.providers.find((provider) => provider.name === editingName);
    if (!row) return null;
    return {
      name: row.name,
      base_url: row.base_url,
      default_model: row.default_model ?? undefined,
      kind: row.kind,
      notes: row.notes ?? undefined,
      has_api_key: row.has_api_key,
      secret_ref: row.secret_ref,
      acknowledged_at: row.acknowledged_at ?? null,
    };
  }, [data.providers, editingName]);

  return (
    <div className="space-y-4">
      <div className="flex flex-col gap-3 rounded-md border bg-background p-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="min-w-0 space-y-1">
          <Label htmlFor="settings-ai-features">AI features</Label>
          <p className="text-sm text-muted-foreground">
            Show the Assistant screen and floating chat. Turning this off keeps
            provider settings saved.
          </p>
        </div>
        <Switch
          id="settings-ai-features"
          checked={aiFeaturesEnabled}
          onCheckedChange={setAiFeaturesEnabled}
          aria-label="Enable AI features"
          className="shrink-0"
        />
      </div>

      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0 space-y-1">
          <h3 className="text-sm font-semibold">Provider configuration</h3>
          <p className="text-sm text-muted-foreground">
            Configure OpenAI-compatible endpoints or Claude/Codex CLI adapters
            for the in-app assistant. Local Ollama runs without a key; remote
            and CLI providers may see prompt content.
          </p>
        </div>
        <Button
          type="button"
          size="sm"
          className="shrink-0"
          onClick={() => setAddOpen(true)}
        >
          <Plus className="size-4" aria-hidden="true" />
          Add provider
        </Button>
      </div>

      {policyWarning ? (
        <div className="rounded-md border border-amber-500/30 bg-amber-500/10 p-3 text-sm text-amber-800 dark:text-amber-200">
          {policyWarning}
        </div>
      ) : null}

      {providersQuery.isLoading ? (
        <div className="rounded-md border bg-background p-4 text-sm text-muted-foreground">
          Loading providers...
        </div>
      ) : providersQuery.isError ? (
        <div className="rounded-md border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive">
          Could not load AI providers.
        </div>
      ) : (
        <div className="overflow-x-auto rounded-md border bg-background">
          <Table>
            <TableHeader>
              <TableRow className="bg-muted/50 hover:bg-muted/50">
                <TableHead>Provider</TableHead>
                <TableHead>Posture</TableHead>
                <TableHead>Default model</TableHead>
                <TableHead>Auth</TableHead>
                <TableHead>Storage</TableHead>
                <TableHead className="text-right">Default</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.providers.length === 0 ? (
                <TableRow>
                  <TableCell
                    colSpan={7}
                    className="text-center text-sm text-muted-foreground"
                  >
                    No providers configured.
                  </TableCell>
                </TableRow>
              ) : (
                data.providers.map((row) => (
                  <TableRow key={row.name}>
                    <TableCell className="min-w-[220px]">
                      <div className="font-medium">{row.name}</div>
                      <div className="max-w-[320px] truncate text-xs text-muted-foreground">
                        {row.base_url}
                      </div>
                    </TableCell>
                    <TableCell>
                      <span
                        className={cn(
                          "inline-flex rounded-full border px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide",
                          AI_KIND_BADGE[row.kind],
                        )}
                      >
                        {row.kind === "tee" ? "TEE" : row.kind}
                      </span>
                    </TableCell>
                    <TableCell className="max-w-[340px] whitespace-normal break-words font-mono text-xs">
                      <AiProviderModelSummary row={row} />
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {row.has_api_key ? "Bearer" : "none"}
                    </TableCell>
                    <TableCell className="min-w-[170px] text-xs text-muted-foreground">
                      <div>{aiSecretStoreLabel(row.secret_ref?.store_id)}</div>
                      <div className="font-mono">{aiSecretStateLabel(row.secret_ref?.state)}</div>
                    </TableCell>
                    <TableCell className="text-right">
                      {row.is_default ? (
                        <span
                          className={cn(
                            "inline-flex rounded-md border px-2 py-1 text-xs font-medium",
                            "border-primary/25 bg-primary/10 text-primary",
                          )}
                        >
                          Default
                        </span>
                      ) : (
                        <Button
                          type="button"
                          size="sm"
                          variant="outline"
                          disabled={setDefault.isPending}
                          onClick={() => setDefault.mutate({ name: row.name })}
                        >
                          Set default
                        </Button>
                      )}
                    </TableCell>
                    <TableCell className="text-right">
                      <div className="flex justify-end gap-1">
                        <Button
                          type="button"
                          size="icon-sm"
                          variant="ghost"
                          aria-label={`Edit ${row.name}`}
                          onClick={() => setEditingName(row.name)}
                        >
                          <Pencil className="size-3.5" aria-hidden="true" />
                        </Button>
                        {nativeStoreId &&
                        nativeAvailable &&
                        row.secret_ref?.store_id === "sqlcipher_inline" &&
                        row.has_api_key ? (
                          <Button
                            type="button"
                            size="icon-sm"
                            variant="ghost"
                            aria-label={`Move ${row.name} key to native storage`}
                            disabled={moveProviderKey.isPending}
                            onClick={() =>
                              moveProviderKey.mutate({
                                name: row.name,
                                store_id: nativeStoreId,
                              })
                            }
                          >
                            <ShieldCheck className="size-3.5" aria-hidden="true" />
                          </Button>
                        ) : null}
                        {row.secret_ref?.store_id &&
                        row.secret_ref.store_id !== "sqlcipher_inline" ? (
                          <Button
                            type="button"
                            size="icon-sm"
                            variant="ghost"
                            aria-label={`Move ${row.name} key to SQLCipher storage`}
                            disabled={moveProviderKey.isPending || !row.has_api_key}
                            onClick={() =>
                              moveProviderKey.mutate({
                                name: row.name,
                                store_id: "sqlcipher_inline",
                              })
                            }
                          >
                            <Database className="size-3.5" aria-hidden="true" />
                          </Button>
                        ) : null}
                        <Button
                          type="button"
                          size="icon-sm"
                          variant="ghost"
                          aria-label={`Delete ${row.name}`}
                          disabled={row.is_default || deleteProvider.isPending}
                          onClick={() => {
                            const ok = window.confirm(
                              `Delete AI provider '${row.name}'? Cannot be undone.`,
                            );
                            if (!ok) return;
                            deleteProvider.mutate({ name: row.name });
                          }}
                        >
                          <Trash2 className="size-3.5" aria-hidden="true" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </div>
      )}

      <AiProviderForm
        open={addOpen}
        initial={null}
        onClose={() => setAddOpen(false)}
        onSaved={() => setAddOpen(false)}
      />
      <AiProviderForm
        open={Boolean(editingProvider)}
        initial={editingProvider}
        onClose={() => setEditingName(null)}
        onSaved={() => setEditingName(null)}
      />
    </div>
  );
}

const NETWORK_LAYER_META: Record<
  NetworkLayer,
  { blurb: string; empty: string; addLabel: string }
> = {
  bitcoin: {
    blurb:
      "Explorer API, Electrum/Fulcrum, or Bitcoin Core RPC endpoints that serve on-chain history to your watch-only wallets.",
    empty:
      "No Bitcoin indexers yet. Add one so on-chain wallets can refresh their balances.",
    addLabel: "Add Bitcoin backend",
  },
  lightning: {
    blurb:
      "Read-only connections to your LND or Core Lightning node for channel accounting and profitability reports.",
    empty:
      "No Lightning nodes connected. Add a read-only LND or Core Lightning connection.",
    addLabel: "Add Lightning node",
  },
  liquid: {
    blurb:
      "Explorer API or Electrum/Fulcrum endpoints that serve Liquid (L-BTC) history to your watch-only wallets.",
    empty:
      "No Liquid indexers yet. Add one so L-BTC wallets can refresh their balances.",
    addLabel: "Add Liquid backend",
  },
};

function NetworkLayerPanel({
  layer,
  backends,
  onAdd,
  onEdit,
  onDelete,
}: {
  layer: NetworkLayer;
  backends: Backend[];
  onAdd: () => void;
  onEdit: (backend: Backend) => void;
  onDelete: (backend: Backend) => void;
}) {
  const meta = NETWORK_LAYER_META[layer];
  const layerBackends = backendsForLayer(backends, layer);
  const explorerLinkBase =
    layer === "lightning"
      ? null
      : layerBackends.map(backendExplorerBaseUrl).find(Boolean) ?? null;
  return (
    <section className="space-y-4">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <p className="max-w-2xl text-sm text-muted-foreground">{meta.blurb}</p>
        <Button type="button" size="sm" className="shrink-0" onClick={onAdd}>
          <Plus className="size-4" aria-hidden="true" />
          {meta.addLabel}
        </Button>
      </div>

      {layer === "lightning" ? (
        <div className="flex items-start gap-2 rounded-md border border-sky-500/25 bg-sky-500/5 p-3 text-xs text-muted-foreground">
          <ShieldCheck
            className="mt-0.5 size-4 shrink-0 text-sky-600 dark:text-sky-400"
            aria-hidden="true"
          />
          <span>
            Lightning connections are strictly read-only. Node identity details —
            operator pubkey, channel points, peer aliases, and short channel ids
            — stay on this machine.
          </span>
        </div>
      ) : null}

      {layerBackends.length === 0 ? (
        <div className="rounded-md border border-dashed bg-muted/20 p-6 text-center text-sm text-muted-foreground">
          {meta.empty}
        </div>
      ) : (
        <div className="grid gap-3">
          {layerBackends.map((backend) => (
            <BackendLayerCard
              key={backend.id}
              backend={backend}
              onEdit={() => onEdit(backend)}
              onDelete={() => onDelete(backend)}
            />
          ))}
        </div>
      )}

      {layer === "bitcoin" || layer === "liquid" ? (
        <p className="text-xs text-muted-foreground">
          {explorerLinkBase
            ? `Transaction links open on ${explorerHostLabel(
                explorerLinkBase,
              )}; this is derived from the Explorer API backend.`
            : `Transaction links use the public ${
                layer === "bitcoin"
                  ? "mempool.bitcoin-austria.at"
                  : "Liquid Network"
              } default until you add an Explorer API backend. Electrum/Fulcrum backends are sync-only.`}
        </p>
      ) : null}
    </section>
  );
}

function BackendLayerCard({
  backend,
  onEdit,
  onDelete,
}: {
  backend: Backend;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const trust = backendTrust(backend);
  const TrustIcon = trust.icon;
  const explorerBaseUrl = backendExplorerBaseUrl(backend);
  return (
    <div className="rounded-md border bg-background p-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-medium">{backend.name}</span>
            {backend.isDefault ? (
              <span className="inline-flex items-center rounded-md border border-primary/25 bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-primary">
                Default
              </span>
            ) : null}
            <StatusBadge active={backend.on} />
          </div>
          <p className="truncate font-mono text-xs text-muted-foreground">
            {backend.url}
          </p>
        </div>
        <div className="flex shrink-0 gap-1">
          <Button
            type="button"
            size="icon-sm"
            variant="ghost"
            aria-label={`Edit ${backend.name}`}
            onClick={onEdit}
          >
            <Pencil className="size-3.5" aria-hidden="true" />
          </Button>
          <Button
            type="button"
            size="icon-sm"
            variant="ghost"
            aria-label={`Delete ${backend.name}`}
            onClick={onDelete}
          >
            <Trash2 className="size-3.5" aria-hidden="true" />
          </Button>
        </div>
      </div>
      <div className="mt-3 flex flex-wrap items-center gap-2">
        <span className="inline-flex items-center rounded-md border bg-muted px-2 py-0.5 text-xs font-medium text-muted-foreground">
          {backendProtocolLabel(backend)}
        </span>
        <span
          className={cn(
            "inline-flex items-center gap-1 rounded-md border px-2 py-0.5 text-xs font-medium",
            trust.className,
          )}
        >
          <TrustIcon className="size-3" aria-hidden="true" />
          {trust.label}
        </span>
        {explorerBaseUrl ? (
          <span className="inline-flex items-center rounded-md border border-sky-500/25 bg-sky-500/10 px-2 py-0.5 text-xs font-medium text-sky-700 dark:text-sky-300">
            Links: {explorerHostLabel(explorerBaseUrl)}
          </span>
        ) : null}
      </div>
      <p className="mt-2 text-xs text-muted-foreground">{trust.note}</p>
    </div>
  );
}

function ExposureStat({
  icon: Icon,
  tone,
  count,
  label,
  hint,
}: {
  icon: LucideIcon;
  tone: "ok" | "info" | "warn";
  count: number;
  label: string;
  hint: string;
}) {
  const toneClass =
    tone === "ok"
      ? "text-emerald-600 dark:text-emerald-400"
      : tone === "info"
        ? "text-sky-600 dark:text-sky-400"
        : count > 0
          ? "text-amber-600 dark:text-amber-400"
          : "text-muted-foreground";
  return (
    <div className="rounded-md border bg-background p-3">
      <div className="flex items-center gap-2">
        <Icon className={cn("size-4", toneClass)} aria-hidden="true" />
        <span className="font-mono text-lg tabular-nums">{count}</span>
      </div>
      <p className="mt-1 text-sm font-medium">{label}</p>
      <p className="text-xs text-muted-foreground">{hint}</p>
    </div>
  );
}

function PrivacySettingsPanel({
  hideSensitive,
  setHideSensitive,
  clearClipboard,
  setClearClipboard,
  backends,
  aiFeaturesEnabled,
  onManageConnections,
  onManageAi,
}: {
  hideSensitive: boolean;
  setHideSensitive: (value: boolean) => void;
  clearClipboard: boolean;
  setClearClipboard: (value: boolean) => void;
  backends: Backend[];
  aiFeaturesEnabled: boolean;
  onManageConnections: () => void;
  onManageAi: () => void;
}) {
  const [exposureExpanded, setExposureExpanded] = React.useState(false);
  const syncBackends = backends.filter((backend) => backend.net !== "FX");
  const postureCount = (posture: TrustPosture) =>
    syncBackends.filter((backend) => backendTrust(backend).posture === posture)
      .length;
  const onDeviceCount = postureCount("on-device");
  const shieldedCount = postureCount("shielded");
  const remoteCount = postureCount("remote");
  const outboundSurfaceCount =
    shieldedCount + remoteCount + (aiFeaturesEnabled ? 1 : 0);
  const exposureSummary =
    outboundSurfaceCount > 0
      ? `${outboundSurfaceCount} outbound surface${
          outboundSurfaceCount === 1 ? "" : "s"
        } configured`
      : "No outbound surfaces configured";
  return (
    <div className="space-y-6">
      <section className="space-y-3">
        <h3 className="text-sm font-semibold">On screen</h3>
        <SettingsSwitchRow
          label="Blur sensitive values"
          description={
            hideSensitive
              ? "Balances, addresses, and amounts are blurred until you reveal them."
              : "Balances, addresses, and amounts are shown in full."
          }
          checked={hideSensitive}
          onCheckedChange={setHideSensitive}
        />
        <SettingsSwitchRow
          label="Clear clipboard after copy"
          description={
            clearClipboard
              ? "Copied addresses and keys are cleared from the system clipboard after 30 seconds."
              : "Copied values stay in the system clipboard until overwritten."
          }
          checked={clearClipboard}
          onCheckedChange={setClearClipboard}
        />
      </section>

      <section className="rounded-md border bg-background">
        <button
          type="button"
          className="flex w-full items-start justify-between gap-4 rounded-t-md px-3 py-3 text-left transition-colors hover:bg-muted/40"
          aria-expanded={exposureExpanded}
          onClick={() => setExposureExpanded((expanded) => !expanded)}
        >
          <span className="min-w-0 space-y-1">
            <span className="block text-sm font-semibold">
              What leaves this machine
            </span>
            <span className="block text-sm text-muted-foreground">
              Kassiber is local-first. Network backends and enabled assistant
              providers are the outbound surfaces.
            </span>
          </span>
          <span className="flex shrink-0 items-center gap-2 text-xs text-muted-foreground">
            {exposureSummary}
            <ChevronDown
              className={cn(
                "size-4 transition-transform",
                exposureExpanded ? "rotate-180" : "",
              )}
              aria-hidden="true"
            />
          </span>
        </button>
        <div className="grid gap-2 border-t p-3 sm:grid-cols-3">
          <ExposureStat
            icon={ShieldCheck}
            tone="ok"
            count={onDeviceCount}
            label="On device"
            hint="Queries never leave your machine"
          />
          <ExposureStat
            icon={Network}
            tone="info"
            count={shieldedCount}
            label="Tor / proxy"
            hint="IP hidden from the server"
          />
          <ExposureStat
            icon={ShieldOff}
            tone="warn"
            count={remoteCount}
            label="Remote"
            hint="Can observe queried addresses"
          />
        </div>
        {exposureExpanded ? (
          <div className="space-y-4 border-t p-3">
            <div className="space-y-2">
              <div className="flex items-center justify-between gap-3">
                <h4 className="text-sm font-medium">Network backends</h4>
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  className="shrink-0"
                  onClick={onManageConnections}
                >
                  Review network
                </Button>
              </div>
              <div className="divide-y rounded-md border">
                {syncBackends.length > 0 ? (
                  syncBackends.map((backend) => {
                    const trust = backendTrust(backend);
                    const TrustIcon = trust.icon;
                    return (
                      <div
                        key={backend.id}
                        className="grid gap-2 p-3 text-sm sm:grid-cols-[1fr_140px_160px]"
                      >
                        <div className="min-w-0">
                          <p className="truncate font-medium">
                            {backend.name}
                          </p>
                          <p className="truncate text-xs text-muted-foreground">
                            {backendProtocolLabel(backend)} ·{" "}
                            {endpointHostLabel(backend.url)}
                          </p>
                        </div>
                        <div className="text-xs text-muted-foreground">
                          {backend.net}
                        </div>
                        <div
                          className={cn(
                            "flex items-center gap-2 rounded-md border px-2 py-1 text-xs",
                            trust.className,
                          )}
                        >
                          <TrustIcon className="size-3.5" aria-hidden="true" />
                          <span className="truncate">{trust.label}</span>
                        </div>
                      </div>
                    );
                  })
                ) : (
                  <p className="p-3 text-sm text-muted-foreground">
                    No network backends configured.
                  </p>
                )}
              </div>
            </div>
            <div className="flex flex-col gap-2 rounded-md border p-3 sm:flex-row sm:items-center sm:justify-between">
              <div className="min-w-0 space-y-1">
                <h4 className="text-sm font-medium">Assistant prompts</h4>
                <p className="text-sm text-muted-foreground">
                  {aiFeaturesEnabled
                    ? "Assistant features are enabled. Local providers keep prompts on this machine; remote and CLI providers can see prompt content."
                    : "Assistant features are off. Provider settings can stay saved without sending prompts."}
                </p>
              </div>
              <Button
                type="button"
                size="sm"
                variant="outline"
                className="shrink-0"
                onClick={onManageAi}
              >
                Review AI providers
              </Button>
            </div>
          </div>
        ) : null}
      </section>
    </div>
  );
}

function DataAndStoragePanel({
  status,
  onResetWelcome,
  onResetBook,
  resetBookDisabled,
  onDeleteBooks,
  deleteBooksDisabled,
}: {
  status: StatusData | null;
  onResetWelcome: () => void;
  onResetBook: () => void;
  resetBookDisabled: boolean;
  onDeleteBooks: () => void;
  deleteBooksDisabled: boolean;
}) {
  return (
    <div className="space-y-6">
      <DataSettingsPanel status={status} />

      <section className="space-y-3 rounded-md border border-destructive/30 bg-destructive/5 p-4">
        <div className="space-y-1">
          <h3 className="flex items-center gap-2 text-sm font-semibold text-destructive">
            <AlertTriangle className="size-4" aria-hidden="true" />
            Danger zone
          </h3>
          <p className="text-sm text-muted-foreground">
            Reset the Welcome gate, clear testing data, or delete the current
            local books set.
          </p>
        </div>
        <div className="flex flex-col gap-3 rounded-md border bg-background p-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0 space-y-1">
            <p className="text-sm font-medium">Reset Welcome state</p>
            <p className="text-sm text-muted-foreground">
              Clear only the local UI identity and return to onboarding.
              Encrypted data on disk is untouched.
            </p>
          </div>
          <Button
            type="button"
            variant="outline"
            className="shrink-0"
            onClick={onResetWelcome}
          >
            Reset Welcome
          </Button>
        </div>
        <div className="flex flex-col gap-3 rounded-md border border-amber-500/30 bg-amber-500/5 p-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0 space-y-1">
            <p className="text-sm font-medium">Reset book data</p>
            <p className="text-sm text-muted-foreground">
              Keep wallet and backend connections, then clear synced
              transactions, journals, swaps, labels, attachments, and
              source-funds work. Shared fiat rates are optional.
            </p>
          </div>
          <Button
            type="button"
            variant="outline"
            className="shrink-0"
            disabled={resetBookDisabled}
            onClick={onResetBook}
          >
            <RefreshCw className="mr-2 size-4" aria-hidden="true" />
            Reset book
          </Button>
        </div>
        <div className="flex flex-col gap-3 rounded-md border border-destructive/30 bg-background p-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="min-w-0 space-y-1">
            <p className="text-sm font-medium text-destructive">
              Delete books set
            </p>
            <p className="text-sm text-muted-foreground">
              Remove the current books records from the local database.
            </p>
          </div>
          <Button
            type="button"
            variant="destructive"
            className="shrink-0"
            disabled={deleteBooksDisabled}
            onClick={onDeleteBooks}
          >
            Delete books
          </Button>
        </div>
      </section>
    </div>
  );
}

function NetworkBadge({ net }: { net: Net }) {
  const classes: Record<Net, string> = {
    BTC: "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
    LIQUID:
      "border-sky-500/25 bg-sky-500/10 text-sky-700 dark:text-sky-300",
    LN: "border-violet-500/25 bg-violet-500/10 text-violet-700 dark:text-violet-300",
    FX: "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  };

  return (
    <span
      className={cn(
        "inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-medium",
        classes[net],
      )}
    >
      {net}
    </span>
  );
}

function NetworkMark({ type }: { type: SyncBackendNetwork }) {
  if (!type.icon) return <NetworkBadge net={type.net} />;
  return (
    <span
      className={cn(
        "flex size-9 shrink-0 items-center justify-center rounded-md border p-1.5",
        type.iconFrameClassName ?? brandLogoFrame,
      )}
      aria-hidden="true"
    >
      <img
        src={type.icon}
        alt=""
        className={cn("size-6 object-contain", type.iconClassName)}
      />
    </span>
  );
}

function PresetMark({
  preset,
  net,
}: {
  preset: SyncBackendPreset;
  net: Net;
}) {
  const image =
    preset.id === "mempool"
      ? mempoolIcon
      : preset.protocol === "coreln"
        ? coreLightningIcon
        : preset.protocol === "lnd"
          ? lightningLabsIcon
          : net === "LIQUID"
            ? liquidIcon
            : preset.protocol === "esplora"
              ? bitcoinIcon
              : null;
  if (image) {
    return (
      <span
        className={cn(
          "flex size-8 shrink-0 items-center justify-center rounded-md border p-1.5",
          preset.protocol === "coreln"
            ? "border-neutral-600 bg-[#494120]"
            : preset.protocol === "lnd"
              ? "border-neutral-700 bg-neutral-950"
              : brandLogoFrame,
        )}
        aria-hidden="true"
      >
        <img
          src={image}
          alt=""
          className={cn(
            "size-5 object-contain",
            preset.protocol === "coreln" && "scale-150",
            net === "LIQUID" && "scale-150",
          )}
        />
      </span>
    );
  }
  return (
    <span
      className="flex size-8 shrink-0 items-center justify-center rounded-md border bg-background text-muted-foreground"
      aria-hidden="true"
    >
      <Server className="size-4" />
    </span>
  );
}

function presetDisplayName(preset: SyncBackendPreset): string {
  return preset.providerLabel ?? preset.name;
}

function selectorButtonClass(active: boolean) {
  return cn(
    "border text-foreground shadow-xs transition-colors",
    active
      ? "border-foreground/50 bg-muted text-foreground ring-1 ring-foreground/10 hover:bg-muted/90 dark:border-white/45 dark:bg-white/[0.10] dark:text-white dark:ring-white/10 dark:hover:bg-white/[0.14]"
      : "border-border bg-background hover:border-foreground/35 hover:bg-muted dark:border-white/20 dark:bg-white/[0.04] dark:text-white dark:hover:border-white/40 dark:hover:bg-white/[0.08]",
  );
}

function StatusBadge({ active }: { active: boolean }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-medium",
        active
          ? "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
          : "border-border bg-muted text-muted-foreground",
      )}
    >
      {active ? "Active" : "Idle"}
    </span>
  );
}

interface SyncBackendPreset {
  id: string;
  name: string;
  url: string;
  protocol:
    | "esplora"
    | "electrum"
    | "bitcoinrpc"
    | "liquid-esplora"
    | "lnd"
    | "coreln";
  label: string;
  providerLabel?: string;
  publicPreset?: boolean;
  disabled?: boolean;
  status?: string;
}

interface SyncBackendNetwork {
  id: "bitcoin" | "liquid" | "coreln" | "lnd";
  label: string;
  net: Net;
  desc: string;
  icon: string;
  iconClassName?: string;
  iconFrameClassName?: string;
  subtitle?: string;
  presets: SyncBackendPreset[];
}

const SYNC_BACKEND_NETWORKS: SyncBackendNetwork[] = [
  {
    id: "bitcoin",
    label: "Bitcoin",
    net: "BTC",
    desc: "Backends used by Bitcoin watch-only wallets.",
    icon: bitcoinIcon,
    subtitle: "Bitcoin",
    presets: [
      {
        id: "mempool",
        name: DEFAULT_BACKEND_NAME,
        url: DEFAULT_BACKEND_URL,
        protocol: "esplora",
        label: "Explorer API",
        providerLabel: "mempool.bitcoin-austria.at",
      },
      {
        id: "electrum",
        name: "Bitcoin Austria Fulcrum",
        url: "ssl://index.bitcoin-austria.at:50002",
        protocol: "electrum",
        label: "Electrum / Fulcrum",
        providerLabel: "Bitcoin Austria",
      },
      {
        id: "core",
        name: "Bitcoin Core RPC",
        url: "http://127.0.0.1:8332",
        protocol: "bitcoinrpc",
        label: "Bitcoin Core RPC",
        publicPreset: false,
      },
    ],
  },
  {
    id: "coreln",
    label: "Core-LN",
    net: "LN",
    desc: "Read-only Core Lightning node accounting sync.",
    icon: coreLightningIcon,
    iconClassName: "scale-150",
    iconFrameClassName: "border-neutral-600 bg-[#494120]",
    subtitle: "Lightning",
    presets: [
      {
        id: "core-lightning",
        name: "Core Lightning read-only",
        url: "cln://commando",
        protocol: "coreln",
        label: "Commando rune",
      },
    ],
  },
  {
    id: "liquid",
    label: "Liquid",
    net: "LIQUID",
    desc: "Backends used by Liquid watch-only wallets.",
    icon: liquidIcon,
    iconClassName: "scale-150",
    subtitle: "Liquid",
    presets: [
      {
        id: "liquid-electrum",
        name: "Liquid Electrum",
        url: "ssl://liquid.example:50002",
        protocol: "electrum",
        label: "Electrum / Fulcrum",
        publicPreset: false,
      },
      {
        id: "liquid-network",
        name: "Liquid Network",
        url: "https://liquid.network/api",
        protocol: "liquid-esplora",
        label: "Explorer API",
        providerLabel: "Liquid Network",
      },
    ],
  },
  {
    id: "lnd",
    label: "LND",
    net: "LN",
    desc: "Read-only Lightning node history for profitability reports.",
    icon: lightningLabsIcon,
    iconFrameClassName: "border-neutral-700 bg-neutral-950",
    subtitle: "Lightning",
    presets: [
      {
        id: "lnd",
        name: "LND",
        url: "https://127.0.0.1:8080",
        protocol: "lnd",
        label: "LND REST",
      },
    ],
  },
];

function scopedBackendTypes(
  initialTypeId?: SyncBackendNetwork["id"],
): SyncBackendNetwork[] {
  if (initialTypeId === "bitcoin") {
    return SYNC_BACKEND_NETWORKS.filter((candidate) => candidate.id === "bitcoin");
  }
  if (initialTypeId === "liquid") {
    return SYNC_BACKEND_NETWORKS.filter((candidate) => candidate.id === "liquid");
  }
  if (initialTypeId === "coreln" || initialTypeId === "lnd") {
    return SYNC_BACKEND_NETWORKS.filter(
      (candidate) => candidate.id === "coreln" || candidate.id === "lnd",
    );
  }
  return SYNC_BACKEND_NETWORKS;
}

function backendModalCopy({
  isEditing,
  typeId,
  scopedTypes,
}: {
  isEditing: boolean;
  typeId: SyncBackendNetwork["id"];
  scopedTypes: SyncBackendNetwork[];
}): { title: string; description: string; selectorLabel: string } {
  if (isEditing) {
    return {
      title: "Edit sync backend",
      description: "Update this wallet-refresh endpoint.",
      selectorLabel: "Network",
    };
  }
  if (scopedTypes.length === 1) {
    if (typeId === "bitcoin") {
      return {
        title: "Add Bitcoin backend",
        description:
          "Connect a Bitcoin indexer or node used by watch-only wallets.",
        selectorLabel: "Network",
      };
    }
    if (typeId === "liquid") {
      return {
        title: "Add Liquid backend",
        description:
          "Connect a Liquid indexer used by Liquid watch-only wallets.",
        selectorLabel: "Network",
      };
    }
  }
  if (scopedTypes.every((candidate) => candidate.net === "LN")) {
    return {
      title: "Add Lightning node",
      description:
        "Connect a read-only Lightning node for accounting and profitability reports.",
      selectorLabel: "Node",
    };
  }
  return {
    title: "Add sync backend",
    description: "Connect a Bitcoin, Liquid, or Lightning backend.",
    selectorLabel: "Network",
  };
}

const AUTH_MODES: Array<{ id: string; label: string }> = [
  { id: "none", label: "None" },
  { id: "apikey", label: "API key" },
  { id: "basic", label: "User + pass" },
  { id: "bearer", label: "Bearer token" },
];

function normalizedBackendKind(kind: string | null | undefined): string {
  return (kind ?? "").toLowerCase().replace(/-/g, "");
}

function backendTypeIdForConnectionSetup(
  intent: DeferredConnectionSetup | null,
): SyncBackendNetwork["id"] | undefined {
  const kind = normalizedBackendKind(intent?.backendKind);
  if (kind === "coreln") return "coreln";
  if (kind === "lnd") return "lnd";
  if (intent?.sourceId === "core-ln") return "coreln";
  if (intent?.sourceId === "lnd") return "lnd";
  return undefined;
}

type TestState = "idle" | "testing" | "ok" | "fail";
type BackendSourceMode = "preset" | "custom";

interface ElectrumEndpointParts {
  host: string;
  port: string;
  useSsl: boolean;
}

function parseElectrumEndpoint(raw: string): ElectrumEndpointParts {
  const candidate = raw.includes("://") ? raw : `ssl://${raw}`;
  try {
    const parsed = new URL(candidate);
    const useSsl = parsed.protocol !== "tcp:";
    return {
      host: parsed.hostname,
      port: parsed.port || (useSsl ? "50002" : "50001"),
      useSsl,
    };
  } catch {
    return { host: "", port: "50002", useSsl: true };
  }
}

function buildElectrumUrl({ host, port, useSsl }: ElectrumEndpointParts): string {
  const trimmedHost = host.trim();
  const trimmedPort = port.trim();
  if (!trimmedHost || !trimmedPort) return "";
  return `${useSsl ? "ssl" : "tcp"}://${trimmedHost}:${trimmedPort}`;
}

function customBackendName(
  type: SyncBackendNetwork,
  preset: SyncBackendPreset | null,
): string {
  if (type.net === "LIQUID") return "My Liquid backend";
  if (type.net === "BTC") return "My Bitcoin backend";
  return preset?.name ?? "My backend";
}

function applyCustomEndpointDefaults(
  preset: SyncBackendPreset | null,
  {
    setUrl,
    setElectrumHost,
    setElectrumPort,
    setElectrumUseSsl,
  }: {
    setUrl: (value: string) => void;
    setElectrumHost: (value: string) => void;
    setElectrumPort: (value: string) => void;
    setElectrumUseSsl: (value: boolean) => void;
  },
) {
  if (preset?.protocol === "electrum") {
    setUrl("");
    setElectrumHost("");
    setElectrumPort("50002");
    setElectrumUseSsl(true);
    return;
  }
  setUrl("");
}

function randomPreset(type: SyncBackendNetwork): SyncBackendPreset | null {
  const candidates = publicBackendPresets(type);
  if (candidates.length === 0) return null;
  const cryptoApi = globalThis.crypto;
  if (cryptoApi?.getRandomValues) {
    const values = new Uint32Array(1);
    cryptoApi.getRandomValues(values);
    return candidates[values[0] % candidates.length];
  }
  return candidates[Math.floor(Math.random() * candidates.length)];
}

function publicBackendPresets(type: SyncBackendNetwork): SyncBackendPreset[] {
  return type.presets.filter(
    (candidate) => candidate.publicPreset !== false && !candidate.disabled,
  );
}

interface BackendModalProps {
  open: boolean;
  initial: Backend | null;
  initialTypeId?: SyncBackendNetwork["id"];
  onClose: () => void;
  onSave: (backend: Backend) => void | Promise<void>;
}

function BackendModal({
  open,
  initial,
  initialTypeId,
  onClose,
  onSave,
}: BackendModalProps) {
  const testElectrum = useDaemonMutation<{
    ok: boolean;
    logs: string[];
  }>("ui.backends.electrum.test");
  const testHttp = useDaemonMutation<{
    ok: boolean;
    logs: string[];
    status?: number;
  }>("ui.backends.http.test");
  const [typeId, setTypeId] = React.useState<SyncBackendNetwork["id"]>("bitcoin");
  const [backendSource, setBackendSource] =
    React.useState<BackendSourceMode>("preset");
  const [presetId, setPresetId] = React.useState("mempool");
  const [name, setName] = React.useState("");
  const [url, setUrl] = React.useState(DEFAULT_BACKEND_URL);
  const [auth, setAuth] = React.useState("none");
  const [authVal, setAuthVal] = React.useState("");
  const [authVal2, setAuthVal2] = React.useState("");
  const [electrumHost, setElectrumHost] = React.useState("");
  const [electrumPort, setElectrumPort] = React.useState("50002");
  const [electrumUseSsl, setElectrumUseSsl] = React.useState(true);
  const [trustSsl, setTrustSsl] = React.useState(false);
  const [infrastructureOwner, setInfrastructureOwner] =
    React.useState<InfrastructureOwnership>("third_party");
  const [certificate, setCertificate] = React.useState("");
  const [useProxy, setUseProxy] = React.useState(false);
  const [proxyHost, setProxyHost] = React.useState("");
  const [proxyPort, setProxyPort] = React.useState("");
  const [commandoPeerId, setCommandoPeerId] = React.useState("");
  const [lightningCli, setLightningCli] = React.useState("");
  const [lightningDir, setLightningDir] = React.useState("");
  const [rpcFile, setRpcFile] = React.useState("");
  const [testState, setTestState] = React.useState<TestState>("idle");
  const [testLog, setTestLog] = React.useState("");
  const [saveState, setSaveState] = React.useState<"idle" | "saving">("idle");

  const type =
    SYNC_BACKEND_NETWORKS.find((candidate) => candidate.id === typeId) ??
    SYNC_BACKEND_NETWORKS[0];
  const preset =
    presetId === "custom"
      ? null
      : type.presets.find((candidate) => candidate.id === presetId) ?? null;
  const isEditing = Boolean(initial);
  const scopedTypes = React.useMemo(
    () => (isEditing ? SYNC_BACKEND_NETWORKS : scopedBackendTypes(initialTypeId)),
    [initialTypeId, isEditing],
  );
  const modalCopy = backendModalCopy({
    isEditing,
    typeId,
    scopedTypes,
  });
  const publicPresets = React.useMemo(() => publicBackendPresets(type), [type]);
  const showTypePicker = scopedTypes.length > 1;
  const showSourcePicker = !isEditing && type.net !== "LN";
  const showPresetPicker =
    !isEditing && publicPresets.length > 0 && backendSource === "preset";
  const showCustomProtocolPicker =
    !isEditing && type.presets.length > 1 && backendSource === "custom";
  const isCoreLightning =
    preset?.protocol === "coreln" || initial?.kind === "coreln";
  const isElectrum = preset?.protocol === "electrum";
  const isLnd = preset?.protocol === "lnd" || initial?.kind === "lnd";
  const showAuth = preset?.protocol === "bitcoinrpc" || isLnd;
  const showElectrumEndpointParts = isElectrum;
  const effectiveUrl = showElectrumEndpointParts
    ? buildElectrumUrl({
        host: electrumHost,
        port: electrumPort,
        useSsl: electrumUseSsl,
      })
    : isCoreLightning
      ? url.trim() || "cln://commando"
      : url.trim();
  const selectedBackendKind =
    preset?.protocol ??
    initial?.kind ??
    (type.net === "LIQUID"
      ? "liquid-esplora"
      : type.net === "LN"
        ? "lnd"
        : "esplora");
  const selectedKindIsExplorerApi =
    selectedBackendKind === "esplora" ||
    selectedBackendKind === "liquid-esplora";
  const connectionTrust = backendTrustFromEndpoint(
    effectiveUrl,
    showElectrumEndpointParts && useProxy && Boolean(proxyHost.trim()),
    backendSource === "custom" ? "self" : infrastructureOwner,
  );
  const ConnectionTrustIcon = connectionTrust.icon;

  React.useEffect(() => {
    if (!open) return;
    if (initial) {
      const parsedElectrum = parseElectrumEndpoint(initial.url);
      const initialKind = normalizedBackendKind(initial.kind);
      const initialType =
        SYNC_BACKEND_NETWORKS.find((candidate) =>
          candidate.presets.some(
            (preset) => normalizedBackendKind(preset.protocol) === initialKind,
          ),
        ) ??
        SYNC_BACKEND_NETWORKS.find((candidate) => candidate.net === initial.net) ??
        SYNC_BACKEND_NETWORKS[0];
      const initialPreset =
        initialType.presets.find((candidate) => candidate.url === initial.url) ??
        (initial.url.match(/^(ssl|tcp):\/\//i)
          ? initialType.presets.find((candidate) => candidate.protocol === "electrum")
          : null);
      setTypeId(initialType.id);
      setBackendSource("custom");
      setPresetId(initialPreset?.id ?? "custom");
      setName(initial.name);
      setUrl(initial.url);
      setAuth(initial.auth);
      setAuthVal("");
      setAuthVal2("");
      setCommandoPeerId(
        initial.commandoPeerId === CLN_PRESENCE_SENTINEL_COMMANDO_PEER
          ? ""
          : initial.commandoPeerId ?? "",
      );
      setLightningCli(initial.lightningCli ?? "");
      setLightningDir(
        initial.lightningDir === CLN_PRESENCE_SENTINEL_LIGHTNING_DIR
          ? ""
          : initial.lightningDir ?? "",
      );
      setRpcFile(
        initial.rpcFile === CLN_PRESENCE_SENTINEL_RPC_FILE
          ? ""
          : initial.rpcFile ?? "",
      );
      setElectrumHost(parsedElectrum.host);
      setElectrumPort(parsedElectrum.port);
      setElectrumUseSsl(parsedElectrum.useSsl);
      setTrustSsl(Boolean(initial.trustSsl));
      setInfrastructureOwner(
        initial.infrastructureOwner ??
          inferredInfrastructureOwnership(initial.url),
      );
      setCertificate(initial.certificate ?? "");
      setUseProxy(Boolean(initial.proxy));
      setProxyHost(initial.proxy?.host ?? "");
      setProxyPort(initial.proxy?.port ?? "");
      setTestState(initial.on ? "ok" : "idle");
      setTestLog("");
      setSaveState("idle");
      return;
    }

    const nextType =
      scopedTypes.find((candidate) => candidate.id === initialTypeId) ??
      scopedTypes[0] ??
      SYNC_BACKEND_NETWORKS[0];
    const nextPreset = randomPreset(nextType);
    setTypeId(nextType.id);
    setBackendSource(nextType.net === "LN" ? "custom" : "preset");
    setPresetId(nextPreset?.id ?? "custom");
    setName(nextPreset?.name ?? DEFAULT_BACKEND_NAME);
    setUrl(nextPreset?.url ?? DEFAULT_BACKEND_URL);
    setAuth("none");
    setAuthVal("");
    setAuthVal2("");
    setElectrumHost("index.bitcoin-austria.at");
    setElectrumPort("50002");
    setElectrumUseSsl(true);
    setTrustSsl(false);
    setInfrastructureOwner(
      inferredInfrastructureOwnership(nextPreset?.url ?? DEFAULT_BACKEND_URL),
    );
    setCertificate("");
    setUseProxy(false);
    setProxyHost("");
    setProxyPort("");
    setTestState("idle");
    setTestLog("");
    setSaveState("idle");
  }, [initial, initialTypeId, open, scopedTypes]);

  React.useEffect(() => {
    if (!open) return;
    if (initial) return;
    if (preset) {
      if (backendSource === "preset" || type.net === "LN") {
        setUrl(preset.url);
        setName(preset.name);
        setInfrastructureOwner(inferredInfrastructureOwnership(preset.url));
      } else {
        setName(customBackendName(type, preset));
        applyCustomEndpointDefaults(preset, {
          setUrl,
          setElectrumHost,
          setElectrumPort,
          setElectrumUseSsl,
        });
        setInfrastructureOwner("self");
      }
      setAuth(preset.protocol === "lnd" ? "apikey" : "none");
      if (backendSource === "preset" && preset.protocol === "electrum") {
        const parsed = parseElectrumEndpoint(preset.url);
        setElectrumHost(parsed.host);
        setElectrumPort(parsed.port);
        setElectrumUseSsl(parsed.useSsl);
      }
    } else if (presetId === "custom") {
      setUrl("");
      setName("");
      setAuth("none");
      setInfrastructureOwner("self");
    }
    setAuthVal("");
    setAuthVal2("");
    setCommandoPeerId("");
    setLightningCli("");
    setLightningDir("");
    setRpcFile("");
    setTestState("idle");
    setTestLog("");
  }, [backendSource, initial, open, preset, presetId, type]);

  const onPickType = (id: SyncBackendNetwork["id"]) => {
    setTypeId(id);
    setTestLog("");
    if (initial) {
      setPresetId("custom");
      return;
    }
    const nextType = SYNC_BACKEND_NETWORKS.find((candidate) => candidate.id === id);
    setBackendSource(nextType?.net === "LN" ? "custom" : "preset");
    setPresetId(nextType ? randomPreset(nextType)?.id ?? "custom" : "custom");
  };

  const testConnection = async () => {
    if (!effectiveUrl) return false;
    if (isCoreLightning) {
      setTestState("ok");
      setTestLog("Core Lightning read-only connection will be checked during wallet sync.");
      return true;
    }
    setTestState("testing");
    if (isElectrum) {
      try {
        const envelope = await testElectrum.mutateAsync({
          url: effectiveUrl,
          trust_self_signed: electrumUseSsl && trustSsl,
          certificate:
            electrumUseSsl && !trustSsl && certificate.trim()
              ? certificate.trim()
              : undefined,
          proxy:
            useProxy && proxyHost.trim() && proxyPort.trim()
              ? `${proxyHost.trim()}:${proxyPort.trim()}`
              : undefined,
        });
        const data = envelope.data;
        setTestState(data?.ok ? "ok" : "fail");
        setTestLog((data?.logs ?? []).join("\n"));
        return Boolean(data?.ok);
      } catch (error) {
        setTestState("fail");
        setTestLog(
          error instanceof Error ? error.message : "Electrum test failed.",
        );
        return false;
      }
    }
    try {
      const envelope = await testHttp.mutateAsync({
        url: effectiveUrl,
      });
      const data = envelope.data;
      setTestState(data?.ok ? "ok" : "fail");
      setTestLog((data?.logs ?? []).join("\n"));
      return Boolean(data?.ok);
    } catch (error) {
      setTestState("fail");
      setTestLog(error instanceof Error ? error.message : "HTTP test failed.");
      return false;
    }
  };

  // Editing an existing Core Lightning backend keeps redacted-but-set fields
  // from being treated as missing. The daemon never returns the rune itself;
  // it only signals presence via `auth === "apikey"` and the "Configured *"
  // sentinel strings used elsewhere in this file.
  const initialCoreLnHasRune =
    isCoreLightning && initial?.auth === "apikey";
  const initialCoreLnHasCommandoPeer =
    isCoreLightning &&
    initial?.commandoPeerId === CLN_PRESENCE_SENTINEL_COMMANDO_PEER;
  const initialCoreLnHasLightningDir =
    isCoreLightning &&
    initial?.lightningDir === CLN_PRESENCE_SENTINEL_LIGHTNING_DIR;
  const initialCoreLnHasRpcFile =
    isCoreLightning && initial?.rpcFile === CLN_PRESENCE_SENTINEL_RPC_FILE;

  const coreLightningModeValid = coreLightningBackendModeValid({
    commandoPeerId: commandoPeerId.trim(),
    rune: authVal.trim(),
    lightningDir: lightningDir.trim(),
    rpcFile: rpcFile.trim(),
    hadRune: initialCoreLnHasRune,
    hadCommandoPeerId: initialCoreLnHasCommandoPeer,
    hadLightningDir: initialCoreLnHasLightningDir,
    hadRpcFile: initialCoreLnHasRpcFile,
  });

  const canAdd =
    name.trim().length > 0 &&
    effectiveUrl.length > 0 &&
    (!isCoreLightning || coreLightningModeValid);
  const save = async () => {
    if (!canAdd) return;
    const normalizedUrl = effectiveUrl;
    let connected = testState === "ok" || isCoreLightning;
    setSaveState("saving");
    if (!connected) {
      connected = await testConnection();
      if (!connected) {
        setSaveState("idle");
        return;
      }
    }
    try {
      const authSecret = authVal.trim();
      const authPassword = authVal2.trim();
      await onSave({
        id: initial?.id ?? name.trim(),
        name: name.trim(),
        url: normalizedUrl,
        net: type.net,
        kind: selectedBackendKind,
        chain:
          type.net === "LIQUID"
            ? "liquid"
            : type.net === "LN"
              ? "bitcoin"
              : "bitcoin",
        network:
          type.net === "LIQUID"
            ? "liquidv1"
            : type.net === "LN"
              ? "main"
              : "main",
        health: initial ? "just checked - ok" : "just added - ok",
        on: connected,
        auth: isCoreLightning ? "apikey" : showAuth ? auth : "none",
        authHeader:
          showAuth && auth === "bearer" && authSecret
            ? `Bearer ${authSecret}`
            : undefined,
        token:
          (showAuth && auth === "apikey" && authSecret) ||
          (isCoreLightning && authSecret)
            ? authSecret
            : undefined,
        username:
          showAuth && auth === "basic" && authSecret ? authSecret : undefined,
        password:
          showAuth && auth === "basic" && authPassword
            ? authPassword
            : undefined,
        commandoPeerId:
          isCoreLightning && commandoPeerId.trim()
            ? commandoPeerId.trim()
            : undefined,
        lightningCli:
          isCoreLightning && lightningCli.trim() ? lightningCli.trim() : undefined,
        lightningDir:
          isCoreLightning && lightningDir.trim() ? lightningDir.trim() : undefined,
        rpcFile: isCoreLightning && rpcFile.trim() ? rpcFile.trim() : undefined,
        trustSsl:
          (showElectrumEndpointParts && electrumUseSsl) || isLnd
            ? trustSsl
            : undefined,
        infrastructureOwner: type.net !== "LN" ? infrastructureOwner : undefined,
        certificate:
          ((showElectrumEndpointParts && electrumUseSsl && !trustSsl) || isLnd) &&
          certificate.trim()
            ? certificate.trim()
            : undefined,
        proxy:
          showElectrumEndpointParts && useProxy && proxyHost.trim() && proxyPort.trim()
            ? { host: proxyHost.trim(), port: proxyPort.trim() }
            : null,
      });
    } catch (error) {
      setTestState("fail");
      setTestLog(error instanceof Error ? error.message : "Could not save backend.");
    } finally {
      setSaveState("idle");
    }
  };
  const isSavingBackend = saveState === "saving";

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      <DialogContent className="top-[6vh] max-h-[88vh] w-full max-w-[760px] translate-y-0 overflow-hidden p-0 sm:max-w-[760px]">
        <DialogHeader className="border-b px-6 py-5">
          <DialogTitle>{modalCopy.title}</DialogTitle>
          <DialogDescription>{modalCopy.description}</DialogDescription>
        </DialogHeader>

        <ScrollArea className="max-h-[calc(88vh-150px)]">
          <div className="space-y-5 p-6">
            {showTypePicker ? (
              <section className="space-y-3">
                <div>
                  <Label>{modalCopy.selectorLabel}</Label>
                  <p className="text-sm text-muted-foreground">{type.desc}</p>
                </div>
                <div className="grid gap-2 sm:grid-cols-2">
                  {scopedTypes.map((backendType) => {
                    const active = backendType.id === typeId;
                    return (
                      <Button
                        key={backendType.id}
                        type="button"
                        variant="outline"
                        className={cn(
                          "h-auto min-h-[72px] items-center justify-start gap-3 whitespace-normal p-3 text-left",
                          selectorButtonClass(active),
                        )}
                        onClick={() => onPickType(backendType.id)}
                      >
                        <NetworkMark type={backendType} />
                        <span className="min-w-0 space-y-0.5">
                          <span className="block text-sm leading-tight font-medium">
                            {backendType.label}
                          </span>
                          <span className="block text-xs leading-tight text-muted-foreground">
                            {backendType.subtitle ?? backendType.net}
                          </span>
                        </span>
                      </Button>
                    );
                  })}
                </div>
              </section>
            ) : null}

            {showSourcePicker ? (
              <section className="space-y-3">
                <div>
                  <Label>Backend source</Label>
                  <p className="text-xs text-muted-foreground">
                    Use a third-party preset, or enter infrastructure you
                    operate.
                  </p>
                </div>
                <Tabs
                  value={backendSource}
                  onValueChange={(value) =>
                    setBackendSource(value as BackendSourceMode)
                  }
                >
                  <TabsList className="w-full justify-start sm:w-fit">
                    <TabsTrigger value="preset">
                      Third-party presets
                    </TabsTrigger>
                    <TabsTrigger value="custom">Custom backend</TabsTrigger>
                  </TabsList>
                </Tabs>
              </section>
            ) : null}

            {showPresetPicker ? (
              <section className="space-y-3">
                <div>
                  <Label>Third-party endpoint</Label>
                  <p className="text-xs text-muted-foreground">
                    Pick from the bundled allowlist. Providers shown here have
                    a stated no-log policy.
                  </p>
                </div>
                <Select value={presetId} onValueChange={setPresetId}>
                  <SelectTrigger className="h-auto min-h-12 w-full py-2 text-left *:data-[slot=select-value]:line-clamp-none *:data-[slot=select-value]:justify-start">
                    <SelectValue placeholder="Choose a provider" />
                  </SelectTrigger>
                  <SelectContent>
                    {publicPresets.map((backendPreset) => {
                      const disabled = Boolean(backendPreset.disabled);
                      return (
                        <SelectItem
                          key={backendPreset.id}
                          value={backendPreset.id}
                          disabled={disabled}
                        >
                          <span className="flex min-w-0 items-center justify-start gap-2 text-left">
                            <PresetMark preset={backendPreset} net={type.net} />
                            <span className="min-w-0 space-y-0.5 text-left">
                              <span className="block truncate font-medium">
                                {presetDisplayName(backendPreset)}
                              </span>
                              <span className="block truncate text-xs text-muted-foreground">
                                {backendPreset.status ?? backendPreset.label}
                              </span>
                            </span>
                          </span>
                        </SelectItem>
                      );
                    })}
                  </SelectContent>
                </Select>
              </section>
            ) : null}

            {showCustomProtocolPicker ? (
              <section className="space-y-3">
                <div>
                  <Label>Endpoint type</Label>
                  <p className="text-xs text-muted-foreground">
                    Choose the protocol your backend exposes.
                  </p>
                </div>
                <Tabs value={presetId} onValueChange={setPresetId}>
                  <TabsList className="w-full flex-wrap justify-start sm:w-fit">
                    {type.presets.map((backendPreset) => (
                      <TabsTrigger
                        key={backendPreset.id}
                        value={backendPreset.id}
                        disabled={backendPreset.disabled}
                      >
                        {backendPreset.label}
                      </TabsTrigger>
                    ))}
                  </TabsList>
                </Tabs>
              </section>
            ) : null}

            <section className="grid gap-4 sm:grid-cols-2">
              <div className="space-y-2">
                <Label htmlFor="backend-name">Display name</Label>
                <Input
                  id="backend-name"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  placeholder="My home node"
                  disabled={isEditing}
                />
                {isEditing ? (
                  <p className="text-xs text-muted-foreground">
                    Backend names are stable because wallets may reference them.
                  </p>
                ) : null}
              </div>
              {showElectrumEndpointParts ? (
                <div className="grid gap-3 sm:grid-cols-[1fr_120px]">
                  <div className="space-y-2">
                    <Label htmlFor="backend-electrum-host">Host</Label>
                    <Input
                      id="backend-electrum-host"
                      value={electrumHost}
                      onChange={(event) => {
                        setElectrumHost(event.target.value);
                        setTestState("idle");
                        setTestLog("");
                      }}
                      placeholder={
                        type.net === "LIQUID"
                          ? "liquid-electrum.example"
                          : "index.bitcoin-austria.at"
                      }
                      disabled={backendSource === "preset" && !isEditing}
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="backend-electrum-port">Port</Label>
                    <Input
                      id="backend-electrum-port"
                      value={electrumPort}
                      onChange={(event) => {
                        setElectrumPort(event.target.value);
                        setTestState("idle");
                        setTestLog("");
                      }}
                      placeholder={electrumUseSsl ? "50002" : "50001"}
                      disabled={backendSource === "preset" && !isEditing}
                    />
                  </div>
                </div>
              ) : (
                <div className="space-y-2">
                  <Label htmlFor="backend-url">Endpoint URL</Label>
                  <Input
                    id="backend-url"
                    value={url}
                    onChange={(event) => {
                      setUrl(event.target.value);
                      setTestState("idle");
                      setTestLog("");
                    }}
                    placeholder="https://..."
                    disabled={backendSource === "preset" && !isEditing}
                  />
                </div>
              )}
            </section>

            {type.net !== "LN" && selectedKindIsExplorerApi ? (
              <div className="rounded-md border border-sky-500/25 bg-sky-500/5 p-3 text-xs text-muted-foreground">
                This Explorer API will also provide transaction links. Electrum
                and Fulcrum backends only provide wallet history sync.
              </div>
            ) : null}

            {type.net !== "LN" && backendSource === "preset" ? (
              <div
                className={cn(
                  "flex items-start gap-2 rounded-md border p-3 text-xs",
                  connectionTrust.className,
                )}
              >
                <ConnectionTrustIcon
                  className="mt-0.5 size-4 shrink-0"
                  aria-hidden="true"
                />
                <div>
                  <div className="text-sm font-medium">
                    {connectionTrust.label}
                  </div>
                  <p className="mt-0.5 leading-relaxed">
                    {connectionTrust.note}
                  </p>
                </div>
              </div>
            ) : null}

            {isCoreLightning && (
              <section className="space-y-3">
                <div>
                  <Label>Core Lightning access</Label>
                  <p className="text-xs text-muted-foreground">
                    Use a restricted commando rune for least-privilege read-only sync,
                    or point at a local lightning-dir / rpc-file when running on the
                    same host.
                  </p>
                </div>
                <div className="grid gap-3 sm:grid-cols-2">
                  <div className="space-y-2">
                    <Label htmlFor="backend-commando-peer">Commando peer id</Label>
                    <Input
                      id="backend-commando-peer"
                      value={commandoPeerId}
                      onChange={(event) => {
                        setCommandoPeerId(event.target.value);
                        setTestState("idle");
                        setTestLog("");
                      }}
                      placeholder="02..."
                    />
                  </div>
                  <SecretField
                    id="backend-commando-rune"
                    label="Rune"
                    value={authVal}
                    onChange={(value) => {
                      setAuthVal(value);
                      setTestState("idle");
                      setTestLog("");
                    }}
                    placeholder="readonly rune"
                  />
                  <div className="space-y-2">
                    <Label htmlFor="backend-lightning-cli">lightning-cli path</Label>
                    <Input
                      id="backend-lightning-cli"
                      value={lightningCli}
                      onChange={(event) => {
                        setLightningCli(event.target.value);
                        setTestState("idle");
                        setTestLog("");
                      }}
                      placeholder="lightning-cli"
                    />
                  </div>
                  <div className="space-y-2">
                    <Label htmlFor="backend-lightning-dir">Lightning directory</Label>
                    <Input
                      id="backend-lightning-dir"
                      value={lightningDir}
                      onChange={(event) => {
                        setLightningDir(event.target.value);
                        setTestState("idle");
                        setTestLog("");
                      }}
                      placeholder="~/.lightning"
                    />
                  </div>
                  <div className="space-y-2 sm:col-span-2">
                    <Label htmlFor="backend-rpc-file">RPC file</Label>
                    <Input
                      id="backend-rpc-file"
                      value={rpcFile}
                      onChange={(event) => {
                        setRpcFile(event.target.value);
                        setTestState("idle");
                        setTestLog("");
                      }}
                      placeholder="lightning-rpc"
                    />
                    <p className="text-xs text-muted-foreground">
                      Local RPC file access is convenient but broader than a read-only rune.
                    </p>
                  </div>
                </div>
              </section>
            )}

            {showElectrumEndpointParts && backendSource === "custom" && (
              <details
                className="group rounded-md border bg-muted/10"
                open={trustSsl || Boolean(certificate) || useProxy || undefined}
              >
                <summary className="flex cursor-pointer list-none items-center justify-between gap-3 px-3 py-2 text-sm font-medium">
                  <span>Advanced connection settings</span>
                  <span className="text-xs text-muted-foreground">
                    TLS certificate and proxy options
                  </span>
                </summary>
                <section className="grid gap-3 border-t p-3 sm:grid-cols-2">
                  <label className="flex items-center justify-between gap-3 rounded-md border bg-background p-3 text-sm">
                    <span>
                      <span className="block font-medium">Use SSL</span>
                      <span className="text-muted-foreground">
                        Common Electrum SSL port is 50002.
                      </span>
                    </span>
                    <Switch
                      checked={electrumUseSsl}
                      onCheckedChange={(checked) => {
                        setElectrumUseSsl(checked);
                        if (!checked) {
                          setTrustSsl(false);
                          setCertificate("");
                        }
                        setElectrumPort((current) =>
                          current === "50002" || current === "50001"
                            ? checked
                              ? "50002"
                              : "50001"
                            : current,
                        );
                        setTestState("idle");
                        setTestLog("");
                      }}
                    />
                  </label>
                  <label className="flex items-center justify-between gap-3 rounded-md border bg-background p-3 text-sm">
                    <span>
                      <span className="block font-medium">
                        Trust self-signed certificate
                      </span>
                      <span className="text-muted-foreground">
                        For self-signed or private CA Electrum servers.
                      </span>
                    </span>
                    <Switch
                      checked={trustSsl}
                      disabled={!electrumUseSsl}
                      onCheckedChange={(checked) => {
                        setTrustSsl(checked);
                        setTestState("idle");
                        setTestLog("");
                      }}
                    />
                  </label>
                  <div className="space-y-2 sm:col-span-2">
                    <Label htmlFor="backend-certificate">Certificate</Label>
                    <Input
                      id="backend-certificate"
                      value={certificate}
                      onChange={(event) => {
                        setCertificate(event.target.value);
                        setTestState("idle");
                        setTestLog("");
                      }}
                      placeholder="Optional server certificate (.crt)"
                      disabled={!electrumUseSsl || trustSsl}
                    />
                    {electrumUseSsl && trustSsl ? (
                      <p className="text-xs text-muted-foreground">
                        Ignored while &ldquo;Trust self-signed certificate&rdquo;
                        is on.
                      </p>
                    ) : null}
                  </div>
                  <label className="flex items-center justify-between gap-3 rounded-md border bg-background p-3 text-sm sm:col-span-2">
                    <span>
                      <span className="block font-medium">Use proxy</span>
                      <span className="text-muted-foreground">
                        Optional Tor or SOCKS proxy for this endpoint.
                      </span>
                    </span>
                    <Switch
                      checked={useProxy}
                      onCheckedChange={(checked) => {
                        setUseProxy(checked);
                        setTestState("idle");
                        setTestLog("");
                      }}
                    />
                  </label>
                  {useProxy && (
                    <>
                      <div className="space-y-2">
                        <Label htmlFor="backend-proxy-host">Proxy host</Label>
                        <Input
                          id="backend-proxy-host"
                          value={proxyHost}
                          onChange={(event) => {
                            setProxyHost(event.target.value);
                            setTestState("idle");
                            setTestLog("");
                          }}
                          placeholder="127.0.0.1"
                        />
                      </div>
                      <div className="space-y-2">
                        <Label htmlFor="backend-proxy-port">Proxy port</Label>
                        <Input
                          id="backend-proxy-port"
                          value={proxyPort}
                          onChange={(event) => {
                            setProxyPort(event.target.value);
                            setTestState("idle");
                            setTestLog("");
                          }}
                          placeholder="9050"
                        />
                      </div>
                    </>
                  )}
                </section>
              </details>
            )}

            {isLnd && (
              <section className="grid gap-3 sm:grid-cols-2">
                <label className="flex items-center justify-between gap-3 rounded-md border p-3 text-sm sm:col-span-2">
                  <span>
                    <span className="block font-medium">
                      Trust self-signed TLS
                    </span>
                    <span className="text-muted-foreground">
                      Use only for a local LND REST endpoint you control.
                    </span>
                  </span>
                  <Switch
                    checked={trustSsl}
                    onCheckedChange={(checked) => {
                      setTrustSsl(checked);
                      setTestState("idle");
                      setTestLog("");
                    }}
                  />
                </label>
                <div className="space-y-2 sm:col-span-2">
                  <Label htmlFor="backend-lnd-certificate">TLS certificate</Label>
                  <Input
                    id="backend-lnd-certificate"
                    value={certificate}
                    onChange={(event) => {
                      setCertificate(event.target.value);
                      setTestState("idle");
                      setTestLog("");
                    }}
                    placeholder="Path to tls.cert or PEM contents"
                    disabled={trustSsl}
                  />
                </div>
              </section>
            )}

            {showAuth && (
              <section className="space-y-3">
                <Label>RPC authentication</Label>
                <div className="flex flex-wrap gap-2">
                  {AUTH_MODES.map((mode) => (
                    <Button
                      key={mode.id}
                      type="button"
                      variant={auth === mode.id ? "default" : "outline"}
                      size="sm"
                      onClick={() => setAuth(mode.id)}
                    >
                      {mode.label}
                    </Button>
                  ))}
                </div>
                {auth === "apikey" && (
                  <SecretField
                    id="backend-api-key"
                    label={isLnd ? "Read-only macaroon hex" : "API key"}
                    value={authVal}
                    onChange={setAuthVal}
                    placeholder={isLnd ? "0201036c6e64..." : "sk_live_..."}
                  />
                )}
                {auth === "bearer" && (
                  <SecretField
                    id="backend-bearer"
                    label="Bearer token"
                    value={authVal}
                    onChange={setAuthVal}
                    placeholder="eyJ..."
                  />
                )}
                {auth === "basic" && (
                  <div className="grid gap-3 sm:grid-cols-2">
                    <div className="space-y-2">
                      <Label htmlFor="backend-username">Username</Label>
                      <Input
                        id="backend-username"
                        value={authVal}
                        onChange={(event) => setAuthVal(event.target.value)}
                      />
                    </div>
                    <SecretField
                      id="backend-password"
                      label="Password"
                      value={authVal2}
                      onChange={setAuthVal2}
                    />
                  </div>
                )}
              </section>
            )}

            <div className="rounded-md border bg-muted/30 p-3 text-sm">
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    void testConnection();
                  }}
                  disabled={
                    !effectiveUrl || testState === "testing" || isSavingBackend
                  }
                >
                  <RefreshCw
                    className={cn(
                      "size-4",
                      testState === "testing" && "animate-spin",
                    )}
                    aria-hidden="true"
                  />
                  {testState === "testing" ? "Testing" : "Test connection"}
                </Button>
                {testState === "ok" && (
                  <span className="inline-flex items-center gap-1 text-emerald-600 dark:text-emerald-400">
                    <CheckCircle2 className="size-4" />
                    Connected
                  </span>
                )}
                {testState === "fail" && (
                  <span className="inline-flex items-center gap-1 text-destructive">
                    <XCircle className="size-4" />
                    Could not reach endpoint
                  </span>
                )}
              </div>
              <textarea
                readOnly
                aria-label="Backend test connection log"
                value={testLog}
                className="mt-3 min-h-32 w-full resize-none rounded-md border bg-background p-3 font-mono text-xs leading-5"
              />
            </div>
          </div>
        </ScrollArea>

        <DialogFooter className="border-t px-6 py-4">
          <Button type="button" variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button
            type="button"
            disabled={!canAdd || isSavingBackend || testState === "testing"}
            onClick={() => {
              void save();
            }}
          >
            {isSavingBackend ? (
              <RefreshCw className="size-4 animate-spin" aria-hidden="true" />
            ) : null}
            {isSavingBackend
              ? "Connecting…"
              : testState === "ok"
                ? isEditing
                  ? "Save backend"
                  : "Add sync backend"
                : "Connect & save"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

interface SecretFieldProps {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
}

function SecretField({
  id,
  label,
  value,
  onChange,
  placeholder,
}: SecretFieldProps) {
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        type="password"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
      />
    </div>
  );
}
