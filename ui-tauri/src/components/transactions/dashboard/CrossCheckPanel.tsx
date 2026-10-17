import { keepPreviousData } from "@tanstack/react-query";
import {
  AlertTriangle,
  ArrowDownLeft,
  ArrowLeftRight,
  ArrowUpRight,
  CircleCheck,
  Clock,
  Download,
  FileUp,
  Info,
  Loader2,
  Search,
} from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { DialogClose } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { exportBasename, saveDaemonExport } from "@/lib/exportFile";
import { isFilePickerAvailable, pickFileWithContentsBase64 } from "@/lib/filePicker";
import { currentUiLocale, formatCount } from "@/lib/localeFormat";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import { ExportFormatMark } from "./ExportFormatPicker";
import { exportFormat, type ToolFormatId } from "./exportFormats";
import { CROSS_CHECK_TOOLS, detectCrossCheckTool, type CrossCheckFile } from "./crossCheckFile";
import { YearPicker } from "./YearPicker";

export type CrossCheckStatus =
  | "different"
  | "only_tool"
  | "only_kassiber"
  | "excluded_in_kassiber"
  | "matched"
  | "not_comparable";

interface ToolSide {
  row: number;
  source: string;
  type: string;
  occurred_at: string;
  direction: string;
  asset: string;
  amount: string;
  fee: string;
  value: string;
  txid: string;
}

interface KassiberSide {
  ref: string;
  wallet: string;
  classification: string;
  occurred_at: string;
  direction: string;
  asset: string;
  amount: string;
  fee: string;
  value: string;
  txid: string;
  excluded: boolean;
}

interface Difference {
  field: "type" | "amount" | "fee" | "value" | "time";
  severity: "tax" | "info";
  tool: string;
  kassiber: string;
  delta?: string;
  message: string;
}

export interface CrossCheckRow {
  id: string;
  status: CrossCheckStatus;
  match: "txid" | "probable" | null;
  tool: ToolSide | null;
  kassiber: KassiberSide | null;
  differences: Difference[];
  reason?: string;
  pair: string;
}

/** A Kassiber wallet next to the tool's name(s) for it. */
export interface WalletPair {
  key: string;
  wallet: string | null;
  tool_names: string[];
  state: "paired" | "kassiber_only" | "tool_only";
  rows: number;
  issues: number;
  counts: Record<CrossCheckStatus, number>;
}

export interface CrossCheckResult {
  tool: ToolFormatId;
  file_name: string;
  book_currency: string;
  year: number | null;
  counts: Record<CrossCheckStatus, number>;
  pairs: WalletPair[];
  insights: { code: string; params: Record<string, unknown>; message: string }[];
  rows: CrossCheckRow[];
}

type View = "attention" | "matched" | "all";

const PAGE = 150;
const ALL = "all";
/** Notes worth a line above the rows; the rest stays in the CLI and workbook. */
const NOTES: Record<string, React.ComponentType<{ className?: string }>> = {
  clock_offset: Clock,
  journals_stale: AlertTriangle,
  tool_other_assets: Info,
  kassiber_other_assets: Info,
};
// Literal keys: two notes have plural forms, which a template key cannot name.
const NOTE_KEYS = {
  clock_offset: "dashboard.export.compare.notes.clock_offset",
  journals_stale: "dashboard.export.compare.notes.journals_stale",
  tool_other_assets: "dashboard.export.compare.notes.tool_other_assets",
  kassiber_other_assets: "dashboard.export.compare.notes.kassiber_other_assets",
} as const;
const DOT: Record<CrossCheckStatus, string> = {
  different: "bg-amber-500",
  only_tool: "bg-amber-500",
  only_kassiber: "bg-amber-500",
  excluded_in_kassiber: "bg-slate-400",
  matched: "bg-emerald-500",
  not_comparable: "bg-rose-500",
};

function formatDate(value: string | undefined): string {
  if (!value) return "";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return new Intl.DateTimeFormat(currentUiLocale(), { dateStyle: "medium" }).format(parsed);
}

function formatTime(value: string | undefined): string {
  if (!value) return "";
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return "";
  return new Intl.DateTimeFormat(currentUiLocale(), { timeStyle: "short" }).format(parsed);
}

function formatMoney(value: string, currency: string): string {
  const amount = Number(value);
  if (!value || !Number.isFinite(amount) || !currency) return value;
  return new Intl.NumberFormat(currentUiLocale(), {
    style: "currency",
    currency,
    signDisplay: "exceptZero",
  }).format(amount);
}

