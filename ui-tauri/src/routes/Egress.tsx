import {
  AlertTriangle,
  CheckCircle2,
  Database,
  LockKeyhole,
  Plane,
  RefreshCw,
} from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { useDaemon } from "@/daemon/client";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { screenShellClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";

type EgressAllowlistStatus = "expected" | "unexpected" | "unknown";

interface EgressRecord {
  id: number;
  ts: string;
  subsystem: string;
  host: string;
  port: number | null;
  scheme: string;
  operation: string;
  method: string | null;
  bytes_out: number;
  via_proxy: boolean;
  allowlist_status: EgressAllowlistStatus;
  allowlist_label: string | null;
  allowlist_source: string | null;
  user_allowlisted: boolean;
}

interface DbHeaderProof {
  exists?: boolean;
  classification?: string;
  sqlite_plaintext_header?: boolean;
  encrypted_like?: boolean;
  prefix_hex?: string;
  error?: string;
}

interface EgressSnapshot {
  records: EgressRecord[];
  last_id: number;
  gap: boolean;
  started_at: string;
  buffer_bytes: number;
  max_bytes: number;
  allowlist_complete: boolean;
  db_header: DbHeaderProof;
  summary: {
    total_records: number;
    unexpected: number;
    update: number;
    by_subsystem: Record<string, { records: number; bytes_out: number }>;
  };
}

const SNAPSHOT_ARGS = { limit: 1000 };
const EMPTY_RECORDS: EgressRecord[] = [];

export function Egress() {
  const { t } = useTranslation(["review", "common"]);
  const [query, setQuery] = React.useState("");
  const snapshotQuery = useDaemon<EgressSnapshot>(
    "ui.egress.snapshot",
    SNAPSHOT_ARGS,
    {
      refetchInterval: 4_000,
      staleTime: 1_000,
    },
  );
  const snapshot = snapshotQuery.data?.data;
  const records = snapshot?.records ?? EMPTY_RECORDS;
  const translateReview = React.useCallback(
    (key: string) => t(key as never),
    [t],
  );
  const filteredRecords = React.useMemo(
    () => filterRecords(records, query),
    [records, query],
  );
  const db = snapshot?.db_header ?? {};
  const dbTone = db.sqlite_plaintext_header
    ? "bad"
    : db.encrypted_like
      ? "good"
      : "neutral";

  return (
    <main className={screenShellClassName}>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <Plane className="size-5 text-muted-foreground" aria-hidden="true" />
            <h1 className="text-xl font-semibold tracking-normal">
              {t("egress.title")}
            </h1>
          </div>
          <p className="mt-1 text-sm text-muted-foreground">
            {t("egress.subtitle")}
          </p>
        </div>
        <Button
          type="button"
          size="sm"
          variant="outline"
          onClick={() => void snapshotQuery.refetch()}
          disabled={snapshotQuery.isFetching}
        >
          <RefreshCw
            className={cn("size-4", snapshotQuery.isFetching && "animate-spin")}
            aria-hidden="true"
          />
          {t("egress.refresh")}
        </Button>
      </div>

      {snapshotQuery.error ? (
        <div className="rounded-md border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">
          {t("egress.loadFailed", { error: snapshotQuery.error.message })}
        </div>
      ) : null}

      <section className="grid gap-3 md:grid-cols-4">
        <Metric
          label={t("egress.metric.total")}
          value={String(snapshot?.summary.total_records ?? 0)}
          tone="neutral"
        />
        <Metric
          label={t("egress.metric.unexpected")}
          value={String(snapshot?.summary.unexpected ?? 0)}
          tone={(snapshot?.summary.unexpected ?? 0) > 0 ? "bad" : "good"}
        />
        <Metric
          label={t("egress.metric.update")}
          value={String(snapshot?.summary.update ?? 0)}
          tone={(snapshot?.summary.update ?? 0) > 0 ? "bad" : "good"}
        />
        <Metric
          label={t("egress.metric.dbHeader")}
          value={dbHeaderLabel(db, translateReview)}
          tone={dbTone}
        />
      </section>

      <section className="grid gap-3 lg:grid-cols-[minmax(0,1fr)_minmax(320px,420px)]">
        <div className="rounded-md border bg-background">
          <div className="flex flex-wrap items-center gap-2 border-b p-3">
            <Input
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder={t("egress.queryPlaceholder")}
              className="h-8 w-full font-mono text-xs sm:w-72"
            />
            {snapshot?.allowlist_complete === false ? (
              <Badge variant="outline" className="gap-1">
                <LockKeyhole className="size-3" aria-hidden="true" />
                {t("egress.db.incompleteAllowlist")}
              </Badge>
            ) : null}
            {snapshot?.gap ? (
              <Badge variant="outline">{t("egress.gap")}</Badge>
            ) : null}
          </div>
          <div className="overflow-x-auto">
            <table className="w-full min-w-[760px] text-left text-sm">
              <thead className="border-b bg-muted/30 text-xs uppercase text-muted-foreground">
                <tr>
                  <th className="px-3 py-2 font-medium">{t("egress.table.time")}</th>
                  <th className="px-3 py-2 font-medium">
                    {t("egress.table.subsystem")}
                  </th>
                  <th className="px-3 py-2 font-medium">
                    {t("egress.table.destination")}
                  </th>
                  <th className="px-3 py-2 font-medium">
                    {t("egress.table.operation")}
                  </th>
                  <th className="px-3 py-2 text-right font-medium">
                    {t("egress.table.bytesOut")}
                  </th>
                  <th className="px-3 py-2 font-medium">
                    {t("egress.table.allowlist")}
                  </th>
                </tr>
              </thead>
              <tbody>
                {filteredRecords.length === 0 ? (
                  <tr>
                    <td
                      colSpan={6}
                      className="px-3 py-10 text-center text-sm text-muted-foreground"
                    >
                      {records.length === 0
                        ? t("egress.empty")
                        : t("egress.table.noRows")}
                    </td>
                  </tr>
                ) : (
                  filteredRecords.map((record) => (
                    <EgressRow key={record.id} record={record} />
                  ))
                )}
              </tbody>
            </table>
          </div>
        </div>

        <aside className="space-y-3">
          <div className="rounded-md border bg-background p-3">
            <div className="mb-3 flex items-center gap-2">
              <Database className="size-4 text-muted-foreground" aria-hidden="true" />
              <h2 className="text-sm font-semibold">{t("egress.db.title")}</h2>
            </div>
            <dl className="space-y-2 text-sm">
              <DetailRow
                label={t("egress.db.classification")}
                value={dbHeaderLabel(db, translateReview)}
                tone={dbTone}
              />
              <DetailRow
                label={t("egress.db.prefix")}
                value={db.prefix_hex || t("egress.db.none")}
                mono
              />
            </dl>
          </div>

          <div className="rounded-md border bg-background p-3">
            <h2 className="mb-3 text-sm font-semibold">
              {t("egress.subsystems.title")}
            </h2>
            <div className="space-y-2">
              {Object.entries(snapshot?.summary.by_subsystem ?? {}).length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  {t("egress.subsystems.empty")}
                </p>
              ) : (
                Object.entries(snapshot?.summary.by_subsystem ?? {}).map(
                  ([name, item]) => (
                    <div
                      key={name}
                      className="flex items-center justify-between gap-3 text-sm"
                    >
                      <span className="capitalize">
                        {t(`egress.subsystem.${name}`, { defaultValue: name })}
                      </span>
                      <span className="font-mono text-xs text-muted-foreground">
                        {item.records} / {formatBytes(item.bytes_out)}
                      </span>
                    </div>
                  ),
                )
              )}
            </div>
          </div>
        </aside>
      </section>
    </main>
  );
}

