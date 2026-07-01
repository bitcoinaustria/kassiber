import type { DeferredConnectionSetup } from "@/store/ui";
import type { Backend } from "./SettingsModel";

export type SyncBackendTypeId = "bitcoin" | "liquid" | "coreln" | "lnd";

const BUILT_IN_LIQUID_BACKEND_IDS = new Set(["liquid", "liquid-blockstream"]);

export function normalizedBackendKind(kind: string | null | undefined): string {
  return (kind ?? "").toLowerCase().replace(/-/g, "");
}

export function backendTypeIdForConnectionSetup(
  intent: DeferredConnectionSetup | null,
): SyncBackendTypeId | undefined {
  const kind = normalizedBackendKind(intent?.backendKind);
  const sourceId = intent?.sourceId?.trim().toLowerCase();
  if (kind === "liquid" || kind === "liquidesplora" || sourceId === "liquid") {
    return "liquid";
  }
  if (
    kind === "bitcoin" ||
    kind === "esplora" ||
    kind === "electrum" ||
    sourceId === "bitcoin" ||
    sourceId === "esplora" ||
    sourceId === "electrum"
  ) {
    return "bitcoin";
  }
  if (kind === "coreln") return "coreln";
  if (kind === "lnd") return "lnd";
  if (sourceId === "core-ln") return "coreln";
  if (sourceId === "lnd") return "lnd";
  return undefined;
}

export function backendTypeIdForSettingsBackend(
  backend: Pick<Backend, "id" | "kind" | "net">,
): SyncBackendTypeId {
  const kind = normalizedBackendKind(backend.kind);
  if (kind === "coreln") return "coreln";
  if (kind === "lnd") return "lnd";

  const backendId = backend.id.trim().toLowerCase();
  if (
    BUILT_IN_LIQUID_BACKEND_IDS.has(backendId) &&
    (!kind ||
      kind === "electrum" ||
      kind === "esplora" ||
      kind === "liquidesplora")
  ) {
    return "liquid";
  }

  if (backend.net === "LIQUID") return "liquid";
  if (backend.net === "BTC") return "bitcoin";

  if (kind === "liquidesplora") return "liquid";
  return "bitcoin";
}