function signed(value: string): string {
  return value && !value.startsWith("-") && Number(value) !== 0 ? `+${value}` : value;
}

function shortTxid(value: string): string {
  return value.length > 20 ? `${value.slice(0, 8)}…${value.slice(-6)}` : value;
}

function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error("read failed"));
    reader.onload = () => {
      const result = String(reader.result ?? "");
      resolve(result.slice(result.indexOf(",") + 1));
    };
    reader.readAsDataURL(file);
  });
}

function rowMatches(row: CrossCheckRow, needle: string): boolean {
  if (!needle) return true;
  return [
    row.kassiber?.wallet,
    row.kassiber?.txid,
    row.kassiber?.classification,
    row.kassiber?.amount,
    row.tool?.source,
    row.tool?.txid,
    row.tool?.type,
    row.tool?.amount,
  ].some((value) => value && value.toLocaleLowerCase().includes(needle));
}

/**
 * Checks a CoinTracking or Blockpit export against the book.
 *
 * One idea per level: the wallets on the left (each Kassiber wallet next to
 * the tool's name for it), the transactions on the right with the tool and
 * Kassiber side by side. A wallet only one side has is a single finding on
 * the left, not a pile of row problems on the right.
 */
export function CrossCheckPanel({
  open,
  years,
  defaultYear,
  onOpenTransaction,
  onDone,
}: {
  open: boolean;
  years: number[];
  defaultYear: number | null;
  onOpenTransaction: (ref: string) => void;
  onDone: () => void;
}) {
  const { t } = useTranslation(["transactions", "common"]);
  const addNotification = useUiStore((s) => s.addNotification);
  const [file, setFile] = React.useState<CrossCheckFile | null>(null);
  const [tool, setTool] = React.useState<ToolFormatId | null>(null);
  const [fileError, setFileError] = React.useState<string | null>(null);
  // `undefined` follows the dialog's default year; `null` is every year.
  const [year, setYear] = React.useState<number | null | undefined>(undefined);
  const [pairKey, setPairKey] = React.useState<string>(ALL);
  const [view, setView] = React.useState<View>("attention");
  const [query, setQuery] = React.useState("");
  const [limit, setLimit] = React.useState(PAGE);
  const inputRef = React.useRef<HTMLInputElement>(null);
  const selectedYear = year === undefined ? defaultYear : year;
  const toolLabel = tool ? t(`dashboard.export.formats.${tool}`) : "";

  const args = React.useMemo(() => {
    if (!file || !tool) return null;
    const next: Record<string, unknown> = {
      tool,
      filename: file.name,
      source_bytes_base64: file.base64,
    };
    if (selectedYear !== null) next.year = selectedYear;
    return next;
  }, [file, tool, selectedYear]);

  const comparison = useDaemon<CrossCheckResult>("ui.reports.cross_check", args ?? undefined, {
    enabled: open && Boolean(args),
    retry: false,
    staleTime: Infinity,
    placeholderData: keepPreviousData,
  });
  const result = comparison.data?.data;
  const exportComparison = useDaemonMutation<{ file?: string }>("ui.reports.export_cross_check", {
    invalidateQueries: false,
  });

  React.useEffect(() => {
    setLimit(PAGE);
  }, [view, query, pairKey, result]);

  const takeFile = (next: CrossCheckFile) => {
    setFileError(null);
    setFile(next);
    setTool(detectCrossCheckTool(next.base64));
    setPairKey(ALL);
    setView("attention");
  };

  const chooseFile = async () => {
    if (!isFilePickerAvailable) {
      inputRef.current?.click();
      return;
    }
    try {
      const picked = await pickFileWithContentsBase64({
        title: t("dashboard.export.compare.chooseFile"),
        filters: [{ name: "CSV", extensions: ["csv"] }],
      });
      if (picked) takeFile({ name: exportBasename(picked.path), base64: picked.contentsBase64 });
    } catch (error) {
      setFileError(error instanceof Error ? error.message : String(error));
    }
  };

  const pairs = result?.pairs ?? [];
  const pairState = new Map(pairs.map((pair) => [pair.key, pair.state]));
  const selectedPair = pairs.find((pair) => pair.key === pairKey);
  // Rows of a wallet only one side has are summarized as that wallet and
  // listed once the wallet itself is picked.
  const scoped = (result?.rows ?? []).filter((row) =>
    selectedPair ? row.pair === selectedPair.key : pairState.get(row.pair) === "paired",
  );
  const attention = scoped.filter((row) => row.status !== "matched");
  const matched = scoped.length - attention.length;
  const needle = query.trim().toLocaleLowerCase();
  const rows = (
    view === "attention"
      ? attention
      : view === "matched"
        ? scoped.filter((row) => row.status === "matched")
        : scoped
  ).filter((row) => rowMatches(row, needle));

  const runExport = () => {
    if (!args) return;
    exportComparison.mutate(args, {
      onSuccess: async (envelope) => {
        onDone();
        try {
          const { savedPath, copied } = await saveDaemonExport({
            exportPath: envelope?.data?.file ?? "",
            title: t("dashboard.export.compare.saveTitle"),
            defaultName: exportBasename(envelope?.data?.file ?? "kassiber-cross-check.xlsx"),
            filters: [exportFormat("xlsx").filter],
          });
          addNotification({
            title: t("dashboard.export.compare.saved"),
            body: copied
              ? t("dashboard.export.savedTo", { name: exportBasename(savedPath) })
              : exportBasename(envelope?.data?.file ?? ""),
            tone: "success",
          });
        } catch (error) {
          addNotification({
            title: t("dashboard.export.failed"),
            body: error instanceof Error ? error.message : String(error),
            tone: "error",
          });
        }
      },
      onError: (error) =>
        addNotification({
          title: t("dashboard.export.failed"),
          body: error instanceof Error ? error.message : String(error),
          tone: "error",
        }),
    });
  };

  return (
    <>
      <input
        ref={inputRef}
        type="file"
        accept=".csv,text/csv"
        className="hidden"
        aria-label={t("dashboard.export.compare.chooseFile")}
        onChange={async (event) => {
          const picked = event.currentTarget.files?.[0];
          event.currentTarget.value = "";
          if (!picked) return;
          try {
            takeFile({ name: picked.name, base64: await fileToBase64(picked) });
          } catch (error) {
            setFileError(error instanceof Error ? error.message : String(error));
          }
        }}
      />

      {!file ? (
        <div className="grid min-h-0 flex-1 place-content-center justify-items-center gap-5 overflow-y-auto p-6 text-center">
          <div className="flex items-center gap-2" aria-hidden="true">
            {CROSS_CHECK_TOOLS.map((option) => (
              <ExportFormatMark key={option} option={exportFormat(option)} className="size-10 p-1.5" />
            ))}
            <ArrowLeftRight className="mx-1 size-5 text-muted-foreground" />
            <span className="flex size-10 items-center justify-center rounded-md bg-primary text-base font-semibold text-primary-foreground">
              K
            </span>
          </div>
          <div className="grid max-w-md gap-1.5">
            <p className="text-base font-medium">{t("dashboard.export.compare.empty.title")}</p>
            <p className="text-sm leading-6 text-muted-foreground">
              {t("dashboard.export.compare.empty.body")}
            </p>
          </div>
          <Button type="button" onClick={() => void chooseFile()}>
            <FileUp className="size-4" aria-hidden="true" />
            {t("dashboard.export.compare.chooseFile")}
          </Button>
          {fileError ? <p className="text-xs text-destructive">{fileError}</p> : null}
        </div>
      ) : (
        <div className="flex min-h-0 flex-1 flex-col">
          <div className="flex shrink-0 flex-wrap items-center gap-2 border-b px-5 py-2.5">
            <button
              type="button"
              onClick={() => void chooseFile()}
              className="flex h-8 max-w-full min-w-0 items-center gap-2 rounded-md border bg-background/70 pr-2.5 pl-1 text-sm transition-colors hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
            >
              {tool ? (
                <ExportFormatMark option={exportFormat(tool)} className="size-6" />
              ) : (
                <FileUp className="ml-1 size-4 text-muted-foreground" aria-hidden="true" />
              )}
              <span className="truncate font-medium">{file.name}</span>
              <span className="shrink-0 text-xs text-muted-foreground">
                {t("dashboard.export.compare.changeFile")}
              </span>
            </button>
            <div className="ml-auto">
              <YearPicker years={years} value={selectedYear} onChange={setYear} allowAll />
            </div>
          </div>

          {!tool ? (
            <ToolQuestion onPick={setTool} />
          ) : comparison.error ? (
            <div
              role="alert"
              className="grid flex-1 place-content-center justify-items-center gap-3 p-6 text-center"
            >
              <p className="text-sm font-medium">{t("dashboard.export.compare.failed")}</p>
              <p className="max-w-md text-xs leading-5 text-muted-foreground">
                {comparison.error instanceof Error
                  ? comparison.error.message
                  : String(comparison.error)}
              </p>
              {CROSS_CHECK_TOOLS.filter((option) => option !== tool).map((option) => (
                <Button
                  key={option}
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => setTool(option)}
                >
                  <ExportFormatMark option={exportFormat(option)} className="size-5" />
                  {t("dashboard.export.compare.readAs", {
                    tool: t(`dashboard.export.formats.${option}`),
                  })}
                </Button>
              ))}
            </div>
          ) : !result ? (
            <div className="grid flex-1 content-start gap-3 p-5">
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <Loader2 className="size-4 animate-spin" aria-hidden="true" />
                {t("dashboard.export.compare.running")}
              </div>
              <Skeleton className="h-14" />
              <Skeleton className="h-64" />
            </div>
          ) : (
            <div className="@container/compare flex min-h-0 flex-1 flex-col">
              <div className="flex min-h-0 flex-1 flex-col @3xl/compare:grid @3xl/compare:grid-cols-[15rem_minmax(0,1fr)]">
                <WalletList
                  pairs={pairs}
                  selected={pairKey}
                  onSelect={setPairKey}
                  toolLabel={toolLabel}
                />
                <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3 p-4 @3xl/compare:p-5">
                  <Headline
                    result={result}
                    pair={selectedPair}
                    attention={attention}
                    pairs={pairs}
                    toolLabel={toolLabel}
                    loading={comparison.isFetching}
                  />
                  <div className="flex flex-wrap items-center gap-2">
                    <div
                      role="radiogroup"
                      aria-label={t("dashboard.export.compare.views.label")}
                      className="inline-flex rounded-md border bg-muted/40 p-0.5"
                    >
                      {(
                        [
                          ["attention", attention.length],
                          ["matched", matched],
                          ["all", scoped.length],
                        ] as const
                      ).map(([key, count]) => (
                        <button
                          key={key}
                          type="button"
                          role="radio"
                          aria-checked={view === key}
                          onClick={() => setView(key)}
                          className={cn(
                            "rounded-sm px-2.5 py-1 text-xs whitespace-nowrap transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
                            view === key
                              ? "bg-background font-medium shadow-sm"
                              : "text-muted-foreground hover:text-foreground",
                          )}
                        >
                          {t(`dashboard.export.compare.views.${key}`)}
                          <span className="ml-1.5 tabular-nums opacity-70">{formatCount(count)}</span>
                        </button>
                      ))}
                    </div>
                    <div className="relative min-w-40 flex-1">
                      <Search
                        className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
                        aria-hidden="true"
                      />
                      <Input
                        value={query}
                        onChange={(event) => setQuery(event.target.value)}
                        placeholder={t("dashboard.export.compare.search")}
                        aria-label={t("dashboard.export.compare.search")}
                        className="h-8 pl-8 text-sm"
                      />
                    </div>
                  </div>
                  <RowList
                    rows={rows.slice(0, limit)}
                    currency={result.book_currency}
                    toolLabel={toolLabel}
                    emptyLabel={
                      view === "attention" && !needle
                        ? t("dashboard.export.compare.nothingToReview")
                        : t("dashboard.export.compare.noRows")
                    }
                    onOpenTransaction={onOpenTransaction}
                  />
                  {rows.length > limit ? (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      className="self-center"
                      onClick={() => setLimit((current) => current + PAGE)}
                    >
                      {t("dashboard.export.compare.showMore", {
                        count: Math.min(PAGE, rows.length - limit),
                      })}
                    </Button>
                  ) : null}
                </div>
              </div>
            </div>
          )}
        </div>
      )}

      <footer className="flex shrink-0 flex-wrap items-center justify-between gap-3 border-t px-5 py-3">
        <p className="min-w-0 truncate text-sm text-muted-foreground">
          {file && tool
            ? t("dashboard.export.compare.summary", {
                tool: toolLabel,
                year:
                  selectedYear === null
                    ? t("dashboard.export.yearPicker.all")
                    : String(selectedYear ?? ""),
              })
            : t("dashboard.export.compare.summaryEmpty")}
        </p>
        <div className="flex gap-2">
          <DialogClose asChild>
            <Button type="button" variant="outline" disabled={exportComparison.isPending}>
              {t("common:actions.cancel")}
            </Button>
          </DialogClose>
          <Button
            type="button"
            disabled={
              !result ||
              comparison.isFetching ||
              exportComparison.isPending ||
              Boolean(comparison.error)
            }
            onClick={runExport}
          >
            <Download className="size-4" aria-hidden="true" />
            {exportComparison.isPending
              ? t("dashboard.export.exporting")
              : t("dashboard.export.compare.export")}
          </Button>
        </div>
      </footer>
    </>
  );
}

