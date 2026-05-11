import * as React from "react";
import { useNavigate } from "@tanstack/react-router";

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
import { useUiStore, type DeferredConnectionSetup } from "@/store/ui";
import { cn } from "@/lib/utils";
import {
  CONNECTION_CATEGORIES,
  CONNECTION_SOURCES,
  type ConnectionCategory,
  type ConnectionSource,
} from "@/lib/connectionCatalog";
import { isFilePickerAvailable, pickFile } from "@/lib/filePicker";
import { detectWalletMaterial } from "@/lib/walletMaterialFormat";

interface AddConnectionDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  initialSourceId?: string | null;
}

interface SetupFormState {
  label: string;
  backend: string;
  walletMaterial: string;
  gapLimit: string;
  sourceFile: string;
  sourceFormat: "csv" | "json";
  btcpayStoreId: string;
  bip329Wallet: string;
  bip329File: string;
  syncAfterCreate: boolean;
}

interface SyncResult {
  wallet: string;
  status: string;
  message?: string;
}

interface BackendOption {
  name: string;
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

type DialogStep = "source" | "setup";
const DESCRIPTOR_BACKEND_KINDS = new Set(["esplora", "electrum"]);

function supportsDescriptorSync(backend: BackendOption) {
  return DESCRIPTOR_BACKEND_KINDS.has(backend.kind);
}

function sourceFileFilters(source: ConnectionSource) {
  if (source.sourceFormat === "phoenix_csv") {
    return [{ name: "Phoenix CSV", extensions: ["csv"] }];
  }
  if (source.sourceFormat === "river_csv") {
    return [{ name: "River CSV", extensions: ["csv"] }];
  }
  if (source.id === "csv") {
    return [{ name: "CSV or JSON", extensions: ["csv", "json"] }];
  }
  return undefined;
}

const formDefaultsFor = (source: ConnectionSource): SetupFormState => {
  const defaultLabel =
    source.id === "csv"
      ? "Imported file"
      : source.id === "bip329"
        ? ""
        : source.title;
  return {
    label: defaultLabel,
    backend: "",
    walletMaterial: "",
    gapLimit: "20",
    sourceFile: "",
    sourceFormat: "csv",
    btcpayStoreId: "",
    bip329Wallet: "",
    bip329File: "",
    syncAfterCreate: source.setupKind === "file-wallet",
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
      {helper && !error ? (
        <p className="text-xs text-muted-foreground">{helper}</p>
      ) : null}
      {error ? <p className="text-xs text-destructive">{error}</p> : null}
    </div>
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
  const navigate = useNavigate();
  const addNotification = useUiStore((state) => state.addNotification);
  const setDeferredConnectionSetup = useUiStore(
    (state) => state.setDeferredConnectionSetup,
  );
  const backendOptions = useDaemon<BackendOptionsData>("ui.backends.options");
  const createWallet =
    useDaemonMutation<{ wallet: { label: string } }>("ui.wallets.create");
  const createBtcpay = useDaemonMutation<{
    backend: { name: string };
    wallet: { label: string };
  }>("ui.connections.btcpay.create");
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
    formDefaultsFor(CONNECTION_SOURCES[0]),
  );
  const [setupError, setSetupError] = React.useState<string | null>(null);
  const [fieldErrors, setFieldErrors] = React.useState<
    Partial<Record<keyof SetupFormState, string>>
  >({});
  const [previewAddresses, setPreviewAddresses] = React.useState<
    { branch: "receive" | "change"; index: number; address: string }[] | null
  >(null);
  const [previewError, setPreviewError] = React.useState<string | null>(null);
  const [btcpayTestStatus, setBtcpayTestStatus] = React.useState<
    { ok: true; storeId: string } | { ok: false; message: string } | null
  >(null);
  const [copiedAddress, setCopiedAddress] = React.useState<string | null>(null);
  const copyAddress = React.useCallback(async (address: string) => {
    try {
      await navigator.clipboard?.writeText(address);
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
  const selectedBackendOptions =
    setupKind === "btcpay" ? btcpayBackends : descriptorBackendOptions;
  const defaultBackendName =
    selectedBackendOptions.find((backend) => backend.is_default)?.name ??
    selectedBackendOptions[0]?.name ??
    "";
  const isSubmitting =
    createWallet.isPending ||
    createBtcpay.isPending ||
    importBip329.isPending ||
    syncWallet.isPending;
  const requiresBackend = setupKind === "descriptor" || setupKind === "btcpay";
  const missingBackend = requiresBackend && selectedBackendOptions.length === 0;
  const submitLabel =
    setupKind === "backend-settings"
      ? "Open backend settings"
      : syncWallet.isPending
        ? "Refreshing…"
        : importBip329.isPending
          ? "Importing labels…"
          : isSubmitting
            ? "Saving…"
            : "Create connection";
  const canContinue = selected.status === "ready" && setupKind !== "planned";

  React.useEffect(() => {
    setForm(formDefaultsFor(selected));
    setSetupError(null);
    setFieldErrors({});
    setPreviewAddresses(null);
    setPreviewError(null);
    setBtcpayTestStatus(null);
    setSyncProgress(null);
  }, [selected]);

  React.useEffect(() => {
    if (!open) return;
    const source =
      CONNECTION_SOURCES.find((candidate) => candidate.id === initialSourceId) ??
      CONNECTION_SOURCES[0];
    setActiveCategory(source.category);
    setSelectedId(source.id);
    setStep(initialSourceId && source.status === "ready" ? "setup" : "source");
    setSetupError(null);
    setSourceQuery("");
  }, [initialSourceId, open]);

  React.useEffect(() => {
    if (!defaultBackendName) return;
    if (setupKind !== "descriptor" && setupKind !== "btcpay") return;
    setForm((current) =>
      current.backend ? current : { ...current, backend: defaultBackendName },
    );
  }, [defaultBackendName, setupKind]);

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
    };
    setDeferredConnectionSetup(intent);
    onOpenChange(false);
    void navigate({ to: "/settings", hash: "backends" });
  };

  const validateSetupForm = (): Partial<Record<keyof SetupFormState, string>> => {
    const errors: Partial<Record<keyof SetupFormState, string>> = {};
    if (
      setupKind === "descriptor" ||
      setupKind === "file-wallet" ||
      setupKind === "btcpay"
    ) {
      if (!form.label.trim()) {
        errors.label = "Connection label is required.";
      }
    }
    if (setupKind === "descriptor") {
      if (!form.walletMaterial.trim()) {
        errors.walletMaterial = "Paste a wallet export, descriptor, or extended public key.";
      } else {
        const detection = detectWalletMaterial(form.walletMaterial);
        if (detection.kind === "bare-xpub" || detection.kind === "unknown") {
          errors.walletMaterial = detection.hint ?? detection.label;
        }
      }
      const gapLimit = Number.parseInt(form.gapLimit, 10);
      if (!Number.isFinite(gapLimit) || gapLimit <= 0) {
        errors.gapLimit = "Gap limit must be a positive integer.";
      }
      if (descriptorBackendOptions.length > 0 && !form.backend.trim()) {
        errors.backend = "Choose a backend.";
      }
    }
    if (setupKind === "file-wallet" && !form.sourceFile.trim()) {
      errors.sourceFile = "Pick the export file.";
    }
    if (setupKind === "btcpay") {
      if (btcpayBackends.length > 0 && !form.backend.trim()) {
        errors.backend = "Choose a BTCPay backend.";
      }
      if (!form.btcpayStoreId.trim()) {
        errors.btcpayStoreId = "Enter the BTCPay store ID.";
      }
    }
    if (setupKind === "bip329" && !form.bip329File.trim()) {
      errors.bip329File = "Pick the BIP329 label file.";
    }
    return errors;
  };

  const onSetupSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (setupKind === "planned") {
      addNotification({
        title: "Connection path is planned",
        body: `${selected.title} is tracked in the catalog but is not wired yet.`,
        tone: "warning",
      });
      return;
    }
    setSetupError(null);
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
            `${label} is still scanning in watch-only mode. Large descriptors or slow backends can take a bit; Kassiber will update when the daemon finishes.`,
          );
          try {
            await syncWallet.mutateAsync({ wallet: label });
          } finally {
            clearSyncNotice();
          }
        }
        addNotification({
          title: "Connection added",
          body: `${label} is configured.`,
          tone: "success",
        });
      } else if (setupKind === "file-wallet") {
        const sourceFormat =
          selected.id === "csv" ? form.sourceFormat : selected.sourceFormat;
        await createWallet.mutateAsync({
          label,
          kind: selected.walletKind ?? "custom",
          source_file: form.sourceFile.trim(),
          source_format: sourceFormat,
        });
        if (form.syncAfterCreate) {
          startSyncNotice(
            `${label} is still refreshing from BTCPay. Slow backends can take a bit; Kassiber will update when the daemon finishes.`,
          );
          try {
            await syncWallet.mutateAsync({ wallet: label });
          } finally {
            clearSyncNotice();
          }
        }
        addNotification({
          title: "Connection added",
          body: `${label} is configured${form.syncAfterCreate ? " and imported" : ""}.`,
          tone: "success",
        });
      } else if (setupKind === "btcpay") {
        await createBtcpay.mutateAsync({
          label,
          backend: form.backend.trim(),
          store_id: form.btcpayStoreId.trim(),
        });
        if (form.syncAfterCreate) {
          startSyncNotice(
            `${label} is still importing. Large CSV exports can take a moment; Kassiber will update when the daemon finishes.`,
          );
          try {
            await syncWallet.mutateAsync({ wallet: label });
          } finally {
            clearSyncNotice();
          }
        }
        addNotification({
          title: "Connection added",
          body: `${label} is configured${form.syncAfterCreate ? " and refreshed" : ""}.`,
          tone: "success",
        });
      } else if (setupKind === "bip329") {
        const envelope = await importBip329.mutateAsync({
          file: form.bip329File.trim(),
          wallet: form.bip329Wallet.trim() || undefined,
        });
        addNotification({
          title: "Labels imported",
          body: `${envelope.data?.records ?? 0} label records processed.`,
          tone: "success",
        });
      }
      onOpenChange(false);
    } catch (error) {
      const message =
        error instanceof Error ? error.message : "Connection setup failed.";
      setSetupError(message);
      addNotification({
        title: "Connection setup failed",
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
            Select backend
          </option>
          {options.map((backend) => (
            <option key={backend.name} value={backend.name}>
              {backend.name}
              {backend.is_default ? " (default)" : ""}
              {backend.kind ? ` · ${backend.kind}` : ""}
            </option>
          ))}
        </select>
      ) : (
        <div className="space-y-2 rounded-md border bg-background p-3">
          <p className="text-sm text-muted-foreground">
            No matching backend is configured.
          </p>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={openBackendSettings}
          >
            Open backend settings
          </Button>
        </div>
      )}
    </SetupField>
  );

  const renderConnectionLabelField = () => (
    <SetupField
      id="connection-label"
      label="Connection label"
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
        Detected: {detection.label}
        {detection.hint ? ` — ${detection.hint}` : ""}
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
          First derived addresses
        </p>
        <ul className="space-y-1 font-mono">
          {previewAddresses.map((entry) => (
            <li
              key={`${entry.branch}-${entry.index}`}
              className="flex items-center gap-2"
            >
              <span className="w-16 shrink-0 text-muted-foreground">
                {entry.branch === "change" ? "change/0" : `recv/${entry.index}`}
              </span>
              <span className="min-w-0 flex-1 truncate">{entry.address}</span>
              <button
                type="button"
                className="rounded border px-2 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground hover:bg-muted/40"
                onClick={() => void copyAddress(entry.address)}
                title="Copy address"
              >
                {copiedAddress === entry.address ? "Copied" : "Copy"}
              </button>
            </li>
          ))}
        </ul>
      </div>
    );
  };

  const renderSetupFields = () => {
    if (setupKind === "descriptor") {
      return (
        <>
          {renderConnectionLabelField()}
          {renderBackendSelect(
            "connection-backend",
            "Backend",
            descriptorBackendOptions,
          )}
          <SetupField
            id="connection-wallet-material"
            label="Wallet export"
            error={fieldErrors.walletMaterial}
          >
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
            label="Gap limit"
            error={fieldErrors.gapLimit}
          >
            <Input
              id="connection-gap-limit"
              type="number"
              min={1}
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
                      : "Could not derive addresses.",
                  );
                }
              }}
            >
              {previewDescriptor.isPending
                ? "Deriving…"
                : "Preview addresses"}
            </Button>
            <span className="text-xs text-muted-foreground">
              Derives the first 5 receive addresses without saving.
            </span>
          </div>
          {renderDescriptorPreview()}
        </>
      );
    }

    if (setupKind === "file-wallet") {
      return (
        <>
          {renderConnectionLabelField()}
          {selected.id === "csv" ? (
            <SetupField id="connection-source-format" label="File format">
              <select
                id="connection-source-format"
                className="h-9 w-full rounded-md border border-input bg-background px-3 text-sm"
                value={form.sourceFormat}
                onChange={(event) =>
                  updateForm("sourceFormat", event.target.value as "csv" | "json")
                }
              >
                <option value="csv">CSV</option>
                <option value="json">JSON</option>
              </select>
            </SetupField>
          ) : null}
          <SetupField
            id="connection-source-file"
            label="Export file path"
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
                  onClick={async () => {
                    const picked = await pickFile({
                      title: `Select ${selected.title} export file`,
                      filters: sourceFileFilters(selected),
                    });
                    if (picked) updateForm("sourceFile", picked);
                  }}
                >
                  Browse…
                </Button>
              ) : null}
            </div>
          </SetupField>
          {renderSyncAfterCreate("Import after setup")}
        </>
      );
    }

    if (setupKind === "btcpay") {
      return (
        <>
          {renderConnectionLabelField()}
          {renderBackendSelect(
            "connection-btcpay-backend",
            "BTCPay backend",
            btcpayBackends,
          )}
          <SetupField
            id="connection-btcpay-store"
            label="Store ID"
            error={fieldErrors.btcpayStoreId}
          >
            <Input
              id="connection-btcpay-store"
              value={form.btcpayStoreId}
              onChange={(event) => {
                updateForm("btcpayStoreId", event.target.value);
                setBtcpayTestStatus(null);
              }}
              required
            />
          </SetupField>
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={
                testBtcpay.isPending ||
                !form.backend.trim() ||
                !form.btcpayStoreId.trim()
              }
              onClick={async () => {
                setBtcpayTestStatus(null);
                try {
                  await testBtcpay.mutateAsync({
                    backend: form.backend.trim(),
                    store_id: form.btcpayStoreId.trim(),
                  });
                  setBtcpayTestStatus({
                    ok: true,
                    storeId: form.btcpayStoreId.trim(),
                  });
                } catch (error) {
                  setBtcpayTestStatus({
                    ok: false,
                    message:
                      error instanceof Error
                        ? error.message
                        : "BTCPay test failed.",
                  });
                }
              }}
            >
              {testBtcpay.isPending ? "Testing…" : "Test connection"}
            </Button>
            {btcpayTestStatus?.ok ? (
              <span className="text-xs text-emerald-700 dark:text-emerald-300">
                Store {btcpayTestStatus.storeId} responded.
              </span>
            ) : null}
            {btcpayTestStatus && !btcpayTestStatus.ok ? (
              <span className="text-xs text-destructive">
                {btcpayTestStatus.message}
              </span>
            ) : null}
          </div>
          {renderSyncAfterCreate("Refresh after setup")}
        </>
      );
    }

    if (setupKind === "bip329") {
      return (
        <>
          <SetupField
            id="connection-bip329-file"
            label="Label file path"
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
                      title: "Select BIP329 label file",
                      filters: [
                        { name: "BIP329 JSONL", extensions: ["jsonl", "json"] },
                      ],
                    });
                    if (picked) updateForm("bip329File", picked);
                  }}
                >
                  Browse…
                </Button>
              ) : null}
            </div>
          </SetupField>
          <SetupField
            id="connection-bip329-wallet"
            label="Target wallet label"
            helper="Optional. When set, label records target this wallet's transactions; otherwise they import without a wallet scope."
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
          <p>
            This connection is configured as a backend endpoint in Settings.
          </p>
          <p>
            Wallet setup can then select that backend by name without exposing
            endpoint URLs or credentials in the connection modal.
          </p>
        </div>
      );
    }

    return (
      <div className="rounded-md border bg-background p-3 text-sm text-muted-foreground">
        Dedicated setup is not wired yet.
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
            {selected.status === "ready" ? "Available" : "Planned"}
          </Badge>
          {selected.docsHref ? (
            <a
              className="rounded-md border px-2 py-0.5 text-xs text-muted-foreground hover:text-foreground"
              href={selected.docsHref}
              target="_blank"
              rel="noreferrer"
            >
              Docs
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
              placeholder="Search sources (e.g. river, descriptor, btcpay)…"
              className="h-9 max-w-sm"
            />
          </div>
          <div className="min-h-0 flex-1 space-y-3 overflow-y-auto p-4">
          {visibleSources.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {sourceQuery.trim()
                ? "No sources match that search."
                : "No sources match this filter."}
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
                      {source.status === "ready" ? "Ready" : "Planned"}
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
                  Importing {syncProgress.wallet}…
                </span>
                <span className="font-medium tabular-nums">
                  {syncProgress.processed.toLocaleString()} /{" "}
                  {syncProgress.total.toLocaleString()} rows
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
        </form>
      </div>
    </div>
  );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="grid h-[calc(100dvh-2rem)] max-h-[calc(100dvh-2rem)] grid-rows-[auto_minmax(0,1fr)_auto] sm:h-[740px] sm:max-w-[960px] lg:max-w-[1040px]">
        <DialogHeader>
          <DialogTitle>
            {isSetupStep ? `Set up ${selected.title}` : "Add connection"}
          </DialogTitle>
          <DialogDescription>
            {isSetupStep
              ? "Enter the local details Kassiber needs for this connection."
              : "Choose a watch-only wallet, node, exchange, or local file source."}
          </DialogDescription>
          <div className="flex items-center gap-2 pt-1 text-xs text-muted-foreground">
            <span
              className={cn(
                "flex size-5 items-center justify-center rounded-full border text-[11px]",
                !isSetupStep && "border-primary bg-primary text-primary-foreground",
              )}
            >
              1
            </span>
            <span>Choose source</span>
            <span className="h-px w-6 bg-border" aria-hidden="true" />
            <span
              className={cn(
                "flex size-5 items-center justify-center rounded-full border text-[11px]",
                isSetupStep && "border-primary bg-primary text-primary-foreground",
              )}
            >
              2
            </span>
            <span>Set up</span>
          </div>
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
              Back
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
              Cancel
            </Button>
            {isSetupStep ? (
              <Button
                type="submit"
                form="connection-setup-form"
                disabled={isSubmitting || setupKind === "planned" || missingBackend}
              >
                {submitLabel}
              </Button>
            ) : (
              <Button
                type="button"
                disabled={!canContinue}
                onClick={() => setStep("setup")}
              >
                {selected.status === "ready" ? "Continue" : "Planned"}
              </Button>
            )}
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