function EgressRow({ record }: { record: EgressRecord }) {
  const { t } = useTranslation("review");
  return (
    <tr
      className={cn(
        "border-b last:border-b-0",
        record.allowlist_status === "unexpected" &&
          "bg-destructive/10 text-destructive",
      )}
    >
      <td className="whitespace-nowrap px-3 py-2 font-mono text-xs text-muted-foreground">
        {formatTime(record.ts)}
      </td>
      <td className="px-3 py-2">
        <Badge variant="outline" className="capitalize">
          {t(`egress.subsystem.${record.subsystem}`, {
            defaultValue: record.subsystem,
          })}
        </Badge>
      </td>
      <td className="px-3 py-2 font-mono text-xs">
        {record.host}
        {record.port ? `:${record.port}` : ""}
        {record.via_proxy ? (
          <span className="ml-2 text-muted-foreground">
            {t("egress.table.proxy")}
          </span>
        ) : null}
      </td>
      <td className="px-3 py-2">
        <span className="font-mono text-xs">{record.operation}</span>
        {record.method ? (
          <span className="ml-2 text-xs text-muted-foreground">
            {record.method}
          </span>
        ) : null}
      </td>
      <td className="px-3 py-2 text-right font-mono text-xs">
        {formatBytes(record.bytes_out)}
      </td>
      <td className="px-3 py-2">
        <StatusBadge record={record} />
      </td>
    </tr>
  );
}

