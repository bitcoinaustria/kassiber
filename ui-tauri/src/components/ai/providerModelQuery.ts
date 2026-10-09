/**
 * The one query behind **Check models**, shared by the model picker and
 * Settings → AI providers so a check in either place fills both.
 *
 * Privacy contract: the query is always created disabled. Mounting a surface
 * never runs it; only an explicit `refetch()` from a user's click does, and it
 * asks only that one provider (docs/reference/ai.md).
 */

import { DaemonRequestError, daemonQueryKey } from "@/daemon/client";
import { getTransport, type DaemonEnvelope } from "@/daemon/transport";
import type { AiModelsListData } from "@/lib/aiCapabilities";

export async function fetchProviderModels(
  provider: string,
): Promise<DaemonEnvelope<AiModelsListData>> {
  const envelope = await getTransport().invoke<AiModelsListData>({
    kind: "ai.list_models",
    args: { provider, refresh: true },
  });
  if (envelope.kind === "error" || envelope.error) {
    throw new DaemonRequestError("ai.list_models", envelope);
  }
  return envelope;
}

export function providerModelsQueryKey(
  dataMode: string,
  daemonSession: number,
  provider: string,
) {
  return daemonQueryKey(dataMode, daemonSession, "ai.list_models", { provider });
}

export function providerModelsQueryOptions(
  dataMode: string,
  daemonSession: number,
  provider: string,
) {
  return {
    queryKey: providerModelsQueryKey(dataMode, daemonSession, provider),
    queryFn: () => fetchProviderModels(provider),
    enabled: false,
    // One click, one probe: a failed check reports its error instead of
    // quietly asking the provider again.
    retry: false,
    refetchOnMount: false,
    staleTime: 5 * 60 * 1000,
    gcTime: 60 * 60 * 1000,
    meta: { shellProgress: false },
  } as const;
}
