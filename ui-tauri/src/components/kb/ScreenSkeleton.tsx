import { Skeleton } from "@/components/ui/skeleton";
import {
  pageHeaderActionsClassName,
  pageHeaderClassName,
  screenShellClassName,
} from "@/lib/screen-layout";
import { cn } from "@/lib/utils";

interface ScreenSkeletonProps {
  className?: string;
  titleWidth?: string;
  metricCount?: number;
}

interface ScreenNoticeProps {
  className?: string;
  title: string;
  body: string;
}

export function ScreenSkeleton({
  className,
  titleWidth = "w-40",
  metricCount = 4,
}: ScreenSkeletonProps) {
  return (
    <div className={cn(screenShellClassName, className)} aria-busy="true">
      <div className={pageHeaderClassName}>
        <div className="min-w-0 space-y-2">
          <Skeleton className="h-3 w-28" />
          <Skeleton className={cn("h-5", titleWidth)} />
        </div>
        <div className={pageHeaderActionsClassName}>
          <Skeleton className="h-8 w-24 rounded-md" />
          <Skeleton className="h-8 w-28 rounded-md" />
        </div>
      </div>

      <div className="grid grid-cols-1 overflow-hidden rounded-lg border bg-card sm:grid-cols-2 xl:grid-cols-4">
        {Array.from({ length: metricCount }).map((_, index) => (
          <div
            key={index}
            className="space-y-2 border-b border-r p-3 last:border-r-0 sm:p-4 xl:border-b-0"
          >
            <Skeleton className="h-3 w-24" />
            <Skeleton className="h-6 w-20" />
            <Skeleton className="h-3 w-32" />
          </div>
        ))}
      </div>

      <div className="grid grid-cols-1 items-start gap-3 2xl:grid-cols-[minmax(0,1fr)_380px]">
        <div className="rounded-lg border bg-card">
          <div className="space-y-2 border-b px-3 py-3 sm:px-4">
            <Skeleton className="h-4 w-36" />
            <Skeleton className="h-3 w-56 max-w-full" />
          </div>
          <div className="divide-y">
            {Array.from({ length: 8 }).map((_, index) => (
              <div
                key={index}
                className="grid grid-cols-[minmax(0,1fr)_96px] gap-3 px-3 py-3 sm:px-4"
              >
                <div className="min-w-0 space-y-2">
                  <Skeleton className="h-4 w-44 max-w-full" />
                  <Skeleton className="h-3 w-64 max-w-full" />
                </div>
                <div className="space-y-2">
                  <Skeleton className="ml-auto h-4 w-20" />
                  <Skeleton className="ml-auto h-3 w-14" />
                </div>
              </div>
            ))}
          </div>
        </div>
        <div className="grid min-w-0 gap-3">
          <div className="rounded-lg border bg-card p-3 sm:p-4">
            <Skeleton className="h-4 w-36" />
            <div className="mt-4 space-y-3">
              <Skeleton className="h-9 w-full" />
              <Skeleton className="h-9 w-full" />
              <Skeleton className="h-9 w-full" />
            </div>
          </div>
          <div className="rounded-lg border bg-card p-3 sm:p-4">
            <Skeleton className="h-4 w-32" />
            <div className="mt-4 space-y-2">
              <Skeleton className="h-3 w-full" />
              <Skeleton className="h-3 w-5/6" />
              <Skeleton className="h-3 w-2/3" />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

export function ScreenNotice({ className, title, body }: ScreenNoticeProps) {
  return (
    <div className={cn(screenShellClassName, className)}>
      <div className="rounded-lg border bg-card px-3 py-3 sm:px-4">
        <p className="text-sm font-semibold text-foreground">{title}</p>
        <p className="mt-1 max-w-2xl text-sm text-muted-foreground">{body}</p>
      </div>
    </div>
  );
}