function StatusBadge({ record }: { record: EgressRecord }) {
  const { t } = useTranslation("review");
  if (record.allowlist_status === "unexpected") {
    return (
      <Badge
        variant="outline"
        className="gap-1 border-destructive/30 bg-destructive/10 text-destructive"
      >
        <AlertTriangle className="size-3" aria-hidden="true" />
        {t("egress.status.unexpected")}
      </Badge>
    );
  }
  if (record.allowlist_status === "unknown") {
    return <Badge variant="outline">{t("egress.status.unknown")}</Badge>;
  }
  return (
    <Badge variant="outline" className="gap-1">
      <CheckCircle2 className="size-3 text-emerald-600" aria-hidden="true" />
      {record.user_allowlisted
        ? t("egress.status.user")
        : t("egress.status.builtIn")}
    </Badge>
  );
}

function Metric({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone: "good" | "bad" | "neutral";
}) {
  return (
    <div
      className={cn(
        "rounded-md border bg-background p-3",
        tone === "good" && "border-emerald-500/30",
        tone === "bad" && "border-destructive/40 bg-destructive/10",
      )}
    >
      <div className="text-xs uppercase text-muted-foreground">{label}</div>
      <div className="mt-1 truncate font-mono text-lg font-semibold">{value}</div>
    </div>
  );
}

function DetailRow({
  label,
  value,
  tone = "neutral",
  mono = false,
}: {
  label: string;
  value: string;
  tone?: "good" | "bad" | "neutral";
  mono?: boolean;
}) {
  return (
    <div className="grid grid-cols-[120px_minmax(0,1fr)] gap-3">
      <dt className="text-muted-foreground">{label}</dt>
      <dd
        className={cn(
          "min-w-0 break-words",
          mono && "font-mono text-xs",
          tone === "good" && "text-emerald-700 dark:text-emerald-300",
          tone === "bad" && "text-destructive",
        )}
      >
        {value}
      </dd>
    </div>
  );
}

function filterRecords(records: EgressRecord[], query: string) {
  const needle = query.trim().toLowerCase();
  if (!needle) return records;
  return records.filter((record) =>
    [
      record.host,
      record.subsystem,
      record.operation,
      record.method,
      record.allowlist_label,
      record.allowlist_source,
    ]
      .filter(Boolean)
      .some((value) => String(value).toLowerCase().includes(needle)),
  );
}

function formatBytes(bytes: number) {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

function formatTime(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function dbHeaderLabel(db: DbHeaderProof, t: (key: string) => string) {
  if (db.sqlite_plaintext_header) return t("egress.db.plaintext");
  if (db.encrypted_like) return t("egress.db.ciphertextLike");
  if (db.classification === "missing") return t("egress.db.missing");
  if (db.classification === "unreadable") return t("egress.db.unreadable");
  return t("egress.db.unknown");
}

export default Egress;
