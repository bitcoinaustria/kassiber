/**
 * SettingsScreen - workspace-wide preferences.
 *
 * Most controls are local UI state until the daemon-backed settings surface
 * lands. Hide-sensitive data is wired to the shared UI store.
 */
import * as React from "react";
import {
  AlertTriangle,
  Database,
  CheckCircle2,
  Download,
  ExternalLink,
  FileInput,
  KeyRound,
  Lock,
  Pencil,
  Plus,
  RefreshCw,
  Server,
  ShieldCheck,
  Trash2,
  Upload,
  XCircle,
} from "lucide-react";
import { useNavigate, useRouterState } from "@tanstack/react-router";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
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
import {
  SettingsIntegrations4,
  type IntegrationItem,
} from "@/components/shadcnblocks/settings-integrations4";
import bitcoinIcon from "@/assets/integrations/bitcoin.svg";
import liquidIcon from "@/assets/integrations/liquid.svg";
import mempoolIcon from "@/assets/integrations/mempool-space.svg";
import {
  AiProviderForm,
  type ExistingAiProvider,
} from "@/components/kb/AiProviderForm";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { clearImportProject } from "@/daemon/transport";
import type { ExplorerSettings } from "@/lib/explorer";
import { isFilePickerAvailable, pickFile } from "@/lib/filePicker";
import { setSessionUnlockPassphrase } from "@/store/sessionLock";
import { useUiStore, type AppLockPolicy } from "@/store/ui";
import type { AiModelsListData, AiModelRow } from "@/lib/aiCapabilities";
import { screenPanelClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
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
  auth: string;
  authHeader?: string;
  token?: string;
  username?: string;
  password?: string;
  trustSsl?: boolean;
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
  has_username?: boolean;
  has_password?: boolean;
  insecure?: boolean;
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

import { selectedIntegrationForHash } from "./settingsSections";

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
    name: "Liquid Electrum",
    url: "ssl://les.bullbitcoin.com:995",
    net: "LIQUID",
    health: "Liquid Electrum",
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

function isSyncBackend(backend: Backend): boolean {
  return backend.net === "BTC" || backend.net === "LIQUID";
}

function backendNetFromRow(row: BackendSettingsRow): Net {
  const chain = (row.chain ?? "").toLowerCase();
  const kind = (row.kind ?? "").toLowerCase();
  if (chain === "liquid" || kind === "liquid-esplora") return "LIQUID";
  return "BTC";
}

function backendAuthLabel(row: BackendSettingsRow): string {
  if (row.has_auth_header) return "bearer";
  if (row.has_token) return "apikey";
  if (row.has_username || row.has_password) return "basic";
  return "none";
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
    auth: backendAuthLabel(row),
    trustSsl: row.insecure,
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
  if (backend.certificate) {
    config.certificate = backend.certificate;
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

const backendIntegrationImage: Partial<Record<Net, string>> = {
  BTC: bitcoinIcon,
  LIQUID: liquidIcon,
};

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

function backendIntegrationArt(backend: Backend): Pick<
  IntegrationItem,
  "className" | "image" | "imageFrameClassName"
> {
  if (backend.name.toLowerCase().includes("mempool")) {
    return {
      image: mempoolIcon,
      className: "size-8",
    };
  }
  if (backend.net === "LIQUID") {
    return {
      image: liquidIcon,
      className: "size-8 scale-150",
    };
  }
  return {
    image: backendIntegrationImage[backend.net],
    className: "size-8",
  };
}

interface SettingsScreenProps {
  onLock?: () => void;
}

export function SettingsScreen({ onLock }: SettingsScreenProps) {
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const setHideSensitive = useUiStore((s) => s.setHideSensitive);
  const currency = useUiStore((s) => s.currency);
  const setCurrency = useUiStore((s) => s.setCurrency);
  const explorerSettings = useUiStore((s) => s.explorerSettings);
  const setExplorerSettings = useUiStore((s) => s.setExplorerSettings);
  const appLockPolicy = useUiStore((s) => s.appLockPolicy);
  const setAppLockPolicy = useUiStore((s) => s.setAppLockPolicy);
  const aiFeaturesEnabled = useUiStore((s) => s.aiFeaturesEnabled);
  const setAiFeaturesEnabled = useUiStore((s) => s.setAiFeaturesEnabled);
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
  const routeSelectedIntegrationId = React.useMemo(
    () => selectedIntegrationForHash(settingsHash),
    [settingsHash],
  );
  const statusQuery = useDaemon<StatusData>("status", undefined, {
    enabled: true,
  });
  const status =
    statusQuery.data?.kind === "status" ? statusQuery.data.data : null;
  const statusLoaded = statusQuery.data?.kind === "status";
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
  const deleteBackend = useDaemonMutation<{ name: string; deleted: boolean }>(
    "ui.backends.delete",
  );
  const [clearClipboard, setClearClipboard] = React.useState(true);
  const [backendDialogOpen, setBackendDialogOpen] = React.useState(false);
  const [editingBackendId, setEditingBackendId] = React.useState<string | null>(
    null,
  );
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
  const [selectedIntegrationId, setSelectedIntegrationId] = React.useState<
    string | null
  >(() => routeSelectedIntegrationId);

  React.useEffect(() => {
    setSelectedIntegrationId(routeSelectedIntegrationId);
  }, [routeSelectedIntegrationId]);

  // Native menu may re-fire for the same section while the URL hash is
  // unchanged (user already on /settings#privacy, clicks Privacy again after
  // closing the panel). The hash effect won't see a diff, so listen for an
  // explicit `kassiber:settings-section` event and force re-selection.
  React.useEffect(() => {
    const handler = (event: Event) => {
      const detail = (event as CustomEvent<{ section?: string | null }>).detail;
      const next = selectedIntegrationForHash(detail?.section ?? "");
      setSelectedIntegrationId(next);
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
  const encryptedWorkspace =
    Boolean(identity?.encrypted) || identity?.databaseMode === "sqlcipher";

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

  const openAddBackend = () => {
    setEditingBackendId(null);
    setBackendDialogOpen(true);
  };

  const openEditBackend = (backend: Backend) => {
    setEditingBackendId(backend.id);
    setBackendDialogOpen(true);
  };

  const onSaveBackend = async (backend: Backend) => {
    const payload = backendPayload(backend);
    if (editingBackend) {
      await updateBackend.mutateAsync({ ...payload, name: editingBackend.id });
    } else {
      await createBackend.mutateAsync(payload);
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

  const settingsIntegrations = React.useMemo<IntegrationItem[]>(
    () => [
      {
        id: "privacy-sensitive",
        icon: ShieldCheck,
        title: "Sensitive values",
        description: hideSensitive
          ? "Balances, addresses, and amounts are blurred."
          : "Balances, addresses, and amounts are visible.",
        isConnected: hideSensitive,
        statusLabel: "On",
        category: "privacy",
        categoryLabel: "Privacy",
        action: (
          <Switch
            checked={hideSensitive}
            onCheckedChange={setHideSensitive}
            aria-label="Hide sensitive data"
          />
        ),
      },
      {
        id: "privacy-clipboard",
        icon: FileInput,
        title: "Clipboard clearing",
        description: clearClipboard
          ? "Copied addresses and keys are cleared after 30 seconds."
          : "Copied values remain in the system clipboard.",
        isConnected: clearClipboard,
        statusLabel: "On",
        category: "privacy",
        categoryLabel: "Privacy",
        action: (
          <Switch
            checked={clearClipboard}
            onCheckedChange={setClearClipboard}
            aria-label="Clear clipboard after 30 seconds"
          />
        ),
      },
      {
        id: "display-currency",
        icon: Database,
        title: "Display currency",
        description:
          currency === "btc"
            ? "Balances are shown in Bitcoin mode."
            : "Balances are shown in Euro mode.",
        isConnected: true,
        statusLabel: "Selected",
        category: "display",
        categoryLabel: "Display",
        actionLabel: "Configure",
      },
      {
        id: "explorer-links",
        icon: ExternalLink,
        title: "Transaction explorers",
        description: "External links for Bitcoin and Liquid transaction details.",
        isConnected:
          explorerSettings.bitcoinBaseUrl.trim().length > 0 ||
          explorerSettings.liquidBaseUrl.trim().length > 0,
        statusLabel: "Configured",
        category: "explorers",
        categoryLabel: "Explorers",
        actionLabel: "Configure",
      },
      {
        id: "security-lock-now",
        icon: Lock,
        title: "Lock database",
        description: appLockPolicy.autoLockWhenIdle
          ? `Auto-locks after ${appLockPolicy.idleMinutes} minutes of inactivity.`
          : "Auto-lock is disabled for idle sessions.",
        isConnected: appLockPolicy.autoLockWhenIdle,
        statusLabel: "On",
        category: "security",
        categoryLabel: "Security",
        actionLabel: "Configure",
      },
      {
        id: "security-passphrase",
        icon: KeyRound,
        title: "Database passphrase",
        description: encryptedWorkspace
          ? "Change the SQLCipher database passphrase."
          : "These books are not using SQLCipher encryption.",
        isConnected: encryptedWorkspace,
        statusLabel: "Enabled",
        category: "security",
        categoryLabel: "Security",
        actionLabel: "Manage",
      },
      ...backends.filter(isSyncBackend).map((backend) => ({
        id: backend.id,
        ...backendIntegrationArt(backend),
        title: backend.name,
        description: `${backend.net} backend - ${backend.url}`,
        isConnected: backend.on,
        category: "sync",
        categoryLabel: "Wallet sync",
        actionLabel: backend.on ? "Configure" : "Connect",
      })),
      {
        id: "sync-add-backend",
        image: bitcoinIcon,
        className: "size-8",
        title: "Add sync backend",
        description: "Add a Bitcoin or Liquid wallet refresh endpoint.",
        isConnected: false,
        category: "sync",
        categoryLabel: "Wallet sync",
        actionLabel: "Add",
      },
      {
        id: "rate-providers",
        icon: Database,
        title: "Rate providers",
        description: "Reference-rate sources are managed separately from wallet sync.",
        isConnected: backends.some((backend) => backend.net === "FX" && backend.on),
        category: "rates",
        categoryLabel: "Rate providers",
        actionLabel: "Review",
      },
      {
        id: "ai-providers",
        icon: Server,
        title: "AI providers",
        description: aiFeaturesEnabled
          ? "Ollama and OpenAI-compatible assistant endpoints for local review."
          : "Assistant UI is disabled; providers stay configured.",
        isConnected: aiFeaturesEnabled,
        category: "assistant",
        categoryLabel: "Assistant",
        actionLabel: "Manage",
      },
      {
        id: "label-file-imports",
        icon: FileInput,
        title: "Label and file imports",
        description: "BIP-329 labels, CSV imports, backups, and restore tools.",
        isConnected: true,
        statusLabel: "Available",
        category: "data",
        categoryLabel: "Data",
        actionLabel: "Imports",
      },
      {
        id: "data-root",
        icon: Database,
        title: "Local database",
        description: status?.database ?? "Local database path is loading.",
        isConnected: Boolean(status?.database),
        statusLabel: "Available",
        category: "data",
        categoryLabel: "Data",
        actionLabel: "Status",
      },
    ],
    [
      appLockPolicy.autoLockWhenIdle,
      appLockPolicy.idleMinutes,
      aiFeaturesEnabled,
      backends,
      clearClipboard,
      currency,
      encryptedWorkspace,
      hideSensitive,
      explorerSettings.bitcoinBaseUrl,
      explorerSettings.liquidBaseUrl,
      setHideSensitive,
      status?.database,
    ],
  );

  const onIntegrationAction = (integration: IntegrationItem) => {
    if (integration.category === "privacy") {
      return;
    }
    setSelectedIntegrationId(integration.id ?? integration.title);
    const backend = backends.find((item) => item.id === integration.id);
    if (backend && isSyncBackend(backend)) {
      openEditBackend(backend);
      return;
    }
    if (integration.id === "sync-add-backend") {
      openAddBackend();
      return;
    }
    if (integration.id === "ai-providers") {
      return;
    }
    if (integration.id === "label-file-imports") {
      return;
    }
  };

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

  return (
    <>
      <div className={screenPanelClassName}>
        <div className="mx-auto flex w-full max-w-[1500px] min-w-0 flex-col gap-4 lg:gap-6">
          <div className="space-y-1">
            <h1 className="text-2xl font-semibold tracking-tight">Settings</h1>
            <p className="text-sm text-muted-foreground">
              Books preferences, privacy controls, integrations, and local data
              tools.
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

          <div className="flex min-w-0 flex-col gap-4">
            <SettingsIntegrations4
              className="min-w-0"
              heading="Settings"
              subHeading="Controls grouped by privacy, display, explorers, security, sync, assistant, and data."
              integrations={settingsIntegrations}
              selectedId={selectedIntegrationId ?? undefined}
              onSelect={onIntegrationAction}
              renderDetail={(integration) => {
                if (integration.category === "display") {
                  return (
                    <DisplaySettingsPanel
                      currency={currency}
                      setCurrency={setCurrency}
                    />
                  );
                }
                if (integration.category === "explorers") {
                  return (
                    <ExplorerSettingsPanel
                      explorerSettings={explorerSettings}
                      setExplorerSettings={setExplorerSettings}
                    />
                  );
                }
                if (integration.category === "security") {
                  return (
                    <SecuritySettingsPanel
                      appLockPolicy={appLockPolicy}
                      setAppLockPolicy={setAppLockPolicy}
                      encryptedWorkspace={encryptedWorkspace}
                      onLockNow={lockNow}
                      onChangePassphrase={openChangePassphrase}
                    />
                  );
                }
                if (
                  integration.category === "sync" ||
                  integration.category === "rates"
                ) {
                  return (
                    <BackendSettingsPanel
                      backends={backends}
                      onAdd={openAddBackend}
                      onEdit={openEditBackend}
                      onDelete={onDeleteBackend}
                    />
                  );
                }
                if (integration.id === "ai-providers") {
                  return (
                    <AiProvidersPanel
                      aiFeaturesEnabled={aiFeaturesEnabled}
                      setAiFeaturesEnabled={setAiFeaturesEnabled}
                    />
                  );
                }
                if (integration.category === "data") {
                  return <DataSettingsPanel status={status ?? null} />;
                }
                return null;
              }}
            />

            <Card className="min-w-0 border-destructive/30">
              <CardHeader>
                <div className="space-y-1">
                  <CardTitle className="flex items-center gap-2 text-base text-destructive">
                    <Trash2 className="size-4" aria-hidden="true" />
                    Danger zone
                  </CardTitle>
                  <CardDescription>
                    Reset testing data, reset the Welcome gate, or delete the
                    current local books set.
                  </CardDescription>
                </div>
              </CardHeader>
              <CardContent className="grid gap-3">
                <div className="flex flex-col gap-3 rounded-lg border p-4 sm:flex-row sm:items-center sm:justify-between">
                  <div className="min-w-0 space-y-1">
                    <p className="text-sm font-medium">Reset Welcome state</p>
                    <p className="text-sm text-muted-foreground">
                      Clear only the local UI identity and return to onboarding.
                    </p>
                  </div>
                  <Button
                    type="button"
                    variant="outline"
                    className="shrink-0"
                    onClick={onResetWorkspace}
                  >
                    Reset Welcome
                  </Button>
                </div>
                <div className="flex flex-col gap-3 rounded-lg border border-amber-500/30 bg-amber-500/5 p-4 sm:flex-row sm:items-center sm:justify-between">
                  <div className="min-w-0 space-y-1">
                    <p className="text-sm font-medium">Reset book data</p>
                    <p className="text-sm text-muted-foreground">
                      Keep wallet and backend connections, then clear synced
                      transactions, journals, swaps, labels, attachments,
                      and source-funds work. Shared fiat rates are optional.
                    </p>
                  </div>
                  <Button
                    type="button"
                    variant="outline"
                    className="shrink-0"
                    disabled={resetBookData.isPending || !resetBookAvailable}
                    onClick={openResetBookData}
                  >
                    <RefreshCw className="mr-2 size-4" aria-hidden="true" />
                    Reset book
                  </Button>
                </div>
                <div className="flex flex-col gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-4 sm:flex-row sm:items-center sm:justify-between">
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
                    disabled={deleteWorkspace.isPending}
                    onClick={openDeleteWorkspace}
                  >
                    Delete books
                  </Button>
                </div>
              </CardContent>
            </Card>
          </div>
        </div>
      </div>

        <BackendModal
          open={backendDialogOpen}
          initial={editingBackend}
          onClose={() => {
            setBackendDialogOpen(false);
            setEditingBackendId(null);
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

function DisplaySettingsPanel({
  currency,
  setCurrency,
}: {
  currency: CurrencyMode;
  setCurrency: (currency: CurrencyMode) => void;
}) {
  return (
    <section className="space-y-3">
      <div>
        <h3 className="text-sm font-semibold">Display currency</h3>
        <p className="text-sm text-muted-foreground">
          Choose how balances and reports are shown across the app.
        </p>
      </div>
      <div className="grid max-w-md grid-cols-2 gap-1 rounded-md border bg-muted p-1">
        {([
          ["eur", "€", "Euro"],
          ["btc", "₿", "Bitcoin"],
        ] satisfies Array<[CurrencyMode, string, string]>).map(
          ([value, symbol, label]) => {
            const active = currency === value;
            return (
              <button
                key={value}
                type="button"
                aria-pressed={active}
                onClick={() => setCurrency(value)}
                className={cn(
                  "flex h-10 min-w-0 items-center justify-center gap-2 rounded-sm px-3 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                  active
                    ? "bg-background text-foreground shadow-sm"
                    : "text-muted-foreground hover:bg-background/60 hover:text-foreground",
                )}
              >
                <span className="text-base leading-none" aria-hidden="true">
                  {symbol}
                </span>
                <span className="truncate">{label}</span>
              </button>
            );
          },
        )}
      </div>
    </section>
  );
}

function ExplorerSettingsPanel({
  explorerSettings,
  setExplorerSettings,
}: {
  explorerSettings: ExplorerSettings;
  setExplorerSettings: (settings: Partial<ExplorerSettings>) => void;
}) {
  return (
    <section className="space-y-3">
      <div className="space-y-3">
        <div>
          <h3 className="text-sm font-semibold">Transaction explorers</h3>
          <p className="text-sm text-muted-foreground">
            Optional explorer bases used by transaction detail links. Leave
            empty to use the public defaults.
          </p>
        </div>
        <div className="grid gap-3 md:grid-cols-2">
          <div className="space-y-2">
            <Label htmlFor="settings-bitcoin-explorer">Bitcoin explorer URL</Label>
            <Input
              id="settings-bitcoin-explorer"
              value={explorerSettings.bitcoinBaseUrl}
              onChange={(event) =>
                setExplorerSettings({ bitcoinBaseUrl: event.target.value })
              }
              placeholder="https://mempool.bitcoin-austria.at"
              inputMode="url"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="settings-liquid-explorer">Liquid explorer URL</Label>
            <Input
              id="settings-liquid-explorer"
              value={explorerSettings.liquidBaseUrl}
              onChange={(event) =>
                setExplorerSettings({ liquidBaseUrl: event.target.value })
              }
              placeholder="https://liquid.network"
              inputMode="url"
            />
          </div>
        </div>
      </div>
    </section>
  );
}

function SecuritySettingsPanel({
  appLockPolicy,
  setAppLockPolicy,
  encryptedWorkspace,
  onLockNow,
  onChangePassphrase,
}: {
  appLockPolicy: AppLockPolicy;
  setAppLockPolicy: (policy: Partial<AppLockPolicy>) => void;
  encryptedWorkspace: boolean;
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
              ? "Encrypted databases always require unlock on launch."
              : "Prompt every time Kassiber opens."
          }
          checked={
            encryptedWorkspace
              ? true
              : appLockPolicy.requirePassphraseOnLaunch
          }
          onCheckedChange={(checked) =>
            setAppLockPolicy({ requirePassphraseOnLaunch: checked })
          }
          disabled={encryptedWorkspace}
        />
        <SettingsSwitchRow
          label="Lock on window close"
          description="Clear in-memory decrypted state when the app window closes."
          checked={appLockPolicy.lockOnWindowClose}
          onCheckedChange={(checked) =>
            setAppLockPolicy({ lockOnWindowClose: checked })
          }
        />
      </div>
      <div className="space-y-4 rounded-md border border-primary/15 bg-background p-4">
        <div className="space-y-1">
          <h3 className="flex items-center gap-2 text-sm font-semibold">
            <KeyRound className="size-4" aria-hidden="true" />
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

function BackendSettingsPanel({
  backends,
  onAdd,
  onEdit,
  onDelete,
}: {
  backends: Backend[];
  onAdd: () => void;
  onEdit: (backend: Backend) => void;
  onDelete: (backend: Backend) => void;
}) {
  const syncBackends = backends.filter(isSyncBackend);
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
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0 space-y-1">
          <h3 className="flex items-center gap-2 text-sm font-semibold">
            <Server className="size-4" aria-hidden="true" />
            Backend connections
          </h3>
          <p className="text-sm text-muted-foreground">
            Wallet refresh endpoints are configured separately from price-rate
            sources.
          </p>
        </div>
        <Button type="button" size="sm" className="shrink-0" onClick={onAdd}>
          <Plus className="size-4" aria-hidden="true" />
          Add sync backend
        </Button>
      </div>

      <div className="space-y-2">
        <div>
          <p className="text-sm font-medium">Wallet sync</p>
          <p className="text-xs text-muted-foreground">
            Bitcoin and Liquid endpoints used for watch-only wallet refresh.
          </p>
        </div>
        <BackendTable
          backends={syncBackends}
          actions
          onEdit={onEdit}
          onDelete={onDelete}
        />
      </div>

      <div className="space-y-2">
        <div>
          <p className="text-sm font-medium">Rate providers</p>
          <p className="text-xs text-muted-foreground">
            Fiat reference rates stay outside the wallet-sync setup flow.
          </p>
        </div>
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
                target="_blank"
                rel="noreferrer"
                className="inline-flex items-center gap-1 text-primary underline-offset-4 hover:underline"
              >
                Get Kraken archive
                <ExternalLink className="size-3" aria-hidden="true" />
              </a>
              <a
                href={KRAKEN_MARKET_DATA_BLOG_URL}
                target="_blank"
                rel="noreferrer"
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
      <div>
        <h3 className="flex items-center gap-2 text-sm font-semibold">
          <Database className="size-4" aria-hidden="true" />
          Data tools
        </h3>
        <p className="text-sm text-muted-foreground">
          Backup, restore, labels, imports, and local database status.
        </p>
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

function NetworkMark({ net }: { net: Net }) {
  const image = net === "LIQUID" ? liquidIcon : net === "BTC" ? bitcoinIcon : null;
  if (image) {
    return (
      <span
        className={cn(
          "flex size-9 shrink-0 items-center justify-center rounded-md border p-1.5",
          brandLogoFrame,
        )}
        aria-hidden="true"
      >
        <img
          src={image}
          alt=""
          className={cn(
            "size-6 object-contain",
            net === "LIQUID" && "scale-150",
          )}
        />
      </span>
    );
  }
  return <NetworkBadge net={net} />;
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
          brandLogoFrame,
        )}
        aria-hidden="true"
      >
        <img
          src={image}
          alt=""
          className={cn(
            "size-5 object-contain",
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
  protocol: "esplora" | "electrum" | "bitcoinrpc" | "liquid-esplora";
  label: string;
  disabled?: boolean;
  status?: string;
}

interface SyncBackendNetwork {
  id: "bitcoin" | "liquid";
  label: string;
  net: Net;
  desc: string;
  presets: SyncBackendPreset[];
}

const SYNC_BACKEND_NETWORKS: SyncBackendNetwork[] = [
  {
    id: "bitcoin",
    label: "Bitcoin",
    net: "BTC",
    desc: "Backends used by Bitcoin watch-only wallets.",
    presets: [
      {
        id: "mempool",
        name: DEFAULT_BACKEND_NAME,
        url: DEFAULT_BACKEND_URL,
        protocol: "esplora",
        label: "Esplora",
      },
      {
        id: "electrum",
        name: "Bitcoin Austria Fulcrum",
        url: "ssl://index.bitcoin-austria.at:50002",
        protocol: "electrum",
        label: "Electrum / Fulcrum",
      },
      {
        id: "core",
        name: "Bitcoin Core RPC",
        url: "http://127.0.0.1:8332",
        protocol: "bitcoinrpc",
        label: "Bitcoin Core RPC",
      },
    ],
  },
  {
    id: "liquid",
    label: "Liquid",
    net: "LIQUID",
    desc: "Backends used by Liquid watch-only wallets.",
    presets: [
      {
        id: "liquid-electrum",
        name: "Liquid Electrum",
        url: "ssl://les.bullbitcoin.com:995",
        protocol: "electrum",
        label: "Electrum / Fulcrum",
      },
      {
        id: "blockstream",
        name: "Blockstream Liquid",
        url: "https://blockstream.info/liquid/api",
        protocol: "liquid-esplora",
        label: "Liquid Esplora",
      },
    ],
  },
];

const AUTH_MODES: Array<{ id: string; label: string }> = [
  { id: "none", label: "None" },
  { id: "apikey", label: "API key" },
  { id: "basic", label: "User + pass" },
  { id: "bearer", label: "Bearer token" },
];

type TestState = "idle" | "testing" | "ok" | "fail";

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

interface BackendModalProps {
  open: boolean;
  initial: Backend | null;
  onClose: () => void;
  onSave: (backend: Backend) => void | Promise<void>;
}

function BackendModal({
  open,
  initial,
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
  const [certificate, setCertificate] = React.useState("");
  const [useProxy, setUseProxy] = React.useState(false);
  const [proxyHost, setProxyHost] = React.useState("");
  const [proxyPort, setProxyPort] = React.useState("");
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
  const isElectrum = preset?.protocol === "electrum";
  const showAuth = preset?.protocol === "bitcoinrpc";
  const effectiveUrl = isElectrum
    ? buildElectrumUrl({
        host: electrumHost,
        port: electrumPort,
        useSsl: electrumUseSsl,
      })
    : url.trim();
  const selectedBackendKind =
    preset?.protocol ??
    initial?.kind ??
    (type.net === "LIQUID" ? "liquid-esplora" : "esplora");

  React.useEffect(() => {
    if (!open) return;
    if (initial) {
      const parsedElectrum = parseElectrumEndpoint(initial.url);
      const initialType =
        SYNC_BACKEND_NETWORKS.find((candidate) => candidate.net === initial.net) ??
        SYNC_BACKEND_NETWORKS[0];
      const initialPreset =
        initialType.presets.find((candidate) => candidate.url === initial.url) ??
        (initial.url.match(/^(ssl|tcp):\/\//i)
          ? initialType.presets.find((candidate) => candidate.protocol === "electrum")
          : null);
      setTypeId(initialType.id);
      setPresetId(initialPreset?.id ?? "custom");
      setName(initial.name);
      setUrl(initial.url);
      setAuth(initial.auth);
      setAuthVal("");
      setAuthVal2("");
      setElectrumHost(parsedElectrum.host);
      setElectrumPort(parsedElectrum.port);
      setElectrumUseSsl(parsedElectrum.useSsl);
      setTrustSsl(Boolean(initial.trustSsl));
      setCertificate(initial.certificate ?? "");
      setUseProxy(Boolean(initial.proxy));
      setProxyHost(initial.proxy?.host ?? "");
      setProxyPort(initial.proxy?.port ?? "");
      setTestState(initial.on ? "ok" : "idle");
      setTestLog("");
      setSaveState("idle");
      return;
    }

    setTypeId("bitcoin");
    setPresetId("mempool");
    setName(DEFAULT_BACKEND_NAME);
    setUrl(DEFAULT_BACKEND_URL);
    setAuth("none");
    setAuthVal("");
    setAuthVal2("");
    setElectrumHost("index.bitcoin-austria.at");
    setElectrumPort("50002");
    setElectrumUseSsl(true);
    setTrustSsl(false);
    setCertificate("");
    setUseProxy(false);
    setProxyHost("");
    setProxyPort("");
    setTestState("idle");
    setTestLog("");
    setSaveState("idle");
  }, [initial, open]);

  React.useEffect(() => {
    if (!open) return;
    if (initial) return;
    if (preset) {
      setUrl(preset.url);
      setName(preset.name);
      if (preset.protocol === "electrum") {
        const parsed = parseElectrumEndpoint(preset.url);
        setElectrumHost(parsed.host);
        setElectrumPort(parsed.port);
        setElectrumUseSsl(parsed.useSsl);
      }
    } else if (presetId === "custom") {
      setUrl("");
      setName("");
    }
    setTestState("idle");
    setTestLog("");
  }, [initial, open, preset, presetId]);

  const onPickType = (id: SyncBackendNetwork["id"]) => {
    setTypeId(id);
    setTestLog("");
    if (initial) {
      setPresetId("custom");
      return;
    }
    const nextType = SYNC_BACKEND_NETWORKS.find((candidate) => candidate.id === id);
    setPresetId(
      nextType?.presets.find((candidate) => !candidate.disabled)?.id ??
        "custom",
    );
  };

  const testConnection = async () => {
    if (!effectiveUrl) return false;
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

  const canAdd = name.trim().length > 0 && effectiveUrl.length > 0;
  const save = async () => {
    if (!canAdd) return;
    const normalizedUrl = effectiveUrl;
    let connected = testState === "ok";
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
        chain: type.net === "LIQUID" ? "liquid" : "bitcoin",
        network: type.net === "LIQUID" ? "liquidv1" : "main",
        health: initial ? "just checked - ok" : "just added - ok",
        on: connected,
        auth: showAuth ? auth : "none",
        authHeader:
          showAuth && auth === "bearer" && authSecret
            ? `Bearer ${authSecret}`
            : undefined,
        token:
          showAuth && auth === "apikey" && authSecret ? authSecret : undefined,
        username:
          showAuth && auth === "basic" && authSecret ? authSecret : undefined,
        password:
          showAuth && auth === "basic" && authPassword
            ? authPassword
            : undefined,
        trustSsl: isElectrum && electrumUseSsl ? trustSsl : undefined,
        certificate:
          isElectrum && electrumUseSsl && !trustSsl && certificate.trim()
            ? certificate.trim()
            : undefined,
        proxy:
          isElectrum && useProxy && proxyHost.trim() && proxyPort.trim()
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
      <DialogContent className="max-h-[88vh] w-full max-w-[760px] overflow-hidden p-0 sm:max-w-[760px]">
        <DialogHeader className="border-b px-6 py-5">
          <DialogTitle>
            {isEditing ? "Edit sync backend" : "Add sync backend"}
          </DialogTitle>
          <DialogDescription>
            {isEditing
              ? "Update this wallet-refresh endpoint."
              : "Connect a Bitcoin or Liquid wallet-refresh backend."}
          </DialogDescription>
        </DialogHeader>

        <ScrollArea className="max-h-[calc(88vh-150px)]">
          <div className="space-y-5 p-6">
            <section className="space-y-3">
              <div>
                <Label>Network</Label>
                <p className="text-sm text-muted-foreground">{type.desc}</p>
              </div>
              <div className="grid gap-2 sm:grid-cols-2">
                {SYNC_BACKEND_NETWORKS.map((backendType) => {
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
                      <NetworkMark net={backendType.net} />
                      <span className="min-w-0 space-y-0.5">
                        <span className="block text-sm leading-tight font-medium">
                          {backendType.label}
                        </span>
                        <span className="block text-xs leading-tight text-muted-foreground">
                          {backendType.net === "BTC" ? "Bitcoin" : "Liquid"}
                        </span>
                      </span>
                    </Button>
                  );
                })}
              </div>
            </section>

            {!isEditing && type.presets.length > 0 && (
              <section className="space-y-3">
                <div>
                  <Label>Protocol</Label>
                  <p className="text-xs text-muted-foreground">
                    Pick the transport first, then edit the endpoint fields.
                  </p>
                </div>
                <div className="grid gap-2 sm:grid-cols-2">
                  {type.presets.map((backendPreset) => {
                    const active = presetId === backendPreset.id;
                    return (
                      <Button
                        key={backendPreset.id}
                        type="button"
                        variant="outline"
                        className={cn(
                          "h-auto min-h-12 justify-start gap-2 whitespace-normal px-3 py-2 text-left",
                          selectorButtonClass(active),
                        )}
                        disabled={backendPreset.disabled}
                        onClick={() => setPresetId(backendPreset.id)}
                      >
                        <PresetMark preset={backendPreset} net={type.net} />
                        <span className="min-w-0 space-y-0.5">
                          <span className="block truncate text-sm leading-tight font-medium">
                            {backendPreset.name}
                          </span>
                          <span className="block truncate text-xs leading-tight text-muted-foreground">
                            {backendPreset.status ?? backendPreset.label}
                          </span>
                        </span>
                      </Button>
                    );
                  })}
                </div>
              </section>
            )}

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
              {isElectrum ? (
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
                      placeholder="index.bitcoin-austria.at"
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
                  />
                </div>
              )}
            </section>

            {isElectrum && (
              <section className="grid gap-3 sm:grid-cols-2">
                <label className="flex items-center justify-between gap-3 rounded-md border p-3 text-sm">
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
                <label className="flex items-center justify-between gap-3 rounded-md border p-3 text-sm">
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
                <label className="flex items-center justify-between gap-3 rounded-md border p-3 text-sm sm:col-span-2">
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
                    label="API key"
                    value={authVal}
                    onChange={setAuthVal}
                    placeholder="sk_live_..."
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
