import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { ArrowDown, ArrowUpRight, ScrollText, Search } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  getAppLogRecords,
  redactLogRecord,
  subscribeAppLogRecords,
  type AppLogLevel,
  type AppLogRecord,
} from "@/lib/appLogs";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import { SurfaceEmpty, SurfaceFrame } from "./SurfaceLayout";

type LevelFilter = "all" | "warning" | "error";

/** The tail it keeps on screen; the Logs page pages through the rest. */
const TAIL = 500;

const LEVEL_TONE: Record<AppLogLevel, string> = {
  trace: "text-muted-foreground/70",
  debug: "text-indigo-600 dark:text-indigo-300",
  info: "text-sky-600 dark:text-sky-300",
  warning: "text-amber-600 dark:text-amber-300",
  error: "text-destructive",
};

const LEVEL_SHORT: Record<AppLogLevel, string> = {
  trace: "TRC",
  debug: "DBG",
  info: "INF",
  warning: "WRN",
  error: "ERR",
};

function keepsLevel(filter: LevelFilter, level: AppLogLevel) {
  if (filter === "error") return level === "error";
  if (filter === "warning") return level === "warning" || level === "error";
  return true;
}

/**
 * A live tail of the app's log, after T3 Code's terminal surface: the same
 * bounded, in-memory records the Logs page reads, always redacted here, and
 * with amounts masked while sensitive values are hidden. It follows new
 * lines until scrolled up. Nothing is written anywhere.
 */
export function LogsSurface() {
  const { t } = useTranslation("assistant");
  const navigate = useNavigate();
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const records = React.useSyncExternalStore(
    subscribeAppLogRecords,
    getAppLogRecords,
    getAppLogRecords,
  );
  const [level, setLevel] = React.useState<LevelFilter>("all");
  const [query, setQuery] = React.useState("");
  const [following, setFollowing] = React.useState(true);
  const scrollRef = React.useRef<HTMLDivElement>(null);

  const visible = React.useMemo(() => {
    const needle = query.trim().toLowerCase();
    const rendered: AppLogRecord[] = [];
    for (const record of records) {
      if (!keepsLevel(level, record.level)) continue;
      const shown = redactLogRecord(record, { redacted: true, maskAmounts: hideSensitive });
      if (needle && !`${shown.module} ${shown.msg} ${fieldText(shown)}`.toLowerCase().includes(needle)) {
        continue;
      }
      rendered.push(shown);
    }
    return rendered.slice(-TAIL);
  }, [records, level, query, hideSensitive]);

  React.useLayoutEffect(() => {
    if (!following) return;
    const node = scrollRef.current;
    if (node) node.scrollTop = node.scrollHeight;
  }, [visible, following]);

  return (
    <SurfaceFrame
      label={t("panel.surfaceLogs")}
      toolbar={
        <>
          <div
            role="radiogroup"
            aria-label={t("panel.logs.level")}
            className="flex shrink-0 items-center gap-0.5 rounded-md bg-muted/60 p-0.5"
          >
            {(["all", "warning", "error"] as const).map((value) => (
              <button
                key={value}
                type="button"
                role="radio"
                aria-checked={level === value}
                className={cn(
                  "h-5 rounded-sm px-2 text-xs outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring",
                  level === value
                    ? "bg-background text-foreground shadow-xs"
                    : "text-muted-foreground hover:text-foreground",
                )}
                onClick={() => setLevel(value)}
              >
                {t(`panel.logs.levels.${value}`)}
              </button>
            ))}
          </div>
          <label className="flex h-6 min-w-0 flex-1 items-center gap-1.5 rounded-md border border-border/70 bg-background px-2 focus-within:ring-2 focus-within:ring-ring">
            <Search className="size-3 shrink-0 text-muted-foreground" aria-hidden="true" />
            <input
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("panel.logs.search")}
              aria-label={t("panel.logs.search")}
              className="min-w-0 flex-1 bg-transparent text-xs outline-none placeholder:text-muted-foreground"
            />
          </label>
        </>
      }
      footer={
        <>
          <span className="text-muted-foreground">{t("panel.logs.redacted")}</span>
          <Button
            type="button"
            variant="ghost"
            size="xs"
            className="ml-auto gap-1 text-muted-foreground"
            onClick={() => void navigate({ to: "/logs" })}
          >
            {t("panel.logs.openPage")}
            <ArrowUpRight className="size-3" aria-hidden="true" />
          </Button>
        </>
      }
    >
      {visible.length === 0 ? (
        <SurfaceEmpty
          icon={ScrollText}
          title={t(records.length ? "panel.logs.noMatch" : "panel.logs.emptyTitle")}
          body={records.length ? undefined : t("panel.logs.emptyBody")}
        />
      ) : (
        <div className="relative h-full">
          <div
            ref={scrollRef}
            role="log"
            aria-label={t("panel.surfaceLogs")}
            tabIndex={0}
            onScroll={(event) => {
              const node = event.currentTarget;
              setFollowing(node.scrollHeight - node.scrollTop - node.clientHeight < 24);
            }}
            className="h-full overflow-y-auto bg-muted/20 py-1 font-mono text-[0.6875rem] leading-[1.125rem] outline-none"
          >
            {visible.map((record) => (
              <LogLine key={record.id} record={record} />
            ))}
          </div>
          {following ? null : (
            <Button
              type="button"
              size="xs"
              variant="secondary"
              className="absolute right-3 bottom-3 gap-1 shadow-sm"
              onClick={() => setFollowing(true)}
            >
              <ArrowDown aria-hidden="true" />
              {t("panel.logs.follow")}
            </Button>
          )}
        </div>
      )}
    </SurfaceFrame>
  );
}

function LogLine({ record }: { record: AppLogRecord }) {
  const fields = fieldText(record);
  return (
    <div
      className={cn(
        "grid grid-cols-[auto_auto_minmax(0,1fr)] gap-x-2 px-3 hover:bg-accent/40",
        record.level === "error" && "bg-destructive/5",
      )}
      title={`${record.ts} ${record.module}`}
    >
      <span className="text-muted-foreground/80 tabular-nums">{lineTime(record.ts)}</span>
      <span className={cn("font-semibold", LEVEL_TONE[record.level])}>{LEVEL_SHORT[record.level]}</span>
      <span className="min-w-0 break-words">
        <span className="text-muted-foreground">{record.module} </span>
        <span className="text-foreground">{record.msg}</span>
        {fields ? <span className="text-muted-foreground"> {fields}</span> : null}
      </span>
    </div>
  );
}

function fieldText(record: AppLogRecord) {
  return Object.entries(record.fields)
    .map(([key, field]) => {
      const value =
        typeof field.value === "string" || typeof field.value === "number" || typeof field.value === "boolean"
          ? String(field.value)
          : JSON.stringify(field.value);
      return `${key}=${value}`;
    })
    .join(" ");
}

function lineTime(ts: string) {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return ts;
  return date.toTimeString().slice(0, 8);
}
