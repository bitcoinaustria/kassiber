import { FileImage, Loader2, ScanText } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

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
  DialogTrigger,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
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
  isFilePickerAvailable,
  pickDocumentImportSource,
  type DocumentImportSourceSelection,
} from "@/lib/filePicker";
import type { AiProvidersListData } from "@/lib/aiCapabilities";
import {
  buildDocumentImportArgs,
  buildDocumentImportPreviewArgs,
  canImportDocumentDraft,
} from "@/lib/documentImport";
import { cn } from "@/lib/utils";
import { bookIdentityKey, useUiStore } from "@/store/ui";

interface WalletListData {
  wallets: Array<{ id: string; label: string; kind?: string }>;
}

export interface DocumentDraftRow {
  id: string;
  status: "ready" | "quarantined" | string;
  flags?: string[];
  confidence?: number;
  confidence_threshold?: number;
  cell_confidences?: Record<string, number>;
  source_region?: {
    page?: number;
    x?: number;
    y?: number;
    width?: number;
    height?: number;
    unit?: string;
  } | null;
  evidence_text?: string | null;
  record?: {
    occurred_at?: string | null;
    direction?: string | null;
    asset?: string | null;
    amount_btc?: string | null;
    fee_btc?: string | null;
    fee_defaulted?: boolean;
    fiat_currency?: string | null;
    fiat_value?: string | null;
    fiat_rate?: string | null;
    counterparty?: string | null;
    description?: string | null;
  };
  import_record?: Record<string, unknown> | null;
}

export interface DocumentDraft {
  document_token: string;
  source: {
    filename: string;
    media_type?: string;
    sha256?: string;
    kind?: "pdf" | "image" | string;
    pdf?: {
      total_pages: number;
      rendered_pages: number[];
      complete: boolean;
      selection_explicit: boolean;
      selection: string;
    };
  };
  model: string;
  recommendations?: Array<{ id: string; command?: string; use?: string }>;
  confidence_threshold?: number;
  rows: DocumentDraftRow[];
  summary: {
    rows: number;
    ready: number;
    quarantined: number;
    has_importable_rows?: boolean;
  };
}

interface DocumentImportResult {
  imported?: number;
  updated?: number;
  unchanged?: number;
  draft_rows_imported?: number;
  quarantined_skipped?: number;
  attached_evidence?: unknown[];
}

function confidenceLabel(value: number | undefined) {
  if (typeof value !== "number" || Number.isNaN(value)) return "—";
  return `${Math.round(value * 100)}%`;
}

interface DocumentDraftReviewTableProps {
  draft: DocumentDraft;
  selectedRows: ReadonlySet<string>;
  onToggleRow?: (rowId: string, checked: boolean) => void;
  onToggleAllReady?: (checked: boolean) => void;
}