function ToolQuestion({ onPick }: { onPick: (tool: ToolFormatId) => void }) {
  const { t } = useTranslation("transactions");
  return (
    <div className="grid flex-1 place-content-center justify-items-center gap-3 p-6 text-center">
      <p className="text-sm font-medium">{t("dashboard.export.compare.whichTool")}</p>
      <div className="flex flex-wrap justify-center gap-2">
        {CROSS_CHECK_TOOLS.map((option) => (
          <Button key={option} type="button" variant="outline" onClick={() => onPick(option)}>
            <ExportFormatMark option={exportFormat(option)} className="size-5" />
            {t(`dashboard.export.formats.${option}`)}
          </Button>
        ))}
      </div>
    </div>
  );
}

function WalletList({
  pairs,
  selected,
  onSelect,
  toolLabel,
}: {
  pairs: WalletPair[];
  selected: string;
  onSelect: (key: string) => void;
  toolLabel: string;
}) {
  const { t } = useTranslation("transactions");
  const issues = pairs
    .filter((pair) => pair.state === "paired")
    .reduce((sum, pair) => sum + pair.issues, 0);
  const badge = (pair: WalletPair | null, count: number) =>
    pair && pair.state !== "paired" ? (
      <Info className="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
    ) : count ? (
      <span className="rounded-full bg-amber-500/15 px-1.5 text-xs font-medium text-amber-700 tabular-nums dark:text-amber-300">
        {formatCount(count)}
      </span>
    ) : (
      <CircleCheck className="size-4 shrink-0 text-emerald-500" aria-hidden="true" />
    );
  const entries: { key: string; title: string; detail: string; pair: WalletPair | null; count: number }[] = [
    { key: ALL, title: t("dashboard.export.compare.allWallets"), detail: "", pair: null, count: issues },
    ...pairs.map((pair) => ({
      key: pair.key,
      title: pair.wallet ?? pair.tool_names.join(", "),
      detail:
        pair.state === "paired"
          ? `${toolLabel}: ${pair.tool_names.join(", ")}`
          : pair.state === "kassiber_only"
            ? t("dashboard.export.compare.notInTool", { tool: toolLabel })
            : t("dashboard.export.compare.notInKassiber", { tool: toolLabel }),
      pair,
      count: pair.issues,
    })),
  ];

  return (
    <nav
      aria-label={t("dashboard.export.compare.wallets")}
      className="shrink-0 overflow-x-auto border-b bg-muted/25 p-2 @3xl/compare:min-h-0 @3xl/compare:overflow-x-hidden @3xl/compare:overflow-y-auto @3xl/compare:border-r @3xl/compare:border-b-0 @3xl/compare:p-3"
    >
      <p className="hidden px-2.5 pb-1.5 text-xs font-medium text-muted-foreground @3xl/compare:block">
        {t("dashboard.export.compare.wallets")}
      </p>
      <ul className="flex gap-1 @3xl/compare:grid @3xl/compare:gap-0.5">
        {entries.map((entry) => (
          <li key={entry.key} className="shrink-0 @3xl/compare:shrink">
            <button
              type="button"
              aria-pressed={selected === entry.key}
              onClick={() => onSelect(entry.key)}
              className={cn(
                "flex w-full max-w-64 min-w-0 items-center gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none @3xl/compare:max-w-none",
                selected === entry.key ? "bg-primary/10" : "hover:bg-muted/60",
              )}
            >
              <span className="min-w-0 flex-1">
                <span
                  className={cn(
                    "block truncate text-sm",
                    entry.pair && entry.pair.state !== "paired"
                      ? "text-muted-foreground"
                      : "font-medium",
                  )}
                >
                  {entry.title}
                </span>
                {entry.detail ? (
                  <span className="block truncate text-xs text-muted-foreground">{entry.detail}</span>
                ) : null}
              </span>
              {badge(entry.pair, entry.count)}
            </button>
          </li>
        ))}
      </ul>
    </nav>
  );
}

