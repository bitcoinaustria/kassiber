import { useContext, useEffect, useState } from "react";
import { DaemonScopeContext, useDaemonMutation } from "@/daemon/client";

export interface AnalysisJob<T> {
  job_id: string; status: "running" | "completed" | "cancelled" | "failed"; cancel_requested: boolean;
  request: Record<string, unknown>; result: T | null; error_code?: string | null; elapsed_ms: number;
  progress: { phase: string; states_explored?: number; row_count?: number; byte_count?: number; rows_deleted?: number; elapsed_ms?: number; interpretation_count_lower_bound?: string };
}
/** One book-scoped poll at a time; unmounts and book changes discard late reads. */
export function useAnalysisJob<T>(kind: string, onError: (error: unknown) => void) {
    const boundary = useContext(DaemonScopeContext);
    const [job, setJob] = useState<AnalysisJob<T> | null>(null);
    const [pollError, setPollError] = useState(false);
    const start = useDaemonMutation<AnalysisJob<T>>(kind, { invalidateQueries: false });
    const { mutateAsync: get } = useDaemonMutation<AnalysisJob<T>>("ui.chain_analysis.jobs.get", { invalidateQueries: false });
    const cancel = useDaemonMutation<AnalysisJob<T>>("ui.chain_analysis.jobs.cancel", { invalidateQueries: false });
    useEffect(() => {
        if (!job || job.status !== "running" || pollError)
            return;
        let live = true;
        const timer = window.setTimeout(() => {
            void get({ job_id: job.job_id }).then(response => {
                if (live && boundary?.isCurrent?.() !== false && response.data)
                    setJob(response.data);
            }).catch(error => {
                if (live) {
                    setPollError(true);
                    onError(error);
                }
            });
        }, 500);
        return () => {
            live = false;
            window.clearTimeout(timer);
        };
    }, [job, pollError, get, boundary, onError]);
    return {
        job,
        pollError,
        busy: start.isPending || job?.status === "running",
        resume: () => setPollError(false),
        async run(args: Record<string, unknown>) {
            setPollError(false);
            setJob(null);
            const response = await start.mutateAsync(args);
            if (boundary?.isCurrent?.() !== false && response.data)
                setJob(response.data);
        },
        async cancel() {
            if (!job)
                return;
            const response = await cancel.mutateAsync({ job_id: job.job_id });
            if (boundary?.isCurrent?.() !== false && response.data)
                setJob(response.data);
        },
    };
}
