import type { DataMode } from "@/store/ui";

export type DataModeLabelKey = "real" | "regtest";

export function dataModeForActiveBackend(
  dataMode: DataMode,
  activeRegtestBackend: boolean,
): DataMode {
  if (activeRegtestBackend && dataMode === "real") return "regtest";
  if (!activeRegtestBackend && dataMode === "regtest") return "real";
  return dataMode;
}

export function dataModeLabelKey(dataMode: DataMode): DataModeLabelKey {
  if (dataMode === "regtest") return "regtest";
  return "real";
}
