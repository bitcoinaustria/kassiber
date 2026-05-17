import {
  Copy,
  Download,
  Eye,
  FileJson,
  FileText,
  Regex,
  Shield,
  Trash2,
} from "lucide-react";
import * as React from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  appLogLevels,
  clearAppLogRecords,
  exportLogRecords,
  formatLogRecord,
  getAppLogRecords,
  getAppLogStorageSize,
  getAppLogSubscriptionLevel,
  logFilename,
  redactLogRecord,
  setAppLogSubscriptionLevel,
  subscribeAppLogRecords,
  type AppLogField,
  type AppLogLevel,
  type AppLogRecord,
} from "@/lib/appLogs";
import { isFilePickerAvailable, saveFile } from "@/lib/filePicker";
import { saveDiagnosticsLogAs } from "@/lib/saveText";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

const LEVEL_CLASS: Record<AppLogLevel, string> = {
  trace: "border-zinc-500/35 bg-zinc-950 text-zinc-200",
  debug: "border-slate-400/35 bg-slate-950 text-slate-200",
  info: "border-sky-400/35 bg-sky-950 text-sky-200",
  warning: "border-amber-400/35 bg-amber-950 text-amber-200",
  error: "border-red-400/35 bg-red-950 text-red-200",
};

const RENDER_STEP = 1000;
const MAX_RENDERED_LINES = 8000;

