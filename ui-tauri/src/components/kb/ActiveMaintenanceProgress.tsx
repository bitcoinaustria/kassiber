import { RefreshCw } from "lucide-react";

import { cn } from "@/lib/utils";
import { useUiStore, type ActiveMaintenanceProgress } from "@/store/ui";

function progressValue(value: number | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, value));
}

function ProgressTrack({
  progress,
  className,
}: {
  progress: ActiveMaintenanceProgress["progress"];
  className?: string;
}) {
  const indeterminate = Boolean(progress.indeterminate);
  return (
    <div
      role="progressbar"
      aria-label={progress.label ?? "Maintenance progress"}
      aria-valuemin={indeterminate ? undefined : 0}
      aria-valuemax={indeterminate ? undefined : 100}
      aria-valuenow={indeterminate ? undefined : progressValue(progress.value)}
      className={cn("h-2 overflow-hidden rounded-full bg-muted", className)}
    >
      <div
        className={cn(
          "h-full rounded-full bg-primary transition-[width] duration-300",
          indeterminate &&
            "w-1/2 will-change-transform motion-safe:animate-[route-progress_0.9s_ease-in-out_infinite] motion-reduce:w-full motion-reduce:will-change-auto",
        )}
        style={
          indeterminate
            ? undefined
            : { width: `${progressValue(progress.value)}%` }
        }
      />
    </div>
  );
}

export function ActiveMaintenanceProgressCard({
  progress,
  className,
}: {
  progress: ActiveMaintenanceProgress;
  className?: string;
}) {
  return (
    <section
      className={cn("bg-primary/5 p-3 sm:p-4", className)}
      role="status"
      aria-live="polite"
    >
      <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
        <div className="min-w-0 space-y-1">
          <div className="flex min-w-0 items-center gap-2">
            <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary">
              <RefreshCw
                className="size-3.5 motion-safe:animate-spin"
                aria-hidden="true"
              />
            </span>
            <h2 className="truncate text-sm font-semibold text-foreground">
              {progress.title}
            </h2>
          </div>
          <p className="text-sm text-muted-foreground">{progress.body}</p>
        </div>
        <div className="min-w-0 lg:w-[min(34rem,46vw)]">
          <ProgressTrack progress={progress.progress} />
          {progress.progress.label ? (
            <p className="mt-1 text-xs text-muted-foreground">
              {progress.progress.label}
            </p>
          ) : null}
        </div>
      </div>
      {progress.details?.length ? (
        <div className="mt-3 flex flex-wrap gap-1.5">
          {progress.details.slice(0, 4).map((detail) => (
            <span
              key={detail}
              className="rounded-md border bg-background/80 px-2 py-1 text-xs font-medium text-muted-foreground"
            >
              {detail}
            </span>
          ))}
        </div>
      ) : null}
    </section>
  );
}

export function ActiveMaintenanceProgressStrip({
  className,
}: {
  className?: string;
}) {
  const progress = useUiStore((state) => state.activeMaintenanceProgress);
  if (!progress?.active) return null;

  return (
    <div
      className={cn(
        "sticky top-px z-20 border-b bg-background/95 px-3 py-2 shadow-sm backdrop-blur",
        className,
      )}
      role="status"
      aria-live="polite"
    >
      <div className="mx-auto flex max-w-screen-2xl flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex min-w-0 items-center gap-2">
          <RefreshCw
            className="size-3.5 shrink-0 text-primary motion-safe:animate-spin"
            aria-hidden="true"
          />
          <span className="truncate text-sm font-medium">
            {progress.title}
          </span>
          <span className="hidden truncate text-sm text-muted-foreground md:inline">
            {progress.body}
          </span>
        </div>
        <div className="min-w-0 sm:w-64">
          <ProgressTrack progress={progress.progress} className="h-1.5" />
        </div>
      </div>
    </div>
  );
}
