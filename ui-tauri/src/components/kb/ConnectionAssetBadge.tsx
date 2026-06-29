import { statusDotStyles } from "@/components/kb/wallets/format";
import bitcoinIcon from "@/assets/integrations/bitcoin.svg";
import liquidIcon from "@/assets/integrations/liquid.svg";
import {
  connectionAssetIconKind,
  connectionAssetLabel,
  type ConnectionAssetInput,
} from "@/lib/connectionDisplay";
import { cn } from "@/lib/utils";
import type { ConnectionStatus } from "@/mocks/seed";

interface ConnectionAssetBadgeProps {
  connection: ConnectionAssetInput;
  /** When set, overlays a small status dot in the bottom-right corner. */
  status?: ConnectionStatus;
  /** `sm` = list rows, `md` = detail headers. */
  size?: "sm" | "md";
  className?: string;
}

export function ConnectionAssetBadge({
  connection,
  status,
  size = "sm",
  className,
}: ConnectionAssetBadgeProps) {
  const asset = connectionAssetLabel(connection);
  const iconKind = connectionAssetIconKind(asset);
  const icon = iconKind === "liquid" ? liquidIcon : bitcoinIcon;

  return (
    <span
      className={cn(
        "relative flex shrink-0 items-center justify-center rounded-md border border-border/70 bg-muted/60 shadow-sm shadow-zinc-950/5 dark:border-border/80 dark:bg-muted/55 dark:shadow-black/20",
        iconKind === "liquid"
          ? size === "md"
            ? "size-9 p-0"
            : "size-8 p-0"
          : size === "md"
            ? "size-9 p-0.5"
            : "size-8 p-0.5",
        className,
      )}
      data-asset={asset}
      aria-hidden="true"
    >
      <img
        src={icon}
        alt=""
        className="max-h-full max-w-full object-contain scale-115"
      />
      {status ? (
        <span
          className={cn(
            "absolute -right-0.5 -bottom-0.5 size-2.5 rounded-full ring-2 ring-card",
            statusDotStyles[status],
          )}
        />
      ) : null}
    </span>
  );
}
