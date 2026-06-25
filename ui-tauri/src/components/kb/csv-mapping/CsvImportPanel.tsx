/**
 * Generic CSV import, embedded in the Add Connection modal.
 *
 * Flow: download a fill-in example → the user pastes their transactions into it
 * (or uses their own file) → choose it back → Kassiber auto-detects the columns
 * and shows a preview to confirm. When detection isn't confident, the user is
 * pointed at the example and can fall back to mapping the columns by hand
 * (the same engine, surfaced as an "Advanced" expander).
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  CheckCircle2,
  Download,
  FileSpreadsheet,
  FolderOpen,
  Loader2,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { isFilePickerAvailable, pickFile, saveFile } from "@/lib/filePicker";
import { useUiStore } from "@/store/ui";

import { MappingControls } from "./MappingControls";
import { MappingPreview } from "./MappingPreview";
import { buildSpec, defaultSpec, specToDraft, validateSpec, type DraftSpec } from "./spec";
import type { CsvExampleResult, CsvPreviewResult, ImportMappedResult } from "./types";

function useDebouncedValue<T>(value: T, delay: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const id = setTimeout(() => setDebounced(value), delay);
    return () => clearTimeout(id);
  }, [value, delay]);
  return debounced;
}

function basename(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

export function CsvImportPanel({ onDone }: { onDone: () => void }) {
  const { t } = useTranslation("csvMapping");
  const lang = useUiStore((state) => state.lang);
  const addNotification = useUiStore((state) => state.addNotification);

  const [label, setLabel] = useState("");
  const [file, setFile] = useState<string | null>(null);
  const [advanced, setAdvanced] = useState(false);
  const [draft, setDraft] = useState<DraftSpec>(() => defaultSpec());
  const [onlyProblems, setOnlyProblems] = useState(false);
  const [downloadMsg, setDownloadMsg] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [result, setResult] = useState<ImportMappedResult | null>(null);
  const seededDraft = useRef(false);

  const csvExample = useDaemonMutation<CsvExampleResult>("ui.wallets.csv_example");
  const createWallet = useDaemonMutation<{ wallet: { label: string } }>("ui.wallets.create");
  const importMapped = useDaemonMutation<ImportMappedResult>("ui.wallets.import_mapped_csv");

  const debouncedDraft = useDebouncedValue(draft, 300);
  const advancedValid = !advanced || validateSpec(debouncedDraft).length === 0;
  const previewEnabled = !!file && advancedValid;
  const previewArgs = useMemo(
    () => ({
      source_file: file ?? "",
      ...(advanced ? { mapping: buildSpec(debouncedDraft) } : {}),
    }),
    [file, advanced, debouncedDraft],
  );
  const previewQuery = useDaemon<CsvPreviewResult>("ui.wallets.csv_preview", previewArgs, {
    enabled: previewEnabled,
    retry: false,
  });
  const preview = previewEnabled ? (previewQuery.data?.data ?? null) : null;
  const previewError =
    previewEnabled && previewQuery.error instanceof Error ? t("error.previewFailed") : null;
  const headers = preview?.headers ?? [];

  // Seed the Advanced editor from the auto-detected guess the first time it opens.
  useEffect(() => {
    if (advanced && !seededDraft.current && preview?.mapping) {
      seededDraft.current = true;
      setDraft(specToDraft(preview.mapping));
    }
  }, [advanced, preview]);

  const onPick = useCallback(async () => {
    try {
      const picked = await pickFile({
        title: t("panel.pick"),
        filters: [{ name: "CSV", extensions: ["csv", "tsv", "txt"] }],
      });
      if (picked) {
        setActionError(null);
        setResult(null);
        setAdvanced(false);
        seededDraft.current = false;
        setDraft(defaultSpec());
        setFile(picked);
        if (!label.trim()) setLabel(basename(picked).replace(/\.[^.]+$/, ""));
      }
    } catch (error: unknown) {
      setActionError(error instanceof Error ? error.message : t("error.inspectFailed"));
    }
  }, [t, label]);

  const onDownload = useCallback(async () => {
    setActionError(null);
    try {
      if (isFilePickerAvailable) {
        const target = await saveFile({
          title: t("panel.download"),
          defaultPath: "kassiber-import-example.csv",
          filters: [{ name: "CSV", extensions: ["csv"] }],
        });
        if (!target) return;
        const envelope = await csvExample.mutateAsync({ target_file: target });
        setDownloadMsg(t("panel.downloaded", { file: basename(envelope.data?.file ?? target) }));
      } else {
        const envelope = await csvExample.mutateAsync({});
        const text = envelope.data?.csv ?? "";
        const url = URL.createObjectURL(new Blob([text], { type: "text/csv" }));
        const anchor = document.createElement("a");
        anchor.href = url;
        anchor.download = "kassiber-import-example.csv";
        anchor.click();
        URL.revokeObjectURL(url);
      }
    } catch (error: unknown) {
      setActionError(error instanceof Error ? error.message : t("error.inspectFailed"));
    }
  }, [t, csvExample]);

  const canImport =
    !!file &&
    !!label.trim() &&
    !!preview &&
    preview.mapped > 0 &&
    advancedValid &&
    !createWallet.isPending &&
    !importMapped.isPending;

  const onImport = useCallback(async () => {
    if (!file || !label.trim()) return;
    setActionError(null);
    const name = label.trim();
    try {
      await createWallet.mutateAsync({ label: name, kind: "custom" });
      const envelope = await importMapped.mutateAsync({
        wallet: name,
        source_file: file,
        ...(advanced ? { mapping: buildSpec(draft) } : {}),
      });
      const data = envelope.data ?? null;
      setResult(data);
      addNotification({
        title: t("panel.doneTitle"),
        body: t("panel.done", { imported: data?.imported ?? 0, skipped: data?.skipped ?? 0 }),
        tone: "success",
      });
    } catch (error: unknown) {
      setActionError(error instanceof Error ? error.message : t("error.importFailed"));
    }
  }, [file, label, advanced, draft, createWallet, importMapped, addNotification, t]);

  // --- success view ---------------------------------------------------- //
  if (result) {
    return (
      <div className="flex flex-col items-center gap-4 py-10 text-center">
        <CheckCircle2 className="size-9 text-emerald-500" aria-hidden="true" />
        <h3 className="text-lg font-semibold">{t("panel.doneTitle")}</h3>
        <p className="text-sm text-muted-foreground">
          {t("panel.done", { imported: result.imported ?? 0, skipped: result.skipped ?? 0 })}
        </p>
        {result.filtered > 0 ? (
          <p className="text-xs text-muted-foreground">
            {t("result.withFiltered", { filtered: result.filtered })}
          </p>
        ) : null}
        <div className="flex gap-2">
          <Button variant="outline" onClick={onDone}>
            {t("result.viewTransactions")}
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="space-y-1.5">
        <Label htmlFor="csv-connection-name" className="text-xs font-medium text-muted-foreground">
          {t("panel.label")}
        </Label>
        <Input
          id="csv-connection-name"
          value={label}
          placeholder={t("panel.labelPlaceholder")}
          onChange={(e) => setLabel(e.target.value)}
        />
      </div>

      {/* Step 1 — download the example */}
      <div className="rounded-md border bg-muted/20 p-3">
        <p className="mb-2 text-sm text-muted-foreground">{t("panel.downloadStep")}</p>
        <div className="flex flex-wrap items-center gap-2">
          <Button type="button" variant="outline" size="sm" onClick={() => void onDownload()} disabled={csvExample.isPending}>
            {csvExample.isPending ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <Download className="size-4" aria-hidden="true" />
            )}
            {csvExample.isPending ? t("panel.downloading") : t("panel.download")}
          </Button>
          {downloadMsg ? <span className="text-xs text-muted-foreground">{downloadMsg}</span> : null}
        </div>
      </div>

      {/* Step 2 — choose the filled-in file */}
      <div className="space-y-2">
        <p className="text-sm text-muted-foreground">{t("panel.uploadStep")}</p>
        <div className="flex flex-wrap items-center gap-3 rounded-md border px-3 py-2">
          <FileSpreadsheet className="size-4 text-muted-foreground" aria-hidden="true" />
          <span className="min-w-0 flex-1 truncate text-sm" data-testid="csv-file-name">
            {file ? basename(file) : t("file.none")}
          </span>
          {isFilePickerAvailable ? (
            <Button type="button" variant="outline" size="sm" onClick={() => void onPick()}>
              <FolderOpen className="size-4" aria-hidden="true" />
              {file ? t("panel.change") : t("panel.pick")}
            </Button>
          ) : (
            <Input
              className="h-9 w-[240px]"
              placeholder="/path/to/file.csv"
              value={file ?? ""}
              onChange={(e) => setFile(e.target.value || null)}
            />
          )}
        </div>
        {!isFilePickerAvailable ? (
          <p className="text-[11px] text-muted-foreground">{t("panel.pickUnavailable")}</p>
        ) : null}
      </div>

      {/* Detection + preview */}
      {file ? (
        <div className="space-y-3">
          {previewQuery.isFetching && !preview ? (
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
              {t("preview.loading")}
            </div>
          ) : null}

          {preview && preview.confident === false && !advanced ? (
            <div className="space-y-1 rounded-md border border-amber-500/40 bg-amber-500/5 px-3 py-2 text-xs text-amber-700 dark:text-amber-400">
              <p className="flex items-center gap-2 font-medium">
                <AlertTriangle className="size-4 shrink-0" aria-hidden="true" />
                {t("panel.notRecognized")}
              </p>
              <p>{t("panel.notRecognizedHint")}</p>
            </div>
          ) : null}

          {preview && preview.detected && preview.detected.length > 0 && preview.confident !== false ? (
            <p className="text-xs text-muted-foreground">
              {t("panel.recognized")}{" "}
              {preview.detected
                .map((d) => `${d.column} → ${t(`panel.field.${d.field}` as never)}`)
                .join(", ")}
            </p>
          ) : null}

          {preview && (preview.confident !== false || advanced) ? (
            <div className="max-h-[320px] overflow-auto rounded-md border p-2">
              <MappingPreview
                preview={preview}
                loading={previewQuery.isFetching}
                error={previewError}
                onlyProblems={onlyProblems}
                setOnlyProblems={setOnlyProblems}
                lang={lang}
              />
            </div>
          ) : null}

          <Collapsible open={advanced} onOpenChange={setAdvanced}>
            <CollapsibleTrigger asChild>
              <Button type="button" variant="ghost" size="sm" className="px-0 text-xs text-muted-foreground">
                {t("panel.advanced")}
              </Button>
            </CollapsibleTrigger>
            <CollapsibleContent className="pt-2">
              <MappingControls draft={draft} setDraft={setDraft} headers={headers} />
            </CollapsibleContent>
          </Collapsible>
        </div>
      ) : null}

      {actionError ? (
        <div className="flex items-center gap-2 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive">
          <AlertTriangle className="size-4 shrink-0" aria-hidden="true" />
          <span>{actionError}</span>
        </div>
      ) : null}

      {/* Import action */}
      <div className="flex flex-wrap items-center justify-between gap-2 border-t pt-3">
        <span className="text-xs text-muted-foreground">
          {label.trim() ? t("panel.createsWallet", { label: label.trim() }) : null}
        </span>
        <Button type="button" disabled={!canImport} onClick={() => void onImport()}>
          {importMapped.isPending || createWallet.isPending
            ? t("panel.importing")
            : t("panel.import", { count: preview?.mapped ?? 0 })}
        </Button>
      </div>
    </div>
  );
}