export function DocumentDraftReviewTable({
  draft,
  selectedRows,
  onToggleRow = () => undefined,
  onToggleAllReady = () => undefined,
}: DocumentDraftReviewTableProps) {
  const { t } = useTranslation("transactions");
  const readyRows = draft.rows.filter((row) => row.status === "ready");

  return (
    <div className="overflow-x-auto rounded-lg border">
      <Table className="min-w-[78rem]">
        <TableHeader>
          <TableRow>
            <TableHead className="w-10">
              <Checkbox
                checked={
                  readyRows.length > 0 && selectedRows.size === readyRows.length
                }
                onCheckedChange={(checked) => onToggleAllReady(checked === true)}
                aria-label={t("documentImport.selectAll")}
              />
            </TableHead>
            <TableHead>{t("documentImport.date")}</TableHead>
            <TableHead>{t("documentImport.direction")}</TableHead>
            <TableHead>{t("documentImport.asset")}</TableHead>
            <TableHead>{t("documentImport.amount")}</TableHead>
            <TableHead>{t("documentImport.fee")}</TableHead>
            <TableHead>{t("documentImport.fiatValue")}</TableHead>
            <TableHead>{t("documentImport.fiatRate")}</TableHead>
            <TableHead>{t("documentImport.counterparty")}</TableHead>
            <TableHead>{t("documentImport.confidence")}</TableHead>
            <TableHead>{t("documentImport.status")}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {draft.rows.map((row) => {
            const record = row.record ?? {};
            const ready = row.status === "ready";
            const flags = row.flags ?? [];
            const confidences = Object.entries(row.cell_confidences ?? {}).sort(
              ([left], [right]) => left.localeCompare(right),
            );
            return (
              <React.Fragment key={row.id}>
                <TableRow>
                  <TableCell>
                    <Checkbox
                      checked={selectedRows.has(row.id)}
                      disabled={!ready}
                      onCheckedChange={(checked) =>
                        onToggleRow(row.id, checked === true)
                      }
                      aria-label={t("documentImport.selectRow", { row: row.id })}
                    />
                  </TableCell>
                  <TableCell>{record.occurred_at ?? "—"}</TableCell>
                  <TableCell>{record.direction ?? "—"}</TableCell>
                  <TableCell>{record.asset ?? "—"}</TableCell>
                  <TableCell>{record.amount_btc ?? "—"}</TableCell>
                  <TableCell>
                    {record.fee_btc ?? "—"}
                    {record.fee_defaulted ? (
                      <span className="block text-xs text-muted-foreground">
                        {t("documentImport.feeNotProvided")}
                      </span>
                    ) : null}
                  </TableCell>
                  <TableCell>
                    {record.fiat_value
                      ? `${record.fiat_value} ${record.fiat_currency ?? "?"}`
                      : "—"}
                  </TableCell>
                  <TableCell>
                    {record.fiat_rate
                      ? `${record.fiat_rate} ${record.fiat_currency ?? "?"}/BTC`
                      : "—"}
                  </TableCell>
                  <TableCell>{record.counterparty ?? "—"}</TableCell>
                  <TableCell>{confidenceLabel(row.confidence)}</TableCell>
                  <TableCell>
                    <Badge
                      variant={ready ? "default" : "outline"}
                      className={cn(!ready && "text-amber-700")}
                    >
                      {ready
                        ? t("documentImport.statusReady")
                        : t("documentImport.statusQuarantined")}
                    </Badge>
                  </TableCell>
                </TableRow>
                <TableRow className="bg-muted/20">
                  <TableCell />
                  <TableCell colSpan={10} className="whitespace-normal py-2">
                    <div className="grid gap-1 text-xs text-muted-foreground lg:grid-cols-2">
                      <p>
                        <span className="font-medium text-foreground">
                          {t("documentImport.descriptionField")}:
                        </span>{" "}
                        {record.description ?? "—"}
                      </p>
                      <p>
                        <span className="font-medium text-foreground">
                          {t("documentImport.sourcePage")}:
                        </span>{" "}
                        {row.source_region?.page ?? "—"}
                      </p>
                      <p className="lg:col-span-2">
                        <span className="font-medium text-foreground">
                          {t("documentImport.evidence")}:
                        </span>{" "}
                        {row.evidence_text ?? "—"}
                      </p>
                      <p className="lg:col-span-2">
                        <span className="font-medium text-foreground">
                          {t("documentImport.cellConfidences")}:
                        </span>{" "}
                        {confidences.length
                          ? confidences
                              .map(
                                ([field, value]) =>
                                  `${field} ${confidenceLabel(value)}`,
                              )
                              .join(" · ")
                          : "—"}
                      </p>
                      {flags.length ? (
                        <p className="lg:col-span-2 text-amber-700">
                          {t("documentImport.flags")}: {flags.join(", ")}
                        </p>
                      ) : null}
                    </div>
                  </TableCell>
                </TableRow>
              </React.Fragment>
            );
          })}
        </TableBody>
      </Table>
    </div>
  );
}

