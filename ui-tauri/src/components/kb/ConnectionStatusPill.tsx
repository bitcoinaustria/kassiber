import { useTranslation } from "react-i18next";

import { connectionStatusStyles } from "@/lib/connectionDisplay";
import { cn } from "@/lib/utils";
import type { ConnectionStatus } from "@/mocks/seed";

interface ConnectionStatusPillProps {
  status: ConnectionStatus;
  /**
   * When true (the default), `synced` collapses to `null` so the pill only
   * draws the eye for states that need user attention.
   */
  hideWhenSynced?: boolean;
  className?: string;
}

export function ConnectionStatusPill({
  status,
  hideWhenSynced = true,
  className,
}: ConnectionStatusPillProps) {
  const { t } = useTranslation("chrome");
  if (hideWhenSynced && status === "synced") return null;
  return (
    <span
      className={cn(
        "inline-flex w-fit items-center rounded-md px-1.5 py-0.5 text-[10px] font-medium ring-1 ring-inset",
        connectionStatusStyles[status],
        className,
      )}
    >
      {t(`connectionStatus.${status}`)}
    </span>
  );
}