function Headline({
  result,
  pair,
  attention,
  pairs,
  toolLabel,
  loading,
}: {
  result: CrossCheckResult;
  pair: WalletPair | undefined;
  attention: CrossCheckRow[];
  pairs: WalletPair[];
  toolLabel: string;
  loading: boolean;
}) {
  const { t } = useTranslation("transactions");
  const count = (status: CrossCheckStatus) =>
    attention.filter((row) => row.status === status).length;
  const walletGaps = pair
    ? []
    : ([
        ["walletsNotInTool", pairs.filter((item) => item.state === "kassiber_only").length],
        ["walletsNotInKassiber", pairs.filter((item) => item.state === "tool_only").length],
      ] as const);
  const parts = [
    ["differ", count("different")],
    ["onlyTool", count("only_tool")],
    ["onlyKassiber", count("only_kassiber")],
    ["excluded", count("excluded_in_kassiber")],
    ["unreadable", count("not_comparable")],
    ...walletGaps,
  ] as const;
  const detail = parts
    .filter(([, n]) => n)
    .map(([key, n]) => t(`dashboard.export.compare.parts.${key}`, { count: n, tool: toolLabel }));
  const unpaired = pair && pair.state !== "paired" ? pair : null;
  // Agreement needs something compared: a file and book with only other
  // assets have nothing in common, which is not the same as agreeing.
  const nothingCompared = !result.rows.length;
  const tone = nothingCompared || unpaired
    ? "gap"
    : attention.length
      ? "attention"
      : detail.length
        ? "gap"
        : "agrees";
  const title = nothingCompared
    ? t("dashboard.export.compare.headline.nothingCompared")
    : unpaired
    ? unpaired.state === "kassiber_only"
      ? t("dashboard.export.compare.headline.walletNotInTool", {
          wallet: unpaired.wallet,
          tool: toolLabel,
        })
      : t("dashboard.export.compare.headline.walletNotInKassiber", {
          wallet: unpaired.tool_names.join(", "),
          tool: toolLabel,
        })
    : attention.length
      ? t("dashboard.export.compare.headline.attention", { count: attention.length })
      : detail.length
        ? t("dashboard.export.compare.headline.rowsAgree")
        : t("dashboard.export.compare.headline.agrees", { tool: toolLabel });
  const notes = pair ? [] : result.insights.filter((insight) => NOTES[insight.code]);

  return (
    <div className="grid gap-1.5">
      <div
        role="status"
        className={cn(
          "flex items-start gap-3 rounded-lg border px-4 py-3",
          tone === "agrees" && "border-emerald-500/40 bg-emerald-500/10",
          tone === "attention" && "border-amber-500/40 bg-amber-500/10",
          tone === "gap" && "bg-muted/40",
        )}
      >
        {tone === "agrees" ? (
          <CircleCheck className="mt-0.5 size-5 shrink-0 text-emerald-600" aria-hidden="true" />
        ) : tone === "attention" ? (
          <AlertTriangle className="mt-0.5 size-5 shrink-0 text-amber-600" aria-hidden="true" />
        ) : (
          <Info className="mt-0.5 size-5 shrink-0 text-muted-foreground" aria-hidden="true" />
        )}
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">{title}</p>
          <p className="text-xs leading-5 text-muted-foreground">
            {nothingCompared
              ? t("dashboard.export.compare.headline.nothingComparedDetail", { tool: toolLabel })
              : unpaired
              ? t("dashboard.export.compare.headline.walletRows", { count: unpaired.rows })
              : detail.length
                ? detail.join(" · ")
                : t("dashboard.export.compare.headline.compared", { count: result.counts.matched })}
          </p>
        </div>
        {loading ? (
          <Loader2 className="size-4 shrink-0 animate-spin text-muted-foreground" aria-hidden="true" />
        ) : null}
      </div>
      {notes.map((insight) => {
        const Icon = NOTES[insight.code];
        const hours = insight.params.hours;
        return (
          <p key={insight.code} className="flex gap-2 px-1 text-xs leading-5 text-muted-foreground">
            <Icon className="mt-0.5 size-3.5 shrink-0" aria-hidden="true" />
            {t(NOTE_KEYS[insight.code as keyof typeof NOTE_KEYS], {
              tool: toolLabel,
              hours: typeof hours === "number" ? signed(String(hours)) : "",
              count: Number(insight.params.count ?? 0),
              assets: String(insight.params.assets ?? ""),
            })}
          </p>
        );
      })}
    </div>
  );
}

