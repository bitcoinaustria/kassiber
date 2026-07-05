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
import { GenericLedgerPreview } from "@/components/kb/GenericLedgerPreview";
import { saveDaemonExport } from "@/lib/exportFile";
import { parseAddressList, stripKeyMaterial } from "@/lib/addressList";
import {
  isFilePickerAvailable,
  pickFile,
  pickFileWithContentsBase64,
} from "@/lib/filePicker";
import {
  buildSamouraiSourceSet,
  type SamouraiSection,
} from "@/lib/samouraiSourceSet";
import {
  buildWasabiBundle,
  type WasabiImportMode,
} from "@/lib/wasabiBundle";
import {
  BARE_XPUB_SCRIPT_TYPES,
  type BareXpubScriptType,
  detectWalletMaterial,
  scriptTypesFromDetectionPayload,
} from "@/lib/walletMaterialFormat";
import {
  regtestBackendConnections,
} from "@/components/kb/backendConnectionRows";
import { PENDING_SETTINGS_BACKEND_EDIT_KEY } from "./settingsSections";
import {
  backendRowToSettingsBackend,
  type BackendSettingsData,
} from "./settings/SettingsModel";


async function fileToBase64(file: File): Promise<string> {
  const buffer = await file.arrayBuffer();
  let binary = "";
  const bytes = new Uint8Array(buffer);
  const chunkSize = 0x8000;
  for (let index = 0; index < bytes.length; index += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(index, index + chunkSize));
  }
  return window.btoa(binary);
}

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
  birthday: string;
  coreRpcUrl: string;
  coreRpcAuthMode: "cookiefile" | "basic";
  coreRpcCookiefile: string;
  coreRpcUsername: string;
  coreRpcPassword: string;
  coreRpcCredentialRef: string;
  coreRpcNetwork: string;
  btcpayInstanceMode: "saved" | "new";
  btcpaySetupMode: "wallet_sources" | "existing_wallets";
  bullWalletSetupMode: "wallet_sources" | "existing_wallets";
  bullWalletNetworks: BullBitcoinWalletNetwork[];
  bullWalletRouteWallets: Record<BullBitcoinWalletNetwork, string>;
  btcpayInstanceLabel: string;
  btcpayServerUrl: string;
  btcpayApiKey: string;
  walletMaterial: string;
  descriptorScriptType: string;
  spDescriptor: string;
  spScanMode: "local_index" | "server_assisted";
  spScanStartHeight: string;
  spScanStartDate: string;
  spFullHistory: boolean;
  spAcknowledgeFullHistoryWarning: boolean;
  spAcknowledgeServerWarning: boolean;
  addressList: string;
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
  bip329ExportMode: "stored" | "synthesized" | "all";
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

type GenericLedgerPreviewSource = {
  filename: string;
  sourceBytesBase64: string;
  importable: boolean;
};

