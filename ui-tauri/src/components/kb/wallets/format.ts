import type { ConnectionStatus } from "@/mocks/seed";

export const hiddenSensitiveClassName = (hidden: boolean) =>
  hidden ? "sensitive" : "";

export const formatBtc = (value: number) => value.toFixed(8);

export const formatEur = (value: number) =>
  "€ " +
  value.toLocaleString("de-AT", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });

export const statusDotStyles: Record<ConnectionStatus, string> = {
  synced: "bg-emerald-500",
  syncing: "bg-amber-500",
  idle: "bg-muted-foreground/50",
  error: "bg-red-500",
};