// Wide: date | tool | Kassiber | difference | open. Narrow: the same cells
// stacked, each side labelled.
const ROW_GRID =
  "@2xl/rows:grid-cols-[6.5rem_minmax(0,1fr)_minmax(0,1fr)_minmax(8rem,13rem)_1.75rem] @2xl/rows:items-start";

function RowList({
  rows,
  currency,
  toolLabel,
  emptyLabel,
  onOpenTransaction,
}: {
  rows: CrossCheckRow[];
  currency: string;
  toolLabel: string;
  emptyLabel: string;
  onOpenTransaction: (ref: string) => void;
}) {
  const { t } = useTranslation("transactions");
  return (
    <div className="@container/rows min-h-40 flex-1 overflow-y-auto rounded-lg border bg-background/70">
      <div
        aria-hidden="true"
        className={cn(
          "sticky top-0 z-10 hidden gap-x-4 border-b bg-muted px-3 py-2 text-xs font-medium text-muted-foreground @2xl/rows:grid",
          ROW_GRID,
        )}
      >
        <span>{t("dashboard.export.compare.columns.date")}</span>
        <span>{toolLabel}</span>
        <span>Kassiber</span>
        <span>{t("dashboard.export.compare.columns.difference")}</span>
        <span />
      </div>
      {rows.length ? (
        <ul aria-label={t("dashboard.export.compare.rows")} className="divide-y">
          {rows.map((row) => (
            <Row
              key={row.id}
              row={row}
              currency={currency}
              toolLabel={toolLabel}
              onOpenTransaction={onOpenTransaction}
            />
          ))}
        </ul>
      ) : (
        <p className="px-3 py-12 text-center text-sm text-muted-foreground">{emptyLabel}</p>
      )}
    </div>
  );
}

