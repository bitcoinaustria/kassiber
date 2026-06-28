/**
 * A read-only preview for the generic-ledger import (Add Connection modal).
 *
 * Calls the `ui.wallets.ledger_preview` daemon kind on the chosen file and shows
 * what would import — counts, a few normalized rows, and row-numbered problems —
 * so the user can confirm before clicking import. Nothing is persisted.
 */
import * as React from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Loader2 } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useDaemon } from "@/daemon/client";
import { useUiStore } from "@/store/ui";

interface LedgerPreviewRow {
  occurred_at?: string;
  direction?: string;
  asset?: string;
  amount?: string;
  fee?: string;
  kind?: string;
  fiat_currency?: string;
  fiat_value?: string;
  description?: string;
}

interface LedgerPreviewResult {
  rows_read: number;
  mapped: number;
  errors: number;
  problems: { row: number; message: string }[];
  preview: LedgerPreviewRow[];
  truncated: boolean;
  confident?: boolean;
  detected?: { column: string; field: string }[] | null;
}

function localeFor(lang: string): string {
  return lang.startsWith("de") ? "de-AT" : "en-US";
}

function formatAssetAmount(
  value: string | undefined,
  asset: string | undefined,
  lang: string,
): string {
  if (!value) return "";
  const num = Number(value);
  return Number.isFinite(num)
    ? `${num.toLocaleString(localeFor(lang), {
        maximumFractionDigits: 8,
      })} ${asset ?? "BTC"}`
    : value;
}

function formatDate(iso: string | undefined, lang: string): string {
  if (!iso) return "";
  const date = new Date(iso);
  return Number.isNaN(date.getTime())
    ? iso
    : date.toLocaleDateString(localeFor(lang), {
        year: "numeric",
        month: "short",
        day: "numeric",
      });
}

export function GenericLedgerPreview({
  file,
  onBlockSubmitChange,
}: {
  file: string;
  onBlockSubmitChange?: (blocked: boolean) => void;
}) {
  const { t } = useTranslation("connections");
  const lang = useUiStore((state) => state.lang);
  const query = useDaemon<LedgerPreviewResult>(
    "ui.wallets.ledger_preview",
    { source_file: file },
    { retry: false },
  );
  const data = query.data?.data;
  const blocksSubmit =
    query.isFetching ||
    Boolean(query.error) ||
    !data ||
    data.confident === false ||
    data.errors > 0 ||
    data.mapped <= 0;

  React.useEffect(() => {
    onBlockSubmitChange?.(blocksSubmit);
  }, [blocksSubmit, onBlockSubmitChange]);

  if (query.isFetching && !data) {
    return (
      <div className="flex items-center gap-2 rounded-md border border-dashed px-3 py-2 text-xs text-muted-foreground">
        <Loader2 className="size-4 animate-spin" aria-hidden="true" />
        {t("add.genericLedger.preview.loading")}
      </div>
    );
  }
  if (query.error) {
    return (
      <div className="flex items-center gap-2 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive">
        <AlertTriangle className="size-4 shrink-0" aria-hidden="true" />
        <span>
          {query.error instanceof Error
            ? query.error.message
            : t("add.genericLedger.preview.failed")}
        </span>
      </div>
    );
  }
  if (!data) return null;

  // Columns couldn't be auto-detected — steer to the template, don't import.
  if (data.confident === false) {
    return (
      <div className="space-y-1 rounded-md border border-amber-500/40 bg-amber-500/5 px-3 py-2 text-xs text-amber-700 dark:text-amber-400">
        <p className="flex items-center gap-2 font-medium">
          <AlertTriangle className="size-4 shrink-0" aria-hidden="true" />
          {t("add.genericLedger.preview.notRecognized")}
        </p>
        <p>{t("add.genericLedger.preview.notRecognizedHint")}</p>
      </div>
    );
  }

  const rows = data.preview.slice(0, 5);
  const detectedColumns = (data.detected ?? []).map((entry) => entry.column);
  return (
    <div className="space-y-2 rounded-md border px-3 py-2">
      <div className="flex flex-wrap items-center gap-2 text-xs">
        <Badge variant="secondary">
          {t("add.genericLedger.preview.ready", {
            mapped: data.mapped,
            rows: data.rows_read,
          })}
        </Badge>
        {data.errors > 0 ? (
          <Badge variant="destructive">
            {t("add.genericLedger.preview.errorCount", { count: data.errors })}
          </Badge>
        ) : null}
      </div>

      {detectedColumns.length > 0 ? (
        <p className="text-[11px] text-muted-foreground">
          {t("add.genericLedger.preview.detected", {
            columns: detectedColumns.join(", "),
          })}
        </p>
      ) : null}

      {rows.length > 0 ? (
        <ScrollArea className="max-h-44">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="text-xs">
                  {t("add.genericLedger.preview.colDate")}
                </TableHead>
                <TableHead className="text-xs">
                  {t("add.genericLedger.preview.colKind")}
                </TableHead>
                <TableHead className="text-right text-xs">
                  {t("add.genericLedger.preview.colAmount")}
                </TableHead>
                <TableHead className="text-right text-xs">
                  {t("add.genericLedger.preview.colValue")}
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {rows.map((row, index) => (
                <TableRow key={index}>
                  <TableCell className="text-xs">{formatDate(row.occurred_at, lang)}</TableCell>
                  <TableCell className="text-xs text-muted-foreground">
                    {row.kind ?? row.direction ?? ""}
                  </TableCell>
                  <TableCell className="text-right font-mono text-xs">
                    {formatAssetAmount(row.amount, row.asset, lang)}
                  </TableCell>
                  <TableCell className="text-right text-xs text-muted-foreground">
                    {row.fiat_value
                      ? `${row.fiat_value}${row.fiat_currency ? ` ${row.fiat_currency}` : ""}`
                      : ""}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </ScrollArea>
      ) : null}

      {data.problems.length > 0 ? (
        <ul className="space-y-0.5">
          {data.problems.slice(0, 3).map((problem) => (
            <li key={problem.row} className="text-xs text-destructive">
              {t("add.genericLedger.preview.problemRow", {
                row: problem.row,
                message: problem.message,
              })}
            </li>
          ))}
          {data.problems.length > 3 ? (
            <li className="text-xs text-muted-foreground">
              {t("add.genericLedger.preview.more", {
                count: data.problems.length - 3,
              })}
            </li>
          ) : null}
        </ul>
      ) : null}
    </div>
  );
}