export function Logs() {
  const addNotification = useUiStore((s) => s.addNotification);
  const records = useAppLogRecords();
  const [level, setLevel] = React.useState<AppLogLevel>(
    getAppLogSubscriptionLevel(),
  );
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
  const viewportRef = React.useRef<HTMLDivElement | null>(null);
  const previousRecordCount = React.useRef(records.length);
  const storageBytes = getAppLogStorageSize();

  React.useEffect(() => {
    setAppLogSubscriptionLevel(level);
  }, [level]);

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
  }, [autoscroll, renderLimit, query, moduleFilter, regex]);

  const moduleCounts = React.useMemo(() => countByModule(records), [records]);
  const filteredRecords = React.useMemo(
    () => filterRecords(records, query, regex, moduleFilter, { redacted, maskAmounts }),
    [records, query, regex, moduleFilter, redacted, maskAmounts],
  );
  const visibleRecords = filteredRecords.slice(
    Math.max(0, filteredRecords.length - Math.min(renderLimit, MAX_RENDERED_LINES)),
  );

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
        activeFilter: filterLabel(level, moduleFilter, query, regex),
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
        await saveDiagnosticsLogAs(destination, contents);
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
    <main className="flex h-full min-h-0 flex-col bg-zinc-950 text-zinc-100">
      <header className="flex flex-wrap items-center gap-2 border-b border-zinc-800 bg-zinc-950 px-3 py-2">
        <span className="flex items-center gap-2 rounded-sm border border-emerald-500/30 bg-emerald-500/10 px-2 py-1 text-xs text-emerald-200">
          <span className="size-2 rounded-full bg-emerald-400" aria-hidden="true" />
          Live
        </span>
        <Select value={level} onValueChange={(value) => setLevel(value as AppLogLevel)}>
          <SelectTrigger className="h-8 w-[122px] border-zinc-700 bg-zinc-900 font-mono text-xs">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {appLogLevels().map((item) => (
              <SelectItem key={item} value={item}>
                {item.toUpperCase()}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <div className="flex min-w-0 flex-1 flex-wrap gap-1">
          {Object.entries(moduleCounts).map(([module, count]) => (
            <button
              key={module}
              type="button"
              aria-pressed={moduleFilter === module}
              onClick={() =>
                setModuleFilter((current) => (current === module ? null : module))
              }
              className={cn(
                "rounded-sm border border-zinc-700 px-2 py-1 font-mono text-xs text-zinc-300 hover:bg-zinc-800",
                moduleFilter === module && "border-sky-400 bg-sky-500/15 text-sky-100",
              )}
            >
              {module} {count}
            </button>
          ))}
        </div>
        <div className="flex items-center gap-1">
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search logs"
            className="h-8 w-40 border-zinc-700 bg-zinc-900 font-mono text-xs text-zinc-100 placeholder:text-zinc-500"
          />
          <Button
            type="button"
            size="icon"
            variant={regex ? "default" : "outline"}
            className="size-8"
            onClick={() => setRegex((current) => !current)}
            title="Regex search"
          >
            <Regex className="size-4" aria-hidden="true" />
          </Button>
        </div>
        <Button
          type="button"
          size="sm"
          variant={redacted ? "secondary" : "destructive"}
          onClick={() => setRedacted((current) => !current)}
        >
          {redacted ? (
            <Shield className="size-4" aria-hidden="true" />
          ) : (
            <Eye className="size-4" aria-hidden="true" />
          )}
          {redacted ? "Redacted" : "Raw"}
        </Button>
        <label className="flex items-center gap-2 rounded-sm border border-zinc-700 px-2 py-1 text-xs text-zinc-300">
          <Checkbox
            checked={maskAmounts}
            onCheckedChange={(checked) => setMaskAmounts(Boolean(checked))}
          />
          Amounts
        </label>
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
          className="size-8 text-zinc-400"
          onClick={clearAppLogRecords}
          title="Clear logs"
        >
          <Trash2 className="size-4" aria-hidden="true" />
        </Button>
      </header>

      {!redacted ? (
        <div className="border-b border-red-500/40 bg-red-950 px-3 py-2 text-sm text-red-100">
          Raw logs are visible until {rawUntil ? new Date(rawUntil).toLocaleTimeString() : "soon"}.
          Exports taken now are watermarked.
        </div>
      ) : null}

      <div className="flex items-center justify-between border-b border-zinc-800 px-3 py-1 font-mono text-xs text-zinc-400">
        <span>{filteredRecords.length} records · ring {formatBytes(storageBytes)}</span>
        <span>rendering {visibleRecords.length} newest records</span>
      </div>

      <section className="relative min-h-0 flex-1">
        <div
          ref={viewportRef}
          onScroll={onScroll}
          className="h-full overflow-auto font-mono text-[12px] leading-5"
        >
          {visibleRecords.length === 0 ? (
            <div className="grid h-full min-h-64 place-items-center text-sm text-zinc-500">
              No records match the current stream and search settings.
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
      </section>
    </main>
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
  const expandable = record.level === "error" || Boolean(record.spantrace?.length);
  return (
    <article className="border-b border-zinc-900/80">
      <button
        type="button"
        className={cn(
          "grid w-full grid-cols-[150px_76px_260px_minmax(320px,1fr)] gap-3 px-3 py-1.5 text-left hover:bg-zinc-900",
          record.level === "error" && "bg-red-950/20",
        )}
        onClick={expandable ? onToggle : undefined}
      >
        <span className="text-zinc-500">{timeOnly(rendered.ts)}</span>
        <Badge variant="outline" className={cn("justify-center uppercase", LEVEL_CLASS[rendered.level])}>
          {rendered.level}
        </Badge>
        <span className="truncate text-zinc-400">
          {rendered.module}:{rendered.file}:{rendered.line}
        </span>
        <span className="min-w-0">
          <span className="text-zinc-100">{rendered.msg}</span>
          {fieldText ? <span className="ml-2 text-zinc-500">{fieldText}</span> : null}
        </span>
      </button>
      {expanded ? (
        <pre className="max-h-96 overflow-auto border-t border-zinc-900 bg-zinc-950 px-3 py-3 text-xs text-zinc-300">
          {JSON.stringify(rendered, null, 2)}
        </pre>
      ) : null}
    </article>
  );
}

function useAppLogRecords(): AppLogRecord[] {
  return React.useSyncExternalStore(
    subscribeAppLogRecords,
    getAppLogRecords,
    getAppLogRecords,
  );
}

function filterRecords(
  records: AppLogRecord[],
  query: string,
  regex: boolean,
  moduleFilter: string | null,
  renderOptions: { redacted: boolean; maskAmounts: boolean },
): AppLogRecord[] {
  const needle = query.trim();
  const matcher = makeMatcher(needle, regex);
  return records.filter((record) => {
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

function countByModule(records: AppLogRecord[]): Record<string, number> {
  return records.reduce<Record<string, number>>((acc, record) => {
    acc[record.module] = (acc[record.module] ?? 0) + 1;
    return acc;
  }, {});
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

function timeOnly(ts: string): string {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return ts;
  return date.toISOString();
}

function timeRangeLabel(records: AppLogRecord[]): string {
  if (!records.length) return "empty";
  return `${records[0].ts} to ${records[records.length - 1].ts}`;
}

function filterLabel(
  level: AppLogLevel,
  moduleFilter: string | null,
  query: string,
  regex: boolean,
): string {
  return [
    `subscription>=${level}`,
    moduleFilter ? `module=${moduleFilter}` : "module=all",
    query ? `search=${regex ? "regex:" : ""}${query}` : "search=none",
  ].join(", ");
}

function appVersionLabel(): string {
  return `${__APP_VERSION__} (${__APP_COMMIT__ ? __APP_COMMIT__.slice(0, 7) : "unknown"})`;
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