function Row({
  row,
  currency,
  toolLabel,
  onOpenTransaction,
}: {
  row: CrossCheckRow;
  currency: string;
  toolLabel: string;
  onOpenTransaction: (ref: string) => void;
}) {
  const { t } = useTranslation("transactions");
  const flagged = new Set(
    row.differences.filter((diff) => diff.severity === "tax").map((diff) => diff.field),
  );
  const when = row.kassiber?.occurred_at || row.tool?.occurred_at;
  const book = row.kassiber;
  const classification = (value: string) =>
    value
      .split("+")
      .map((part) => t(`dashboard.export.compare.classification.${part}`, { defaultValue: part }))
      .join(" + ");
  return (
    <li className={cn("grid gap-x-4 gap-y-1.5 px-3 py-2.5 text-xs", ROW_GRID)}>
      <div className="flex items-baseline gap-2 @2xl/rows:block">
        <span className="inline-flex items-center gap-1.5 font-medium tabular-nums">
          <span className={cn("size-2 shrink-0 rounded-full", DOT[row.status])} aria-hidden="true" />
          {formatDate(when)}
        </span>
        <span className="text-muted-foreground tabular-nums @2xl/rows:block @2xl/rows:pl-3.5">
          {formatTime(when)}
        </span>
        {book ? (
          <button
            type="button"
            className="ml-auto text-primary underline-offset-2 hover:underline @2xl/rows:hidden"
            onClick={() => onOpenTransaction(book.ref)}
          >
            {t("dashboard.export.compare.openTransaction")}
          </button>
        ) : null}
      </div>
      <Side
        label={toolLabel}
        missing={t("dashboard.export.compare.missingIn", { side: toolLabel })}
        type={row.tool?.type ?? ""}
        typeDiffers={flagged.has("type")}
        amount={row.tool?.amount ?? ""}
        asset={row.tool?.asset ?? ""}
        direction={row.tool?.direction ?? ""}
        amountDiffers={flagged.has("amount") || flagged.has("fee")}
        where={row.tool ? `${row.tool.source} · #${row.tool.row}` : ""}
        txid={row.tool?.txid ?? ""}
        present={Boolean(row.tool)}
      />
      <Side
        label="Kassiber"
        missing={t("dashboard.export.compare.missingIn", { side: "Kassiber" })}
        type={book ? classification(book.classification) : ""}
        typeDiffers={flagged.has("type")}
        amount={book?.amount ?? ""}
        asset={book?.asset ?? ""}
        direction={book?.direction ?? ""}
        amountDiffers={flagged.has("amount") || flagged.has("fee")}
        where={book?.wallet ?? ""}
        txid={book && book.txid !== row.tool?.txid ? book.txid : ""}
        present={Boolean(book)}
        excluded={book?.excluded}
      />
      <Issue row={row} currency={currency} toolLabel={toolLabel} />
      <div className="hidden @2xl/rows:block">
        {book ? (
          <Button
            type="button"
            size="icon"
            variant="ghost"
            className="-my-1 size-7"
            aria-label={t("dashboard.export.compare.openTransaction")}
            title={t("dashboard.export.compare.openTransaction")}
            onClick={() => onOpenTransaction(book.ref)}
          >
            <ArrowUpRight className="size-3.5" aria-hidden="true" />
          </Button>
        ) : null}
      </div>
    </li>
  );
}

