import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";
import type { ConnectionStatus } from "@/mocks/seed";

interface SyncDotProps {
  status: ConnectionStatus;
  className?: string;
}

// `titleKey` indexes the `chrome:syncDot.*` namespace; keep the keys in sync
// with the ConnectionStatus union rather than deriving labels from the status.
const CONFIG: Record<
  ConnectionStatus,
  { color: string; pulse: boolean; titleKey: string }
> = {
  synced: { color: "bg-[#3fa66a]", pulse: false, titleKey: "synced" },
  syncing: { color: "bg-[#c9a43a]", pulse: true, titleKey: "syncing" },
  idle: { color: "bg-ink-3", pulse: false, titleKey: "idle" },
  error: { color: "bg-accent", pulse: false, titleKey: "error" },
};

export function SyncDot({ status, className }: SyncDotProps) {
  const { t } = useTranslation("chrome");
  const cfg = CONFIG[status];
  return (
    <span
      title={t(`syncDot.${cfg.titleKey}` as never) /* dynamic key */}
      className={cn(
        "inline-block size-1.5 shrink-0 rounded-full",
        cfg.color,
        cfg.pulse && "animate-pulse",
        className,
      )}
    />
  );
}
