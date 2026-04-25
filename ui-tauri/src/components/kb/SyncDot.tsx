import { cn } from "@/lib/utils";
import type { ConnectionStatus } from "@/mocks/seed";

interface SyncDotProps {
  status: ConnectionStatus;
  className?: string;
}

const CONFIG: Record<
  ConnectionStatus,
  { color: string; pulse: boolean; title: string }
> = {
  synced: { color: "bg-[#3fa66a]", pulse: false, title: "In sync" },
  syncing: { color: "bg-[#c9a43a]", pulse: true, title: "Syncing" },
  idle: { color: "bg-ink-3", pulse: false, title: "Idle" },
  error: { color: "bg-accent", pulse: false, title: "Needs attention" },
};

export function SyncDot({ status, className }: SyncDotProps) {
  const cfg = CONFIG[status];
  return (
    <span
      title={cfg.title}
      className={cn(
        "inline-block size-1.5 shrink-0 rounded-full",
        cfg.color,
        cfg.pulse && "animate-pulse",
        className,
      )}
    />
  );
}
