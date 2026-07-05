import { statusDotStyles } from "@/components/kb/wallets/format";
import bitcoinIcon from "@/assets/integrations/bitcoin.svg";
import coreLightningIcon from "@/assets/integrations/core-lightning.svg";
import lightningIcon from "@/assets/integrations/lightning.svg";
import lightningLabsIcon from "@/assets/integrations/lightning-labs.svg";
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
  const isCoreLightning = connection.kind === "core-ln";
  const isLnd = connection.kind === "lnd";
  const isLightning =
    isCoreLightning ||
    isLnd ||
    connection.kind === "nwc" ||
    connection.kind === "phoenix";
  const icon = isCoreLightning
    ? coreLightningIcon
    : isLnd
      ? lightningLabsIcon
      : isLightning
        ? lightningIcon
        : iconKind === "liquid"
          ? liquidIcon
          : bitcoinIcon;
  const lightningBackground = isCoreLightning
    ? "border-zinc-950/80 bg-zinc-950 dark:border-zinc-700 dark:bg-zinc-950"
    : isLnd
      ? "border-[#6f4cff]/30 bg-[#6f4cff]/10 dark:border-[#8b72ff]/40 dark:bg-[#6f4cff]/20"
      : isLightning
        ? "border-[#792EE5]/25 bg-[#792EE5]/10 dark:border-[#9f6af0]/35 dark:bg-[#792EE5]/20"
        : null;

  return (
    <span
      className={cn(
        "relative flex shrink-0 items-center justify-center rounded-md border border-border/70 bg-muted/60 shadow-sm shadow-zinc-950/5 dark:border-border/80 dark:bg-muted/55 dark:shadow-black/20",
        lightningBackground,
        iconKind === "liquid" || isLightning
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
        className={cn(
          "max-h-full max-w-full object-contain",
          isCoreLightning ? "scale-150" : "scale-115",
        )}
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
