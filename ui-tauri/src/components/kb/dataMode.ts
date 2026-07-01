import type { DataMode } from "@/store/ui";

export type DataModeLabelKey = "preview" | "real" | "regtest";

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
  if (dataMode === "real") return "real";
  return "preview";
}

export function dataModeFromSourceSwitch(
  checked: boolean,
  activeRegtestBackend: boolean,
): DataMode {
  if (checked) return "real";
  return activeRegtestBackend ? "regtest" : "mock";
}