function Side({
  label,
  missing,
  type,
  typeDiffers,
  amount,
  asset,
  direction,
  amountDiffers,
  where,
  txid,
  present,
  excluded,
}: {
  label: string;
  missing: string;
  type: string;
  typeDiffers: boolean;
  amount: string;
  asset: string;
  direction: string;
  amountDiffers: boolean;
  where: string;
  txid: string;
  present: boolean;
  excluded?: boolean;
}) {
  const { t } = useTranslation("transactions");
  const differs = "font-medium text-amber-700 dark:text-amber-300";
  return (
    <div className="grid min-w-0 grid-cols-[5.5rem_minmax(0,1fr)] gap-x-2 @2xl/rows:block">
      <span className="truncate text-muted-foreground @2xl/rows:hidden">{label}</span>
      {present ? (
        <div className="min-w-0">
          <div className="flex min-w-0 flex-wrap items-baseline gap-x-2">
            <span className={cn("tabular-nums", amountDiffers && differs)}>
              {direction === "inbound" ? (
                <ArrowDownLeft className="mr-0.5 inline size-3 text-emerald-600" aria-hidden="true" />
              ) : direction ? (
                <ArrowUpRight className="mr-0.5 inline size-3 text-rose-600" aria-hidden="true" />
              ) : null}
              {amount ? `${amount} ${asset}` : ""}
            </span>
            {type ? (
              <span className={typeDiffers ? differs : "text-muted-foreground"}>{type}</span>
            ) : null}
          </div>
          <div className="truncate text-muted-foreground">
            {where}
            {excluded ? (
              <span className="ml-1.5 rounded-sm border px-1 text-[10px]">
                {t("dashboard.export.compare.excludedBadge")}
              </span>
            ) : null}
            {txid ? (
              <span className="ml-1.5 font-mono text-[11px]" title={txid}>
                {shortTxid(txid)}
              </span>
            ) : null}
          </div>
        </div>
      ) : (
        <span className="text-muted-foreground italic">{missing}</span>
      )}
    </div>
  );
}

