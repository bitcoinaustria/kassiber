import { Download, Trash2 } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useUiStore, type AppLogEntry, type AppLogLevel } from "@/store/ui";
import { cn } from "@/lib/utils";

const LEVEL_CLASS: Record<AppLogLevel, string> = {
  debug: "border-slate-500/25 bg-slate-500/10 text-slate-700 dark:text-slate-300",
  info: "border-sky-500/25 bg-sky-500/10 text-sky-700 dark:text-sky-300",
  warning:
    "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  error: "border-destructive/30 bg-destructive/10 text-destructive",
};

export function Diagnostics() {
  const entries = useUiStore((s) => s.logEntries);
  const clearLogEntries = useUiStore((s) => s.clearLogEntries);

  const downloadLog = () => {
    const payload = {
      exportedAt: new Date().toISOString(),
      entries,
    };
    const blob = new Blob([JSON.stringify(payload, null, 2)], {
      type: "application/json",
    });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `kassiber-diagnostics-${timestampForFilename()}.json`;
    document.body.append(anchor);
    anchor.click();
    anchor.remove();
    window.setTimeout(() => URL.revokeObjectURL(url), 0);
  };

  return (
    <main className="flex h-full min-h-0 flex-col gap-3 bg-background p-3 sm:gap-4 sm:p-4 md:p-5">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0 space-y-1">
          <h2 className="text-base font-semibold">Daemon log</h2>
          <p className="text-sm text-muted-foreground">
            Recent local daemon requests, terminal status, and renderer-side
            transport errors.
          </p>
        </div>
        <div className="flex shrink-0 flex-wrap gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={downloadLog}
            disabled={entries.length === 0}
          >
            <Download className="size-4" aria-hidden="true" />
            Download log
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={clearLogEntries}
            disabled={entries.length === 0}
          >
            <Trash2 className="size-4" aria-hidden="true" />
            Clear
          </Button>
        </div>
      </div>

      <section className="min-h-0 flex-1 overflow-hidden rounded-md border bg-background">
        {entries.length === 0 ? (
          <div className="flex h-full min-h-64 items-center justify-center p-6 text-sm text-muted-foreground">
            No daemon activity has been recorded in this app session.
          </div>
        ) : (
          <ScrollArea className="h-full">
            <div className="divide-y">
              {entries.map((entry) => (
                <LogEntryRow key={entry.id} entry={entry} />
              ))}
            </div>
          </ScrollArea>
        )}
      </section>
    </main>
  );
}

function LogEntryRow({ entry }: { entry: AppLogEntry }) {
  const details = formatDetails(entry.details);
  return (
    <article className="grid gap-2 p-3 text-sm">
      <div className="flex flex-wrap items-center gap-2">
        <Badge
          variant="outline"
          className={cn("uppercase", LEVEL_CLASS[entry.level])}
        >
          {entry.level}
        </Badge>
        <span className="font-mono text-xs text-muted-foreground">
          {formatTimestamp(entry.createdAt)}
        </span>
        <span className="rounded-sm bg-muted px-1.5 py-0.5 font-mono text-xs">
          {entry.source}
        </span>
      </div>
      <p className="m-0 font-medium">{entry.message}</p>
      {details ? (
        <pre className="max-h-72 overflow-auto rounded-md bg-muted p-3 font-mono text-xs leading-5 whitespace-pre-wrap text-muted-foreground">
          {details}
        </pre>
      ) : null}
    </article>
  );
}

function formatDetails(details: unknown): string | null {
  if (details === null || details === undefined) return null;
  if (typeof details === "string") return details;
  try {
    return JSON.stringify(details, null, 2);
  } catch {
    return String(details);
  }
}

function formatTimestamp(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function timestampForFilename(): string {
  return new Date().toISOString().replace(/[:.]/g, "-");
}
