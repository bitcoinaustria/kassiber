import * as React from "react";
import { Trans, useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { useNavigate } from "@tanstack/react-router";
import { Loader2, ScanLine } from "lucide-react";

import { Badge } from "@/components/ui/badge";
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
import { Textarea } from "@/components/ui/textarea";
import {
  useDaemon,
  useDaemonMutation,
  useDaemonStreamMutation,
} from "@/daemon/client";
import { useSyncProgressNotice } from "@/hooks/useSyncProgressNotice";
import { copyTextWithPolicy } from "@/lib/clipboard";
import { useUiStore, type DeferredConnectionSetup } from "@/store/ui";
import { cn } from "@/lib/utils";
import {
  CONNECTION_CATEGORIES,
  CONNECTION_SOURCES,
  type ConnectionCategory,
  type ConnectionSource,
  type ConnectionSourceFormat,
} from "@/lib/connectionCatalog";
import { isFilePickerAvailable, pickFile } from "@/lib/filePicker";
import {
  buildSamouraiSourceSet,
  type SamouraiSection,
} from "@/lib/samouraiSourceSet";
import {
  buildWasabiBundle,
  type WasabiImportMode,
} from "@/lib/wasabiBundle";
import { detectWalletMaterial } from "@/lib/walletMaterialFormat";

const WalletMaterialScannerDialog = React.lazy(() =>
  import("./WalletMaterialScannerDialog").then((module) => ({
    default: module.WalletMaterialScannerDialog,
  })),
);

interface AddConnectionDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initialSourceId?: string | null;
}

interface SetupFormState {
  label: string;
  backend: string;
  btcpayInstanceMode: "saved" | "new";
  btcpaySetupMode: "wallet_sources" | "existing_wallets";
  bullWalletSetupMode: "wallet_sources" | "existing_wallets";
  bullWalletNetworks: BullBitcoinWalletNetwork[];
  bullWalletRouteWallets: Record<BullBitcoinWalletNetwork, string>;
  btcpayInstanceLabel: string;
  btcpayServerUrl: string;
  btcpayApiKey: string;
  walletMaterial: string;
  gapLimit: string;
  targetWallet: string;
  sourceFile: string;
  samouraiDeposit: string;
  samouraiBadbank: string;
  samouraiPremix: string;
  samouraiPostmix: string;
  wasabiImportMode: WasabiImportMode;
  wasabiHistory: string;
  wasabiCoins: string;
  wasabiWalletInfo: string;
  wasabiAdditional: string;
  sourceFormat: ConnectionSourceFormat;
  bullImportMode: "relevant" | "full";
  btcpayStoreId: string;
  btcpayPaymentMethodId: string;
  btcpayPaymentMethodIds: string[];
  btcpayRouteWallets: Record<string, string>;
  bip329Wallet: string;
  bip329File: string;
  syncAfterCreate: boolean;
}

interface SyncResult {
  wallet: string;
  status: string;
  message?: string;
  source?: string;
  imported?: number;
  skipped?: number;
  matched?: number;
  unchanged?: number;
  skipped_unmatched?: number;
  skipped_ambiguous?: number;
  unmatched?: number;
  ambiguous?: number;
  excluded?: number;
  updated?: number;
  bullbitcoin_rows?: number;
  bullbitcoin_wallet_rows?: number;
  coinfinity_rows?: number;
  twentyonebitcoin_rows?: number;
  strike_rows?: number;
  wasabi_transactions?: number;
  wasabi_coins_observed?: number;
  wasabi_coins_active?: number;
  wasabi_coins_marked_spent?: number;
  wasabi_payments_in_coinjoin?: number;
  inserted_records?: ImportChangeRecord[];
  updated_records?: ImportChangeRecord[];
  reconciliation_records?: ImportChangeRecord[];
}

interface BackendOption {
  name: string;
  display_name?: string;
  kind: string;
  chain?: string;
  network?: string;
  is_default?: boolean;
}

interface BackendOptionsData {
  backends: BackendOption[];
  summary?: {
    default_backend?: string | null;
  };
  suggestions?: Array<{
    name: string;
    label: string;
    chain: string;
    network: string;
  }>;
}

function backendOptionLabel(backend: BackendOption): string {
  const label = backend.display_name?.trim() || backend.name;
  return label === backend.name ? label : `${label} (${backend.name})`;
}

interface BtcpayDiscoveryData {
  backend: string;
  stores: Array<{
    id: string;
    name: string;
    default_currency?: string | null;
  }>;
  payment_methods: Array<{
    store_id: string;
    payment_method_id: string;
    label: string;
    enabled: boolean;
    sync_supported: boolean;
  }>;
}

interface WalletListData {
  wallets: Array<{
    label: string;
    kind: string;
    chain: string;
    sync_mode: string;
    sync_source: string;
    transaction_count: number;
  }>;
}

interface ImportChangeRecord {
  transaction_id: string;
  external_id?: string | null;
  wallet?: string | null;
  asset?: string | null;
  direction?: string | null;
  amount_msat?: number | null;
  changed_fields?: string[];
  pricing_external_ref?: string | null;
  status?: "matched" | "unmatched" | "ambiguous";
  matched_wallet?: string | null;
}

interface ImportFileResult {
  wallet?: string;
  scope?: string;
  source?: string;
  imported: number;
  skipped: number;
  matched?: number;
  unchanged?: number;
  skipped_unmatched?: number;
  skipped_ambiguous?: number;
  unmatched?: number;
  ambiguous?: number;
  excluded?: number;
  updated?: number;
  bullbitcoin_rows?: number;
  bullbitcoin_wallet_rows?: number;
  coinfinity_rows?: number;
  twentyonebitcoin_rows?: number;
  strike_rows?: number;
  wasabi_transactions?: number;
  wasabi_coins_observed?: number;
  wasabi_coins_active?: number;
  wasabi_coins_marked_spent?: number;
  wasabi_payments_in_coinjoin?: number;
  inserted_records?: ImportChangeRecord[];
  updated_records?: ImportChangeRecord[];
  reconciliation_records?: ImportChangeRecord[];
}

interface SamouraiImportResult {
  group: { label: string };
  children: Array<{ label: string }>;
  warnings?: Array<{ code: string; message: string }>;
}

type DialogStep = "source" | "setup";
const DESCRIPTOR_BACKEND_KINDS = new Set(["esplora", "electrum"]);
const DEFAULT_BTCPAY_PAYMENT_METHOD_ID = "BTC-CHAIN";
const MAX_DESCRIPTOR_GAP_LIMIT = 5000;
type BullBitcoinWalletNetwork = "bitcoin" | "liquid" | "lightning";
const BULLBITCOIN_WALLET_NETWORKS: Array<{
  id: BullBitcoinWalletNetwork;
  labelKey: string;
  helperKey: string;
}> = [
  {
    id: "bitcoin",
    labelKey: "add.bullWallet.networkBitcoin",
    helperKey: "add.bullWallet.networkBitcoinHelper",
  },
  {
    id: "liquid",
    labelKey: "add.bullWallet.networkLiquid",
    helperKey: "add.bullWallet.networkLiquidHelper",
  },
  {
    id: "lightning",
    labelKey: "add.bullWallet.networkLightning",
    helperKey: "add.bullWallet.networkLightningHelper",
  },
];
type SamouraiFormKey =
  | "samouraiDeposit"
  | "samouraiBadbank"
  | "samouraiPremix"
  | "samouraiPostmix";

const SAMOURAI_SOURCE_FIELDS: Array<{
  section: SamouraiSection;
  key: SamouraiFormKey;
  id: string;
  labelKey: string;
  helperKey: string;
}> = [
  {
    section: "deposit",
    key: "samouraiDeposit",
    id: "connection-samourai-deposit",
    labelKey: "add.samourai.depositLabel",
    helperKey: "add.samourai.depositHelper",
  },
  {
    section: "badbank",
    key: "samouraiBadbank",
    id: "connection-samourai-badbank",
    labelKey: "add.samourai.badbankLabel",
    helperKey: "add.samourai.badbankHelper",
  },
  {
    section: "premix",
    key: "samouraiPremix",
    id: "connection-samourai-premix",
    labelKey: "add.samourai.premixLabel",
    helperKey: "add.samourai.premixHelper",
  },
  {
    section: "postmix",
    key: "samouraiPostmix",
    id: "connection-samourai-postmix",
    labelKey: "add.samourai.postmixLabel",
    helperKey: "add.samourai.postmixHelper",
  },
];

function supportsDescriptorSync(backend: BackendOption) {
  return DESCRIPTOR_BACKEND_KINDS.has(backend.kind);
}

function isExchangeEvidenceFormat(sourceFormat?: string) {
  return (
    sourceFormat === "bullbitcoin_csv" ||
    sourceFormat === "coinfinity_csv" ||
    sourceFormat === "21bitcoin_csv"
  );
}

function samouraiFieldKey(section: SamouraiSection) {
  return SAMOURAI_SOURCE_FIELDS.find((field) => field.section === section)
    ?.key;
}

function samouraiSourceFields(form: SetupFormState) {
  return {
    deposit: form.samouraiDeposit,
    badbank: form.samouraiBadbank,
    premix: form.samouraiPremix,
    postmix: form.samouraiPostmix,
  };
}

function fileWalletSourceField(
  source: ConnectionSource,
  t: TFunction<"connections">,
) {
  if (source.sourceFormat === "wasabi_bundle") {
    return {
      label: t("add.exportFile.wasabiLabel"),
      helper: t("add.exportFile.wasabiHelper"),
    };
  }
  return {
    label: t("add.exportFile.label"),
    helper: undefined,
  };
}

function sourceFileFilters(
  source: ConnectionSource,
  t: TFunction<"connections">,
) {
  if (source.sourceFormat === "phoenix_csv") {
    return [{ name: t("add.fileFilter.phoenixCsv"), extensions: ["csv"] }];
  }
  if (source.sourceFormat === "river_csv") {
    return [{ name: t("add.fileFilter.riverCsv"), extensions: ["csv"] }];
  }
  if (source.sourceFormat === "bullbitcoin_csv") {
    return [{ name: t("add.fileFilter.bullbitcoinCsv"), extensions: ["csv"] }];
  }
  if (source.sourceFormat === "bullbitcoin_wallet_csv") {
    return [
      { name: t("add.fileFilter.bullbitcoinWalletCsv"), extensions: ["csv"] },
    ];
  }
  if (source.sourceFormat === "coinfinity_csv") {
    return [{ name: t("add.fileFilter.coinfinityCsv"), extensions: ["csv"] }];
  }
  if (source.sourceFormat === "21bitcoin_csv") {
    return [
      { name: t("add.fileFilter.twentyonebitcoinCsv"), extensions: ["csv"] },
    ];
  }
  if (source.sourceFormat === "strike_csv") {
    return [{ name: t("add.fileFilter.strikeCsv"), extensions: ["csv"] }];
  }
  if (source.sourceFormat === "wasabi_bundle") {
    return [{ name: t("add.fileFilter.wasabiBundle"), extensions: ["json"] }];
  }
  if (source.setupKind === "samourai") {
    return [
      { name: t("add.fileFilter.samouraiSourceSet"), extensions: ["json"] },
    ];
  }
  if (source.id === "csv") {
    return [{ name: t("add.fileFilter.csvOrJson"), extensions: ["csv", "json"] }];
  }
  return undefined;
}

const formDefaultsFor = (
  source: ConnectionSource,
  t: TFunction<"connections">,
): SetupFormState => {
  const defaultLabel =
    source.id === "csv"
      ? t("add.defaultLabel.importedFile")
      : source.id === "bip329"
        ? ""
        : source.title;
  return {
    label: defaultLabel,
    backend: "",
    btcpayInstanceMode: "new",
    btcpaySetupMode: "wallet_sources",
    bullWalletSetupMode: "wallet_sources",
    bullWalletNetworks: ["bitcoin", "liquid", "lightning"],
    bullWalletRouteWallets: {
      bitcoin: "",
      liquid: "",
      lightning: "",
    },
    btcpayInstanceLabel: "btcpay",
    btcpayServerUrl: "",
    btcpayApiKey: "",
    walletMaterial: "",
    gapLimit: "40",
    targetWallet: "",
    sourceFile: "",
    samouraiDeposit: "",
    samouraiBadbank: "",
    samouraiPremix: "",
    samouraiPostmix: "",
    wasabiImportMode: source.sourceFormat === "wasabi_bundle" ? "rpc" : "bundle-file",
    wasabiHistory: "",
    wasabiCoins: "",
    wasabiWalletInfo: "",
    wasabiAdditional: "",
    sourceFormat: "csv",
    bullImportMode: "relevant",
    btcpayStoreId: "",
    btcpayPaymentMethodId: DEFAULT_BTCPAY_PAYMENT_METHOD_ID,
    btcpayPaymentMethodIds: [DEFAULT_BTCPAY_PAYMENT_METHOD_ID],
    btcpayRouteWallets: {},
    bip329Wallet: "",
    bip329File: "",
    syncAfterCreate:
      source.setupKind === "file-wallet" ||
      source.setupKind === "bullbitcoin-wallet",
  };
};