function Issue({
  row,
  currency,
  toolLabel,
}: {
  row: CrossCheckRow;
  currency: string;
  toolLabel: string;
}) {
  const { t } = useTranslation("transactions");
  const tag = (key: string, text: string, tone: "tax" | "info" | "plain") => (
    <span
      key={key}
      className={cn(
        "rounded-sm px-1.5 py-0.5 whitespace-nowrap",
        tone === "tax" && "bg-amber-500/15 text-amber-800 dark:text-amber-200",
        tone === "info" && "bg-muted text-muted-foreground",
        tone === "plain" && "px-0 text-muted-foreground",
      )}
    >
      {text}
    </span>
  );
  const diffText = (diff: Difference) => {
    const label = t(`dashboard.export.compare.diff.${diff.field}`);
    if (diff.field === "time") {
      return diff.severity === "tax"
        ? t("dashboard.export.compare.diff.taxYear")
        : `${label} ${t("dashboard.export.compare.diff.minutes", { minutes: signed(diff.delta ?? "") })}`;
    }
    if (diff.field === "value" && diff.delta) return `${label} ${formatMoney(diff.delta, currency)}`;
    if (diff.delta) return `${label} ${signed(diff.delta)}`;
    return label;
  };
  let body: React.ReactNode;
  if (row.status === "not_comparable") {
    body = (
      <>
        {tag("unreadable", t("dashboard.export.compare.issue.unreadable"), "tax")}
        <span title={row.reason} className="line-clamp-2 text-muted-foreground">
          {row.reason}
        </span>
      </>
    );
  } else if (row.status === "only_tool") {
    body = tag("only_tool", t("dashboard.export.compare.issue.onlyTool"), "tax");
  } else if (row.status === "only_kassiber") {
    body = tag(
      "only_kassiber",
      t("dashboard.export.compare.issue.onlyKassiber", { tool: toolLabel }),
      "tax",
    );
  } else if (row.status === "excluded_in_kassiber") {
    body = tag("excluded", t("dashboard.export.compare.issue.excluded"), "info");
  } else {
    const tags = row.differences.map((diff) =>
      tag(diff.field, diffText(diff), diff.severity === "tax" ? "tax" : "info"),
    );
    if (row.status === "matched") {
      tags.unshift(tag("matched", t("dashboard.export.compare.issue.matched"), "plain"));
    }
    if (row.match === "probable") {
      tags.push(tag("probable", t("dashboard.export.compare.probable"), "info"));
    }
    body = tags;
  }
  return <div className="flex min-w-0 flex-wrap items-start gap-1">{body}</div>;
}
