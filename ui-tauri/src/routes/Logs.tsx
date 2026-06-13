import {
  ChevronRight,
  Copy,
  Download,
  FileArchive,
  FileJson,
  FileText,
  Trash2,
} from "lucide-react";
import * as React from "react";

import {
  LogsTableControls,
  type LogLevelFilter,
} from "@/components/kb/LogsTableControls";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Textarea } from "@/components/ui/textarea";
import {
  APP_LOG_MAX_BYTES,
  clearAppLogRecords,
  exportLogRecords,
  exportSupportBundleRecords,
  formatLogRecord,
  getAppLogBufferSize,
  getAppLogRecords,
  logFilename,
  redactLogRecord,
  subscribeAppLogRecords,
  supportBundleFilename,
  type AppLogField,
  type AppLogLevel,
  type AppLogRedactionMode,
  type AppLogRecord,
} from "@/lib/appLogs";
import { isFilePickerAvailable, saveFile } from "@/lib/filePicker";
import { appVersionLabel } from "@/lib/appVersion";
import { screenShellClassName } from "@/lib/screen-layout";
import { saveLogsExportAs } from "@/lib/saveText";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

const LEVEL_CLASS: Record<AppLogLevel, string> = {
  trace: "border-muted-foreground/30 bg-muted text-muted-foreground",
  debug: "border-indigo-500/30 bg-indigo-500/10 text-indigo-700 dark:text-indigo-300",
  info: "border-sky-500/30 bg-sky-500/10 text-sky-700 dark:text-sky-300",
  warning: "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  error: "border-destructive/30 bg-destructive/10 text-destructive",
};

const RENDER_STEP = 1000;
const MAX_RENDERED_LINES = 8000;
const SUPPORT_BUNDLE_PREVIEW_LINES = 30;
const SEARCH_INPUT_ID = "kb-logs-search";

