import {
  freshnessRunNeedsAttention,
  type FreshnessRunData,
} from "@/lib/syncResults";

export const DAEMON_EVENT_CHANNEL = "daemon://event";

export type DaemonFreshnessSignal =
  | "refresh"
  | "needs-attention"
  | "worker-error";

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function freshnessRunData(value: unknown): FreshnessRunData | null {
  if (!isRecord(value)) return null;
  const completed = Array.isArray(value.completed)
    ? value.completed.filter(isRecord).map((job) => ({
        id: typeof job.id === "string" ? job.id : undefined,
        job_type:
          typeof job.job_type === "string" ? job.job_type : undefined,
        source_label:
          typeof job.source_label === "string" ? job.source_label : undefined,
        source_type:
          typeof job.source_type === "string" ? job.source_type : undefined,
        status: typeof job.status === "string" ? job.status : undefined,
        result: isRecord(job.result) ? job.result : undefined,
      }))
    : [];
  const sources = Array.isArray(value.sources)
    ? value.sources.filter(isRecord).map((source) => ({
        source_key:
          typeof source.source_key === "string" ? source.source_key : "",
        source_type:
          typeof source.source_type === "string" ? source.source_type : "",
        source_label:
          typeof source.source_label === "string" ? source.source_label : "",
        status: typeof source.status === "string" ? source.status : "",
        blocking_reports: source.blocking_reports === true,
      }))
    : [];
  const summary = isRecord(value.summary)
    ? {
        failed:
          typeof value.summary.failed === "number"
            ? value.summary.failed
            : undefined,
        blocking_reports:
          typeof value.summary.blocking_reports === "number"
            ? value.summary.blocking_reports
            : undefined,
        rate_limited:
          typeof value.summary.rate_limited === "number"
            ? value.summary.rate_limited
            : undefined,
      }
    : undefined;
  return { completed, sources, summary };
}

export function classifyDaemonFreshnessEvent(
  value: unknown,
): DaemonFreshnessSignal | null {
  if (!isRecord(value) || value.event !== true || "request_id" in value) {
    return null;
  }
  if (
    typeof value.kind !== "string" ||
    typeof value.schema_version !== "number"
  ) {
    return null;
  }

  if (value.kind === "ui.freshness.background") {
    const data = freshnessRunData(value.data);
    if (!data) return null;
    return freshnessRunNeedsAttention(data) ? "needs-attention" : "refresh";
  }

  if (value.kind === "ui.freshness.worker" && isRecord(value.data)) {
    return ["error", "unavailable"].includes(String(value.data.status ?? ""))
      ? "worker-error"
      : null;
  }

  return null;
}