export function DocumentImportDialog() {
  const { t } = useTranslation(["transactions", "common"]);
  const addNotification = useUiStore((s) => s.addNotification);
  const aiFeaturesEnabled = useUiStore((s) => s.aiFeaturesEnabled);
  const daemonSession = useUiStore((s) => s.daemonSession);
  const bookKey = useUiStore((s) => bookIdentityKey(s.identity));
  const walletsQuery = useDaemon<WalletListData>("ui.wallets.list");
  const providersQuery = useDaemon<AiProvidersListData>("ai.providers.list");
  const previewDocument = useDaemonMutation<DocumentDraft>(
    "ui.wallets.document_import.preview",
  );
  const importDocument = useDaemonMutation<DocumentImportResult>(
    "ui.wallets.document_import.import",
  );
  const wallets = React.useMemo(
    () =>
      walletsQuery.data?.kind === "ui.wallets.list"
        ? (walletsQuery.data.data?.wallets ?? [])
        : [],
    [walletsQuery.data],
  );
  const localProviders = React.useMemo(
    () =>
      providersQuery.data?.kind === "ai.providers.list"
        ? (providersQuery.data.data?.providers ?? []).filter(
            (entry) => entry.kind === "local",
          )
        : [],
    [providersQuery.data],
  );
  const [open, setOpen] = React.useState(false);
  const [sourceSelection, setSourceSelection] =
    React.useState<DocumentImportSourceSelection | null>(null);
  const [wallet, setWallet] = React.useState("");
  const [provider, setProvider] = React.useState("");
  const [model, setModel] = React.useState("");
  const [pageRange, setPageRange] = React.useState("");
  const [draft, setDraft] = React.useState<DocumentDraft | null>(null);
  const [selectedRows, setSelectedRows] = React.useState<Set<string>>(
    () => new Set(),
  );
  const [pickerBusy, setPickerBusy] = React.useState(false);
  const requestGeneration = React.useRef(0);
  const scopeKey = `${daemonSession}:${bookKey ?? "none"}`;
  const previousScopeKey = React.useRef(scopeKey);

  const resetSensitiveState = React.useCallback(() => {
    requestGeneration.current += 1;
    setSourceSelection(null);
    setWallet("");
    setModel("");
    setPageRange("");
    setDraft(null);
    setSelectedRows(new Set());
    setPickerBusy(false);
  }, []);

  React.useEffect(() => {
    if (previousScopeKey.current === scopeKey) return;
    previousScopeKey.current = scopeKey;
    setOpen(false);
    resetSensitiveState();
  }, [resetSensitiveState, scopeKey]);

  React.useEffect(() => {
    if (wallet && wallets.some((entry) => entry.id === wallet)) return;
    setWallet(wallets[0]?.id ?? "");
  }, [wallet, wallets]);

  React.useEffect(() => {
    if (!provider && localProviders[0]?.name) {
      setProvider(localProviders[0].name);
    }
  }, [localProviders, provider]);

  const isBusy = previewDocument.isPending || importDocument.isPending || pickerBusy;
  const readyRows = React.useMemo(
    () => draft?.rows.filter((row) => row.status === "ready") ?? [],
    [draft],
  );
  const selectedCount = selectedRows.size;
  const canPreview =
    Boolean(sourceSelection?.document_token) &&
    Boolean(provider) &&
    !previewDocument.isPending &&
    !pickerBusy;
  const canImport = canImportDocumentDraft({
    hasDraft: Boolean(draft),
    wallet,
    selectedCount,
    pickerBusy,
    previewPending: previewDocument.isPending,
    importPending: importDocument.isPending,
  });

  const chooseFile = React.useCallback(async () => {
    const generation = ++requestGeneration.current;
    setDraft(null);
    setSelectedRows(new Set());
    setPickerBusy(true);
    try {
      const picked = await pickDocumentImportSource();
      if (generation === requestGeneration.current && picked) {
        setSourceSelection(picked);
      }
    } catch (error) {
      if (generation !== requestGeneration.current) return;
      addNotification({
        title: t("documentImport.pickFailed"),
        body: error instanceof Error ? error.message : String(error),
        tone: "error",
      });
    } finally {
      if (generation === requestGeneration.current) setPickerBusy(false);
    }
  }, [addNotification, t]);

  const runPreview = React.useCallback(() => {
    if (!sourceSelection) return;
    const generation = ++requestGeneration.current;
    setDraft(null);
    setSelectedRows(new Set());
    previewDocument.mutate(
      buildDocumentImportPreviewArgs(
        sourceSelection.document_token,
        provider,
        model,
        pageRange,
      ),
      {
        onSuccess: (envelope) => {
          if (generation !== requestGeneration.current) return;
          const payload = envelope.data;
          setDraft(payload ?? null);
          setSelectedRows(new Set());
        },
        onError: (error) => {
          if (generation !== requestGeneration.current) return;
          addNotification({
            title: t("documentImport.previewFailed"),
            body: error instanceof Error ? error.message : String(error),
            tone: "error",
          });
        },
      },
    );
  }, [addNotification, model, pageRange, previewDocument, provider, sourceSelection, t]);

  const runImport = React.useCallback(() => {
    if (!draft || !canImport) return;
    const generation = requestGeneration.current;
    importDocument.mutate(
      buildDocumentImportArgs(draft.document_token, wallet, selectedRows),
      {
        onSuccess: (envelope) => {
          if (generation !== requestGeneration.current) return;
          const payload = envelope.data;
          addNotification({
            title: t("documentImport.importDone"),
            body: t("documentImport.importSummary", {
              count: payload?.draft_rows_imported ?? 0,
              attachments: payload?.attached_evidence?.length ?? 0,
            }),
            tone: "success",
          });
          setOpen(false);
          resetSensitiveState();
        },
        onError: (error) => {
          if (generation !== requestGeneration.current) return;
          addNotification({
            title: t("documentImport.importFailed"),
            body: error instanceof Error ? error.message : String(error),
            tone: "error",
          });
        },
      },
    );
  }, [
    addNotification,
    canImport,
    draft,
    importDocument,
    resetSensitiveState,
    selectedRows,
    t,
    wallet,
  ]);

  const handleOpenChange = React.useCallback(
    (nextOpen: boolean) => {
      setOpen(nextOpen);
      if (!nextOpen) resetSensitiveState();
    },
    [resetSensitiveState],
  );

  const toggleRow = React.useCallback((rowId: string, checked: boolean) => {
    setSelectedRows((current) => {
      const next = new Set(current);
      if (checked) next.add(rowId);
      else next.delete(rowId);
      return next;
    });
  }, []);

  const toggleAllReady = React.useCallback((checked: boolean) => {
    setSelectedRows(checked ? new Set(readyRows.map((row) => row.id)) : new Set());
  }, [readyRows]);

  if (!aiFeaturesEnabled) return null;

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          className="h-8 gap-2 sm:h-9"
          aria-label={t("documentImport.triggerAria")}
          disabled
        >
          <ScanText className="size-4" aria-hidden="true" />
          <span className="hidden sm:inline">{t("documentImport.trigger")}</span>
        </Button>
      </DialogTrigger>
      <DialogContent className="flex max-h-[calc(100vh-1rem)] max-w-[calc(100vw-1rem)] flex-col overflow-hidden p-0 sm:max-w-[58rem]">
        <DialogHeader className="shrink-0 px-5 pt-4 pb-2 pr-12">
          <DialogTitle>{t("documentImport.title")}</DialogTitle>
          <DialogDescription>{t("documentImport.description")}</DialogDescription>
        </DialogHeader>

        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 pb-3">
          <div className="grid gap-3 lg:grid-cols-[1.4fr_1fr]">
            <div className="space-y-1.5">
              <Label htmlFor="document-import-source">
                {t("documentImport.source")}
              </Label>
              <div className="flex gap-2">
                <Input
                  id="document-import-source"
                  value={sourceSelection?.source.filename ?? ""}
                  readOnly
                  disabled={previewDocument.isPending || pickerBusy}
                  placeholder={t("documentImport.sourcePlaceholder")}
                />
                <Button
                  type="button"
                  variant="outline"
                  className="shrink-0 gap-2"
                  disabled={!isFilePickerAvailable || pickerBusy || previewDocument.isPending}
                  onClick={() => void chooseFile()}
                >
                  {pickerBusy ? (
                    <Loader2 className="size-4 animate-spin" aria-hidden="true" />
                  ) : (
                    <FileImage className="size-4" aria-hidden="true" />
                  )}
                  {t("documentImport.pick")}
                </Button>
              </div>
            </div>
            <div className="grid gap-3 sm:grid-cols-3 lg:grid-cols-1">
              <div className="space-y-1.5">
                <Label>{t("documentImport.wallet")}</Label>
                <Select value={wallet} onValueChange={setWallet}>
                  <SelectTrigger>
                    <SelectValue placeholder={t("documentImport.walletPlaceholder")} />
                  </SelectTrigger>
                  <SelectContent>
                    {wallets.map((entry) => (
                      <SelectItem key={entry.id} value={entry.id}>
                        {entry.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-1.5">
                <Label>{t("documentImport.provider")}</Label>
                <Select
                  value={provider}
                  disabled={previewDocument.isPending || pickerBusy}
                  onValueChange={(value) => {
                    setProvider(value);
                    setDraft(null);
                    setSelectedRows(new Set());
                  }}
                >
                  <SelectTrigger>
                    <SelectValue placeholder={t("documentImport.providerPlaceholder")} />
                  </SelectTrigger>
                  <SelectContent>
                    {localProviders.map((entry) => (
                      <SelectItem key={entry.name} value={entry.name}>
                        {entry.display_name || entry.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="document-import-model">
                  {t("documentImport.model")}
                </Label>
                <Input
                  id="document-import-model"
                  value={model}
                  disabled={previewDocument.isPending || pickerBusy}
                  onChange={(event) => {
                    setModel(event.target.value);
                    setDraft(null);
                    setSelectedRows(new Set());
                  }}
                  placeholder={t("documentImport.modelPlaceholder")}
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="document-import-pages">
                  {t("documentImport.pages")}
                </Label>
                <Input
                  id="document-import-pages"
                  value={pageRange}
                  disabled={previewDocument.isPending || pickerBusy}
                  onChange={(event) => {
                    setPageRange(event.target.value);
                    setDraft(null);
                    setSelectedRows(new Set());
                  }}
                  placeholder={t("documentImport.pagesPlaceholder")}
                />
              </div>
            </div>
          </div>

          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
              {draft ? (
                <>
                  <Badge variant="outline">{draft.model}</Badge>
                  <span>{t("documentImport.ready", { count: draft.summary.ready })}</span>
                  <span>{t("documentImport.quarantined", { count: draft.summary.quarantined })}</span>
                  {draft.source.pdf ? (
                    <Badge
                      variant="outline"
                      className={cn(!draft.source.pdf.complete && "border-amber-500 text-amber-700")}
                    >
                      {draft.source.pdf.complete
                        ? t("documentImport.pdfComplete", {
                            count: draft.source.pdf.total_pages,
                          })
                        : t("documentImport.pdfPartial", {
                            selection: draft.source.pdf.selection,
                            total: draft.source.pdf.total_pages,
                          })}
                    </Badge>
                  ) : null}
                </>
              ) : (
                <span>
                  {localProviders.length
                    ? t("documentImport.recommendations")
                    : t("documentImport.noLocalProvider")}
                </span>
              )}
            </div>
            <Button
              type="button"
              variant="secondary"
              className="gap-2"
              disabled={!canPreview}
              onClick={runPreview}
            >
              {previewDocument.isPending ? (
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              ) : (
                <ScanText className="size-4" aria-hidden="true" />
              )}
              {t("documentImport.preview")}
            </Button>
          </div>

          {draft ? (
            <DocumentDraftReviewTable
              draft={draft}
              selectedRows={selectedRows}
              onToggleRow={toggleRow}
              onToggleAllReady={toggleAllReady}
            />
          ) : null}
        </div>

        <DialogFooter className="shrink-0 border-t px-5 py-3">
          <Button
            type="button"
            variant="outline"
            onClick={() => handleOpenChange(false)}
            disabled={isBusy}
          >
            {t("common:actions.cancel")}
          </Button>
          <Button type="button" disabled={!canImport} onClick={runImport} className="gap-2">
            {importDocument.isPending ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : null}
            {t("documentImport.importSelected", { count: selectedCount })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