function SetupField({
  id,
  label,
  children,
  error,
  helper,
}: {
  id: string;
  label: string;
  children: React.ReactNode;
  error?: string;
  helper?: React.ReactNode;
}) {
  return (
    <div className="space-y-2">
      <Label htmlFor={id}>{label}</Label>
      {children}
      {helper && !error ? renderSetupHelper(helper) : null}
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </div>
  );
}

function renderSetupHelper(helper: React.ReactNode) {
  if (typeof helper === "string") {
    return <p className="text-xs text-muted-foreground">{helper}</p>;
  }
  return <div className="text-xs text-muted-foreground">{helper}</div>;
}

function InlineCode({ children }: { children?: React.ReactNode }) {
  return (
    <code className="rounded bg-muted px-1 py-0.5 font-mono text-[11px] text-foreground">
      {children}
    </code>
  );
}

function CommandSnippet({ children }: { children: React.ReactNode }) {
  return (
    <code className="block overflow-x-auto rounded border bg-background px-2 py-1.5 font-mono text-[11px] leading-relaxed text-foreground">
      {children}
    </code>
  );
}

function SourceArtwork({
  source,
  className,
}: {
  source: ConnectionSource;
  className?: string;
}) {
  return (
    <span
      className={cn(
        "flex shrink-0 items-center justify-center rounded-lg border bg-background p-1.5",
        className ?? "size-12",
        source.imageFrameClassName,
      )}
      aria-hidden="true"
    >
      {source.image ? (
        <img
          src={source.image}
          alt=""
          className={cn(
            "max-h-full max-w-full object-contain",
            source.imageClassName,
          )}
        />
      ) : source.icon ? (
        <source.icon className="size-6 text-muted-foreground" />
      ) : null}
    </span>
  );
}

