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
import { isFilePickerAvailable, pickFile } from "@/lib/filePicker";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

interface WalletListData {
  wallets: Array<{ id: string; label: string; kind?: string }>;
}

interface DocumentDraftRow {
  id: string;
  status: "ready" | "quarantined" | string;
  flags?: string[];
  confidence?: number;
  evidence_text?: string | null;
  record?: {
    occurred_at?: string | null;
    direction?: string | null;
    asset?: string | null;
    amount_btc?: string | null;
    fee_btc?: string | null;
    fiat_currency?: string | null;
    fiat_value?: string | null;
    counterparty?: string | null;
    description?: string | null;
  };
  import_record?: Record<string, unknown> | null;
}

interface DocumentDraft {
  source: {
    path: string;
    filename: string;
    media_type?: string;
    sha256?: string;
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

function rowTitle(row: DocumentDraftRow) {
  const record = row.record ?? {};
  return [
    record.occurred_at,
    record.direction,
    record.amount_btc ? `${record.amount_btc} ${record.asset ?? "BTC"}` : null,
    record.counterparty,
  ]
    .filter(Boolean)
    .join(" · ");
}

function confidenceLabel(value: number | undefined) {
  if (typeof value !== "number" || Number.isNaN(value)) return "—";
  return `${Math.round(value * 100)}%`;
}

export function DocumentImportDialog() {
  const { t } = useTranslation(["transactions", "common"]);
  const addNotification = useUiStore((s) => s.addNotification);
  const walletsQuery = useDaemon<WalletListData>("ui.wallets.list");
  const previewDocument = useDaemonMutation<DocumentDraft>(
    "ui.wallets.document_import.preview",
  );
  const importDocument = useDaemonMutation<DocumentImportResult>(
    "ui.wallets.document_import.import",
  );
  const wallets =
    walletsQuery.data?.kind === "ui.wallets.list"
      ? (walletsQuery.data.data?.wallets ?? [])
      : [];
  const [open, setOpen] = React.useState(false);
  const [sourceFile, setSourceFile] = React.useState("");
  const [wallet, setWallet] = React.useState("");
  const [model, setModel] = React.useState("");
  const [draft, setDraft] = React.useState<DocumentDraft | null>(null);
  const [selectedRows, setSelectedRows] = React.useState<Set<string>>(
    () => new Set(),
  );
  const [pickerBusy, setPickerBusy] = React.useState(false);

  React.useEffect(() => {
    if (!wallet && wallets[0]?.id) {
      setWallet(wallets[0].id);
    }
  }, [wallet, wallets]);

  const isBusy = previewDocument.isPending || importDocument.isPending || pickerBusy;
  const readyRows = draft?.rows.filter((row) => row.status === "ready") ?? [];
  const selectedCount = selectedRows.size;
  const canPreview = Boolean(sourceFile.trim()) && !previewDocument.isPending;
  const canImport =
    Boolean(draft) && Boolean(wallet) && selectedCount > 0 && !importDocument.isPending;

  const chooseFile = React.useCallback(async () => {
    setPickerBusy(true);
    try {
      const picked = await pickFile({
        title: t("documentImport.pickTitle"),
        filters: [
          {
            name: t("documentImport.fileFilter"),
            extensions: ["png", "jpg", "jpeg", "webp", "gif", "pdf"],
          },
        ],
      });
      if (picked) {
        setSourceFile(picked);
        setDraft(null);
        setSelectedRows(new Set());
      }
    } catch (error) {
      addNotification({
        title: t("documentImport.pickFailed"),
        body: error instanceof Error ? error.message : String(error),
        tone: "error",
      });
    } finally {
      setPickerBusy(false);
    }
  }, [addNotification, t]);

  const runPreview = React.useCallback(() => {
    previewDocument.mutate(
      {
        source_file: sourceFile.trim(),
        ...(model.trim() ? { model: model.trim() } : {}),
      },
      {
        onSuccess: (envelope) => {
          const payload = envelope.data;
          setDraft(payload ?? null);
          setSelectedRows(
            new Set(
              (payload?.rows ?? [])
                .filter((row) => row.status === "ready")
                .map((row) => row.id),
            ),
          );
        },
        onError: (error) => {
          addNotification({
            title: t("documentImport.previewFailed"),
            body: error instanceof Error ? error.message : String(error),
            tone: "error",
          });
        },
      },
    );
  }, [addNotification, model, previewDocument, sourceFile, t]);

  const runImport = React.useCallback(() => {
    if (!draft) return;
    importDocument.mutate(
      {
        wallet,
        draft,
        selected_row_ids: Array.from(selectedRows),
      },
      {
        onSuccess: (envelope) => {
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
          setDraft(null);
          setSelectedRows(new Set());
        },
        onError: (error) => {
          addNotification({
            title: t("documentImport.importFailed"),
            body: error instanceof Error ? error.message : String(error),
            tone: "error",
          });
        },
      },
    );
  }, [addNotification, draft, importDocument, selectedRows, t, wallet]);

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

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          className="h-8 gap-2 sm:h-9"
          aria-label={t("documentImport.triggerAria")}
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
                  value={sourceFile}
                  onChange={(event) => {
                    setSourceFile(event.target.value);
                    setDraft(null);
                    setSelectedRows(new Set());
                  }}
                  placeholder={t("documentImport.sourcePlaceholder")}
                />
                <Button
                  type="button"
                  variant="outline"
                  className="shrink-0 gap-2"
                  disabled={!isFilePickerAvailable || pickerBusy}
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
            <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-1">
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
                <Label htmlFor="document-import-model">
                  {t("documentImport.model")}
                </Label>
                <Input
                  id="document-import-model"
                  value={model}
                  onChange={(event) => setModel(event.target.value)}
                  placeholder={t("documentImport.modelPlaceholder")}
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
                </>
              ) : (
                <span>{t("documentImport.recommendations")}</span>
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
            <div className="rounded-lg border">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead className="w-10">
                      <Checkbox
                        checked={
                          readyRows.length > 0 && selectedRows.size === readyRows.length
                        }
                        onCheckedChange={(checked) => toggleAllReady(checked === true)}
                        aria-label={t("documentImport.selectAll")}
                      />
                    </TableHead>
                    <TableHead>{t("documentImport.row")}</TableHead>
                    <TableHead>{t("documentImport.confidence")}</TableHead>
                    <TableHead>{t("documentImport.status")}</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {draft.rows.map((row) => {
                    const ready = row.status === "ready";
                    const flags = row.flags ?? [];
                    return (
                      <TableRow key={row.id}>
                        <TableCell>
                          <Checkbox
                            checked={selectedRows.has(row.id)}
                            disabled={!ready}
                            onCheckedChange={(checked) =>
                              toggleRow(row.id, checked === true)
                            }
                            aria-label={t("documentImport.selectRow", {
                              row: row.id,
                            })}
                          />
                        </TableCell>
                        <TableCell className="min-w-[16rem] whitespace-normal">
                          <div className="space-y-1">
                            <p className="text-sm font-medium">
                              {rowTitle(row) || row.id}
                            </p>
                            {row.evidence_text ? (
                              <p className="line-clamp-2 text-xs text-muted-foreground">
                                {row.evidence_text}
                              </p>
                            ) : null}
                          </div>
                        </TableCell>
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
                          {flags.length ? (
                            <p className="mt-1 max-w-[14rem] truncate text-xs text-muted-foreground">
                              {flags.join(", ")}
                            </p>
                          ) : null}
                        </TableCell>
                      </TableRow>
                    );
                  })}
                </TableBody>
              </Table>
            </div>
          ) : null}
        </div>

        <DialogFooter className="shrink-0 border-t px-5 py-3">
          <Button
            type="button"
            variant="outline"
            onClick={() => setOpen(false)}
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
