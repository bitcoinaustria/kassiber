import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import type { AnalysisJob } from "./useAnalysisJob";

export function JobProgress({ job, pollError, onResume, onCancel }: { job: AnalysisJob<unknown>; pollError: boolean; onResume: () => void; onCancel: () => void }) {
  const { t } = useTranslation("chainAnalysis");
  return <div className="space-y-3 rounded-lg border bg-muted/15 p-3">
    <div className="flex flex-wrap items-center justify-between gap-3">
      <p className="text-xs" role="status">
        {job.status} · {job.progress.phase}
      </p>
      {job.status === "running" && <Button
        type="button"
        size="sm"
        variant="outline"
        disabled={job.cancel_requested}
        onClick={onCancel}>
        {t(job.cancel_requested ? "workbench.cancelling" : "workbench.cancel")}
      </Button>}
    </div>
    <div className="flex flex-wrap gap-4 font-mono text-xs">
      {job.progress.row_count != null && <span>
        {t("datasets.progressRows", { count: job.progress.row_count })}
      </span>}
      {job.progress.byte_count != null && <span>
        {t("datasets.progressBytes", { count: job.progress.byte_count })}
      </span>}
      {job.progress.rows_deleted != null && <span>
        {t("datasets.progressDeleted", { count: job.progress.rows_deleted })}
      </span>}
      <span>
        {t("workbench.elapsed", { ms: job.progress.elapsed_ms ?? job.elapsed_ms })}
      </span>
    </div>
    {pollError && <Button
      type="button"
      variant="outline"
      size="sm"
      onClick={onResume}>
      {t("workbench.resume")}
    </Button>}
    {job.error_code && <p role="alert" className="text-xs text-destructive">
      {job.error_code}
    </p>}
  </div>;
}
