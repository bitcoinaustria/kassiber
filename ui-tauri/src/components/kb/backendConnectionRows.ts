import {
  backendProtocolLabel,
  type Backend,
} from "@/components/kb/settings/SettingsModel";
import { endpointWithPort, settingsHashForConnection } from "@/lib/connectionHealth";
import type { Connection } from "@/mocks/seed";

const REGTEST_NETWORKS = new Set(["regtest", "elementsregtest"]);

export function isRegtestBackend(
  backend: Pick<Backend, "network">,
): boolean {
  return REGTEST_NETWORKS.has(String(backend.network ?? "").toLowerCase());
}

export function visibleConnectionBackends(savedBackends: Backend[]): Backend[] {
  const hasRegtestBackends = savedBackends.some(isRegtestBackend);
  if (!hasRegtestBackends) return savedBackends;

  return savedBackends.filter(
    (backend) =>
      isRegtestBackend(backend) ||
      backend.isDefault ||
      (backend.walletRefs?.length ?? 0) > 0,
  );
}

export function regtestBackendConnections(backends: Backend[]): Connection[] {
  return visibleConnectionBackends(backends)
    .filter(isRegtestBackend)
    .map((backend) => backendConnectionFromSettingsBackend(backend));
}

export function backendConnectionFromSettingsBackend(
  backend: Backend,
): Connection {
  const protocol = backendProtocolLabel(backend);
  return {
    id: `backend:${backend.id}`,
    role: "backend",
    kind: "backend",
    chain: backend.chain ?? (backend.net === "LIQUID" ? "liquid" : "bitcoin"),
    network: backend.network ?? null,
    label: backend.name,
    last: backend.isDefault ? "Default" : "Configured",
    balance: 0,
    status: backend.on ? "idle" : "error",
    syncMode: "backend",
    syncSource: protocol,
    sourceFormat: backend.kind,
    transactionCount: 0,
    backendId: backend.id,
    backendKind: backend.kind ?? null,
    endpoint: endpointWithPort(backend.url),
    isDefaultBackend: backend.isDefault === true,
    settingsHash: settingsHashForConnection(backend),
    walletRefs: backend.walletRefs ?? [],
  };
}