interface BackendOption {
  name: string;
  display_name?: string;
  kind: string;
  chain?: string;
  network?: string;
  is_default?: boolean;
  silent_payments?: boolean;
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

interface CoreDetectionCandidate {
  url: string;
  chain?: string | null;
  network?: string | null;
  auth_source?: string | null;
  credential_source?: string | null;
  credential_ref?: string | null;
  cookiefile?: string | null;
  blocks?: number | null;
  headers?: number | null;
  peers?: number | null;
  status?: string | null;
  pruned?: boolean | null;
  ibd?: boolean | null;
  wallet_rpc?: CoreCapabilityPayload | null;
  block_filters?: CoreCapabilityPayload | null;
  warnings?: string[];
}

interface CoreDetectData {
  candidates: CoreDetectionCandidate[];
}

interface CoreCapabilityPayload {
  available?: boolean;
  error?: {
    message?: string;
    hint?: string;
  };
}

interface CoreProbeData {
  reachable: boolean;
  chain?: string | null;
  network?: string | null;
  blocks?: number | null;
  headers?: number | null;
  peers?: number | null;
  status?: string | null;
  pruned?: boolean | null;
  pruneheight?: number | null;
  version?: number | null;
  ibd?: boolean | null;
  wallet_rpc?: CoreCapabilityPayload | null;
  block_filters?: CoreCapabilityPayload | null;
  warnings?: string[];
  error?: {
    message?: string;
    hint?: string;
  };
}

function backendOptionLabel(backend: BackendOption): string {
  const label = backend.display_name?.trim() || backend.name;
  return label === backend.name ? label : `${label} (${backend.name})`;
}

function backendNameFromLabel(label: string) {
  const slug = label
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return slug || "bitcoin-core";
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

type Bip329MatchStatus = "exact" | "ambiguous" | "unmatched" | "preserved";

interface Bip329PreviewResult {
  file: string;
  records: number;
  counts: {
    exact: number;
    ambiguous: number;
    unmatched: number;
    preserved: number;
    conflicts: number;
    duplicate_refs: number;
    duplicate_records: number;
    tag_additions: number;
    tag_unchanged: number;
    tag_skipped_ambiguous: number;
    tag_skipped_duplicate: number;
    tag_skipped_label_too_long: number;
  };
  warnings?: string[];
  apply_policy: string;
  rows?: Array<{
    line: number;
    type: string;
    ref: string;
    ref_preview: string;
    ref_redacted: boolean;
    label: string;
    match_status: Bip329MatchStatus;
    wallets: string[];
    conflicts: string[];
    duplicate: boolean;
    tag_effects?: Array<{ action: string }>;
  }>;
}

interface SamouraiImportResult {
  group: { label: string };
  children: Array<{ label: string }>;
  warnings?: Array<{ code: string; message: string }>;
}

type DialogStep = "source" | "setup";
const DESCRIPTOR_BACKEND_KINDS = new Set(["esplora", "electrum", "bitcoinrpc"]);
const ADDRESS_BACKEND_KINDS = new Set([
  "esplora",
  "electrum",
  "bitcoinrpc",
]);
const CORE_DEFAULT_RPC_URLS: Record<string, string> = {
  main: "http://127.0.0.1:8332",
  test: "http://127.0.0.1:18332",
  signet: "http://127.0.0.1:38332",
  regtest: "http://127.0.0.1:18443",
};
const DEFAULT_BTCPAY_PAYMENT_METHOD_ID = "BTC-CHAIN";
const MAX_DESCRIPTOR_GAP_LIMIT = 5000;
const CONNECTION_SOURCE_ALIASES: Record<string, string> = {
  xpub: "descriptor",
};
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

function supportsAddressListSync(backend: BackendOption) {
  return ADDRESS_BACKEND_KINDS.has(backend.kind);
}

function isExchangeEvidenceFormat(sourceFormat?: string) {
  return (
    sourceFormat === "bullbitcoin_csv" ||
    sourceFormat === "coinfinity_csv" ||
    sourceFormat === "pocketbitcoin_csv" ||
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
  if (source.sourceFormat === "pocketbitcoin_csv") {
    return [{ name: t("add.fileFilter.pocketbitcoinCsv"), extensions: ["csv"] }];
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
  if (source.sourceFormat === "generic_ledger") {
    return [
      {
        name: t("add.fileFilter.genericLedger"),
        extensions: ["xlsx", "csv", "tsv"],
      },
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
    birthday: "",
    coreRpcUrl: CORE_DEFAULT_RPC_URLS.main,
    coreRpcAuthMode: "cookiefile",
    coreRpcCookiefile: "",
    coreRpcUsername: "",
    coreRpcPassword: "",
    coreRpcCredentialRef: "",
    coreRpcNetwork: source.network ?? "main",
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
    descriptorScriptType: "",
    spDescriptor: "",
    spScanMode: "local_index",
    spScanStartHeight: "",
    spScanStartDate: "",
    spFullHistory: false,
    spAcknowledgeFullHistoryWarning: false,
    spAcknowledgeServerWarning: false,
    addressList: "",
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
    bip329ExportMode: "stored",
    syncAfterCreate:
      source.setupKind === "file-wallet" ||
      source.setupKind === "bullbitcoin-wallet" ||
      source.setupKind === "address-list",
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

function coreNodeStatusKey(status?: string | null) {
  switch (status) {
    case "synchronized":
    case "synchronizing":
    case "connecting":
    case "unresponsive":
      return status;
    default:
      return "unknown";
  }
}

function coreCandidateAuthLabel(
  candidate: CoreDetectionCandidate,
  t: TFunction<readonly ["connections", "common"]>,
) {
  if (
    candidate.auth_source === "basic" &&
    candidate.credential_source === "bitcoin.conf"
  ) {
    return t("connections:add.core.authBasicBitcoinConf");
  }
  if (candidate.auth_source === "basic") {
    return t("connections:add.core.authBasic");
  }
  if (candidate.credential_source === "bitcoin.conf") {
    return t("connections:add.core.authCookiefileBitcoinConf");
  }
  if (candidate.auth_source === "cookiefile") {
    return t("connections:add.core.authCookiefile");
  }
  return t("connections:add.core.credentialUnknown");
}

function applyCoreCandidateToForm(
  current: SetupFormState,
  candidate: CoreDetectionCandidate,
): SetupFormState {
  const authMode =
    candidate.auth_source === "basic"
      ? "basic"
      : candidate.auth_source === "cookiefile"
        ? "cookiefile"
        : current.coreRpcAuthMode;
  return {
    ...current,
    coreRpcUrl: candidate.url || current.coreRpcUrl,
    coreRpcNetwork: candidate.network || current.coreRpcNetwork,
    coreRpcAuthMode: authMode,
    coreRpcCookiefile: candidate.cookiefile || current.coreRpcCookiefile,
    coreRpcUsername:
      candidate.auth_source === "basic" ? "" : current.coreRpcUsername,
    coreRpcPassword:
      candidate.auth_source === "basic" ? "" : current.coreRpcPassword,
    coreRpcCredentialRef: candidate.credential_ref || "",
  };
}

function coreReadinessMessages(
  payload: CoreDetectionCandidate | CoreProbeData,
  t: TFunction<readonly ["connections", "common"]>,
) {
  const messages: string[] = [];
  if (payload.pruned) {
    messages.push(t("connections:add.core.prunedWarning"));
  }
  if (payload.ibd) {
    messages.push(t("connections:add.core.initialBlockDownloadWarning"));
  }
  if (payload.wallet_rpc?.available === false) {
    messages.push(
      payload.wallet_rpc.error?.hint ??
        payload.wallet_rpc.error?.message ??
        t("connections:add.core.walletRpcUnavailable"),
    );
  }
  if (payload.block_filters?.available === false) {
    messages.push(
      payload.block_filters.error?.hint ??
        t("connections:add.core.blockFiltersUnavailable"),
    );
  }
  return messages;
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
        "flex shrink-0 items-center justify-center rounded-lg border border-border/70 bg-muted/60 p-1 shadow-sm shadow-zinc-950/5 dark:border-border/80 dark:bg-muted/55 dark:shadow-black/20",
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
  const { t } = useTranslation(["connections", "common"]);
  const navigate = useNavigate();
  const addNotification = useUiStore((state) => state.addNotification);
  const setDeferredConnectionSetup = useUiStore(
    (state) => state.setDeferredConnectionSetup,
  );
  const backendOptions = useDaemon<BackendOptionsData>("ui.backends.options");
  const backendSettingsQuery = useDaemon<BackendSettingsData>(
    "ui.backends.settings.list",
  );
  const walletsList = useDaemon<WalletListData>("ui.wallets.list");
  const createWallet =
    useDaemonMutation<{ wallet: { label: string } }>("ui.wallets.create");
  const createBackend = useDaemonMutation<{ name: string }>(
    "ui.backends.create",
  );
  const importFile =
    useDaemonMutation<ImportFileResult>("ui.wallets.import_file");
  const ledgerTemplate = useDaemonMutation<{
    file: string;
    filename: string;
    format: string;
  }>("ui.transactions.ledger_template");
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
  const previewBip329 = useDaemonMutation<Bip329PreviewResult>(
    "ui.metadata.bip329.preview",
  );
  const importBip329 = useDaemonMutation<{
    records: number;
    transaction_tags_added: number;
    preview?: Pick<Bip329PreviewResult, "counts" | "apply_policy">;
  }>("ui.metadata.bip329.import");
  const exportBip329 = useDaemonMutation<{
    file: string;
    filename: string;
    exported: number;
    exported_stored: number;
    exported_synthesized: number;
    mode: string;
    wallet: string;
  }>("ui.metadata.bip329.export");
  const previewDescriptor = useDaemonMutation<{
    chain: string;
    network: string;
    addresses: {
      // "receive"/"change" for a single descriptor, or a script-type-qualified
      // label like "p2tr receive" when an xpub watches several script types.
      branch: string;
      index: number;
      address: string;
      derivation_path?: string | null;
    }[];
    has_change_branch: boolean;
  }>("ui.wallets.preview_descriptor");
  const detectScriptTypes = useDaemonMutation<{
    probed: boolean;
    detected: { script_type: string; has_history: boolean }[];
    active: string[];
    fallback_used: boolean;
    reason?: string | null;
  }>("ui.wallets.detect_script_types");
  const testBtcpay = useDaemonMutation<{
    backend: string;
    store_id: string;
    payment_method_id: string;
    ok: boolean;
  }>("ui.connections.btcpay.test");
  const detectCore = useDaemonMutation<CoreDetectData>("ui.backends.detect_core");
  const testCore = useDaemonMutation<CoreProbeData>(
    "ui.backends.bitcoinrpc.test",
  );
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
  const [selectedId, setSelectedId] = React.useState("descriptor");
  const [sourceQuery, setSourceQuery] = React.useState("");
  const [step, setStep] = React.useState<DialogStep>("source");
  const [form, setForm] = React.useState(() =>
    formDefaultsFor(CONNECTION_SOURCES[0], t),
  );
  const [purgedKeys, setPurgedKeys] = React.useState<{
    privateKeys: number;
    publicKeys: number;
  } | null>(null);
  // Parse once per input change, not once per render — an address-list paste can
  // hold thousands of entries and this drives the live summary, validation, and
  // submit.
  const parsedAddressList = React.useMemo(
    () => parseAddressList(form.addressList),
    [form.addressList],
  );
  const [setupError, setSetupError] = React.useState<string | null>(null);
  const [lastImportResult, setLastImportResult] =
    React.useState<ImportFileResult | null>(null);
  const [genericLedgerPreviewBlocksSubmit, setGenericLedgerPreviewBlocksSubmit] =
    React.useState(false);
  const [genericLedgerPreviewSource, setGenericLedgerPreviewSource] =
    React.useState<GenericLedgerPreviewSource | null>(null);
  const [bip329Preview, setBip329Preview] =
    React.useState<Bip329PreviewResult | null>(null);
  const [fieldErrors, setFieldErrors] = React.useState<
    Partial<Record<keyof SetupFormState, string>>
  >({});
  const [previewAddresses, setPreviewAddresses] = React.useState<
    { branch: string; index: number; address: string }[] | null
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
  const [coreDetection, setCoreDetection] =
    React.useState<CoreDetectData | null>(null);
  const [coreTestStatus, setCoreTestStatus] = React.useState<
    | ({ ok: true } & CoreProbeData)
    | { ok: false; message: string; hint?: string | null }
    | null
  >(null);
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
  const bitcoinAddressBackends = allBackends.filter(
    (backend) =>
      supportsAddressListSync(backend) &&
      (!backend.chain || backend.chain === "bitcoin"),
  );
  const silentPaymentBackendOptions = allBackends.filter(
    (backend) =>
      backend.silent_payments === true &&
      (!backend.chain || backend.chain === "bitcoin") &&
      (form.spScanMode !== "server_assisted" || backend.kind !== "electrum"),
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
  const addressBackendOptions =
    selected.chain === "bitcoin" ? bitcoinAddressBackends : descriptorBackendOptions;
  const selectedBackendOptions =
    setupKind === "address-list"
      ? addressBackendOptions
      : setupKind === "silent-payment"
        ? silentPaymentBackendOptions
        : descriptorBackendOptions;
  const selectedBackend = selectedBackendOptions.find(
    (backend) => backend.name === form.backend,
  );
  const configuredLocalBackends = React.useMemo(
    () =>
      regtestBackendConnections(
        backendSettingsQuery.data?.data?.backends.map(
          backendRowToSettingsBackend,
        ) ?? [],
      ),
    [backendSettingsQuery.data?.data?.backends],
  );
  const selectedBackendOptionKey = selectedBackendOptions
    .map((backend) => backend.name)
    .join("\0");
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
    createBackend.isPending ||
    createBtcpay.isPending ||
    createBullBitcoinWallet.isPending ||
    discoverBtcpay.isPending ||
    detectCore.isPending ||
    testCore.isPending ||
    previewBip329.isPending ||
    importBip329.isPending ||
    exportBip329.isPending ||
    syncWallet.isPending;
  const requiresBackend =
    setupKind === "descriptor" ||
    setupKind === "samourai" ||
    setupKind === "silent-payment" ||
    setupKind === "address-list";
  const missingBackend = requiresBackend && selectedBackendOptions.length === 0;
  const submitLabel =
    setupKind === "backend-settings" && selected.id !== "bitcoin-core"
      ? t("add.submit.openBackendSettings")
      : syncWallet.isPending
        ? t("add.submit.refreshing")
        : importSamourai.isPending
          ? t("add.submit.importingSamourai")
        : importFile.isPending
          ? t("add.submit.importing")
        : previewBip329.isPending
          ? t("add.submit.previewingLabels")
        : importBip329.isPending
          ? t("add.submit.importingLabels")
        : setupKind === "bip329" && !bip329Preview
          ? t("add.submit.previewLabels")
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
    setGenericLedgerPreviewBlocksSubmit(false);
    setGenericLedgerPreviewSource(null);
    setBip329Preview(null);
    setPreviewAddresses(null);
    setPreviewError(null);
    setBtcpayTestStatus(null);
    setBtcpayDiscovery(null);
    setCoreDetection(null);
    setCoreTestStatus(null);
    setSyncProgress(null);
    setPurgedKeys(null);
  }, [selected, t]);

  React.useEffect(() => {
    if (!open) return;
    const resolvedSourceId = initialSourceId
      ? CONNECTION_SOURCE_ALIASES[initialSourceId] ?? initialSourceId
      : null;
    const requestedSource = CONNECTION_SOURCES.find(
      (candidate) => candidate.id === resolvedSourceId,
    );
    const source = requestedSource ?? CONNECTION_SOURCES[0];
    setActiveCategory(source.category);
    setSelectedId(source.id);
    setStep(requestedSource && source.status === "ready" ? "setup" : "source");
    setSetupError(null);
    setLastImportResult(null);
    setGenericLedgerPreviewBlocksSubmit(false);
    setGenericLedgerPreviewSource(null);
    setBip329Preview(null);
    setSourceQuery("");
  }, [initialSourceId, open]);

  React.useEffect(() => {
    if (open) return;
    setScannerOpen(false);
    setForm(formDefaultsFor(selected, t));
    setFieldErrors({});
    setSetupError(null);
    setLastImportResult(null);
    setGenericLedgerPreviewBlocksSubmit(false);
    setGenericLedgerPreviewSource(null);
    setBip329Preview(null);
    setPreviewAddresses(null);
    setPreviewError(null);
    setPurgedKeys(null);
  }, [open, selected, t]);

  React.useEffect(() => {
    if (!defaultBackendName) return;
    if (
      setupKind !== "descriptor" &&
      setupKind !== "samourai" &&
      setupKind !== "address-list" &&
      setupKind !== "silent-payment"
    )
      return;
    setForm((current) => {
      if (setupKind !== "silent-payment") {
        return current.backend
          ? current
          : { ...current, backend: defaultBackendName };
      }
      const backendStillAvailable = selectedBackendOptions.some(
        (backend) => backend.name === current.backend,
      );
      return backendStillAvailable
        ? current
        : { ...current, backend: defaultBackendName };
    });
  }, [defaultBackendName, selectedBackendOptionKey, setupKind]);

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
    if (key === "sourceFile" && selected.sourceFormat === "generic_ledger") {
      setGenericLedgerPreviewBlocksSubmit(String(value ?? "").trim().length > 0);
      setGenericLedgerPreviewSource(null);
    }
    if (key === "bip329File") {
      setBip329Preview(null);
    }
    if (
      key === "sourceFile" ||
      key === "bip329File" ||
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

  const applyAddressInput = (rawText: string) => {
    const stripped = stripKeyMaterial(rawText);
    const removed = stripped.privateKeys + stripped.publicKeys > 0;
    // Only reflow to the scrubbed text when key material was actually removed,
    // so normal typing/formatting of addresses is left undisturbed.
    setForm((current) => ({
      ...current,
      addressList: removed ? stripped.text : rawText,
    }));
    if (removed) {
      setPurgedKeys((prev) => ({
        privateKeys: (prev?.privateKeys ?? 0) + stripped.privateKeys,
        publicKeys: (prev?.publicKeys ?? 0) + stripped.publicKeys,
      }));
    } else if (!rawText.trim()) {
      setPurgedKeys(null);
    }
    setFieldErrors((current) => {
      if (!("addressList" in current)) return current;
      const next = { ...current };
      delete next.addressList;
      return next;
    });
  };

  const addressFileInputRef = React.useRef<HTMLInputElement | null>(null);

  const handleAddressFile = async (
    event: React.ChangeEvent<HTMLInputElement>,
  ) => {
    const input = event.target;
    const files = Array.from(input.files ?? []);
    input.value = ""; // allow re-picking the same file
    if (files.length === 0) return;
    try {
      const texts = await Promise.all(files.map((file) => file.text()));
      const loaded = texts.join("\n").trim();
      if (!loaded) return;
      const existing = form.addressList.trim();
      applyAddressInput(existing ? `${existing}\n${loaded}` : loaded);
    } catch (error) {
      setSetupError(
        error instanceof Error
          ? error.message
          : t("add.addressList.fileReadFailed"),
      );
    }
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

  const openConfiguredBackendSettings = (
    backendId: string,
    settingsHash?: string,
  ) => {
    window.sessionStorage.setItem(PENDING_SETTINGS_BACKEND_EDIT_KEY, backendId);
    onOpenChange(false);
    void navigate({ to: "/settings", hash: settingsHash ?? "bitcoin" });
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

  const coreRpcConfig = () => {
    const config: Record<string, string> = {
      display_name: form.label.trim(),
    };
    if (form.coreRpcAuthMode === "cookiefile") {
      config.cookiefile = form.coreRpcCookiefile.trim();
    } else {
      config.username = form.coreRpcUsername.trim();
      config.password = form.coreRpcPassword;
    }
    return config;
  };

  const updateCoreForm = <K extends keyof SetupFormState>(
    key: K,
    value: SetupFormState[K],
  ) => {
    setForm((current) => ({
      ...current,
      [key]: value,
      coreRpcCredentialRef: "",
    }));
    setCoreTestStatus(null);
    setFieldErrors((current) => {
      if (!(key in current)) return current;
      const next = { ...current };
      delete next[key];
      return next;
    });
  };

  const coreRpcBackendArgs = () => {
    const args: Record<string, unknown> = {
      name: backendNameFromLabel(form.label),
      kind: "bitcoinrpc",
      url: form.coreRpcUrl.trim(),
      chain: "bitcoin",
      network: form.coreRpcNetwork || "main",
      config: coreRpcConfig(),
    };
    if (form.coreRpcCredentialRef) {
      args.credential_ref = form.coreRpcCredentialRef;
    }
    return args;
  };

  const coreRpcProbeArgs = () =>
    form.coreRpcCredentialRef
      ? { credential_ref: form.coreRpcCredentialRef, timeout: 10 }
      : {
          url: form.coreRpcUrl.trim(),
          network: form.coreRpcNetwork || "main",
          config: coreRpcConfig(),
          timeout: 10,
        };

  const validateSetupForm = (): Partial<Record<keyof SetupFormState, string>> => {
    const errors: Partial<Record<keyof SetupFormState, string>> = {};
    if (
      setupKind === "descriptor" ||
      setupKind === "silent-payment" ||
      setupKind === "address-list" ||
      setupKind === "file-wallet" ||
      setupKind === "samourai" ||
      setupKind === "btcpay" ||
      setupKind === "bullbitcoin-wallet" ||
      (setupKind === "backend-settings" && selected.id === "bitcoin-core")
    ) {
      if (!form.label.trim()) {
        errors.label = t("add.validation.labelRequired");
      }
    }
    if (setupKind === "address-list") {
      if (parsedAddressList.valid.length === 0) {
        errors.addressList = t("add.addressList.errorNeedAddress");
      }
      if (addressBackendOptions.length > 0 && !form.backend.trim()) {
        errors.backend = t("add.validation.chooseBackend");
      }
    }
    if (setupKind === "descriptor") {
      if (!form.walletMaterial.trim()) {
        errors.walletMaterial = t("add.validation.pasteWalletMaterial");
      } else {
        const detection = detectWalletMaterial(form.walletMaterial);
        if (detection.kind === "unknown") {
          errors.walletMaterial = detection.hint ?? detection.label;
        }
        // A bare xpub no longer needs a manual pick: an empty script-type
        // selection means "detect automatically" at submit time.
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
      if (form.birthday && Number.isNaN(Date.parse(form.birthday))) {
        errors.birthday = t("add.validation.birthdayInvalid");
      }
    }
    if (setupKind === "backend-settings" && selected.id === "bitcoin-core") {
      if (!form.coreRpcUrl.trim()) {
        errors.coreRpcUrl = t("add.core.errorUrl");
      }
      if (
        form.coreRpcAuthMode === "cookiefile" &&
        !form.coreRpcCookiefile.trim() &&
        !form.coreRpcCredentialRef
      ) {
        errors.coreRpcCookiefile = t("add.core.errorCookiefile");
      }
      if (form.coreRpcAuthMode === "basic" && !form.coreRpcCredentialRef) {
        if (!form.coreRpcUsername.trim()) {
          errors.coreRpcUsername = t("add.core.errorUsername");
        }
        if (!form.coreRpcPassword.trim()) {
          errors.coreRpcPassword = t("add.core.errorPassword");
        }
      }
    }
    if (setupKind === "silent-payment") {
      if (!form.spDescriptor.trim()) {
        errors.spDescriptor = t("add.silentPayments.errorMaterial");
      }
      if (!form.backend.trim()) {
        errors.backend = t("add.validation.chooseBackend");
      }
      const startHeightText = form.spScanStartHeight.trim();
      const hasStartHeight = startHeightText.length > 0;
      const hasStartDate = form.spScanStartDate.trim().length > 0;
      if (hasStartHeight) {
        const startHeight = Number.parseInt(startHeightText, 10);
        if (
          !Number.isFinite(startHeight) ||
          startHeight < 0 ||
          String(startHeight) !== startHeightText
        ) {
          errors.spScanStartHeight = t(
            "add.silentPayments.errorStartHeight",
          );
        }
      }
      if (!hasStartHeight && !hasStartDate && !form.spFullHistory) {
        errors.spScanStartHeight = t("add.silentPayments.errorStartPoint");
      }
      if (
        form.spFullHistory &&
        !form.spAcknowledgeFullHistoryWarning
      ) {
        errors.spAcknowledgeFullHistoryWarning = t(
          "add.silentPayments.errorFullHistoryAck",
        );
      }
      if (
        form.spScanMode === "server_assisted" &&
        !form.spAcknowledgeServerWarning
      ) {
        errors.spAcknowledgeServerWarning = t(
          "add.silentPayments.errorServerAck",
        );
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
      } else if (
        selected.sourceFormat === "generic_ledger" &&
        genericLedgerPreviewBlocksSubmit
      ) {
        errors.sourceFile = t("add.genericLedger.preview.submitBlocked");
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
      if (setupKind === "backend-settings" && selected.id === "bitcoin-core") {
        const backendArgs = coreRpcBackendArgs();
        const testEnvelope = await testCore.mutateAsync(coreRpcProbeArgs());
        const probe = testEnvelope.data;
        if (!probe?.reachable) {
          throw new Error(
            probe?.error?.message ?? t("add.core.testFailed"),
          );
        }
        if (probe.wallet_rpc?.available === false) {
          throw new Error(
            probe.wallet_rpc.error?.hint ??
              probe.wallet_rpc.error?.message ??
              t("add.core.walletRpcUnavailable"),
          );
        }
        await createBackend.mutateAsync(backendArgs);
        addNotification({
          title: t("add.added.title"),
          body: t("add.core.addedBody", { label }),
          tone: "success",
        });
      } else if (setupKind === "backend-settings") {
        openBackendSettings();
        return;
      } else if (setupKind === "descriptor") {
        const gapLimit = Number.parseInt(form.gapLimit, 10);
        const isBareXpub =
          detectWalletMaterial(form.walletMaterial).kind === "bare-xpub";
        // A bare xpub watches the manually pinned type, or — when left on
        // "Detect automatically" — whichever types the backend shows history
        // for (the daemon falls back to Native SegWit if none / unreachable).
        let scriptTypes: string[] | undefined;
        if (isBareXpub) {
          if (form.descriptorScriptType) {
            scriptTypes = [form.descriptorScriptType];
          } else {
            const detected = await detectScriptTypes.mutateAsync({
              wallet_material: form.walletMaterial.trim(),
              backend: form.backend.trim() || undefined,
              chain: selected.chain,
              network: selected.network,
            });
            scriptTypes = requireAutoDetectedScriptTypes(detected.data);
          }
        }
        await createWallet.mutateAsync({
          label,
          kind: selected.walletKind ?? "descriptor",
          backend: form.backend.trim() || undefined,
          chain: selected.chain,
          network: selected.network,
          wallet_material: form.walletMaterial.trim(),
          script_types: scriptTypes,
          gap_limit: Number.isFinite(gapLimit) ? gapLimit : undefined,
          birthday: form.birthday || undefined,
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
      } else if (setupKind === "address-list") {
        const parsed = parsedAddressList;
        await createWallet.mutateAsync({
          label,
          kind: selected.walletKind ?? "address",
          backend: form.backend.trim() || undefined,
          chain: selected.chain,
          network: selected.network,
          addresses: parsed.valid,
        });
        if (form.syncAfterCreate) {
          startSyncNotice(t("add.addressList.stillScanning", { label }));
          try {
            await syncWallet.mutateAsync({ wallet: label });
          } finally {
            clearSyncNotice();
          }
        }
        addNotification({
          title: t("add.added.title"),
          body: t("add.addressList.addedBody", {
            label,
            count: parsed.valid.length,
          }),
          tone: "success",
        });
      } else if (setupKind === "silent-payment") {
        const startHeight = Number.parseInt(form.spScanStartHeight.trim(), 10);
        const fullHistory = form.spFullHistory;
        await createWallet.mutateAsync({
          label,
          kind: "silent-payment",
          backend: form.backend.trim() || undefined,
          chain: "bitcoin",
          network: selected.network,
          sp_descriptor: form.spDescriptor.trim(),
          sp_scan_mode: form.spScanMode,
          sp_scan_start_height: !fullHistory && Number.isFinite(startHeight)
            ? startHeight
            : undefined,
          sp_scan_start_date: !fullHistory
            ? form.spScanStartDate.trim() || undefined
            : undefined,
          sp_full_history: fullHistory,
          sp_acknowledge_full_history_warning:
            form.spAcknowledgeFullHistoryWarning,
          sp_acknowledge_server_warning: form.spAcknowledgeServerWarning,
        });
        startSyncNotice(t("add.silentPayments.stillScanning", { label }));
        try {
          await syncWallet.mutateAsync({ wallet: label });
        } finally {
          clearSyncNotice();
        }
        addNotification({
          title: t("add.added.title"),
          body: t("add.silentPayments.addedBodyScanning", { label }),
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
        const isFullBookImport =
          isBookWideImport && form.bullImportMode === "full";
        startSyncNotice(
          isFullBookImport
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
        if (!bip329Preview) {
          const envelope = await previewBip329.mutateAsync({
            file: form.bip329File.trim(),
          });
          setBip329Preview(envelope.data ?? null);
          addNotification({
            title: t("add.bip329.previewReadyTitle"),
            body: t("add.bip329.previewReadyBody", {
              count: envelope.data?.records ?? 0,
              exact: envelope.data?.counts.exact ?? 0,
              ambiguous: envelope.data?.counts.ambiguous ?? 0,
            }),
            tone: envelope.data?.counts.conflicts ? "warning" : "success",
          });
          return;
        }
        const envelope = await importBip329.mutateAsync({
          file: form.bip329File.trim(),
        });
        addNotification({
          title: t("add.bip329.labelsImportedTitle"),
          body: t("add.bip329.labelsImportedBody", {
            count: envelope.data?.records ?? 0,
            added: envelope.data?.transaction_tags_added ?? 0,
            skipped: envelope.data?.preview?.counts.tag_skipped_ambiguous ?? 0,
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
    helper?: React.ReactNode,
  ) => (
    <SetupField id={id} label={label} error={fieldErrors.backend} helper={helper}>
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

  const bip329MatchLabel = (status: Bip329MatchStatus) => {
    switch (status) {
      case "exact":
        return t("add.bip329.preview.exact");
      case "ambiguous":
        return t("add.bip329.preview.ambiguous");
      case "preserved":
        return t("add.bip329.preview.preserved");
      case "unmatched":
      default:
        return t("add.bip329.preview.unmatched");
    }
  };

  const bip329MatchVariant = (status: Bip329MatchStatus) => {
    if (status === "ambiguous") return "secondary";
    if (status === "unmatched") return "destructive";
    return "outline";
  };

  // Static literal keys so the typed `t()` resolves them (it rejects
  // template-literal keys). The hyphen in p2sh-p2wpkh isn't a valid i18n key
  // segment, hence the p2shp2wpkh suffix.
  const scriptTypeLabelKeys = {
    p2wpkh: "add.descriptor.scriptType.p2wpkh",
    "p2sh-p2wpkh": "add.descriptor.scriptType.p2shp2wpkh",
    p2pkh: "add.descriptor.scriptType.p2pkh",
    p2tr: "add.descriptor.scriptType.p2tr",
  } as const satisfies Record<BareXpubScriptType, string>;

  const scriptTypeLabel = (value: BareXpubScriptType) =>
    t(scriptTypeLabelKeys[value]);

  const requireAutoDetectedScriptTypes = (
    payload:
      | {
          probed?: boolean;
          active?: unknown;
          reason?: string | null;
        }
      | null
      | undefined,
  ) => {
    const selection = scriptTypesFromDetectionPayload(payload);
    if (!selection.ok) {
      throw new Error(
        t("add.descriptor.autoDetectUnavailable", {
          reason:
            selection.reason ??
            t("add.descriptor.autoDetectUnavailableReasonUnknown"),
        }),
      );
    }
    return selection.scriptTypes;
  };

  const renderWalletMaterialFeedback = () => {
    const detection = detectWalletMaterial(form.walletMaterial);
    if (detection.kind === "empty") return null;
    // A bare xpub is no longer a dead end: pinning a script type resolves it,
    // and leaving the picker on "Detect automatically" auto-detects at submit.
    if (detection.kind === "bare-xpub") {
      return (
        <p className="text-xs text-emerald-700 dark:text-emerald-300">
          {form.descriptorScriptType
            ? t("add.descriptor.bareXpubResolved", {
                label: detection.label,
                scriptType: scriptTypeLabel(
                  form.descriptorScriptType as BareXpubScriptType,
                ),
              })
            : t("add.descriptor.bareXpubAutoDetect", {
                label: detection.label,
              })}
        </p>
      );
    }
    const tone =
      detection.kind === "unknown"
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

  const renderBareXpubScriptType = () => {
    if (detectWalletMaterial(form.walletMaterial).kind !== "bare-xpub") {
      return null;
    }
    return (
      <SetupField
        id="connection-script-type"
        label={t("add.descriptor.scriptTypeLabel")}
        helper={t("add.descriptor.scriptTypeHelperAuto")}
      >
        <select
          id="connection-script-type"
          className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
          value={form.descriptorScriptType}
          onChange={(event) =>
            updateForm("descriptorScriptType", event.target.value)
          }
        >
          <option value="">{t("add.descriptor.scriptTypeDetect")}</option>
          {BARE_XPUB_SCRIPT_TYPES.map((value) => (
            <option key={value} value={value}>
              {scriptTypeLabel(value)}
            </option>
          ))}
        </select>
      </SetupField>
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
              <span className="w-32 shrink-0 text-muted-foreground">
                {entry.branch === "change"
                  ? t("add.descriptor.branchChange")
                  : entry.branch === "receive"
                    ? t("add.descriptor.branchReceive", { index: entry.index })
                    : entry.branch.endsWith("change")
                      ? entry.branch
                      : `${entry.branch} ${entry.index}`}
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

  const downloadLedgerTemplate = async (format: "xlsx" | "csv") => {
    try {
      const envelope = await ledgerTemplate.mutateAsync({ format });
      const file = envelope.data?.file;
      if (!file) return;
      const extension = format === "csv" ? "csv" : "xlsx";
      await saveDaemonExport({
        exportPath: file,
        title: t("add.genericLedger.saveTemplateTitle"),
        defaultName: `kassiber-ledger-template.${extension}`,
        filters: [
          {
            name: t("add.fileFilter.genericLedger"),
            extensions: [extension],
          },
        ],
      });
      addNotification({
        title: t("add.genericLedger.templateReadyTitle"),
        body: t("add.genericLedger.templateReadyBody"),
        tone: "success",
      });
    } catch (error) {
      addNotification({
        title: t("add.genericLedger.templateFailedTitle"),
        body: error instanceof Error ? error.message : String(error),
        tone: "error",
      });
    }
  };

  const exportBip329Labels = async () => {
    try {
      const envelope = await exportBip329.mutateAsync({
        mode: form.bip329ExportMode,
        wallet: form.bip329Wallet.trim() || undefined,
      });
      const file = envelope.data?.file;
      if (!file) return;
      const { copied, savedPath } = await saveDaemonExport({
        exportPath: file,
        title: t("add.bip329.exportSaveTitle"),
        defaultName: envelope.data?.filename || "kassiber-bip329-labels.jsonl",
        filters: [
          {
            name: t("add.bip329.labelFileFilter"),
            extensions: ["jsonl", "json"],
          },
        ],
      });
      addNotification({
        title: t("add.bip329.exportReadyTitle"),
        body: t("add.bip329.exportReadyBody", {
          count: envelope.data?.exported ?? 0,
          path: copied ? savedPath : file,
        }),
        tone: "success",
      });
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
              readOnly={selected.sourceFormat === "generic_ledger"}
              required
            />
            {selected.sourceFormat === "generic_ledger" && !isFilePickerAvailable ? (
              <Input
                aria-label={sourceFileField.label}
                type="file"
                accept=".csv,.tsv,.xlsx,.xlsm,text/csv,text/tab-separated-values,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet,application/vnd.ms-excel.sheet.macroEnabled.12"
                onChange={async (event) => {
                  const file = event.currentTarget.files?.[0];
                  if (!file) return;
                  updateForm("sourceFile", file.name);
                  setGenericLedgerPreviewBlocksSubmit(true);
                  setGenericLedgerPreviewSource({
                    filename: file.name,
                    sourceBytesBase64: await fileToBase64(file),
                    importable: false,
                  });
                }}
              />
            ) : null}
            {isFilePickerAvailable ? (
              <Button
                type="button"
                variant="outline"
                onClick={async () => {
                  const picked =
                    selected.sourceFormat === "generic_ledger"
                      ? await pickFileWithContentsBase64({
                          title: t("add.field.selectExportFileTitle", {
                            title: selected.title,
                          }),
                          filters: sourceFileFilters(selected, t),
                        })
                      : await pickFile({
                          title: t("add.field.selectExportFileTitle", {
                            title: selected.title,
                          }),
                          filters: sourceFileFilters(selected, t),
                        });
                  if (picked) {
                    const sourceFile =
                      typeof picked === "string" ? picked : picked.path;
                    updateForm("sourceFile", sourceFile);
                    if (typeof picked !== "string") {
                      setGenericLedgerPreviewSource({
                        filename: picked.path,
                        sourceBytesBase64: picked.contentsBase64,
                        importable: true,
                      });
                    }
                  }
                }}
              >
                {t("add.field.browse")}
              </Button>
            ) : null}
          </div>
        </SetupField>
        {selected.sourceFormat === "generic_ledger" ? (
          <div className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-dashed border-border/70 px-3 py-2">
            <p className="text-xs text-muted-foreground">
              {t("add.genericLedger.templateHint")}
            </p>
            <div className="flex gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                disabled={ledgerTemplate.isPending}
                onClick={() => downloadLedgerTemplate("xlsx")}
              >
                {ledgerTemplate.isPending
                  ? t("add.genericLedger.templateWorking")
                  : t("add.genericLedger.downloadXlsx")}
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={ledgerTemplate.isPending}
                onClick={() => downloadLedgerTemplate("csv")}
              >
                {t("add.genericLedger.downloadCsv")}
              </Button>
            </div>
          </div>
        ) : null}
        {selected.sourceFormat === "generic_ledger" && genericLedgerPreviewSource ? (
          <GenericLedgerPreview
            source={genericLedgerPreviewSource}
            onBlockSubmitChange={setGenericLedgerPreviewBlocksSubmit}
          />
        ) : null}
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
            addressBackendOptions,
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
                updateForm("descriptorScriptType", "");
                setPreviewAddresses(null);
                setPreviewError(null);
              }}
              required
            />
            {renderWalletMaterialFeedback()}
          </SetupField>
          {renderBareXpubScriptType()}
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
          <SetupField
            id="connection-birthday"
            label={t("add.descriptor.birthday")}
            error={fieldErrors.birthday}
            helper={
              selectedBackend?.kind === "bitcoinrpc"
                ? t("add.descriptor.birthdayHelperCore")
                : t("add.descriptor.birthdayHelper")
            }
          >
            <Input
              id="connection-birthday"
              type="date"
              value={form.birthday}
              onChange={(event) => updateForm("birthday", event.target.value)}
            />
          </SetupField>
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={
                previewDescriptor.isPending ||
                detectScriptTypes.isPending ||
                !form.walletMaterial.trim()
              }
              onClick={async () => {
                setPreviewError(null);
                try {
                  const isBareXpub =
                    detectWalletMaterial(form.walletMaterial).kind ===
                    "bare-xpub";
                  // Auto mode previews whatever the backend has history for, so
                  // the preview matches what creating the wallet would watch.
                  let scriptTypes: string[] | undefined;
                  if (isBareXpub) {
                    if (form.descriptorScriptType) {
                      scriptTypes = [form.descriptorScriptType];
                    } else {
                      const detected = await detectScriptTypes.mutateAsync({
                        wallet_material: form.walletMaterial.trim(),
                        backend: form.backend.trim() || undefined,
                        chain: selected.chain,
                        network: selected.network,
                      });
                      scriptTypes = requireAutoDetectedScriptTypes(
                        detected.data,
                      );
                    }
                  }
                  const envelope = await previewDescriptor.mutateAsync({
                    wallet_material: form.walletMaterial.trim(),
                    script_types: scriptTypes,
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
              {previewDescriptor.isPending || detectScriptTypes.isPending
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

    if (setupKind === "address-list") {
      const parsed = parsedAddressList;
      const invalidSamples = parsed.invalid.slice(0, 3).join(", ");
      return (
        <>
          {renderConnectionLabelField()}
          {renderBackendSelect(
            "connection-backend",
            t("add.field.backend"),
            descriptorBackendOptions,
          )}
          <SetupField
            id="connection-address-list"
            label={t("add.addressList.addresses")}
            error={fieldErrors.addressList}
            helper={t("add.addressList.helper")}
          >
            <Textarea
              id="connection-address-list"
              className="min-h-32 font-mono text-xs"
              value={form.addressList}
              onChange={(event) => applyAddressInput(event.target.value)}
              placeholder={t("add.addressList.placeholder")}
            />
            <input
              ref={addressFileInputRef}
              type="file"
              accept=".txt,.csv,text/plain,text/csv"
              multiple
              className="hidden"
              onChange={handleAddressFile}
            />
            {purgedKeys ? (
              <p className="text-xs font-medium text-destructive" role="alert">
                {purgedKeys.privateKeys > 0
                  ? t("add.addressList.privateKeysPurged", {
                      count: purgedKeys.privateKeys,
                    })
                  : ""}
                {purgedKeys.privateKeys > 0 && purgedKeys.publicKeys > 0
                  ? " "
                  : ""}
                {purgedKeys.publicKeys > 0
                  ? t("add.addressList.publicKeysPurged", {
                      count: purgedKeys.publicKeys,
                    })
                  : ""}
              </p>
            ) : null}
            <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => addressFileInputRef.current?.click()}
              >
                {t("add.addressList.loadFile")}
              </Button>
              {parsed.entries.length > 0 ? (
                <span className="text-xs text-muted-foreground">
                  {t("add.addressList.summary", {
                    count: parsed.valid.length,
                  })}
                  {parsed.duplicates > 0
                    ? ` · ${t("add.addressList.summaryDuplicates", {
                        count: parsed.duplicates,
                      })}`
                    : ""}
                  {parsed.invalid.length > 0
                    ? ` · ${t("add.addressList.summaryInvalid", {
                        count: parsed.invalid.length,
                      })}`
                    : ""}
                </span>
              ) : null}
            </div>
            {parsed.invalid.length > 0 ? (
              <p className="text-xs text-amber-700 dark:text-amber-300">
                {t("add.addressList.invalidHint", { samples: invalidSamples })}
              </p>
            ) : null}
          </SetupField>
          {renderSyncAfterCreate(t("add.addressList.scanAfter"))}
        </>
      );
    }

    if (setupKind === "silent-payment") {
      return (
        <>
          {renderConnectionLabelField()}
          {renderBackendSelect(
            "connection-backend",
            t("add.silentPayments.backend"),
            silentPaymentBackendOptions,
            t("add.silentPayments.backendHelper"),
          )}
          <SetupField
            id="connection-sp-descriptor"
            label={t("add.silentPayments.material")}
            error={fieldErrors.spDescriptor}
            helper={t("add.silentPayments.materialHelper")}
          >
            <Textarea
              id="connection-sp-descriptor"
              className="min-h-28 font-mono text-xs"
              value={form.spDescriptor}
              onChange={(event) =>
                updateForm("spDescriptor", event.target.value)
              }
              placeholder="sp(spscan1q...)"
              required
            />
          </SetupField>
          <SetupField
            id="connection-sp-scan-mode"
            label={t("add.silentPayments.scanMode")}
            helper={t("add.silentPayments.scanModeHelper")}
          >
            <select
              id="connection-sp-scan-mode"
              className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
              value={form.spScanMode}
              onChange={(event) =>
                updateForm(
                  "spScanMode",
                  event.target.value as SetupFormState["spScanMode"],
                )
              }
            >
              <option value="local_index">
                {t("add.silentPayments.scanModeLocal")}
              </option>
              <option value="server_assisted">
                {t("add.silentPayments.scanModeServer")}
              </option>
            </select>
          </SetupField>
          <div className="grid gap-3 sm:grid-cols-2">
            <SetupField
              id="connection-sp-start-height"
              label={t("add.silentPayments.startHeight")}
              error={fieldErrors.spScanStartHeight}
            >
              <Input
                id="connection-sp-start-height"
                type="number"
                min={0}
                value={form.spScanStartHeight}
                onChange={(event) =>
                  updateForm("spScanStartHeight", event.target.value)
                }
                disabled={form.spFullHistory}
              />
            </SetupField>
            <SetupField
              id="connection-sp-start-date"
              label={t("add.silentPayments.startDate")}
              error={fieldErrors.spScanStartDate}
              helper={t("add.silentPayments.startDateHelper")}
            >
              <Input
                id="connection-sp-start-date"
                type="date"
                value={form.spScanStartDate}
                onChange={(event) =>
                  updateForm("spScanStartDate", event.target.value)
                }
                disabled={form.spFullHistory}
              />
            </SetupField>
          </div>
          <div className="space-y-2 rounded-md border bg-background p-3">
            <label className="flex items-start gap-2 text-sm">
              <Checkbox
                checked={form.spFullHistory}
                onCheckedChange={(checked) =>
                  updateForm("spFullHistory", checked === true)
                }
              />
              <span>{t("add.silentPayments.fullHistory")}</span>
            </label>
            {form.spFullHistory ? (
              <label className="flex items-start gap-2 text-sm">
                <Checkbox
                  checked={form.spAcknowledgeFullHistoryWarning}
                  onCheckedChange={(checked) =>
                    updateForm(
                      "spAcknowledgeFullHistoryWarning",
                      checked === true,
                    )
                  }
                />
                <span>{t("add.silentPayments.fullHistoryAck")}</span>
              </label>
            ) : null}
            {fieldErrors.spAcknowledgeFullHistoryWarning ? (
              <p className="text-xs text-destructive">
                {fieldErrors.spAcknowledgeFullHistoryWarning}
              </p>
            ) : null}
          </div>
          {form.spScanMode === "server_assisted" ? (
            <div className="space-y-2 rounded-md border border-amber-500/30 bg-amber-500/10 p-3">
              <label className="flex items-start gap-2 text-sm">
                <Checkbox
                  checked={form.spAcknowledgeServerWarning}
                  onCheckedChange={(checked) =>
                    updateForm("spAcknowledgeServerWarning", checked === true)
                  }
                />
                <span>{t("add.silentPayments.serverAck")}</span>
              </label>
              {fieldErrors.spAcknowledgeServerWarning ? (
                <p className="text-xs text-destructive">
                  {fieldErrors.spAcknowledgeServerWarning}
                </p>
              ) : null}
            </div>
          ) : null}
          <div className="space-y-3 rounded-md border bg-background p-3">
            <div className="grid gap-3 sm:grid-cols-2">
              <SetupField
                id="connection-bip329-export-mode"
                label={t("add.bip329.exportModeLabel")}
              >
                <select
                  id="connection-bip329-export-mode"
                  className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
                  value={form.bip329ExportMode}
                  onChange={(event) =>
                    updateForm(
                      "bip329ExportMode",
                      event.target.value as SetupFormState["bip329ExportMode"],
                    )
                  }
                >
                  <option value="stored">{t("add.bip329.exportModeStored")}</option>
                  <option value="synthesized">
                    {t("add.bip329.exportModeSynthesized")}
                  </option>
                  <option value="all">{t("add.bip329.exportModeAll")}</option>
                </select>
              </SetupField>
              <SetupField
                id="connection-bip329-export-wallet"
                label={t("add.bip329.exportWalletLabel")}
              >
                <select
                  id="connection-bip329-export-wallet"
                  className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
                  value={form.bip329Wallet}
                  onChange={(event) => updateForm("bip329Wallet", event.target.value)}
                >
                  <option value="">{t("add.bip329.exportWalletProfile")}</option>
                  {(walletsList.data?.data?.wallets ?? []).map((wallet) => (
                    <option key={wallet.label} value={wallet.label}>
                      {wallet.label}
                    </option>
                  ))}
                </select>
              </SetupField>
            </div>
            <Button
              type="button"
              variant="outline"
              onClick={() => void exportBip329Labels()}
              disabled={exportBip329.isPending}
            >
              {exportBip329.isPending
                ? t("add.bip329.exporting")
                : t("add.bip329.exportButton")}
            </Button>
          </div>
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
                label={t(field.labelKey as never) /* dynamic key */}
                error={fieldErrors[field.key]}
                helper={t(field.helperKey as never) /* dynamic key */}
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
                      <span>{t(network.labelKey as never) /* dynamic key */}</span>
                      <span className="text-xs text-muted-foreground">
                        {t(network.helperKey as never) /* dynamic key */}
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
          {bip329Preview ? (
            <div className="space-y-3 rounded-md border bg-muted/30 p-3">
              <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
                {[
                  {
                    key: "exact",
                    label: t("add.bip329.preview.exact"),
                    value: bip329Preview.counts.exact,
                  },
                  {
                    key: "ambiguous",
                    label: t("add.bip329.preview.ambiguous"),
                    value: bip329Preview.counts.ambiguous,
                  },
                  {
                    key: "unmatched",
                    label: t("add.bip329.preview.unmatched"),
                    value: bip329Preview.counts.unmatched,
                  },
                  {
                    key: "preserved",
                    label: t("add.bip329.preview.preserved"),
                    value: bip329Preview.counts.preserved,
                  },
                ].map((item) => (
                  <div key={item.key} className="rounded-md border bg-background p-2">
                    <p className="text-[11px] text-muted-foreground">
                      {item.label}
                    </p>
                    <p className="text-lg font-semibold tabular-nums">{item.value}</p>
                  </div>
                ))}
              </div>
              <div className="flex flex-wrap gap-2 text-xs">
                <Badge variant={bip329Preview.counts.conflicts ? "destructive" : "outline"}>
                  {t("add.bip329.preview.conflicts", {
                    count: bip329Preview.counts.conflicts,
                  })}
                </Badge>
                <Badge variant={bip329Preview.counts.duplicate_refs ? "secondary" : "outline"}>
                  {t("add.bip329.preview.duplicates", {
                    count: bip329Preview.counts.duplicate_refs,
                  })}
                </Badge>
                <Badge variant="outline">
                  {t("add.bip329.preview.tagAdditions", {
                    count: bip329Preview.counts.tag_additions,
                  })}
                </Badge>
                <Badge variant="outline">
                  {t("add.bip329.preview.ambiguousSkipped", {
                    count: bip329Preview.counts.tag_skipped_ambiguous,
                  })}
                </Badge>
              </div>
              {(bip329Preview.rows ?? []).slice(0, 6).length > 0 ? (
                <div className="space-y-2">
                  {(bip329Preview.rows ?? []).slice(0, 6).map((row) => (
                    <div
                      key={`${row.line}-${row.type}-${row.ref_preview}`}
                      className="grid gap-1 rounded-md border bg-background p-2 text-xs sm:grid-cols-[64px_1fr_auto]"
                    >
                      <span className="font-medium uppercase text-muted-foreground">
                        {row.type}
                      </span>
                      <span className="min-w-0 truncate font-mono">
                        {row.ref_preview}
                      </span>
                      <Badge variant={bip329MatchVariant(row.match_status)}>
                        {bip329MatchLabel(row.match_status)}
                      </Badge>
                      {row.wallets.length > 0 ? (
                        <span className="min-w-0 truncate text-muted-foreground sm:col-span-3">
                          {row.wallets.join(", ")}
                        </span>
                      ) : null}
                    </div>
                  ))}
                </div>
              ) : null}
              {bip329Preview.warnings?.length ? (
                <p className="text-xs text-muted-foreground">
                  {bip329Preview.warnings.slice(0, 2).join(" ")}
                </p>
              ) : null}
            </div>
          ) : null}
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
        </>
      );
    }

    if (setupKind === "backend-settings" && selected.id === "bitcoin-core") {
      const candidates = coreDetection?.candidates ?? [];
      return (
        <>
          {renderConnectionLabelField()}
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={detectCore.isPending}
              onClick={async () => {
                setCoreTestStatus(null);
                try {
                  const envelope = await detectCore.mutateAsync({});
                  const data = envelope.data ?? { candidates: [] };
                  setCoreDetection(data);
                  const first = data.candidates[0];
                  if (first) {
                    setForm((current) =>
                      applyCoreCandidateToForm(current, first),
                    );
                  }
                } catch (error) {
                  setCoreDetection({ candidates: [] });
                  setCoreTestStatus({
                    ok: false,
                    message:
                      error instanceof Error
                        ? error.message
                        : t("add.core.detectFailed"),
                  });
                }
              }}
            >
              {detectCore.isPending ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : null}
              {t("add.core.detect")}
            </Button>
            {coreDetection && candidates.length === 0 ? (
              <span className="text-xs text-muted-foreground">
                {t("add.core.noneDetected")}
              </span>
            ) : null}
          </div>
          {candidates.length > 0 ? (
            <div className="space-y-2 rounded-md border border-border/70 p-3">
              <p className="text-xs font-medium text-muted-foreground">
                {t("add.core.detectedNodes")}
              </p>
              <div className="grid gap-2">
                {candidates.map((candidate) => {
                  const candidateKey = [
                    candidate.url,
                    candidate.network ?? "",
                    candidate.auth_source ?? "",
                    candidate.credential_source ?? "",
                    candidate.cookiefile ?? candidate.credential_ref ?? "",
                  ].join("|");
                  const readinessMessages = coreReadinessMessages(candidate, t);
                  return (
                    <Button
                      key={candidateKey}
                      type="button"
                      variant="outline"
                      className="h-auto justify-start text-left"
                      onClick={() => {
                        setForm((current) =>
                          applyCoreCandidateToForm(current, candidate),
                        );
                        setCoreTestStatus(null);
                      }}
                    >
                      <span className="min-w-0">
                        <span className="block truncate">
                          {candidate.network || "main"} · {candidate.url}
                        </span>
                        <span className="block truncate text-[11px] font-normal text-muted-foreground">
                          {t("add.core.candidateMeta", {
                            status: t(
                              `add.core.status.${coreNodeStatusKey(candidate.status)}`,
                            ),
                            blocks: candidate.blocks ?? "n/a",
                            headers: candidate.headers ?? "n/a",
                            peers: candidate.peers ?? "n/a",
                            auth: coreCandidateAuthLabel(candidate, t),
                          })}
                        </span>
                        {readinessMessages.length > 0 ? (
                          <span className="block whitespace-normal text-[11px] font-normal text-amber-700 dark:text-amber-300">
                            {readinessMessages.join(" ")}
                          </span>
                        ) : null}
                      </span>
                    </Button>
                  );
                })}
              </div>
            </div>
          ) : null}
          <div className="grid gap-3 md:grid-cols-[minmax(0,1.4fr)_minmax(0,0.7fr)]">
            <SetupField
              id="connection-core-url"
              label={t("add.core.url")}
              error={fieldErrors.coreRpcUrl}
            >
              <Input
                id="connection-core-url"
                value={form.coreRpcUrl}
                onChange={(event) => {
                  updateCoreForm("coreRpcUrl", event.target.value);
                }}
              />
            </SetupField>
            <SetupField
              id="connection-core-network"
              label={t("add.core.network")}
            >
              <select
                id="connection-core-network"
                className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
                value={form.coreRpcNetwork}
                onChange={(event) => {
                  const network = event.target.value;
                  setForm((current) => ({
                    ...current,
                    coreRpcNetwork: network,
                    coreRpcUrl: Object.values(CORE_DEFAULT_RPC_URLS).includes(
                      current.coreRpcUrl.trim(),
                    )
                      ? CORE_DEFAULT_RPC_URLS[network]
                      : current.coreRpcUrl,
                    coreRpcCredentialRef: "",
                  }));
                  setCoreTestStatus(null);
                }}
              >
                <option value="main">{t("add.core.networkMain")}</option>
                <option value="test">{t("add.core.networkTest")}</option>
                <option value="signet">{t("add.core.networkSignet")}</option>
                <option value="regtest">{t("add.core.networkRegtest")}</option>
              </select>
            </SetupField>
          </div>
          <SetupField
            id="connection-core-auth"
            label={t("add.core.authMode")}
          >
            <select
              id="connection-core-auth"
              className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
              value={form.coreRpcAuthMode}
              onChange={(event) => {
                updateCoreForm(
                  "coreRpcAuthMode",
                  event.target.value === "basic" ? "basic" : "cookiefile",
                );
              }}
            >
              <option value="cookiefile">{t("add.core.authCookiefile")}</option>
              <option value="basic">{t("add.core.authBasic")}</option>
            </select>
          </SetupField>
          {form.coreRpcAuthMode === "cookiefile" ? (
            <SetupField
              id="connection-core-cookiefile"
              label={t("add.core.cookiefile")}
              error={fieldErrors.coreRpcCookiefile}
            >
              <Input
                id="connection-core-cookiefile"
                value={form.coreRpcCookiefile}
                onChange={(event) => {
                  updateCoreForm("coreRpcCookiefile", event.target.value);
                }}
              />
            </SetupField>
          ) : (
            <div className="grid gap-3 md:grid-cols-2">
              <SetupField
                id="connection-core-username"
                label={t("add.core.username")}
                error={fieldErrors.coreRpcUsername}
              >
                <Input
                  id="connection-core-username"
                  value={form.coreRpcUsername}
                  onChange={(event) => {
                    updateCoreForm("coreRpcUsername", event.target.value);
                  }}
                />
              </SetupField>
              <SetupField
                id="connection-core-password"
                label={t("add.core.password")}
                error={fieldErrors.coreRpcPassword}
              >
                <Input
                  id="connection-core-password"
                  type="password"
                  value={form.coreRpcPassword}
                  onChange={(event) => {
                    updateCoreForm("coreRpcPassword", event.target.value);
                  }}
                />
              </SetupField>
            </div>
          )}
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={testCore.isPending || !form.coreRpcUrl.trim()}
              onClick={async () => {
                setCoreTestStatus(null);
                try {
                  const envelope = await testCore.mutateAsync(coreRpcProbeArgs());
                  const payload = envelope.data;
                  setCoreTestStatus(
                    payload?.reachable
                      ? {
                          ok: true,
                          ...payload,
                        }
                      : {
                          ok: false,
                          message:
                            payload?.error?.message ??
                            t("add.core.testFailed"),
                          hint: payload?.error?.hint,
                        },
                  );
                } catch (error) {
                  setCoreTestStatus({
                    ok: false,
                    message:
                      error instanceof Error
                        ? error.message
                        : t("add.core.testFailed"),
                  });
                }
              }}
            >
              {testCore.isPending ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : null}
              {t("add.core.test")}
            </Button>
            {coreTestStatus?.ok ? (
              <div className="space-y-1 text-xs">
                <p className="text-emerald-700 dark:text-emerald-300">
                  {t("add.core.testOkDetailed", {
                    network: coreTestStatus.network || form.coreRpcNetwork,
                    status: t(
                      `add.core.status.${coreNodeStatusKey(coreTestStatus.status)}`,
                    ),
                    blocks: coreTestStatus.blocks ?? "n/a",
                    headers: coreTestStatus.headers ?? "n/a",
                    peers: coreTestStatus.peers ?? "n/a",
                  })}
                </p>
                {coreTestStatus.pruned ? (
                  <p className="text-amber-700 dark:text-amber-300">
                    {t("add.core.prunedWarning")}
                  </p>
                ) : null}
                {coreTestStatus.ibd ? (
                  <p className="text-amber-700 dark:text-amber-300">
                    {t("add.core.initialBlockDownloadWarning")}
                  </p>
                ) : null}
                {coreTestStatus.wallet_rpc?.available === false ? (
                  <p className="text-destructive">
                    {coreTestStatus.wallet_rpc.error?.hint ??
                      coreTestStatus.wallet_rpc.error?.message ??
                      t("add.core.walletRpcUnavailable")}
                  </p>
                ) : null}
                {coreTestStatus.block_filters?.available === false ? (
                  <p className="text-muted-foreground">
                    {coreTestStatus.block_filters.error?.hint ??
                      t("add.core.blockFiltersUnavailable")}
                  </p>
                ) : null}
              </div>
            ) : null}
            {coreTestStatus && !coreTestStatus.ok ? (
              <span className="text-xs text-destructive">
                {coreTestStatus.hint ?? coreTestStatus.message}
              </span>
            ) : null}
          </div>
        </>
      );
    }

    if (setupKind === "backend-settings") {
      return (
        <div className="space-y-3 rounded-md border bg-background p-3 text-sm text-muted-foreground">
          <p>{t("add.backendSettings.line1")}</p>
          <p>{t("add.backendSettings.line2")}</p>
          {backendSettingsQuery.isLoading ? (
            <p className="text-xs">{t("add.backendSettings.loading")}</p>
          ) : configuredLocalBackends.length > 0 ? (
            <div className="space-y-2">
              <p className="text-xs font-medium text-foreground">
                {t("add.backendSettings.localTitle")}
              </p>
              <div className="grid gap-1.5">
                {configuredLocalBackends.map((backend) => (
                  <button
                    key={backend.id}
                    type="button"
                    className="flex min-w-0 items-center justify-between gap-3 rounded-md border bg-muted/30 px-3 py-2 text-left hover:bg-muted/55 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                    onClick={() =>
                      openConfiguredBackendSettings(
                        backend.backendId ?? backend.id,
                        backend.settingsHash,
                      )
                    }
                  >
                    <span className="min-w-0">
                      <span className="block truncate text-sm font-medium text-foreground">
                        {backend.label}
                      </span>
                      <span className="block truncate font-mono text-xs text-muted-foreground">
                        {backend.endpoint}
                      </span>
                    </span>
                    <span className="shrink-0 text-xs text-muted-foreground">
                      {backend.syncSource}
                    </span>
                  </button>
                ))}
              </div>
            </div>
          ) : null}
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
          {selected.status === "planned" ? (
            <Badge variant="outline">{t("add.plannedLabel")}</Badge>
          ) : null}
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
                    {source.status === "planned" ? (
                      <Badge variant="outline">{t("add.plannedLabel")}</Badge>
                    ) : null}
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
                    missingBtcpayMappingDiscovery ||
                    (selected.sourceFormat === "generic_ledger" &&
                      genericLedgerPreviewBlocksSubmit)
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
