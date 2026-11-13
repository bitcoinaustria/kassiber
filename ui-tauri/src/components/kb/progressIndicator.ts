import type { JobEntry } from "@/store/notifications";
import type { ActiveMaintenanceProgress } from "@/store/ui";

export interface RouteProgressState {
  indeterminate: boolean;
  label: string;
  value?: number;
}

export function daemonQueryUsesShellProgress(query: {
  state: { data: unknown };
  meta?: Record<string, unknown>;
}) {
  return query.state.data === undefined && query.meta?.shellProgress !== false;
}

function compactProgressTitle(title: string) {
  return title
    .replace(/\s+(started|running)$/i, "")
    .replace(/^BTC price refresh$/i, "BTC price")
    .replace(/^Journal processing$/i, "Journals")
    .trim();
}

/**
 * The shell's progress line for the newest running job in the notification
 * center. `describe` gives the job's title and phase as the center shows
 * them in the current language and privacy mode.
 */
export function routeProgressFromJobs(
  jobs: readonly JobEntry[],
  describe: (job: JobEntry) => { title: string; label?: string },
): RouteProgressState | null {
  const job = [...jobs].sort((a, b) => b.startedAt.localeCompare(a.startedAt))[0];
  if (!job) return null;
  const { title, label } = describe(job);
  const displayLabel = (() => {
    const phase = label?.trim();
    if (!phase) return title || "In progress";
    if (!title) return phase;
    if (title.toLowerCase() === phase.toLowerCase()) return title;
    return `${title}: ${phase}`;
  })();

  return {
    indeterminate: Boolean(job.progress.indeterminate),
    label: displayLabel,
    value: job.progress.value,
  };
}

export function routeProgressFromActiveMaintenance(
  progress: ActiveMaintenanceProgress | null,
): RouteProgressState | null {
  if (progress?.state !== "running") return null;
  const title = compactProgressTitle(progress.title);
  const label = progress.progress.label?.trim();
  const detail = label || progress.body.trim() || "In progress";
  const displayLabel =
    title && title.toLowerCase() !== detail.toLowerCase()
      ? `${title}: ${detail}`
      : detail || title || "In progress";

  return {
    indeterminate: Boolean(progress.progress.indeterminate),
    label: displayLabel,
    value: progress.progress.value,
  };
}
