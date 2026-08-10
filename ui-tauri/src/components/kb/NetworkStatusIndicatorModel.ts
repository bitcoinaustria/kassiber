import type { ConnectionProbeKind } from "@/lib/connectionHealth";

import {
  connectionProbeKind,
  endpointWithPort,
  settingsHashForConnection,
} from "@/lib/connectionHealth";
import {
  backendProtocolLabel,
  type Backend,
} from "./settings/SettingsModel";

export type ConnectionHealthRow = {
  id: string;
  backendId?: string;
  name: string;
  endpoint: string;
  fingerprint: string;
  rawUrl: string;
  protocol: string;
  probeKind: ConnectionProbeKind;
  settingsHash: string;
  proxy?: string;
  trustSelfSigned?: boolean;
  certificate?: string;
};

export function connectionRowFromBackend(
  backend: Backend,
  id: string,
  backendId?: string,
): ConnectionHealthRow {
  return {
    id,
    backendId,
    name: backend.name,
    endpoint: endpointWithPort(backend.url),
    fingerprint: [
      backend.url,
      backend.proxy ? `${backend.proxy.host}:${backend.proxy.port}` : "",
      backend.trustSsl ? "trust-self-signed" : "",
      backend.certificate ?? "",
      backend.kind,
    ].join("|"),
    rawUrl: backend.url,
    protocol: backendProtocolLabel(backend),
    probeKind: connectionProbeKind({
      ...backend,
      allowDisplayHttpProbe:
        backendId === undefined || backend.urlSafeForHttpProbe === true,
    }),
    settingsHash: settingsHashForConnection(backend),
    proxy: backend.proxy
      ? `${backend.proxy.host}:${backend.proxy.port}`
      : undefined,
    trustSelfSigned: Boolean(backend.trustSsl),
    certificate: backend.certificate,
  };
}

export function connectionProbeRequest(
  row: ConnectionHealthRow,
): { kind: ConnectionProbeKind; args: Record<string, unknown> } {
  if (row.probeKind === "electrum") {
    return {
      kind: row.probeKind,
      args: {
        url: row.rawUrl,
        trust_self_signed: row.trustSelfSigned,
        certificate: row.certificate,
        proxy: row.proxy,
        timeout: 5,
      },
    };
  }
  if (row.probeKind === "bitcoinrpc") {
    return {
      kind: row.probeKind,
      args: {
        backend: row.backendId,
        url: row.backendId ? undefined : row.rawUrl,
        proxy: row.proxy,
        timeout: 5,
        config: {
          insecure: row.trustSelfSigned,
          ...(row.certificate ? { certificate: row.certificate } : {}),
        },
      },
    };
  }
  if (row.probeKind === "lightning" || row.probeKind === "btcpay") {
    return {
      kind: row.probeKind,
      args: {
        backend: row.backendId,
        timeout: 5,
      },
    };
  }
  return {
    kind: row.probeKind,
    args: {
      url: row.rawUrl,
      proxy: row.proxy,
      insecure: row.trustSelfSigned,
      certificate: row.certificate,
      timeout: 5,
    },
  };
}