export function AddConnectionDialog({
  open,
  onOpenChange,
  initialSourceId,
}: AddConnectionDialogProps) {
  const { t } = useTranslation("connections");
  const navigate = useNavigate();
  const addNotification = useUiStore((state) => state.addNotification);
  const setDeferredConnectionSetup = useUiStore(
    (state) => state.setDeferredConnectionSetup,
  );
  const backendOptions = useDaemon<BackendOptionsData>("ui.backends.options");
  const walletsList = useDaemon<WalletListData>("ui.wallets.list");
  const createWallet =
    useDaemonMutation<{ wallet: { label: string } }>("ui.wallets.create");
  const importFile =
    useDaemonMutation<ImportFileResult>("ui.wallets.import_file");
  const importSamourai =
    useDaemonMutation<SamouraiImportResult>("ui.wallets.import_samourai");
  const createBtcpay = useDaemonMutation<{
    backend: { name: string };
    wallet: { label: string };
    wallets?: Array<{ label: string }>;
  }>("ui.connections.btcpay.create");
  const createBullBitcoinWallet = useDaemonMutation<{
    mode: "wallet_sources" | "existing_wallets";
    wallet: { label: string };
    wallets?: Array<{ label: string }>;
    routes?: Array<{ wallet: string; network: BullBitcoinWalletNetwork }>;
  }>("ui.connections.bullbitcoin_wallet.create");
  const discoverBtcpay = useDaemonMutation<BtcpayDiscoveryData>(
    "ui.connections.btcpay.discover",
  );
  const importBip329 = useDaemonMutation<{
    records: number;
    transaction_tags_added: number;
  }>("ui.metadata.bip329.import");
  const previewDescriptor = useDaemonMutation<{
    chain: string;
    network: string;
    addresses: {
      branch: "receive" | "change";
      index: number;
      address: string;
      derivation_path?: string | null;
    }[];
    has_change_branch: boolean;
  }>("ui.wallets.preview_descriptor");
  const testBtcpay = useDaemonMutation<{
    backend: string;
    store_id: string;
    payment_method_id: string;
    ok: boolean;
  }>("ui.connections.btcpay.test");
  const [syncProgress, setSyncProgress] = React.useState<{
    wallet: string;
    processed: number;
    total: number;
    imported: number;
    skipped: number;
  } | null>(null);
  const syncWallet = useDaemonStreamMutation<
    { results: SyncResult[] },
    {
      phase: string;
      wallet: string;
      processed: number;
      total: number;
      imported?: number;
      skipped?: number;
    }
  >("ui.wallets.sync", {
    onProgress: (record) => {
      setSyncProgress({
        wallet: record.wallet,
        processed: record.processed ?? 0,
        total: record.total ?? 0,
        imported: record.imported ?? 0,
        skipped: record.skipped ?? 0,
      });
    },
  });
  const { startSyncNotice, clearSyncNotice } = useSyncProgressNotice();
  const [activeCategory, setActiveCategory] =
    React.useState<ConnectionCategory>("wallets");
  const [selectedId, setSelectedId] = React.useState("xpub");
  const [sourceQuery, setSourceQuery] = React.useState("");
  const [step, setStep] = React.useState<DialogStep>("source");
  const [form, setForm] = React.useState(() =>
    formDefaultsFor(CONNECTION_SOURCES[0], t),
  );
  const [setupError, setSetupError] = React.useState<string | null>(null);
  const [lastImportResult, setLastImportResult] =
    React.useState<ImportFileResult | null>(null);
  const [fieldErrors, setFieldErrors] = React.useState<
    Partial<Record<keyof SetupFormState, string>>
  >({});
  const [previewAddresses, setPreviewAddresses] = React.useState<
    { branch: "receive" | "change"; index: number; address: string }[] | null
  >(null);
  const [previewError, setPreviewError] = React.useState<string | null>(null);
  const [scannerOpen, setScannerOpen] = React.useState(false);
  const [btcpayTestStatus, setBtcpayTestStatus] = React.useState<
    | { ok: true; storeId: string; paymentMethodId: string }
    | { ok: false; message: string }
    | null
  >(null);
  const [btcpayDiscovery, setBtcpayDiscovery] =
    React.useState<BtcpayDiscoveryData | null>(null);
  const [copiedAddress, setCopiedAddress] = React.useState<string | null>(null);
  const copyAddress = React.useCallback(async (address: string) => {
    try {
      await copyTextWithPolicy(address);
      setCopiedAddress(address);
      window.setTimeout(
        () =>
          setCopiedAddress((current) => (current === address ? null : current)),
        1500,
      );
    } catch {
      // Clipboard may be denied; silently ignore — the address is still on screen.
    }
  }, []);

  const visibleSources = React.useMemo(() => {
    const query = sourceQuery.trim().toLowerCase();
    const isSearching = query.length > 0;
    return CONNECTION_SOURCES.filter((source) => {
      if (isSearching) {
        const haystack = `${source.title} ${source.description} ${source.id}`.toLowerCase();
        return haystack.includes(query);
      }
      return source.category === activeCategory;
    });
  }, [activeCategory, sourceQuery]);
  const selected =
    CONNECTION_SOURCES.find((source) => source.id === selectedId) ??
    CONNECTION_SOURCES[0];
  const setupKind = selected.setupKind ?? "planned";
  const isSetupStep = step === "setup";
  const allBackends = backendOptions.data?.data?.backends ?? [];
  const bitcoinBackends = allBackends.filter(
    (backend) =>
      supportsDescriptorSync(backend) &&
      (!backend.chain || backend.chain === "bitcoin"),
  );
  const liquidBackends = allBackends.filter(
    (backend) =>
      supportsDescriptorSync(backend) &&
      backend.chain === "liquid",
  );
  const btcpayBackends = allBackends.filter(
    (backend) => backend.kind === "btcpay",
  );
  const descriptorBackendOptions =
    selected.chain === "liquid" ? liquidBackends : bitcoinBackends;
  const selectedBackendOptions = descriptorBackendOptions;
  const defaultBackendName =
    selectedBackendOptions.find((backend) => backend.is_default)?.name ??
    selectedBackendOptions[0]?.name ??
    "";
  const defaultBtcpayBackendName =
    btcpayBackends.find((backend) => backend.is_default)?.name ??
    btcpayBackends[0]?.name ??
    "";
  const discoveredStoreOptions = btcpayDiscovery?.stores ?? [];
  const discoveredPaymentMethodOptions = (
    btcpayDiscovery?.payment_methods ?? []
  ).filter(
    (method) => !form.btcpayStoreId || method.store_id === form.btcpayStoreId,
  );
  const syncableDiscoveredPaymentMethodOptions =
    discoveredPaymentMethodOptions.filter((method) => method.sync_supported);
  const discoveredPaymentMethodIds = new Set(
    syncableDiscoveredPaymentMethodOptions.map(
      (method) => method.payment_method_id,
    ),
  );
  const selectedBtcpayPaymentMethodIds =
    syncableDiscoveredPaymentMethodOptions.length > 0
      ? form.btcpayPaymentMethodIds.filter((id) =>
          discoveredPaymentMethodIds.has(id),
        )
      : btcpayDiscovery || form.btcpaySetupMode === "existing_wallets"
        ? []
        : [
            form.btcpayPaymentMethodId.trim() ||
              DEFAULT_BTCPAY_PAYMENT_METHOD_ID,
          ];
  const existingWalletOptions = (walletsList.data?.data?.wallets ?? []).filter(
    (wallet) => wallet.sync_source !== "btcpay",
  );
  const walletForPaymentMethod = React.useCallback(
    (paymentMethodId: string) => {
      // Only auto-select when the chain matches; otherwise leave it blank so
      // the form-validation gate forces the user to pick deliberately rather
      // than silently routing an LBTC method into a BTC wallet (or vice versa).
      const normalized = paymentMethodId.toUpperCase();
      const desiredChain = normalized.startsWith("LBTC") ? "liquid" : "bitcoin";
      return (
        existingWalletOptions.find((wallet) => wallet.chain === desiredChain)
          ?.label ?? ""
      );
    },
    [existingWalletOptions],
  );
  const walletForBullNetwork = React.useCallback(
    (network: BullBitcoinWalletNetwork) => {
      if (network === "lightning") {
        return (
          existingWalletOptions.find((wallet) =>
            ["phoenix", "lnd", "coreln", "nwc"].includes(wallet.kind),
          )?.label ?? ""
        );
      }
      return (
        existingWalletOptions.find(
          (wallet) => wallet.chain === network && wallet.kind !== "bullbitcoin",
        )?.label ??
        existingWalletOptions.find((wallet) => wallet.chain === network)
          ?.label ??
        ""
      );
    },
    [existingWalletOptions],
  );
  const selectedBtcpayRoutes = selectedBtcpayPaymentMethodIds.map((id) => ({
    paymentMethodId: id,
    wallet: form.btcpayRouteWallets[id] || walletForPaymentMethod(id),
  }));
  const selectedBullWalletRoutes = form.bullWalletNetworks.map((network) => ({
    network,
    wallet:
      form.bullWalletRouteWallets[network] || walletForBullNetwork(network),
  }));
  const missingBtcpayMappingDiscovery =
    setupKind === "btcpay" &&
    form.btcpaySetupMode === "existing_wallets" &&
    (!btcpayDiscovery || syncableDiscoveredPaymentMethodOptions.length === 0);
  const isSubmitting =
    createWallet.isPending ||
    importFile.isPending ||
    importSamourai.isPending ||
    createBtcpay.isPending ||
    createBullBitcoinWallet.isPending ||
    discoverBtcpay.isPending ||
    importBip329.isPending ||
    syncWallet.isPending;
  const requiresBackend = setupKind === "descriptor" || setupKind === "samourai";
  const missingBackend = requiresBackend && selectedBackendOptions.length === 0;
  const submitLabel =
    setupKind === "backend-settings"
      ? t("add.submit.openBackendSettings")
      : syncWallet.isPending
        ? t("add.submit.refreshing")
        : importSamourai.isPending
          ? t("add.submit.importingSamourai")
        : importFile.isPending
          ? t("add.submit.importing")
        : importBip329.isPending
          ? t("add.submit.importingLabels")
          : isSubmitting
            ? t("add.submit.saving")
            : setupKind === "btcpay" &&
                form.btcpaySetupMode === "existing_wallets"
              ? t("add.submit.saveWalletMapping")
            : setupKind === "btcpay" &&
                selectedBtcpayPaymentMethodIds.length > 1
              ? t("add.submit.createConnections")
              : setupKind === "bullbitcoin-wallet" &&
                  form.bullWalletSetupMode === "existing_wallets"
                ? t("add.submit.saveWalletMapping")
              : setupKind === "bullbitcoin-wallet" &&
                  form.bullWalletNetworks.length > 1
                ? t("add.submit.createConnections")
              : setupKind === "file-enrichment"
                ? t("add.submit.importPricing")
              : t("add.submit.createConnection");
  const canContinue = selected.status === "ready" && setupKind !== "planned";

  React.useEffect(() => {
    setForm(formDefaultsFor(selected, t));
    setSetupError(null);
    setFieldErrors({});
    setLastImportResult(null);
    setPreviewAddresses(null);
    setPreviewError(null);
    setBtcpayTestStatus(null);
    setBtcpayDiscovery(null);
    setSyncProgress(null);
  }, [selected, t]);

  React.useEffect(() => {
    if (!open) return;
    const source =
      CONNECTION_SOURCES.find((candidate) => candidate.id === initialSourceId) ??
      CONNECTION_SOURCES[0];
    setActiveCategory(source.category);
    setSelectedId(source.id);
    setStep(initialSourceId && source.status === "ready" ? "setup" : "source");
    setSetupError(null);
    setLastImportResult(null);
    setSourceQuery("");
  }, [initialSourceId, open]);

  React.useEffect(() => {
    if (open) return;
    setScannerOpen(false);
    setForm(formDefaultsFor(selected, t));
    setFieldErrors({});
    setSetupError(null);
    setLastImportResult(null);
    setPreviewAddresses(null);
    setPreviewError(null);
  }, [open, selected, t]);

  React.useEffect(() => {
    if (!defaultBackendName) return;
    if (setupKind !== "descriptor" && setupKind !== "samourai") return;
    setForm((current) =>
      current.backend ? current : { ...current, backend: defaultBackendName },
    );
  }, [defaultBackendName, setupKind]);

  React.useEffect(() => {
    if (setupKind !== "btcpay") return;
    setForm((current) => {
      if (btcpayBackends.length === 0) {
        return current.btcpayInstanceMode === "new"
          ? current
          : { ...current, btcpayInstanceMode: "new", backend: "" };
      }
      if (current.btcpayInstanceMode === "new" && current.btcpayServerUrl) {
        return current;
      }
      return {
        ...current,
        btcpayInstanceMode: "saved",
        backend: current.backend || defaultBtcpayBackendName,
      };
    });
  }, [btcpayBackends.length, defaultBtcpayBackendName, setupKind]);

  const selectCategory = (category: ConnectionCategory) => {
    setActiveCategory(category);
    const firstSource = CONNECTION_SOURCES.find(
      (source) => source.category === category,
    );
    if (firstSource) {
      setSelectedId(firstSource.id);
    }
  };

  const updateForm = <Key extends keyof SetupFormState>(
    key: Key,
    value: SetupFormState[Key],
  ) => {
    setForm((current) => ({ ...current, [key]: value }));
    if (
      key === "sourceFile" ||
      key === "samouraiDeposit" ||
      key === "samouraiBadbank" ||
      key === "samouraiPremix" ||
      key === "samouraiPostmix" ||
      key === "wasabiHistory" ||
      key === "wasabiCoins" ||
      key === "wasabiWalletInfo" ||
      key === "wasabiAdditional" ||
      key === "targetWallet" ||
      key === "sourceFormat" ||
      key === "bullImportMode" ||
      key === "bullWalletSetupMode" ||
      key === "bullWalletNetworks" ||
      key === "bullWalletRouteWallets"
    ) {
      setLastImportResult(null);
    }
    setFieldErrors((current) => {
      if (!(key in current)) return current;
      const next = { ...current };
      delete next[key];
      return next;
    });
  };

  const openBackendSettings = () => {
    const intent: DeferredConnectionSetup = {
      sourceId: selected.id,
      reason: `Adding ${selected.title}`,
      backendKind: selected.walletKind,
    };
    setDeferredConnectionSetup(intent);
    onOpenChange(false);
    void navigate({ to: "/settings", hash: "backends" });
  };

  const btcpayInstanceArgs = () => {
    if (form.btcpayInstanceMode === "saved") {
      return { backend: form.backend.trim() };
    }
    return {
      backend_label: form.btcpayInstanceLabel.trim(),
      server_url: form.btcpayServerUrl.trim(),
      api_key: form.btcpayApiKey.trim(),
    };
  };

  const validateSetupForm = (): Partial<Record<keyof SetupFormState, string>> => {
    const errors: Partial<Record<keyof SetupFormState, string>> = {};
    if (
      setupKind === "descriptor" ||
      setupKind === "file-wallet" ||
      setupKind === "samourai" ||
      setupKind === "btcpay" ||
      setupKind === "bullbitcoin-wallet"
    ) {
      if (!form.label.trim()) {
        errors.label = t("add.validation.labelRequired");
      }
    }
    if (setupKind === "descriptor") {
      if (!form.walletMaterial.trim()) {
        errors.walletMaterial = t("add.validation.pasteWalletMaterial");
      } else {
        const detection = detectWalletMaterial(form.walletMaterial);
        if (detection.kind === "bare-xpub" || detection.kind === "unknown") {
          errors.walletMaterial = detection.hint ?? detection.label;
        }
      }
      const gapLimit = Number.parseInt(form.gapLimit, 10);
      if (!Number.isFinite(gapLimit) || gapLimit <= 0) {
        errors.gapLimit = t("add.validation.gapPositive");
      } else if (gapLimit > MAX_DESCRIPTOR_GAP_LIMIT) {
        errors.gapLimit = t("add.validation.gapMax", {
          max: MAX_DESCRIPTOR_GAP_LIMIT.toLocaleString(),
        });
      }
      if (descriptorBackendOptions.length > 0 && !form.backend.trim()) {
        errors.backend = t("add.validation.chooseBackend");
      }
    }
    if (setupKind === "samourai") {
      const gapLimit = Number.parseInt(form.gapLimit, 10);
      if (!Number.isFinite(gapLimit) || gapLimit <= 0) {
        errors.gapLimit = t("add.validation.gapPositive");
      } else if (gapLimit > MAX_DESCRIPTOR_GAP_LIMIT) {
        errors.gapLimit = t("add.validation.gapMax", {
          max: MAX_DESCRIPTOR_GAP_LIMIT.toLocaleString(),
        });
      }
      if (descriptorBackendOptions.length > 0 && !form.backend.trim()) {
        errors.backend = t("add.validation.chooseBackend");
      }
      const sourceSetResult = buildSamouraiSourceSet(
        samouraiSourceFields(form),
        selected.network,
      );
      for (const [section, message] of Object.entries(
        sourceSetResult.errors,
      )) {
        const fieldKey = samouraiFieldKey(section as SamouraiSection);
        if (fieldKey) errors[fieldKey] = message;
      }
      if (
        sourceSetResult.sourceSet.children.length === 0 &&
        sourceSetResult.sourceSet.xpubs.length === 0 &&
        Object.keys(sourceSetResult.errors).length === 0
      ) {
        errors.samouraiDeposit = t("add.samourai.errorNeedMaterial");
      }
    }
    if (setupKind === "file-wallet") {
      if (
        selected.sourceFormat === "wasabi_bundle" &&
        form.wasabiImportMode === "rpc"
      ) {
        const { errors: wasabiErrors } = buildWasabiBundle({
          history: form.wasabiHistory,
          coins: form.wasabiCoins,
          walletInfo: form.wasabiWalletInfo,
          additional: form.wasabiAdditional,
        });
        if (wasabiErrors.history) errors.wasabiHistory = wasabiErrors.history;
        if (wasabiErrors.coins) errors.wasabiCoins = wasabiErrors.coins;
        if (wasabiErrors.walletInfo) {
          errors.wasabiWalletInfo = wasabiErrors.walletInfo;
        }
        if (wasabiErrors.additional) {
          errors.wasabiAdditional = wasabiErrors.additional;
        }
      } else if (!form.sourceFile.trim()) {
        errors.sourceFile = t("add.enrichment.errorPickExportFile");
      }
    }
    if (setupKind === "file-enrichment") {
      if (!isExchangeEvidenceFormat(selected.sourceFormat) && !form.targetWallet.trim()) {
        errors.targetWallet = t("add.enrichment.errorChooseWallet");
      }
      if (!form.sourceFile.trim()) {
        errors.sourceFile = t("add.enrichment.errorPickExportFile");
      }
    }
    if (setupKind === "bullbitcoin-wallet") {
      if (!form.sourceFile.trim()) {
        errors.sourceFile = t("add.bullWallet.errorPickExport");
      }
      if (form.bullWalletNetworks.length === 0) {
        errors.sourceFile = t("add.bullWallet.errorSelectNetwork");
      }
      if (form.bullWalletSetupMode === "existing_wallets") {
        if (existingWalletOptions.length === 0) {
          errors.sourceFile = t("add.bullWallet.errorCreateWalletsFirst");
        } else if (selectedBullWalletRoutes.some((route) => !route.wallet)) {
          errors.sourceFile = t("add.bullWallet.errorChooseWalletEach");
        }
      }
    }
    if (setupKind === "btcpay") {
      if (form.btcpayInstanceMode === "saved") {
        if (!form.backend.trim()) {
          errors.backend = t("add.btcpay.errorChooseInstance");
        }
      } else {
        if (!form.btcpayInstanceLabel.trim()) {
          errors.btcpayInstanceLabel = t("add.btcpay.errorInstanceName");
        }
        if (!form.btcpayServerUrl.trim()) {
          errors.btcpayServerUrl = t("add.btcpay.errorServerUrl");
        }
        if (!form.btcpayApiKey.trim()) {
          errors.btcpayApiKey = t("add.btcpay.errorApiKey");
        }
      }
      if (!form.btcpayStoreId.trim()) {
        errors.btcpayStoreId = t("add.btcpay.errorStoreId");
      }
      if (
        syncableDiscoveredPaymentMethodOptions.length > 0 &&
        selectedBtcpayPaymentMethodIds.length === 0
      ) {
        errors.btcpayPaymentMethodId = t("add.btcpay.errorSelectMethod");
      }
      if (form.btcpaySetupMode === "existing_wallets") {
        if (!btcpayDiscovery) {
          errors.btcpayPaymentMethodId = t("add.btcpay.errorDiscoverFirst");
        } else if (syncableDiscoveredPaymentMethodOptions.length === 0) {
          errors.btcpayPaymentMethodId = t("add.btcpay.errorNoSupportedMethods");
        } else if (existingWalletOptions.length === 0) {
          errors.btcpayPaymentMethodId = t(
            "add.btcpay.errorCreateSettlementFirst",
          );
        } else if (selectedBtcpayRoutes.some((route) => !route.wallet)) {
          errors.btcpayPaymentMethodId = t(
            "add.btcpay.errorChooseSettlementEach",
          );
        }
      }
    }
    if (setupKind === "bip329" && !form.bip329File.trim()) {
      errors.bip329File = t("add.bip329.errorPickFile");
    }
    return errors;
  };

  const onSetupSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (setupKind === "planned") {
      addNotification({
        title: t("add.planned.title"),
        body: t("add.planned.body", { title: selected.title }),
        tone: "warning",
      });
      return;
    }
    setSetupError(null);
    setLastImportResult(null);
    const errors = validateSetupForm();
    setFieldErrors(errors);
    if (Object.keys(errors).length > 0) return;
    const label = form.label.trim();
    try {
      if (setupKind === "backend-settings") {
        openBackendSettings();
        return;
      }
      if (setupKind === "descriptor") {
        const gapLimit = Number.parseInt(form.gapLimit, 10);
        await createWallet.mutateAsync({
          label,
          kind: selected.walletKind ?? "descriptor",
          backend: form.backend.trim() || undefined,
          chain: selected.chain,
          network: selected.network,
          wallet_material: form.walletMaterial.trim(),
          gap_limit: Number.isFinite(gapLimit) ? gapLimit : undefined,
        });
        if (form.syncAfterCreate) {
          startSyncNotice(
            t("add.descriptorWallet.stillScanning", { label }),
          );
          try {
            await syncWallet.mutateAsync({ wallet: label });
          } finally {
            clearSyncNotice();
          }
        }
        addNotification({
          title: t("add.added.title"),
          body: t("add.added.body", { label }),
          tone: "success",
        });
      } else if (setupKind === "samourai") {
        const gapLimit = Number.parseInt(form.gapLimit, 10);
        const { sourceSet } = buildSamouraiSourceSet(
          samouraiSourceFields(form),
          selected.network,
        );
        const envelope = await importSamourai.mutateAsync({
          label,
          backend: form.backend.trim() || undefined,
          network: selected.network,
          gap_limit: Number.isFinite(gapLimit) ? gapLimit : undefined,
          source_set: sourceSet,
        });
        const childLabels = envelope.data?.children.map((child) => child.label) ?? [];
        if (form.syncAfterCreate && childLabels.length > 0) {
          startSyncNotice(
            t("add.samourai.stillScanning", { label }),
          );
          try {
            for (const childLabel of childLabels) {
              await syncWallet.mutateAsync({ wallet: childLabel });
            }
          } finally {
            clearSyncNotice();
          }
        }
        addNotification({
          title: t("add.samourai.importedTitle"),
          body: form.syncAfterCreate
            ? t("add.samourai.importedBodyScanning", {
                label,
                value: childLabels.length.toLocaleString("en-US"),
              })
            : t("add.samourai.importedBody", {
                label,
                value: childLabels.length.toLocaleString("en-US"),
              }),
          tone: "success",
        });
        setForm((current) => ({
          ...current,
          samouraiDeposit: "",
          samouraiBadbank: "",
          samouraiPremix: "",
          samouraiPostmix: "",
        }));
      } else if (setupKind === "file-wallet") {
        const sourceFormat =
          selected.id === "csv" ? form.sourceFormat : selected.sourceFormat;
        await createWallet.mutateAsync({
          label,
          kind: selected.walletKind ?? "custom",
          ...(selected.sourceFormat === "wasabi_bundle" &&
          form.wasabiImportMode === "rpc"
            ? {}
            : { source_file: form.sourceFile.trim() }),
          source_format: sourceFormat,
        });
        if (
          selected.sourceFormat === "wasabi_bundle" &&
          form.wasabiImportMode === "rpc"
        ) {
          const { bundle } = buildWasabiBundle({
            history: form.wasabiHistory,
            coins: form.wasabiCoins,
            walletInfo: form.wasabiWalletInfo,
            additional: form.wasabiAdditional,
          });
          startSyncNotice(t("add.wasabi.importingRpc", { label }));
          try {
            const envelope = await importFile.mutateAsync({
              wallet: label,
              source_format: "wasabi_bundle",
              source_bundle: bundle,
            });
            setLastImportResult(envelope.data ?? null);
          } finally {
            clearSyncNotice();
          }
        } else if (form.syncAfterCreate) {
          startSyncNotice(
            t("add.fileWallet.stillImporting", { label }),
          );
          try {
            const envelope = await syncWallet.mutateAsync({ wallet: label });
            const result = envelope.data?.results.find(
              (item) => item.wallet === label,
            );
            const importSummary = importResultFromSyncResult(result);
            if (importSummary) {
              setLastImportResult(importSummary);
            }
          } finally {
            clearSyncNotice();
          }
        }
        addNotification({
          title: t("add.added.title"),
          body: form.syncAfterCreate
            ? t("add.added.bodyImported", { label })
            : t("add.added.body", { label }),
          tone: "success",
        });
      } else if (setupKind === "file-enrichment") {
        const sourceFormat = selected.sourceFormat;
        if (!sourceFormat) {
          throw new Error(t("add.enrichment.errorNoFormat"));
        }
        const isBookWideImport = isExchangeEvidenceFormat(sourceFormat);
        const isFullBullImport =
          isBookWideImport && form.bullImportMode === "full";
        startSyncNotice(
          isFullBullImport
            ? t("add.enrichment.matchingFull", { title: selected.title })
            : isBookWideImport
            ? t("add.enrichment.matchingBook", { title: selected.title })
            : t("add.enrichment.matchingWallet", {
                title: selected.title,
                wallet: form.targetWallet,
              }),
        );
        let importResult: ImportFileResult | undefined;
        try {
          const envelope = await importFile.mutateAsync({
            ...(isBookWideImport ? {} : { wallet: form.targetWallet }),
            source_file: form.sourceFile.trim(),
            source_format: sourceFormat,
            ...(isBookWideImport ? { mode: form.bullImportMode } : {}),
          });
          importResult = envelope.data;
          setLastImportResult(importResult ?? null);
        } finally {
          clearSyncNotice();
        }
        addNotification({
          title: t("add.enrichment.finishedTitle"),
          body: t("add.enrichment.finishedBody", {
            scope: isBookWideImport
              ? t("add.enrichment.bookScope")
              : form.targetWallet,
            updated: (importResult?.updated ?? 0).toLocaleString("en-US"),
            skipped: (importResult?.skipped ?? 0).toLocaleString("en-US"),
          }),
          tone: "success",
        });
      } else if (setupKind === "bullbitcoin-wallet") {
        const envelope = await createBullBitcoinWallet.mutateAsync(
          form.bullWalletSetupMode === "existing_wallets"
            ? {
                mode: "existing_wallets",
                label,
                source_file: form.sourceFile.trim(),
                routes: selectedBullWalletRoutes.map((route) => ({
                  wallet: route.wallet,
                  network: route.network,
                })),
              }
            : {
                mode: "wallet_sources",
                label,
                source_file: form.sourceFile.trim(),
                networks: form.bullWalletNetworks,
              },
        );
        const walletLabels =
          form.bullWalletSetupMode === "existing_wallets"
            ? Array.from(
                new Set(selectedBullWalletRoutes.map((route) => route.wallet)),
              )
            : (envelope.data?.wallets ?? [envelope.data?.wallet])
                .filter((wallet): wallet is { label: string } =>
                  Boolean(wallet),
                )
                .map((wallet) => wallet.label);
        if (form.syncAfterCreate && walletLabels.length > 0) {
          startSyncNotice(
            form.bullWalletSetupMode === "existing_wallets"
              ? t("add.bullWallet.stillImportingExisting", { label })
              : t("add.bullWallet.stillImportingSources", { label }),
          );
          try {
            for (const walletLabel of walletLabels) {
              const syncEnvelope = await syncWallet.mutateAsync({
                wallet: walletLabel,
              });
              const result = syncEnvelope.data?.results.find(
                (item) => item.wallet === walletLabel,
              );
              const importSummary = importResultFromSyncResult(result);
              if (importSummary) {
                setLastImportResult(importSummary);
              }
            }
          } finally {
            clearSyncNotice();
          }
        }
        addNotification({
          title:
            form.bullWalletSetupMode === "existing_wallets"
              ? t("add.bullWallet.mappingSavedTitle")
              : walletLabels.length > 1
                ? t("add.added.titlePlural")
                : t("add.added.title"),
          body:
            form.bullWalletSetupMode === "existing_wallets"
              ? form.syncAfterCreate
                ? t("add.bullWallet.mappedBodyRefreshed", {
                    count: walletLabels.length,
                  })
                : t("add.bullWallet.mappedBody", {
                    count: walletLabels.length,
                  })
              : form.syncAfterCreate
                ? t("add.bullWallet.sourcesBodyImported", {
                    count: walletLabels.length,
                  })
                : t("add.bullWallet.sourcesBody", {
                    count: walletLabels.length,
                  }),
          tone: "success",
        });
      } else if (setupKind === "btcpay") {
        const btcpayPayload =
          form.btcpaySetupMode === "existing_wallets"
            ? {
                ...btcpayInstanceArgs(),
                mode: "existing_wallets",
                label,
                store_id: form.btcpayStoreId.trim(),
                routes: selectedBtcpayRoutes.map((route) => ({
                  wallet: route.wallet,
                  payment_method_id: route.paymentMethodId,
                })),
              }
            : {
                ...btcpayInstanceArgs(),
                mode: "wallet_sources",
                label,
                store_id: form.btcpayStoreId.trim(),
                payment_method_ids: selectedBtcpayPaymentMethodIds,
              };
        const envelope = await createBtcpay.mutateAsync({
          ...btcpayPayload,
        });
        const createdLabels = (
          envelope.data?.wallets ?? [envelope.data?.wallet]
        )
          .filter((wallet): wallet is { label: string } => Boolean(wallet))
          .map((wallet) => wallet.label);
        if (form.syncAfterCreate && form.btcpaySetupMode === "wallet_sources") {
          startSyncNotice(
            t("add.btcpay.stillRefreshing", { label }),
          );
          try {
            for (const createdLabel of createdLabels) {
              await syncWallet.mutateAsync({ wallet: createdLabel });
            }
          } finally {
            clearSyncNotice();
          }
        }
        if (form.syncAfterCreate && form.btcpaySetupMode === "existing_wallets") {
          const walletsToRefresh = Array.from(
            new Set(selectedBtcpayRoutes.map((route) => route.wallet)),
          );
          startSyncNotice(
            t("add.btcpay.refreshingSettlement", { label }),
          );
          try {
            for (const walletLabel of walletsToRefresh) {
              await syncWallet.mutateAsync({ wallet: walletLabel });
            }
          } finally {
            clearSyncNotice();
          }
        }
        addNotification({
          title:
            form.btcpaySetupMode === "existing_wallets"
              ? t("add.btcpay.mappingSavedTitle")
              : createdLabels.length > 1
                ? t("add.added.titlePlural")
                : t("add.added.title"),
          body:
            form.btcpaySetupMode === "existing_wallets"
              ? form.syncAfterCreate
                ? t("add.btcpay.mappedBodyRefreshed", {
                    count: createdLabels.length,
                  })
                : t("add.btcpay.mappedBody", { count: createdLabels.length })
              : createdLabels.length > 1
              ? form.syncAfterCreate
                ? t("add.btcpay.methodsBodyRefreshed", {
                    count: createdLabels.length,
                  })
                : t("add.btcpay.methodsBody", { count: createdLabels.length })
              : form.syncAfterCreate
                ? t("add.added.bodyRefreshed", { label })
                : t("add.added.body", { label }),
          tone: "success",
        });
      } else if (setupKind === "bip329") {
        const envelope = await importBip329.mutateAsync({
          file: form.bip329File.trim(),
          wallet: form.bip329Wallet.trim() || undefined,
        });
        addNotification({
          title: t("add.bip329.labelsImportedTitle"),
          body: t("add.bip329.labelsImportedBody", {
            count: envelope.data?.records ?? 0,
          }),
          tone: "success",
        });
      }
      onOpenChange(false);
    } catch (error) {
      const message =
        error instanceof Error ? error.message : t("add.setupFailed.fallback");
      setSetupError(message);
      addNotification({
        title: t("add.setupFailed.title"),
        body: message,
        tone: "error",
      });
    }
  };

  const renderBackendSelect = (
    id: string,
    label: string,
    options: BackendOption[],
  ) => (
    <SetupField id={id} label={label} error={fieldErrors.backend}>
      {options.length ? (
        <select
          id={id}
          className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
          value={form.backend}
          onChange={(event) => {
            updateForm("backend", event.target.value);
            setBtcpayTestStatus(null);
          }}
          required
        >
          <option value="" disabled>
            {t("add.field.selectBackend")}
          </option>
          {options.map((backend) => (
            <option key={backend.name} value={backend.name}>
              {backendOptionLabel(backend)}
              {backend.is_default ? t("add.field.backendOptionDefault") : ""}
              {backend.kind ? ` · ${backend.kind}` : ""}
            </option>
          ))}
        </select>
      ) : (
        <div className="space-y-2 rounded-md border bg-background p-3">
          <p className="text-sm text-muted-foreground">
            {t("add.field.noBackendConfigured")}
          </p>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={openBackendSettings}
          >
            {t("add.field.openBackendSettings")}
          </Button>
        </div>
      )}
    </SetupField>
  );

  const renderConnectionLabelField = () => (
    <SetupField
      id="connection-label"
      label={t("add.field.connectionLabel")}
      error={fieldErrors.label}
    >
      <Input
        id="connection-label"
        value={form.label}
        onChange={(event) => updateForm("label", event.target.value)}
        required
      />
    </SetupField>
  );

  const renderSyncAfterCreate = (label: string) => (
    <label className="flex items-center gap-2 text-sm">
      <Checkbox
        checked={form.syncAfterCreate}
        onCheckedChange={(checked) =>
          updateForm("syncAfterCreate", checked === true)
        }
      />
      {label}
    </label>
  );

  const renderWalletMaterialFeedback = () => {
    const detection = detectWalletMaterial(form.walletMaterial);
    if (detection.kind === "empty") return null;
    const tone =
      detection.kind === "bare-xpub" || detection.kind === "unknown"
        ? "text-amber-700 dark:text-amber-300"
        : "text-emerald-700 dark:text-emerald-300";
    return (
      <p className={cn("text-xs", tone)}>
        {detection.hint
          ? t("add.descriptor.detectedWithHint", {
              label: detection.label,
              hint: detection.hint,
            })
          : t("add.descriptor.detected", { label: detection.label })}
      </p>
    );
  };

  const renderDescriptorPreview = () => {
    if (previewError) {
      return (
        <div className="rounded-md border border-destructive/30 bg-destructive/10 p-2 text-xs text-destructive">
          {previewError}
        </div>
      );
    }
    if (!previewAddresses || previewAddresses.length === 0) return null;
    return (
      <div className="rounded-md border bg-background p-3 text-xs">
        <p className="mb-2 font-medium text-muted-foreground">
          {t("add.descriptor.firstDerived")}
        </p>
        <ul className="space-y-1 font-mono">
          {previewAddresses.map((entry) => (
            <li
              key={`${entry.branch}-${entry.index}`}
              className="flex items-center gap-2"
            >
              <span className="w-16 shrink-0 text-muted-foreground">
                {entry.branch === "change"
                  ? t("add.descriptor.branchChange")
                  : t("add.descriptor.branchReceive", { index: entry.index })}
              </span>
              <span className="min-w-0 flex-1 truncate">{entry.address}</span>
              <button
                type="button"
                className="rounded border px-2 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground hover:bg-muted/40"
                onClick={() => void copyAddress(entry.address)}
                title={t("add.descriptor.copyAddress")}
              >
                {copiedAddress === entry.address
                  ? t("add.descriptor.copied")
                  : t("add.descriptor.copy")}
              </button>
            </li>
          ))}
        </ul>
      </div>
    );
  };

  const renderSetupFields = () => {
    const sourceFileField = fileWalletSourceField(selected, t);
    const renderSourceFileSetup = (syncLabel?: string) => (
      <>
        <SetupField
          id="connection-source-file"
          label={sourceFileField.label}
          error={fieldErrors.sourceFile}
          helper={sourceFileField.helper}
        >
          <div className="flex gap-2">
            <Input
              id="connection-source-file"
              value={form.sourceFile}
              onChange={(event) => updateForm("sourceFile", event.target.value)}
              required
            />
            {isFilePickerAvailable ? (
              <Button
                type="button"
                variant="outline"
                onClick={async () => {
                  const picked = await pickFile({
                    title: t("add.field.selectExportFileTitle", {
                      title: selected.title,
                    }),
                    filters: sourceFileFilters(selected, t),
                  });
                  if (picked) updateForm("sourceFile", picked);
                }}
              >
                {t("add.field.browse")}
              </Button>
            ) : null}
          </div>
        </SetupField>
        {syncLabel ? renderSyncAfterCreate(syncLabel) : null}
      </>
    );

    if (setupKind === "descriptor") {
      return (
        <>
          {renderConnectionLabelField()}
          {renderBackendSelect(
            "connection-backend",
            t("add.field.backend"),
            descriptorBackendOptions,
          )}
          <SetupField
            id="connection-wallet-material"
            label={t("add.descriptor.walletExport")}
            error={fieldErrors.walletMaterial}
          >
            <div className="flex items-center justify-between gap-3">
              <p className="text-xs text-muted-foreground">
                {t("add.descriptor.scanHint")}
              </p>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="shrink-0"
                onClick={() => setScannerOpen(true)}
              >
                <ScanLine className="size-4" aria-hidden="true" />
                {t("add.descriptor.scan")}
              </Button>
            </div>
            <Textarea
              id="connection-wallet-material"
              className="min-h-32 font-mono text-xs"
              value={form.walletMaterial}
              onChange={(event) => {
                updateForm("walletMaterial", event.target.value);
                setPreviewAddresses(null);
                setPreviewError(null);
              }}
              required
            />
            {renderWalletMaterialFeedback()}
          </SetupField>
          <SetupField
            id="connection-gap-limit"
            label={t("add.descriptor.gapLimit")}
            error={fieldErrors.gapLimit}
            helper={t("add.descriptor.gapLimitHelper")}
          >
            <Input
              id="connection-gap-limit"
              type="number"
              min={1}
              max={MAX_DESCRIPTOR_GAP_LIMIT}
              value={form.gapLimit}
              onChange={(event) => updateForm("gapLimit", event.target.value)}
            />
          </SetupField>
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={
                previewDescriptor.isPending || !form.walletMaterial.trim()
              }
              onClick={async () => {
                setPreviewError(null);
                try {
                  const envelope = await previewDescriptor.mutateAsync({
                    wallet_material: form.walletMaterial.trim(),
                    chain: selected.chain,
                    network: selected.network,
                    count: 5,
                  });
                  setPreviewAddresses(envelope.data?.addresses ?? []);
                } catch (error) {
                  setPreviewAddresses(null);
                  setPreviewError(
                    error instanceof Error
                      ? error.message
                      : t("add.descriptor.couldNotDerive"),
                  );
                }
              }}
            >
              {previewDescriptor.isPending
                ? t("add.descriptor.deriving")
                : t("add.descriptor.previewAddresses")}
            </Button>
            <span className="text-xs text-muted-foreground">
              {t("add.descriptor.previewHint")}
            </span>
          </div>
          {renderDescriptorPreview()}
        </>
      );
    }

    if (setupKind === "samourai") {
      return (
        <>
          {renderConnectionLabelField()}
          {renderBackendSelect(
            "connection-backend",
            t("add.field.backend"),
            descriptorBackendOptions,
          )}
          <div className="space-y-3">
            <div className="rounded-md border bg-muted/25 p-3 text-xs text-muted-foreground">
              {t("add.samourai.intro")}
            </div>
            {SAMOURAI_SOURCE_FIELDS.map((field) => (
              <SetupField
                key={field.key}
                id={field.id}
                label={t(field.labelKey)}
                error={fieldErrors[field.key]}
                helper={t(field.helperKey)}
              >
                <Textarea
                  id={field.id}
                  className="min-h-20 font-mono text-xs"
                  value={form[field.key]}
                  onChange={(event) =>
                    updateForm(field.key, event.target.value)
                  }
                />
              </SetupField>
            ))}
          </div>
          <SetupField
            id="connection-samourai-gap-limit"
            label={t("add.samourai.gapLimit")}
            error={fieldErrors.gapLimit}
            helper={t("add.samourai.gapLimitHelper")}
          >
            <Input
              id="connection-samourai-gap-limit"
              type="number"
              min={1}
              max={MAX_DESCRIPTOR_GAP_LIMIT}
              value={form.gapLimit}
              onChange={(event) => updateForm("gapLimit", event.target.value)}
            />
          </SetupField>
          {renderSyncAfterCreate(t("add.samourai.scanAfter"))}
        </>
      );
    }

    if (setupKind === "file-wallet") {
      if (selected.sourceFormat === "wasabi_bundle") {
        return (
          <>
            {renderConnectionLabelField()}
            <SetupField
              id="connection-wasabi-mode"
              label={t("add.wasabi.importSource")}
            >
              <div className="grid grid-cols-2 gap-2">
                {(
                  [
                    ["rpc", t("add.wasabi.pasteRpc")],
                    ["bundle-file", t("add.wasabi.bundleFile")],
                  ] as const
                ).map(([value, text]) => (
                  <Button
                    key={value}
                    type="button"
                    variant={
                      form.wasabiImportMode === value ? "secondary" : "outline"
                    }
                    onClick={() => {
                      updateForm("wasabiImportMode", value);
                      setLastImportResult(null);
                    }}
                  >
                    {text}
                  </Button>
                ))}
              </div>
            </SetupField>
            {form.wasabiImportMode === "rpc" ? (
              <div className="space-y-3">
                <div className="space-y-2 rounded-md border bg-muted/25 p-3 text-xs text-muted-foreground">
                  <p>
                    <Trans
                      t={t}
                      i18nKey="add.wasabi.rpcIntro1"
                      components={{ code: <InlineCode /> }}
                      values={{
                        endpoint: "http://127.0.0.1:37128/WalletName",
                        placeholder: "WalletName",
                      }}
                    />
                  </p>
                  <p>
                    <Trans
                      t={t}
                      i18nKey="add.wasabi.rpcIntro2"
                      components={{ code: <InlineCode /> }}
                      values={{
                        user: "JsonRpcUser",
                        password: "JsonRpcPassword",
                        auth: "-u user:password",
                      }}
                    />
                  </p>
                </div>
                <SetupField
                  id="connection-wasabi-history"
                  label={t("add.wasabi.historyLabel")}
                  error={fieldErrors.wasabiHistory}
                  helper={
                    <div className="space-y-1.5">
                      <p>{t("add.wasabi.historyHelper")}</p>
                      <CommandSnippet>
                        curl -s --data-binary
                        {' \'{"jsonrpc":"2.0","id":"1","method":"gethistory","params":[]}\''}{" "}
                        http://127.0.0.1:37128/WalletName | jq
                      </CommandSnippet>
                    </div>
                  }
                >
                  <Textarea
                    id="connection-wasabi-history"
                    className="min-h-28 font-mono text-xs"
                    value={form.wasabiHistory}
                    onChange={(event) =>
                      updateForm("wasabiHistory", event.target.value)
                    }
                    required
                  />
                </SetupField>
                <SetupField
                  id="connection-wasabi-coins"
                  label={t("add.wasabi.coinsLabel")}
                  error={fieldErrors.wasabiCoins}
                  helper={
                    <div className="space-y-1.5">
                      <p>{t("add.wasabi.coinsHelper")}</p>
                      <CommandSnippet>
                        curl -s --data-binary
                        {' \'{"jsonrpc":"2.0","id":"1","method":"listcoins","params":[]}\''}{" "}
                        http://127.0.0.1:37128/WalletName | jq
                      </CommandSnippet>
                    </div>
                  }
                >
                  <Textarea
                    id="connection-wasabi-coins"
                    className="min-h-24 font-mono text-xs"
                    value={form.wasabiCoins}
                    onChange={(event) =>
                      updateForm("wasabiCoins", event.target.value)
                    }
                  />
                </SetupField>
                <SetupField
                  id="connection-wasabi-wallet-info"
                  label={t("add.wasabi.walletInfoLabel")}
                  error={fieldErrors.wasabiWalletInfo}
                  helper={
                    <div className="space-y-1.5">
                      <p>{t("add.wasabi.walletInfoHelper")}</p>
                      <CommandSnippet>
                        curl -s --data-binary
                        {' \'{"jsonrpc":"2.0","id":"1","method":"getwalletinfo","params":[]}\''}{" "}
                        http://127.0.0.1:37128/WalletName | jq
                      </CommandSnippet>
                    </div>
                  }
                >
                  <Textarea
                    id="connection-wasabi-wallet-info"
                    className="min-h-20 font-mono text-xs"
                    value={form.wasabiWalletInfo}
                    onChange={(event) =>
                      updateForm("wasabiWalletInfo", event.target.value)
                    }
                  />
                </SetupField>
                <SetupField
                  id="connection-wasabi-additional"
                  label={t("add.wasabi.additionalLabel")}
                  error={fieldErrors.wasabiAdditional}
                  helper={
                    <span>
                      <Trans
                        t={t}
                        i18nKey="add.wasabi.additionalHelper"
                        components={{ code: <InlineCode /> }}
                        values={{
                          listkeys: "listkeys",
                          listpayments: "listpaymentsincoinjoin",
                          walletJson: "wallet_json",
                        }}
                      />
                    </span>
                  }
                >
                  <Textarea
                    id="connection-wasabi-additional"
                    className="min-h-20 font-mono text-xs"
                    value={form.wasabiAdditional}
                    onChange={(event) =>
                      updateForm("wasabiAdditional", event.target.value)
                    }
                  />
                </SetupField>
              </div>
            ) : (
              renderSourceFileSetup(t("add.wasabi.importAfter"))
            )}
          </>
        );
      }
      return (
        <>
          {renderConnectionLabelField()}
          {selected.id === "csv" ? (
            <SetupField
              id="connection-source-format"
              label={t("add.fileWallet.fileFormat")}
            >
              <select
                id="connection-source-format"
                className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
                value={form.sourceFormat}
                onChange={(event) =>
                  updateForm("sourceFormat", event.target.value as "csv" | "json")
                }
              >
                <option value="csv">{t("add.fileWallet.formatCsv")}</option>
                <option value="json">{t("add.fileWallet.formatJson")}</option>
              </select>
            </SetupField>
          ) : null}
          {renderSourceFileSetup(t("add.fileWallet.importAfter"))}
        </>
      );
    }

    if (setupKind === "bullbitcoin-wallet") {
      return (
        <>
          {renderConnectionLabelField()}
          <SetupField
            id="connection-bull-wallet-mode"
            label={t("add.bullWallet.importBehavior")}
          >
            <div className="grid grid-cols-2 gap-2">
              <Button
                type="button"
                variant={
                  form.bullWalletSetupMode === "wallet_sources"
                    ? "secondary"
                    : "outline"
                }
                onClick={() =>
                  updateForm("bullWalletSetupMode", "wallet_sources")
                }
              >
                {t("add.bullWallet.createBullWallets")}
              </Button>
              <Button
                type="button"
                variant={
                  form.bullWalletSetupMode === "existing_wallets"
                    ? "secondary"
                    : "outline"
                }
                onClick={() =>
                  updateForm("bullWalletSetupMode", "existing_wallets")
                }
              >
                {t("add.bullWallet.mapExistingWallets")}
              </Button>
            </div>
          </SetupField>
          <SetupField
            id="connection-bull-wallet-networks"
            label={t("add.bullWallet.networks")}
            helper={
              form.bullWalletSetupMode === "wallet_sources"
                ? t("add.bullWallet.networksHelperCreate")
                : t("add.bullWallet.networksHelperMap")
            }
          >
            <div className="grid gap-2 sm:grid-cols-3">
              {BULLBITCOIN_WALLET_NETWORKS.map((network) => {
                const checked = form.bullWalletNetworks.includes(network.id);
                return (
                  <label
                    key={network.id}
                    className="flex min-h-20 items-start gap-3 rounded-md border border-border/70 p-3 text-sm"
                  >
                    <Checkbox
                      checked={checked}
                      onCheckedChange={(value) => {
                        const nextChecked = value === true;
                        setForm((current) => {
                          const networks = new Set(current.bullWalletNetworks);
                          if (nextChecked) {
                            networks.add(network.id);
                          } else {
                            networks.delete(network.id);
                          }
                          const routeWallets = {
                            ...current.bullWalletRouteWallets,
                          };
                          if (nextChecked && !routeWallets[network.id]) {
                            routeWallets[network.id] =
                              walletForBullNetwork(network.id);
                          }
                          return {
                            ...current,
                            bullWalletNetworks: Array.from(networks),
                            bullWalletRouteWallets: routeWallets,
                          };
                        });
                      }}
                    />
                    <span className="grid gap-1">
                      <span>{t(network.labelKey)}</span>
                      <span className="text-xs text-muted-foreground">
                        {t(network.helperKey)}
                      </span>
                    </span>
                  </label>
                );
              })}
            </div>
          </SetupField>
          {form.bullWalletSetupMode === "existing_wallets" ? (
            <div className="space-y-3 rounded-md border border-border/70 p-3">
              <div>
                <p className="text-sm font-medium">
                  {t("add.bullWallet.walletMapping")}
                </p>
                <p className="text-xs text-muted-foreground">
                  {t("add.bullWallet.walletMappingHelper")}
                </p>
              </div>
              {selectedBullWalletRoutes.length === 0 ? (
                <div className="rounded-md border border-dashed border-border/70 p-3 text-sm text-muted-foreground">
                  {t("add.bullWallet.selectToMap")}
                </div>
              ) : (
                selectedBullWalletRoutes.map((route) => (
                  <div
                    key={route.network}
                    className="grid gap-3 md:grid-cols-[minmax(0,0.8fr)_minmax(0,1.2fr)]"
                  >
                    <div className="space-y-2">
                      <Label>{t("add.bullWallet.exportNetwork")}</Label>
                      <div className="flex h-9 items-center rounded-md border border-border/70 bg-muted/40 px-3 text-sm capitalize">
                        {route.network}
                      </div>
                    </div>
                    <SetupField
                      id={`connection-bull-route-${route.network}`}
                      label={t("add.bullWallet.kassiberWallet")}
                    >
                      <select
                        id={`connection-bull-route-${route.network}`}
                        className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
                        value={route.wallet}
                        onChange={(event) => {
                          setForm((current) => ({
                            ...current,
                            bullWalletRouteWallets: {
                              ...current.bullWalletRouteWallets,
                              [route.network]: event.target.value,
                            },
                          }));
                        }}
                        disabled={existingWalletOptions.length === 0}
                      >
                        {existingWalletOptions.length === 0 ? (
                          <option value="">
                            {t("add.bullWallet.noWalletsYet")}
                          </option>
                        ) : (
                          <option value="" disabled>
                            {t("add.bullWallet.selectWallet")}
                          </option>
                        )}
                        {existingWalletOptions.map((wallet) => (
                          <option key={wallet.label} value={wallet.label}>
                            {wallet.label}
                            {wallet.chain ? ` (${wallet.chain})` : ""}
                          </option>
                        ))}
                      </select>
                    </SetupField>
                  </div>
                ))
              )}
            </div>
          ) : null}
          {renderSourceFileSetup(t("add.bullWallet.refreshAfter"))}
        </>
      );
    }

    if (setupKind === "file-enrichment") {
      if (isExchangeEvidenceFormat(selected.sourceFormat)) {
        return (
          <>
            <SetupField
              id="connection-bull-import-mode"
              label={t("add.enrichment.importMode")}
              helper={
                form.bullImportMode === "full"
                  ? t("add.enrichment.importModeHelperFull", {
                      title: selected.title,
                    })
                  : selected.sourceFormat === "21bitcoin_csv"
                  ? t("add.enrichment.importModeHelper21bitcoin")
                  : t("add.enrichment.importModeHelperRelevant", {
                      title: selected.title,
                    })
              }
            >
              <select
                id="connection-bull-import-mode"
                className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
                value={form.bullImportMode}
                onChange={(event) =>
                  updateForm(
                    "bullImportMode",
                    event.target.value as SetupFormState["bullImportMode"],
                  )
                }
              >
                <option value="relevant">
                  {t("add.enrichment.relevantOnly")}
                </option>
                <option value="full">{t("add.enrichment.fullImport")}</option>
              </select>
            </SetupField>
            <SetupField
              id="connection-source-file"
              label={t("add.enrichment.csvPath")}
              error={fieldErrors.sourceFile}
              helper={t("add.enrichment.csvPathHelper", {
                title: selected.title,
              })}
            >
              <div className="flex gap-2">
                <Input
                  id="connection-source-file"
                  value={form.sourceFile}
                  onChange={(event) => updateForm("sourceFile", event.target.value)}
                  required
                />
                {isFilePickerAvailable ? (
                  <Button
                    type="button"
                    variant="outline"
                    onClick={async () => {
                      const picked = await pickFile({
                        title: t("add.field.selectExportFileTitle", {
                          title: selected.title,
                        }),
                        filters: sourceFileFilters(selected, t),
                      });
                      if (picked) updateForm("sourceFile", picked);
                    }}
                  >
                    {t("add.field.browse")}
                  </Button>
                ) : null}
              </div>
            </SetupField>
          </>
        );
      }
      return (
        <>
          <SetupField
            id="connection-target-wallet"
            label={t("add.enrichment.walletToEnrich")}
            error={fieldErrors.targetWallet}
            helper={t("add.enrichment.walletToEnrichHelper")}
          >
            <select
              id="connection-target-wallet"
              className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
              value={form.targetWallet}
              onChange={(event) => updateForm("targetWallet", event.target.value)}
              required
            >
              <option value="">{t("add.enrichment.chooseWallet")}</option>
              {existingWalletOptions.map((wallet) => (
                <option key={wallet.label} value={wallet.label}>
                  {wallet.label}
                </option>
              ))}
            </select>
          </SetupField>
          <SetupField
            id="connection-source-file"
            label={t("add.exportFile.label")}
            error={fieldErrors.sourceFile}
          >
            <div className="flex gap-2">
              <Input
                id="connection-source-file"
                value={form.sourceFile}
                onChange={(event) => updateForm("sourceFile", event.target.value)}
                required
              />
              {isFilePickerAvailable ? (
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => {
                    void pickFile({
                      title: t("add.field.chooseExportTitle", {
                        title: selected.title,
                      }),
                      filters: sourceFileFilters(selected, t),
                    })
                      .then((path) => {
                        if (path) updateForm("sourceFile", path);
                      })
                      .catch((error) => {
                        addNotification({
                          title: t("add.field.filePickerUnavailableTitle"),
                          body: t("add.field.filePickerUnavailableBody", {
                            message:
                              error instanceof Error
                                ? error.message
                                : t("add.field.filePickerFallback"),
                          }),
                          tone: "warning",
                        });
                      });
                  }}
                >
                  {t("add.field.browseDots")}
                </Button>
              ) : null}
            </div>
          </SetupField>
        </>
      );
    }

    if (setupKind === "btcpay") {
      const canUseSavedInstance = btcpayBackends.length > 0;
      const canDiscover =
        form.btcpayInstanceMode === "saved"
          ? Boolean(form.backend.trim())
          : Boolean(form.btcpayServerUrl.trim() && form.btcpayApiKey.trim());
      return (
        <>
          {renderConnectionLabelField()}
          <div className="grid grid-cols-2 gap-2">
            <Button
              type="button"
              variant={
                form.btcpayInstanceMode === "saved" ? "secondary" : "outline"
              }
              disabled={!canUseSavedInstance}
              onClick={() => {
                updateForm("btcpayInstanceMode", "saved");
                if (!form.backend && defaultBtcpayBackendName) {
                  updateForm("backend", defaultBtcpayBackendName);
                }
                setBtcpayDiscovery(null);
                setBtcpayTestStatus(null);
              }}
            >
              {t("add.btcpay.savedInstance")}
            </Button>
            <Button
              type="button"
              variant={
                form.btcpayInstanceMode === "new" ? "secondary" : "outline"
              }
              onClick={() => {
                updateForm("btcpayInstanceMode", "new");
                setBtcpayDiscovery(null);
                setBtcpayTestStatus(null);
              }}
            >
              {t("add.btcpay.newInstance")}
            </Button>
          </div>
          {form.btcpayInstanceMode === "saved" ? (
            <SetupField
              id="connection-btcpay-instance"
              label={t("add.btcpay.instance")}
              error={fieldErrors.backend}
            >
              <select
                id="connection-btcpay-instance"
                className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
                value={form.backend}
                onChange={(event) => {
                  updateForm("backend", event.target.value);
                  setBtcpayDiscovery(null);
                  setBtcpayTestStatus(null);
                }}
                required
              >
                <option value="" disabled>
                  {t("add.btcpay.selectInstance")}
                </option>
                {btcpayBackends.map((backend) => (
                  <option key={backend.name} value={backend.name}>
                    {backendOptionLabel(backend)}
                    {backend.is_default
                      ? t("add.field.backendOptionDefault")
                      : ""}
                  </option>
                ))}
              </select>
            </SetupField>
          ) : (
            <>
              <SetupField
                id="connection-btcpay-instance-label"
                label={t("add.btcpay.instanceName")}
                error={fieldErrors.btcpayInstanceLabel}
              >
                <Input
                  id="connection-btcpay-instance-label"
                  value={form.btcpayInstanceLabel}
                  onChange={(event) =>
                    updateForm("btcpayInstanceLabel", event.target.value)
                  }
                  required
                />
              </SetupField>
              <SetupField
                id="connection-btcpay-url"
                label={t("add.btcpay.serverUrl")}
                error={fieldErrors.btcpayServerUrl}
              >
                <Input
                  id="connection-btcpay-url"
                  value={form.btcpayServerUrl}
                  onChange={(event) => {
                    updateForm("btcpayServerUrl", event.target.value);
                    setBtcpayDiscovery(null);
                    setBtcpayTestStatus(null);
                  }}
                  placeholder={t("add.btcpay.serverUrlPlaceholder")}
                  required
                />
              </SetupField>
              <SetupField
                id="connection-btcpay-api-key"
                label={t("add.btcpay.apiKey")}
                error={fieldErrors.btcpayApiKey}
              >
                <Input
                  id="connection-btcpay-api-key"
                  type="password"
                  value={form.btcpayApiKey}
                  onChange={(event) => {
                    updateForm("btcpayApiKey", event.target.value);
                    setBtcpayDiscovery(null);
                    setBtcpayTestStatus(null);
                  }}
                  required
                />
              </SetupField>
            </>
          )}
          <div className="grid grid-cols-2 gap-2">
            <Button
              type="button"
              variant={
                form.btcpaySetupMode === "wallet_sources"
                  ? "secondary"
                  : "outline"
              }
              onClick={() => {
                updateForm("btcpaySetupMode", "wallet_sources");
                setBtcpayTestStatus(null);
              }}
            >
              {t("add.btcpay.createFromBtcpay")}
            </Button>
            <Button
              type="button"
              variant={
                form.btcpaySetupMode === "existing_wallets"
                  ? "secondary"
                  : "outline"
              }
              onClick={() => {
                updateForm("btcpaySetupMode", "existing_wallets");
                setBtcpayTestStatus(null);
              }}
            >
              {t("add.btcpay.mapExistingWallets")}
            </Button>
          </div>
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={discoverBtcpay.isPending || !canDiscover}
              onClick={async () => {
                setBtcpayTestStatus(null);
                try {
                  const envelope = await discoverBtcpay.mutateAsync(
                    btcpayInstanceArgs(),
                  );
                  const data = envelope.data;
                  setBtcpayDiscovery(data ?? null);
                  const firstStore = data?.stores?.[0]?.id ?? "";
                  setForm((current) => {
                    const targetStore = current.btcpayStoreId || firstStore;
                    const syncableMethodsForStore = (
                      data?.payment_methods ?? []
                    ).filter(
                      (method) =>
                        method.sync_supported &&
                        (!targetStore || method.store_id === targetStore),
                    );
                    const firstMethod =
                      syncableMethodsForStore[0]?.payment_method_id ??
                      DEFAULT_BTCPAY_PAYMENT_METHOD_ID;
                    const routeWallets = Object.fromEntries(
                      syncableMethodsForStore.map((method) => [
                        method.payment_method_id,
                        current.btcpayRouteWallets[method.payment_method_id] ||
                          walletForPaymentMethod(method.payment_method_id),
                      ]),
                    );
                    return {
                      ...current,
                      btcpayStoreId: targetStore,
                      btcpayPaymentMethodIds: syncableMethodsForStore.length
                        ? syncableMethodsForStore.map(
                            (method) => method.payment_method_id,
                          )
                        : [firstMethod],
                      btcpayPaymentMethodId:
                        current.btcpayPaymentMethodId &&
                        current.btcpayPaymentMethodId !==
                          DEFAULT_BTCPAY_PAYMENT_METHOD_ID
                          ? current.btcpayPaymentMethodId
                          : firstMethod,
                      btcpayRouteWallets: {
                        ...current.btcpayRouteWallets,
                        ...routeWallets,
                      },
                    };
                  });
                } catch (error) {
                  setBtcpayDiscovery(null);
                  setBtcpayTestStatus({
                    ok: false,
                    message:
                      error instanceof Error
                        ? error.message
                        : t("add.btcpay.discoveryFailed"),
                  });
                }
              }}
            >
              {discoverBtcpay.isPending
                ? t("add.btcpay.discovering")
                : t("add.btcpay.discoverStores")}
            </Button>
            {btcpayDiscovery ? (
              <span className="text-xs text-muted-foreground">
                {t("add.btcpay.storesFound", {
                  count: btcpayDiscovery.stores.length,
                })}
              </span>
            ) : null}
          </div>
          <SetupField
            id="connection-btcpay-store"
            label={t("add.btcpay.storeId")}
            error={fieldErrors.btcpayStoreId}
          >
            <Input
              id="connection-btcpay-store"
              list={
                discoveredStoreOptions.length
                  ? "connection-btcpay-store-options"
                  : undefined
              }
              value={form.btcpayStoreId}
              onChange={(event) => {
                updateForm("btcpayStoreId", event.target.value);
                setForm((current) => {
                  const matchingMethods = (
                    btcpayDiscovery?.payment_methods ?? []
                  ).filter(
                    (method) =>
                      method.sync_supported &&
                      method.store_id === event.target.value,
                  );
                  return matchingMethods.length
                    ? {
                        ...current,
                        btcpayPaymentMethodId:
                          matchingMethods[0].payment_method_id,
                        btcpayPaymentMethodIds: matchingMethods.map(
                          (method) => method.payment_method_id,
                        ),
                        btcpayRouteWallets: {
                          ...current.btcpayRouteWallets,
                          ...Object.fromEntries(
                            matchingMethods.map((method) => [
                              method.payment_method_id,
                              current.btcpayRouteWallets[
                                method.payment_method_id
                              ] ||
                                walletForPaymentMethod(
                                  method.payment_method_id,
                                ),
                            ]),
                          ),
                        },
                      }
                    : current;
                });
                setBtcpayTestStatus(null);
              }}
              required
            />
            {discoveredStoreOptions.length ? (
              <datalist id="connection-btcpay-store-options">
                {discoveredStoreOptions.map((store) => (
                  <option key={store.id} value={store.id}>
                    {store.name}
                  </option>
                ))}
              </datalist>
            ) : null}
          </SetupField>
          <SetupField
            id="connection-btcpay-payment-method"
            label={
              form.btcpaySetupMode === "existing_wallets" ||
              discoveredPaymentMethodOptions.length
                ? t("add.btcpay.paymentMethods")
                : t("add.btcpay.onChainPaymentMethodId")
            }
            error={fieldErrors.btcpayPaymentMethodId}
            helper={
              discoveredPaymentMethodOptions.length
                ? form.btcpaySetupMode === "existing_wallets"
                  ? t("add.btcpay.paymentMethodsHelperMap")
                  : t("add.btcpay.paymentMethodsHelperCreate")
                : form.btcpaySetupMode === "existing_wallets"
                  ? t("add.btcpay.paymentMethodsHelperDiscoverMap")
                  : t("add.btcpay.paymentMethodsHelperDefault")
            }
          >
            {discoveredPaymentMethodOptions.length ? (
              <div className="space-y-2 rounded-md border border-border/70 p-3">
                {discoveredPaymentMethodOptions.map((method) => {
                  const checked = form.btcpayPaymentMethodIds.includes(
                    method.payment_method_id,
                  );
                  return (
                    <label
                      key={`${method.store_id}-${method.payment_method_id}`}
                      className={cn(
                        "flex items-start gap-3 text-sm",
                        !method.sync_supported && "opacity-55",
                      )}
                    >
                      <Checkbox
                        checked={method.sync_supported && checked}
                        disabled={!method.sync_supported}
                        onCheckedChange={(value) => {
                          if (!method.sync_supported) return;
                          const nextChecked = value === true;
                          setForm((current) => {
                            const currentIds = new Set(
                              current.btcpayPaymentMethodIds,
                            );
                            if (nextChecked) {
                              currentIds.add(method.payment_method_id);
                            } else {
                              currentIds.delete(method.payment_method_id);
                            }
                            const nextIds = Array.from(currentIds);
                            const routeWallets = { ...current.btcpayRouteWallets };
                            if (nextChecked && !routeWallets[method.payment_method_id]) {
                              routeWallets[method.payment_method_id] =
                                walletForPaymentMethod(method.payment_method_id);
                            }
                            return {
                              ...current,
                              btcpayPaymentMethodIds: nextIds,
                              btcpayPaymentMethodId:
                                nextIds[0] ?? method.payment_method_id,
                              btcpayRouteWallets: routeWallets,
                            };
                          });
                          setFieldErrors((current) => {
                            if (!current.btcpayPaymentMethodId) return current;
                            const next = { ...current };
                            delete next.btcpayPaymentMethodId;
                            return next;
                          });
                          setBtcpayTestStatus(null);
                        }}
                      />
                      <span className="grid gap-0.5">
                        <span>{method.label}</span>
                        <span className="text-xs text-muted-foreground">
                          {method.payment_method_id}
                          {!method.sync_supported
                            ? t("add.btcpay.notSupportedYet")
                            : ""}
                        </span>
                      </span>
                    </label>
                  );
                })}
              </div>
            ) : form.btcpaySetupMode === "existing_wallets" ? (
              <div className="rounded-md border border-dashed border-border/70 p-3 text-sm text-muted-foreground">
                {t("add.btcpay.discoverToLoad")}
              </div>
            ) : (
              <Input
                id="connection-btcpay-payment-method"
                value={form.btcpayPaymentMethodId}
                onChange={(event) => {
                  updateForm("btcpayPaymentMethodId", event.target.value);
                  updateForm("btcpayPaymentMethodIds", [
                    event.target.value || DEFAULT_BTCPAY_PAYMENT_METHOD_ID,
                  ]);
                  updateForm("btcpayRouteWallets", {
                    [event.target.value || DEFAULT_BTCPAY_PAYMENT_METHOD_ID]:
                      walletForPaymentMethod(
                        event.target.value || DEFAULT_BTCPAY_PAYMENT_METHOD_ID,
                      ),
                  });
                  setBtcpayTestStatus(null);
                }}
                placeholder={DEFAULT_BTCPAY_PAYMENT_METHOD_ID}
              />
            )}
          </SetupField>
          {form.btcpaySetupMode === "existing_wallets" &&
          selectedBtcpayRoutes.length > 0 ? (
            <div className="space-y-3 rounded-md border border-border/70 p-3">
              <div>
                <p className="text-sm font-medium">
                  {t("add.btcpay.settlementMapping")}
                </p>
                <p className="text-xs text-muted-foreground">
                  {t("add.btcpay.settlementMappingHelper")}
                </p>
              </div>
              {selectedBtcpayRoutes.map((route) => (
                <div
                  key={route.paymentMethodId}
                  className="grid gap-3 md:grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)]"
                >
                  <div className="space-y-2">
                    <Label>{t("add.btcpay.paymentMethod")}</Label>
                    <div
                      className="flex h-9 min-w-0 items-center rounded-md border border-border/70 bg-muted/40 px-3 text-sm"
                      title={route.paymentMethodId}
                    >
                      <span className="truncate">{route.paymentMethodId}</span>
                    </div>
                  </div>
                  <SetupField
                    id={`connection-btcpay-route-${route.paymentMethodId}`}
                    label={t("add.btcpay.settlementWallet")}
                  >
                    <select
                      id={`connection-btcpay-route-${route.paymentMethodId}`}
                      className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
                      value={route.wallet}
                      onChange={(event) => {
                        setForm((current) => ({
                          ...current,
                          btcpayRouteWallets: {
                            ...current.btcpayRouteWallets,
                            [route.paymentMethodId]: event.target.value,
                          },
                        }));
                        setBtcpayTestStatus(null);
                      }}
                      disabled={existingWalletOptions.length === 0}
                    >
                      {existingWalletOptions.length === 0 ? (
                        <option value="">
                          {t("add.btcpay.noSettlementWalletsYet")}
                        </option>
                      ) : (
                        <option value="" disabled>
                          {t("add.btcpay.selectSettlementWallet")}
                        </option>
                      )}
                      {existingWalletOptions.map((wallet) => (
                        <option key={wallet.label} value={wallet.label}>
                          {wallet.label}
                          {wallet.chain ? ` (${wallet.chain})` : ""}
                        </option>
                      ))}
                    </select>
                  </SetupField>
                </div>
              ))}
            </div>
          ) : null}
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={
                testBtcpay.isPending ||
                !canDiscover ||
                !form.btcpayStoreId.trim() ||
                selectedBtcpayPaymentMethodIds.length === 0
              }
              onClick={async () => {
                setBtcpayTestStatus(null);
                try {
                  await testBtcpay.mutateAsync({
                    ...btcpayInstanceArgs(),
                    store_id: form.btcpayStoreId.trim(),
                    payment_method_id:
                      selectedBtcpayPaymentMethodIds[0] ||
                      DEFAULT_BTCPAY_PAYMENT_METHOD_ID,
                  });
                  setBtcpayTestStatus({
                    ok: true,
                    storeId: form.btcpayStoreId.trim(),
                    paymentMethodId:
                      selectedBtcpayPaymentMethodIds[0] ||
                      DEFAULT_BTCPAY_PAYMENT_METHOD_ID,
                  });
                } catch (error) {
                  setBtcpayTestStatus({
                    ok: false,
                    message:
                      error instanceof Error
                        ? error.message
                        : t("add.btcpay.testFailed"),
                  });
                }
              }}
            >
              {testBtcpay.isPending
                ? t("add.btcpay.testing")
                : t("add.btcpay.testConnection")}
            </Button>
            {btcpayTestStatus?.ok ? (
              <span className="text-xs text-emerald-700 dark:text-emerald-300">
                {t("add.btcpay.testResponded", {
                  store: btcpayTestStatus.storeId,
                  method: btcpayTestStatus.paymentMethodId,
                })}
              </span>
            ) : null}
            {btcpayTestStatus && !btcpayTestStatus.ok ? (
              <span className="text-xs text-destructive">
                {btcpayTestStatus.message}
              </span>
            ) : null}
          </div>
          {renderSyncAfterCreate(t("add.btcpay.refreshAfter"))}
        </>
      );
    }

    if (setupKind === "bip329") {
      return (
        <>
          <SetupField
            id="connection-bip329-file"
            label={t("add.bip329.labelFilePath")}
            error={fieldErrors.bip329File}
          >
            <div className="flex gap-2">
              <Input
                id="connection-bip329-file"
                value={form.bip329File}
                onChange={(event) => updateForm("bip329File", event.target.value)}
                required
              />
              {isFilePickerAvailable ? (
                <Button
                  type="button"
                  variant="outline"
                  onClick={async () => {
                    const picked = await pickFile({
                      title: t("add.bip329.selectLabelFileTitle"),
                      filters: [
                        {
                          name: t("add.bip329.labelFileFilter"),
                          extensions: ["jsonl", "json"],
                        },
                      ],
                    });
                    if (picked) updateForm("bip329File", picked);
                  }}
                >
                  {t("add.field.browse")}
                </Button>
              ) : null}
            </div>
          </SetupField>
          <SetupField
            id="connection-bip329-wallet"
            label={t("add.bip329.targetWalletLabel")}
            helper={t("add.bip329.targetWalletHelper")}
          >
            <Input
              id="connection-bip329-wallet"
              value={form.bip329Wallet}
              onChange={(event) => updateForm("bip329Wallet", event.target.value)}
            />
          </SetupField>
        </>
      );
    }

    if (setupKind === "backend-settings") {
      return (
        <div className="space-y-2 rounded-md border bg-background p-3 text-sm text-muted-foreground">
          <p>{t("add.backendSettings.line1")}</p>
          <p>{t("add.backendSettings.line2")}</p>
        </div>
      );
    }

    return (
      <div className="rounded-md border bg-background p-3 text-sm text-muted-foreground">
        {t("add.notWired")}
      </div>
    );
  };

  const renderSourceSummary = () => (
    <div className="space-y-4">
      <div className="space-y-2">
        <div className="flex items-center gap-3">
          <SourceArtwork source={selected} />
          <div className="min-w-0">
            <p className="truncate text-sm font-medium">{selected.title}</p>
            <p className="text-xs text-muted-foreground">
              {selected.pathLabel}
              {selected.formatLabel ? ` · ${selected.formatLabel}` : ""}
            </p>
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          <Badge variant={selected.status === "ready" ? "secondary" : "outline"}>
            {selected.status === "ready"
              ? t("add.available")
              : t("add.plannedLabel")}
          </Badge>
          {selected.docsHref ? (
            <a
              className="rounded-md border px-2 py-0.5 text-xs text-muted-foreground hover:text-foreground"
              href={selected.docsHref}
              target="_blank"
              rel="noreferrer"
            >
              {t("add.docs")}
            </a>
          ) : null}
        </div>
      </div>

      <ul className="space-y-2 text-sm text-muted-foreground">
        {selected.details.map((detail) => (
          <li key={detail} className="flex gap-2">
            <span className="mt-2 size-1 shrink-0 rounded-full bg-muted-foreground/60" />
            <span>{detail}</span>
          </li>
        ))}
      </ul>
    </div>
  );

  const formatImportCount = (value: number | null | undefined) =>
    (value ?? 0).toLocaleString("en-US");

  const importResultFromSyncResult = (
    result: SyncResult | undefined,
  ): ImportFileResult | null => {
    if (!result) return null;
    const hasImportShape =
      typeof result.imported === "number" ||
      typeof result.skipped === "number" ||
      typeof result.updated === "number" ||
      Array.isArray(result.inserted_records) ||
      Array.isArray(result.updated_records);
    if (!hasImportShape) return null;
    return {
      wallet: result.wallet,
      source: result.source,
      imported: result.imported ?? 0,
      skipped: result.skipped ?? 0,
      matched: result.matched,
      unchanged: result.unchanged,
      skipped_unmatched: result.skipped_unmatched,
      skipped_ambiguous: result.skipped_ambiguous,
      unmatched: result.unmatched,
      ambiguous: result.ambiguous,
      excluded: result.excluded,
      updated: result.updated,
      bullbitcoin_rows: result.bullbitcoin_rows,
      bullbitcoin_wallet_rows: result.bullbitcoin_wallet_rows,
      coinfinity_rows: result.coinfinity_rows,
      twentyonebitcoin_rows: result.twentyonebitcoin_rows,
      strike_rows: result.strike_rows,
      wasabi_transactions: result.wasabi_transactions,
      wasabi_coins_observed: result.wasabi_coins_observed,
      wasabi_coins_active: result.wasabi_coins_active,
      wasabi_coins_marked_spent: result.wasabi_coins_marked_spent,
      wasabi_payments_in_coinjoin: result.wasabi_payments_in_coinjoin,
      inserted_records: result.inserted_records,
      updated_records: result.updated_records,
      reconciliation_records: result.reconciliation_records,
    };
  };

  const renderImportSummary = () => {
    if (!lastImportResult) return null;
    const changedRecords = [
      ...(lastImportResult.inserted_records ?? []),
      ...(lastImportResult.updated_records ?? []),
    ];
    const shownRecords = changedRecords.slice(0, 5);
    const hiddenRecords = Math.max(0, changedRecords.length - shownRecords.length);
    const rowsRead =
      lastImportResult.bullbitcoin_rows ??
      lastImportResult.bullbitcoin_wallet_rows ??
      lastImportResult.coinfinity_rows ??
      lastImportResult.twentyonebitcoin_rows ??
      lastImportResult.strike_rows ??
      lastImportResult.wasabi_transactions ??
      lastImportResult.imported + lastImportResult.skipped;
    const isBookWide = lastImportResult.scope === "book";
    const changedCount =
      lastImportResult.imported + (lastImportResult.updated ?? 0);
    const metrics: Array<[string, number | null | undefined]> = [
      [t("add.summary.metric.rowsRead"), rowsRead],
      [t("add.summary.metric.inserted"), lastImportResult.imported],
      [t("add.summary.metric.updated"), lastImportResult.updated],
      [t("add.summary.metric.unchanged"), lastImportResult.unchanged],
    ];
    if (lastImportResult.matched !== undefined) {
      metrics.push([t("add.summary.metric.matched"), lastImportResult.matched]);
    }
    if (lastImportResult.wasabi_coins_observed !== undefined) {
      metrics.push([
        t("add.summary.metric.coinsObserved"),
        lastImportResult.wasabi_coins_observed,
      ]);
    }
    if (lastImportResult.wasabi_coins_active !== undefined) {
      metrics.push([
        t("add.summary.metric.activeCoins"),
        lastImportResult.wasabi_coins_active,
      ]);
    }
    if (lastImportResult.wasabi_payments_in_coinjoin !== undefined) {
      metrics.push([
        t("add.summary.metric.paymentsInCoinjoin"),
        lastImportResult.wasabi_payments_in_coinjoin,
      ]);
    }
    if (lastImportResult.skipped_unmatched !== undefined) {
      metrics.push([
        t("add.summary.metric.unmatched"),
        lastImportResult.skipped_unmatched,
      ]);
    }
    if (lastImportResult.unmatched !== undefined) {
      metrics.push([
        t("add.summary.metric.unmatched"),
        lastImportResult.unmatched,
      ]);
    }
    if (lastImportResult.skipped_ambiguous !== undefined) {
      metrics.push([
        t("add.summary.metric.ambiguous"),
        lastImportResult.skipped_ambiguous,
      ]);
    }
    if (lastImportResult.ambiguous !== undefined) {
      metrics.push([
        t("add.summary.metric.ambiguous"),
        lastImportResult.ambiguous,
      ]);
    }
    if (lastImportResult.excluded !== undefined) {
      metrics.push([
        t("add.summary.metric.excluded"),
        lastImportResult.excluded,
      ]);
    }
    return (
      <div className="space-y-3 rounded-md border bg-background p-3 text-sm">
        <div className="flex flex-wrap items-start justify-between gap-2">
          <div>
            <p className="font-medium">{t("add.summary.title")}</p>
            <p className="text-xs text-muted-foreground">
              {isBookWide
                ? t("add.summary.matchedBook")
                : t("add.summary.matchedWallet")}
            </p>
          </div>
          <Badge variant="secondary">
            {t("add.summary.changed", {
              value: formatImportCount(changedCount),
            })}
          </Badge>
        </div>
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
          {metrics.map(([label, value]) => (
            <div key={label} className="rounded-md border border-border/60 p-2">
              <p className="text-[11px] text-muted-foreground">{label}</p>
              <p className="font-medium tabular-nums">
                {formatImportCount(value as number | null | undefined)}
              </p>
            </div>
          ))}
        </div>
        {shownRecords.length > 0 ? (
          <div className="rounded-md border border-border/60">
            {shownRecords.map((record) => (
              <div
                key={record.transaction_id}
                className="space-y-1 border-b border-border/40 p-2.5 last:border-b-0"
              >
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <span className="min-w-0 truncate text-xs font-medium">
                    {record.wallet || t("add.summary.recordBook")} ·{" "}
                    {record.external_id || record.transaction_id}
                  </span>
                  <span className="text-xs text-muted-foreground">
                    {record.status ??
                      record.pricing_external_ref ??
                      t("add.summary.recordBullOrder")}
                  </span>
                </div>
                <p className="text-xs text-muted-foreground">
                  {record.matched_wallet
                    ? t("add.summary.matchedWalletRecord", {
                        wallet: record.matched_wallet,
                      })
                    : (record.changed_fields ?? []).length > 0
                      ? t("add.summary.changedFields", {
                          fields: (record.changed_fields ?? []).join(", "),
                        })
                      : t("add.summary.changedMetadata")}
                </p>
              </div>
            ))}
            {hiddenRecords > 0 ? (
              <div className="p-2.5 text-xs text-muted-foreground">
                {t("add.summary.moreUpdated", {
                  count: hiddenRecords,
                })}
              </div>
            ) : null}
          </div>
        ) : (
          <p className="rounded-md border border-border/60 p-2.5 text-xs text-muted-foreground">
            {t("add.summary.noChange")}
          </p>
        )}
      </div>
    );
  };

  const renderSourceStep = () => (
    <div className="grid min-h-0 grid-cols-1 overflow-hidden rounded-lg border lg:grid-cols-[190px_minmax(0,1fr)]">
      <div className="overflow-y-auto border-b bg-muted/30 p-2 lg:border-r lg:border-b-0">
        {CONNECTION_CATEGORIES.map((category) => {
          const Icon = category.icon;
          const active = activeCategory === category.id && !sourceQuery.trim();
          return (
            <button
              key={category.id}
              type="button"
              className={cn(
                "flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm transition-colors",
                active
                  ? "bg-background text-foreground shadow-sm"
                  : "text-muted-foreground hover:bg-background/70 hover:text-foreground",
              )}
              onClick={() => {
                setSourceQuery("");
                selectCategory(category.id);
              }}
            >
              <Icon className="size-4" aria-hidden="true" />
              {category.label}
            </button>
          );
        })}
      </div>

      <div className="grid min-h-0 grid-cols-1 lg:grid-cols-[minmax(0,1fr)_340px]">
        <div className="flex min-h-0 flex-col overflow-hidden border-b lg:border-r lg:border-b-0">
          <div className="flex flex-wrap items-center gap-2 border-b bg-muted/20 px-4 py-3">
            <Input
              type="search"
              value={sourceQuery}
              onChange={(event) => setSourceQuery(event.target.value)}
              placeholder={t("add.searchPlaceholder")}
              className="h-9 max-w-sm"
            />
          </div>
          <div className="min-h-0 flex-1 space-y-3 overflow-y-auto p-4">
          {visibleSources.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {sourceQuery.trim()
                ? t("add.noSearchMatch")
                : t("add.noFilterMatch")}
            </p>
          ) : null}
          {visibleSources.map((source) => {
            const selectedSource = selectedId === source.id;
            return (
              <button
                key={source.id}
                type="button"
                className={cn(
                  "flex w-full items-start gap-4 rounded-lg border p-4 text-left transition-colors hover:bg-muted/40",
                  selectedSource && "border-primary bg-primary/5",
                )}
                onClick={() => setSelectedId(source.id)}
              >
                <SourceArtwork source={source} />
                <span className="min-w-0 flex-1 space-y-1">
                  <span className="flex flex-wrap items-center gap-2">
                    <span className="font-medium">{source.title}</span>
                    <Badge
                      variant={source.status === "ready" ? "secondary" : "outline"}
                    >
                      {source.status === "ready"
                        ? t("add.ready")
                        : t("add.plannedLabel")}
                    </Badge>
                  </span>
                  <span className="block text-sm text-muted-foreground">
                    {source.description}
                  </span>
                  <span className="block text-xs text-muted-foreground">
                    {source.pathLabel}
                    {source.formatLabel ? ` · ${source.formatLabel}` : ""}
                  </span>
                </span>
              </button>
            );
          })}
          </div>
        </div>

        <div className="min-h-0 overflow-y-auto bg-muted/20 p-4">
          {renderSourceSummary()}
        </div>
      </div>
    </div>
  );

  const renderSetupStep = () => (
    <div className="grid min-h-0 grid-cols-1 overflow-hidden rounded-lg border lg:grid-cols-[310px_minmax(0,1fr)]">
      <div className="min-h-0 overflow-y-auto border-b bg-muted/20 p-5 lg:border-r lg:border-b-0">
        {renderSourceSummary()}
      </div>
      <div className="min-h-0 overflow-y-auto p-5">
        <form
          id="connection-setup-form"
          className="mx-auto max-w-xl space-y-4"
          onSubmit={onSetupSubmit}
        >
          {renderSetupFields()}
          {syncProgress ? (
            <div className="space-y-1 rounded-md border bg-background p-3 text-xs">
              <div className="flex items-center justify-between">
                <span className="text-muted-foreground">
                  {t("add.progress.importing", { wallet: syncProgress.wallet })}
                </span>
                <span className="font-medium tabular-nums">
                  {t("add.progress.rows", {
                    processed: syncProgress.processed.toLocaleString(),
                    total: syncProgress.total.toLocaleString(),
                  })}
                </span>
              </div>
              <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                <div
                  className="h-full bg-primary transition-all"
                  style={{
                    width:
                      syncProgress.total > 0
                        ? `${Math.min(
                            100,
                            (syncProgress.processed / syncProgress.total) * 100,
                          )}%`
                        : "0%",
                  }}
                />
              </div>
            </div>
          ) : null}
          {setupError ? (
            <div className="rounded-md border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">
              {setupError}
            </div>
          ) : null}
          {renderImportSummary()}
        </form>
      </div>
    </div>
  );

  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent className="grid h-[calc(100dvh-2rem)] max-h-[calc(100dvh-2rem)] grid-rows-[auto_minmax(0,1fr)_auto] sm:h-[740px] sm:max-w-[960px] lg:max-w-[1040px]">
          <DialogHeader>
            <DialogTitle>
              {isSetupStep
                ? t("add.setupTitle", { title: selected.title })
                : t("add.title")}
            </DialogTitle>
            <DialogDescription>
              {isSetupStep
                ? t("add.setupDescription")
                : t("add.description")}
            </DialogDescription>
          </DialogHeader>

          {isSetupStep ? renderSetupStep() : renderSourceStep()}

          <DialogFooter className="gap-2 sm:justify-between">
            {isSetupStep ? (
              <Button
                type="button"
                variant="outline"
                onClick={() => setStep("source")}
                disabled={isSubmitting}
              >
                {t("add.back")}
              </Button>
            ) : (
              <span />
            )}
            <div className="flex flex-col-reverse gap-2 sm:flex-row">
              <Button
                type="button"
                variant="outline"
                onClick={() => onOpenChange(false)}
                disabled={isSubmitting}
              >
                {t("common:actions.cancel")}
              </Button>
              {isSetupStep ? (
                <Button
                  type="submit"
                  form="connection-setup-form"
                  disabled={
                    isSubmitting ||
                    setupKind === "planned" ||
                    missingBackend ||
                    missingBtcpayMappingDiscovery
                  }
                >
                  {submitLabel}
                </Button>
              ) : (
                <Button
                  type="button"
                  disabled={!canContinue}
                  onClick={() => setStep("setup")}
                >
                  {selected.status === "ready"
                    ? t("add.continue")
                    : t("add.plannedLabel")}
                </Button>
              )}
            </div>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      {scannerOpen ? (
        <React.Suspense
          fallback={
            <div className="fixed inset-0 z-50 flex items-center justify-center bg-background/80">
              <div className="flex items-center gap-2 rounded-md border bg-background px-3 py-2 text-sm shadow-sm">
                <Loader2 className="size-4 animate-spin" />
                <span>{t("add.openingScanner")}</span>
              </div>
            </div>
          }
        >
          <WalletMaterialScannerDialog
            open={scannerOpen}
            onOpenChange={setScannerOpen}
            title={t("scanner.scanTitle", { title: selected.title })}
            onMaterialScanned={(material) => {
              updateForm("walletMaterial", material);
              setPreviewAddresses(null);
              setPreviewError(null);
            }}
          />
        </React.Suspense>
      ) : null}
    </>
  );
}