export function Logs() {
  const addNotification = useUiStore((s) => s.addNotification);
  const records = useAppLogRecords();
  const [levelFilter, setLevelFilter] = React.useState<LogLevelFilter>("all");
  const [redacted, setRedacted] = React.useState(true);
  const [maskAmounts, setMaskAmounts] = React.useState(false);
  const [rawUntil, setRawUntil] = React.useState<number | null>(null);
  const [query, setQuery] = React.useState("");
  const [regex, setRegex] = React.useState(false);
  const [moduleFilter, setModuleFilter] = React.useState<string | null>(null);
  const [renderLimit, setRenderLimit] = React.useState(RENDER_STEP);
  const [expanded, setExpanded] = React.useState<Set<string>>(new Set());
  const [autoscroll, setAutoscroll] = React.useState(true);
  const [newWhilePaused, setNewWhilePaused] = React.useState(0);
  const [supportBundleOpen, setSupportBundleOpen] = React.useState(false);
  const [supportIssueDescription, setSupportIssueDescription] = React.useState("");
  const [supportBundleMode, setSupportBundleMode] =
    React.useState<AppLogRedactionMode>("high_signal");
  const viewportRef = React.useRef<HTMLDivElement | null>(null);
  const previousRecordCount = React.useRef(records.length);
  const bufferBytes = useAppLogBufferSize();
  const bufferFillPct = Math.min(
    100,
    Math.round((bufferBytes / APP_LOG_MAX_BYTES) * 100),
  );

  React.useEffect(() => {
    if (redacted) {
      setRawUntil(null);
      return;
    }
    const until = Date.now() + 5 * 60 * 1000;
    setRawUntil(until);
    const timeout = window.setTimeout(() => setRedacted(true), 5 * 60 * 1000);
    return () => window.clearTimeout(timeout);
  }, [redacted]);

  React.useEffect(() => {
    const delta = records.length - previousRecordCount.current;
    previousRecordCount.current = records.length;
    if (delta <= 0) return;
    if (autoscroll) {
      scrollToBottom();
    } else {
      setNewWhilePaused((current) => current + delta);
    }
  }, [records.length, autoscroll]);

  React.useEffect(() => {
    if (autoscroll) scrollToBottom();
  }, [autoscroll, renderLimit, query, levelFilter, moduleFilter, regex]);

  // Keyboard shortcuts: `/` focuses search, `Esc` clears active table filters.
  React.useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target;
      const inField =
        target instanceof HTMLInputElement ||
        target instanceof HTMLTextAreaElement ||
        target instanceof HTMLSelectElement ||
        (target instanceof HTMLElement && target.isContentEditable);
      if (event.key === "/" && !inField && !event.metaKey && !event.ctrlKey) {
        event.preventDefault();
        document.getElementById(SEARCH_INPUT_ID)?.focus();
        return;
      }
      if (event.key === "Escape" && (query || moduleFilter || levelFilter !== "all")) {
        setQuery("");
        setLevelFilter("all");
        setModuleFilter(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [query, levelFilter, moduleFilter]);

  const filteredRecords = React.useMemo(
    () =>
      filterRecords(records, query, regex, levelFilter, moduleFilter, {
        redacted,
        maskAmounts,
      }),
    [records, query, regex, levelFilter, moduleFilter, redacted, maskAmounts],
  );
  const visibleRecords = filteredRecords.slice(
    Math.max(0, filteredRecords.length - Math.min(renderLimit, MAX_RENDERED_LINES)),
  );
  const hasTableFilters = Boolean(query.trim()) || levelFilter !== "all" || Boolean(moduleFilter);
  const isEmptyBecauseFilters = records.length > 0 && filteredRecords.length === 0;
  const settingsActive = regex || !redacted || maskAmounts;

  const supportBundlePreview = React.useMemo(() => {
    if (!supportBundleOpen) return "";
    return buildSupportBundleContents({
      records,
      issueDescription: supportIssueDescription || "Preview issue description",
      mode: supportBundleMode,
      generatedAt: new Date().toISOString(),
      levelFilter,
      moduleFilter,
      query,
      regex,
    })
      .split("\n")
      .slice(0, SUPPORT_BUNDLE_PREVIEW_LINES)
      .join("\n");
  }, [
    levelFilter,
    moduleFilter,
    query,
    records,
    regex,
    supportBundleMode,
    supportBundleOpen,
    supportIssueDescription,
  ]);

  const clearTableFilters = () => {
    setQuery("");
    setRegex(false);
    setLevelFilter("all");
    setModuleFilter(null);
  };

  const exportFormat = async (format: "md" | "log" | "jsonl") => {
    if (!redacted && !window.confirm("Export raw logs? They may contain sensitive local data.")) {
      return;
    }
    const redaction = redacted
      ? maskAmounts
        ? "redacted-amounts"
        : "redacted"
      : "raw";
    const filename = logFilename(format, redaction);
    const contents = exportLogRecords(filteredRecords, format, {
      redacted,
      maskAmounts,
      header: {
        appVersion: appVersionLabel(),
        os: osLabel(),
        generatedAt: new Date().toISOString(),
        timeRange: timeRangeLabel(filteredRecords),
        activeFilter: filterLabel(levelFilter, moduleFilter, query, regex),
        redaction,
      },
    });
    try {
      if (isFilePickerAvailable) {
        const destination = await saveFile({
          title: "Export Kassiber logs",
          defaultPath: filename,
          filters: [{ name: format.toUpperCase(), extensions: [format] }],
        });
        if (!destination) return;
        await saveLogsExportAs(destination, contents);
        return;
      }
      triggerBrowserDownload(filename, contents, contentType(format));
    } catch (error) {
      addNotification({
        title: "Could not export logs",
        body: error instanceof Error ? error.message : String(error),
        tone: "error",
      });
    }
  };

  const exportSupportBundle = async () => {
    const description = supportIssueDescription.trim();
    if (!description) {
      addNotification({
        title: "Support bundle not exported",
        body: "Add a short issue description first.",
        tone: "warning",
      });
      return;
    }
    const generatedAt = new Date().toISOString();
    const filename = supportBundleFilename(new Date(generatedAt));
    const contents = buildSupportBundleContents({
      records,
      issueDescription: description,
      mode: supportBundleMode,
      generatedAt,
      levelFilter,
      moduleFilter,
      query,
      regex,
    });
    try {
      if (isFilePickerAvailable) {
        const destination = await saveFile({
          title: "Export Kassiber support bundle",
          defaultPath: filename,
          filters: [{ name: "JSONL", extensions: ["jsonl"] }],
        });
        if (!destination) return;
        await saveLogsExportAs(destination, contents);
        setSupportBundleOpen(false);
        return;
      }
      triggerBrowserDownload(filename, contents, "application/x-ndjson");
      setSupportBundleOpen(false);
    } catch (error) {
      addNotification({
        title: "Could not export support bundle",
        body: error instanceof Error ? error.message : String(error),
        tone: "error",
      });
    }
  };

  const copyLast200 = async () => {
    const text = filteredRecords
      .slice(-200)
      .map((record) => formatLogRecord(record, { redacted, maskAmounts }))
      .join("\n");
    try {
      await navigator.clipboard.writeText(text);
      addNotification({
        title: "Copied log excerpt",
        body: `${Math.min(filteredRecords.length, 200)} lines copied.`,
        tone: "success",
      });
    } catch (error) {
      addNotification({
        title: "Could not copy logs",
        body: error instanceof Error ? error.message : String(error),
        tone: "error",
      });
    }
  };

  const onScroll: React.UIEventHandler<HTMLDivElement> = (event) => {
    const node = event.currentTarget;
    const atBottom = node.scrollHeight - node.scrollTop - node.clientHeight < 12;
    if (node.scrollTop < 24 && renderLimit < MAX_RENDERED_LINES) {
      setRenderLimit((current) =>
        Math.min(MAX_RENDERED_LINES, current + RENDER_STEP),
      );
    }
    setAutoscroll(atBottom);
    if (atBottom) setNewWhilePaused(0);
  };

  function scrollToBottom() {
    window.requestAnimationFrame(() => {
      const node = viewportRef.current;
      if (!node) return;
      node.scrollTop = node.scrollHeight;
      setNewWhilePaused(0);
    });
  }

  return (
    <div className={cn(screenShellClassName, "flex h-full min-h-0 flex-col")}>
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="min-w-0 space-y-1">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            Developer tools
          </p>
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="text-2xl font-semibold tracking-tight">Logs</h2>
            <Badge
              variant="outline"
              className="gap-1 border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300"
            >
              <span
                className="size-2 animate-pulse rounded-full bg-emerald-500"
                aria-hidden="true"
              />
              Live
            </Badge>
          </div>
          <p className="text-sm text-muted-foreground">
            Local-only RAM buffer. Nothing is written to disk unless you export.
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button type="button" size="sm" variant="outline" onClick={copyLast200}>
            <Copy className="size-4" aria-hidden="true" />
            Copy 200
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button type="button" size="sm">
                <Download className="size-4" aria-hidden="true" />
                Export
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onClick={() => setSupportBundleOpen(true)}>
                <FileArchive className="size-4" aria-hidden="true" />
                Support bundle
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => void exportFormat("md")}>
                <FileText className="size-4" aria-hidden="true" />
                Markdown
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => void exportFormat("jsonl")}>
                <FileJson className="size-4" aria-hidden="true" />
                JSONL
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => void exportFormat("log")}>
                <FileText className="size-4" aria-hidden="true" />
                Log
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <Button
            type="button"
            size="icon"
            variant="ghost"
            className="size-8"
            onClick={clearAppLogRecords}
            title="Clear logs"
          >
            <Trash2 className="size-4" aria-hidden="true" />
          </Button>
        </div>
      </div>

      <Dialog open={supportBundleOpen} onOpenChange={setSupportBundleOpen}>
        <DialogContent className="grid max-h-[88vh] max-w-3xl grid-rows-[auto_minmax(0,1fr)_auto] gap-0 overflow-hidden p-0">
          <DialogHeader className="border-b px-5 py-4 pr-12">
            <DialogTitle>Export support bundle</DialogTitle>
            <DialogDescription>
              Review the generated JSONL before saving.
            </DialogDescription>
          </DialogHeader>
          <div className="min-h-0 space-y-4 overflow-auto px-5 py-4">
            <label className="space-y-2 text-sm">
              <span className="font-medium">Issue description</span>
              <Textarea
                value={supportIssueDescription}
                onChange={(event) => setSupportIssueDescription(event.target.value)}
                placeholder="What went wrong?"
                className="min-h-24 resize-y"
              />
            </label>

            <div className="space-y-2">
              <p className="text-sm font-medium">Bundle mode</p>
              <div className="grid gap-2 sm:grid-cols-2">
                <SupportBundleModeButton
                  active={supportBundleMode === "high_signal"}
                  title="High-signal"
                  description="Keeps amounts, addresses, txids, paths, labels, URLs, and error text readable."
                  onClick={() => setSupportBundleMode("high_signal")}
                />
                <SupportBundleModeButton
                  active={supportBundleMode === "public_safe"}
                  title="Public-safe"
                  description="Masks operational fields and exact amounts after stripping wallet and credential material."
                  onClick={() => setSupportBundleMode("public_safe")}
                />
              </div>
            </div>

            <div className="space-y-2">
              <div className="flex items-center justify-between gap-3">
                <p className="text-sm font-medium">Preview</p>
                <span className="text-xs text-muted-foreground">
                  First {SUPPORT_BUNDLE_PREVIEW_LINES} lines
                </span>
              </div>
              <pre className="max-h-80 overflow-auto rounded-md border bg-muted/40 p-3 font-mono text-xs whitespace-pre-wrap text-foreground">
                {supportBundlePreview}
              </pre>
            </div>
          </div>
          <DialogFooter className="border-t px-5 py-4">
            <DialogClose asChild>
              <Button type="button" variant="outline">
                Cancel
              </Button>
            </DialogClose>
            <Button
              type="button"
              onClick={() => void exportSupportBundle()}
              disabled={!supportIssueDescription.trim()}
            >
              Save bundle
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border bg-card">
        <LogsTableControls
          hasTableFilters={hasTableFilters}
          levelFilter={levelFilter}
          maskAmounts={maskAmounts}
          moduleFilter={moduleFilter}
          query={query}
          records={records}
          redacted={redacted}
          regex={regex}
          searchInputId={SEARCH_INPUT_ID}
          settingsActive={settingsActive}
          onClearFilters={clearTableFilters}
          onLevelFilterChange={setLevelFilter}
          onMaskAmountsChange={setMaskAmounts}
          onModuleFilterChange={setModuleFilter}
          onQueryChange={setQuery}
          onRedactedChange={setRedacted}
          onRegexChange={setRegex}
        />

        {!redacted ? (
          <div className="border-b border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            Raw logs are visible until {rawUntil ? new Date(rawUntil).toLocaleTimeString() : "soon"}.
            Exports taken now are watermarked.
          </div>
        ) : null}

        <div className="relative border-b">
          <div className="flex items-center justify-between px-3 py-1.5 font-mono text-xs text-foreground/70">
            <span>
              {filteredRecords.length} records · RAM buffer {formatBytes(bufferBytes)} ({bufferFillPct}%)
            </span>
            <span>rendering {visibleRecords.length} newest</span>
          </div>
          <div
            aria-hidden="true"
            className="pointer-events-none absolute bottom-0 left-0 h-px bg-primary/60"
            style={{ width: `${bufferFillPct}%` }}
          />
        </div>

        <div className="relative min-h-0 flex-1">
          <div
            ref={viewportRef}
            onScroll={onScroll}
            className="h-full overflow-auto font-mono text-[12px] leading-5"
          >
            {visibleRecords.length === 0 ? (
              <div className="flex h-full min-h-64 flex-col items-center justify-center gap-1 text-sm text-muted-foreground">
                <span>{isEmptyBecauseFilters ? "No matching log records" : "Waiting for daemon traffic…"}</span>
                {hasTableFilters ? (
                  <span className="text-xs">
                    {[
                      levelFilter !== "all" ? `level ${levelFilter}` : null,
                      moduleFilter ? `module ${moduleFilter}` : null,
                      query ? `search "${query}"` : null,
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </span>
                ) : null}
              </div>
            ) : (
              <div className="min-w-[960px]">
                {visibleRecords.map((record) => (
                  <LogRow
                    key={record.id}
                    record={record}
                    redacted={redacted}
                    maskAmounts={maskAmounts}
                    expanded={expanded.has(record.id)}
                    onToggle={() =>
                      setExpanded((current) => {
                        const next = new Set(current);
                        if (next.has(record.id)) next.delete(record.id);
                        else next.add(record.id);
                        return next;
                      })
                    }
                  />
                ))}
              </div>
            )}
          </div>
          {!autoscroll && newWhilePaused > 0 ? (
            <Button
              type="button"
              size="sm"
              className="absolute bottom-4 left-1/2 -translate-x-1/2 shadow-lg"
              onClick={() => {
                setAutoscroll(true);
                scrollToBottom();
              }}
            >
              ↓ Jump to latest ({newWhilePaused} new)
            </Button>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function buildSupportBundleContents({
  records,
  issueDescription,
  mode,
  generatedAt,
  levelFilter,
  moduleFilter,
  query,
  regex,
}: {
  records: AppLogRecord[];
  issueDescription: string;
  mode: AppLogRedactionMode;
  generatedAt: string;
  levelFilter: LogLevelFilter;
  moduleFilter: string | null;
  query: string;
  regex: boolean;
}): string {
  return exportSupportBundleRecords(records, {
    issueDescription,
    mode,
    header: {
      appVersion: appVersionLabel(),
      os: osLabel(),
      generatedAt,
      timeRange: timeRangeLabel(records),
      activeFilter: `support_bundle=all_records, ui_filter=(${filterLabel(
        levelFilter,
        moduleFilter,
        query,
        regex,
      )})`,
      redaction: mode,
    },
  });
}

function SupportBundleModeButton({
  active,
  title,
  description,
  onClick,
}: {
  active: boolean;
  title: string;
  description: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      className={cn(
        "min-h-24 rounded-md border px-3 py-2 text-left transition-colors",
        active
          ? "border-primary bg-primary/10 text-primary"
          : "border-border bg-muted/20 text-foreground hover:bg-muted/50",
      )}
      onClick={onClick}
    >
      <span className="block text-sm font-semibold">{title}</span>
      <span className="mt-1 block text-xs leading-5 text-muted-foreground">
        {description}
      </span>
    </button>
  );
}

function LogRow({
  record,
  redacted,
  maskAmounts,
  expanded,
  onToggle,
}: {
  record: AppLogRecord;
  redacted: boolean;
  maskAmounts: boolean;
  expanded: boolean;
  onToggle: () => void;
}) {
  const rendered = redactLogRecord(record, { redacted, maskAmounts });
  const fieldText = fieldsForScreen(rendered.fields);
  const time = rowTime(rendered.ts);
  return (
    <article className="border-b">
      <button
        type="button"
        aria-expanded={expanded}
        className={cn(
          "grid w-full grid-cols-[20px_112px_76px_180px_minmax(320px,1fr)] items-center gap-3 px-3 py-1.5 text-left transition-colors hover:bg-muted/60",
          record.level === "error" && "bg-destructive/10",
        )}
        onClick={onToggle}
      >
        <ChevronRight
          className={cn(
            "size-3.5 text-muted-foreground transition-transform",
            expanded && "rotate-90",
          )}
          aria-hidden="true"
        />
        <span
          className="whitespace-nowrap text-muted-foreground"
          title={time.full}
        >
          {time.display}
        </span>
        <Badge variant="outline" className={cn("justify-center uppercase", LEVEL_CLASS[rendered.level])}>
          {rendered.level}
        </Badge>
        <span
          className="truncate text-muted-foreground"
          title={logLocation(rendered)}
        >
          {rendered.module}
        </span>
        <span className="min-w-0">
          <span className="text-foreground">{rendered.msg}</span>
          {fieldText ? <span className="ml-2 text-muted-foreground">{fieldText}</span> : null}
        </span>
      </button>
      {expanded ? (
        <pre className="max-h-96 overflow-auto border-t bg-muted/40 px-3 py-3 text-xs text-foreground">
          {JSON.stringify(rendered, null, 2)}
        </pre>
      ) : null}
    </article>
  );
}

function logLocation(record: AppLogRecord): string {
  const base = `${record.module}:${record.file}`;
  return record.line > 0 ? `${base}:${record.line}` : base;
}

function useAppLogRecords(): AppLogRecord[] {
  return React.useSyncExternalStore(
    subscribeAppLogRecords,
    getAppLogRecords,
    getAppLogRecords,
  );
}

function useAppLogBufferSize(): number {
  return React.useSyncExternalStore(
    subscribeAppLogRecords,
    getAppLogBufferSize,
    getAppLogBufferSize,
  );
}

function filterRecords(
  records: AppLogRecord[],
  query: string,
  regex: boolean,
  levelFilter: LogLevelFilter,
  moduleFilter: string | null,
  renderOptions: { redacted: boolean; maskAmounts: boolean },
): AppLogRecord[] {
  const needle = query.trim();
  const matcher = makeMatcher(needle, regex);
  return records.filter((record) => {
    if (levelFilter !== "all" && record.level !== levelFilter) return false;
    if (moduleFilter && record.module !== moduleFilter) return false;
    if (!matcher) return true;
    const text = formatLogRecord(record, renderOptions);
    return matcher(text);
  });
}

function makeMatcher(query: string, regex: boolean): ((value: string) => boolean) | null {
  if (!query) return null;
  if (!regex) {
    const lowered = query.toLowerCase();
    return (value) => value.toLowerCase().includes(lowered);
  }
  try {
    const pattern = new RegExp(query, "i");
    return (value) => pattern.test(value);
  } catch {
    return () => false;
  }
}

function fieldsForScreen(fields: Record<string, AppLogField>): string {
  return Object.entries(fields)
    .map(([key, field]) => {
      const value =
        typeof field.value === "string" ||
        typeof field.value === "number" ||
        typeof field.value === "boolean"
          ? String(field.value)
          : JSON.stringify(field.value);
      return `${key}=${value}`;
    })
    .join(" ");
}

function rowTime(ts: string): { display: string; full: string } {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return { display: ts, full: ts };
  const full = date.toISOString();
  const today = new Date().toISOString().slice(0, 10);
  const display = full.startsWith(today)
    ? full.slice(11, 23) // HH:MM:SS.mmm
    : `${full.slice(5, 10)} ${full.slice(11, 19)}`; // MM-DD HH:MM:SS
  return { display, full };
}

function timeRangeLabel(records: AppLogRecord[]): string {
  if (!records.length) return "empty";
  return `${records[0].ts} to ${records[records.length - 1].ts}`;
}

function filterLabel(
  levelFilter: LogLevelFilter,
  moduleFilter: string | null,
  query: string,
  regex: boolean,
): string {
  return [
    levelFilter === "all" ? "level=all" : `level=${levelFilter}`,
    moduleFilter ? `module=${moduleFilter}` : "module=all",
    query ? `search=${regex ? "regex:" : ""}${query}` : "search=none",
  ].join(", ");
}

function osLabel(): string {
  if (typeof navigator === "undefined") return "unknown";
  const nav = navigator as Navigator & {
    userAgentData?: { platform?: string };
  };
  return nav.userAgentData?.platform ?? nav.platform ?? "unknown";
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

function contentType(format: "md" | "log" | "jsonl"): string {
  if (format === "md") return "text/markdown";
  if (format === "jsonl") return "application/x-ndjson";
  return "text/plain";
}

function triggerBrowserDownload(filename: string, contents: string, type: string): void {
  const blob = new Blob([contents], { type });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = filename;
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
  window.setTimeout(() => URL.revokeObjectURL(url), 0);
}
