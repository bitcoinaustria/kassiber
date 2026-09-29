import type { DataMode } from "@/store/ui";

export function dataModeForActiveBackend(
  dataMode: DataMode,
  activeRegtestBackend: boolean,
): DataMode {
  if (activeRegtestBackend && dataMode === "real") return "regtest";
  if (!activeRegtestBackend && dataMode === "regtest") return "real";
  return dataMode;
}
