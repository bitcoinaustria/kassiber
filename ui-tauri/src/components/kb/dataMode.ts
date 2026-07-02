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
  // ON is the daemon-backed side (regtest books stay regtest); OFF always
  // reaches the mock preview so the switch never becomes a dead control.
  if (checked) return activeRegtestBackend ? "regtest" : "real";
  return "mock";
}
