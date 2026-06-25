/**
 * Right pane of the CSV mapping workbench: a live, locale-formatted preview of
 * the transformed transactions with per-row problem highlighting. Signal-only:
 * filtered/error counts and rows render only when there is something to report.
 */
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Loader2 } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

import { localeFor, type CsvPreviewResult, type PreviewProblem } from "./types";

interface PreviewProps {
  preview: CsvPreviewResult | null;
  loading: boolean;
  error: string | null;
  onlyProblems: boolean;
  setOnlyProblems: (value: boolean) => void;
  lang: string;
}

function formatBtc(value: string | undefined, lang: string): string {
  if (value === undefined) return "";
  const num = Number(value);
  if (!Number.isFinite(num)) return value;
  return `${num.toLocaleString(localeFor(lang), { maximumFractionDigits: 8 })} BTC`;
}

function formatDate(iso: string, lang: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleDateString(localeFor(lang), {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function formatFiat(value: string | undefined, currency: string | undefined, lang: string): string {
  if (value === undefined || value === "") return "";
  const num = Number(value);
  const formatted = Number.isFinite(num)
    ? num.toLocaleString(localeFor(lang), { maximumFractionDigits: 2 })
    : value;
  return currency ? `${formatted} ${currency}` : formatted;
}

export function MappingPreview({
  preview,
  loading,
  error,
  onlyProblems,
  setOnlyProblems,
  lang,
}: PreviewProps) {
  const { t } = useTranslation("csvMapping");

  const errorProblems = useMemo<PreviewProblem[]>(
    () => preview?.problems.filter((p) => p.kind === "error") ?? [],
    [preview],
  );

  const looksLikeWrongUnit = useMemo(
    () => (preview?.preview ?? []).some((r) => Number(r.amount) >= 1000),
    [preview],
  );

  return (
    <div className="flex h-full flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <h3 className="text-sm font-semibold">{t("preview.heading")}</h3>
          {preview ? (
            <>
              <Badge variant="secondary">{t("counts.ready", { count: preview.mapped })}</Badge>
              {preview.filtered > 0 ? (
                <Badge variant="outline" className="text-muted-foreground">
                  {t("counts.filtered", { count: preview.filtered })}
                </Badge>
              ) : null}
              {preview.errors > 0 ? (
                <Badge variant="destructive">{t("counts.errors", { count: preview.errors })}</Badge>
              ) : null}
            </>
          ) : null}
          {loading ? <Loader2 className="size-4 animate-spin text-muted-foreground" aria-hidden="true" /> : null}
        </div>
        {preview && preview.problems.length > 0 ? (
          <label className="flex items-center gap-2 text-xs text-muted-foreground">
            <Switch checked={onlyProblems} onCheckedChange={setOnlyProblems} />
            {t("preview.onlyProblems")}
          </label>
        ) : null}
      </div>

      {error ? (
        <div className="flex items-center gap-2 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive">
          <AlertTriangle className="size-4 shrink-0" aria-hidden="true" />
          <span>{error}</span>
        </div>
      ) : null}

      {looksLikeWrongUnit ? (
        <div className="rounded-md border border-amber-500/40 bg-amber-500/5 px-3 py-2 text-xs text-amber-700 dark:text-amber-400">
          {t("preview.unitHint")}
        </div>
      ) : null}

      {!preview && !loading && !error ? (
        <div className="grid flex-1 place-items-center rounded-md border border-dashed text-sm text-muted-foreground">
          {t("preview.empty")}
        </div>
      ) : null}

      {!preview && loading ? (
        <div className="grid flex-1 place-items-center rounded-md border border-dashed text-sm text-muted-foreground">
          {t("preview.loading")}
        </div>
      ) : null}

      {preview ? (
        <ScrollArea className="min-h-0 flex-1 rounded-md border">
          <Table>
            <TableHeader className="sticky top-0 bg-background">
              <TableRow>
                <TableHead className="w-[110px]">{t("preview.colDate")}</TableHead>
                <TableHead className="w-[90px]">{t("preview.colDirection")}</TableHead>
                <TableHead className="text-right">{t("preview.colAmount")}</TableHead>
                <TableHead className="text-right">{t("preview.colFee")}</TableHead>
                <TableHead className="text-right">{t("preview.colValue")}</TableHead>
                <TableHead>{t("preview.colDescription")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {!onlyProblems
                ? preview.preview.map((record, index) => (
                    <TableRow key={`r-${index}`}>
                      <TableCell className="text-xs">{formatDate(record.occurred_at, lang)}</TableCell>
                      <TableCell>
                        <Badge
                          variant={record.direction === "inbound" ? "secondary" : "outline"}
                          className="text-[11px]"
                        >
                          {t(`direction.${record.direction}` as never)}
                        </Badge>
                      </TableCell>
                      <TableCell className="text-right font-mono text-xs">
                        {formatBtc(record.amount, lang)}
                      </TableCell>
                      <TableCell className="text-right font-mono text-xs text-muted-foreground">
                        {record.fee && Number(record.fee) > 0 ? formatBtc(record.fee, lang) : ""}
                      </TableCell>
                      <TableCell className="text-right text-xs text-muted-foreground">
                        {formatFiat(record.fiat_value, record.fiat_currency, lang)}
                      </TableCell>
                      <TableCell className="max-w-[180px] truncate text-xs text-muted-foreground">
                        {record.description ?? ""}
                      </TableCell>
                    </TableRow>
                  ))
                : null}
              {errorProblems.map((problem, index) => (
                <TableRow key={`e-${index}`} className="border-l-2 border-destructive/70 bg-destructive/5">
                  <TableCell className="text-xs text-muted-foreground">
                    {t("problem.row", { row: problem.row })}
                  </TableCell>
                  <TableCell colSpan={5} className="text-xs text-destructive">
                    {t(`problem.reason.${problem.reason}` as never)}
                    {problem.column ? <span className="text-muted-foreground"> · {problem.column}</span> : null}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </ScrollArea>
      ) : null}

      {preview?.truncated ? (
        <p className="text-[11px] text-muted-foreground">
          {t("preview.truncated", { count: preview.preview.length })}
        </p>
      ) : null}
    </div>
  );
}
