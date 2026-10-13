/**
 * The one query behind **Check models**, shared by the model picker and
 * Settings → AI providers so a check in either place fills both.
 *
 * Privacy contract: the query is always created disabled. Mounting a surface
 * never runs it; only an explicit `refetch()` from a user's click does, and it
 * asks only that one provider (docs/reference/ai.md).
 *
 * A result belongs to the endpoint that was checked: the key carries the
 * provider's `base_url` as well as its name, so editing the URL leaves the
 * provider "not checked" instead of showing the old endpoint's models, and
 * nothing probes the new endpoint until the user checks it.
 */

import { useQuery } from "@tanstack/react-query";

import { DaemonRequestError, daemonQueryKey } from "@/daemon/client";
import { getTransport, type DaemonEnvelope } from "@/daemon/transport";
import type { AiModelsListData, AiProviderRow } from "@/lib/aiCapabilities";
import { useUiStore } from "@/store/ui";

/**
 * Lists `provider`'s models now. The request names the endpoint being
 * checked, so if the row was edited meanwhile (in Settings or from the CLI)
 * the daemon refuses instead of contacting an endpoint nobody checked.
 */
export async function fetchProviderModels(
  provider: ProviderModelsTarget,
): Promise<DaemonEnvelope<AiModelsListData>> {
  const envelope = await getTransport().invoke<AiModelsListData>({
    kind: "ai.list_models",
    args: { provider: provider.name, base_url: provider.base_url, refresh: true },
  });
  if (envelope.kind === "error" || envelope.error) {
    throw new DaemonRequestError("ai.list_models", envelope);
  }
  return envelope;
}

/** What a model check is keyed by: the provider and the endpoint it checked. */
export type ProviderModelsTarget = Pick<AiProviderRow, "name" | "base_url">;

export function providerModelsQueryKey(
  dataMode: string,
  daemonSession: number,
  provider: ProviderModelsTarget,
) {
  return daemonQueryKey(dataMode, daemonSession, "ai.list_models", {
    provider: provider.name,
    base_url: provider.base_url,
  });
}

export function providerModelsQueryOptions(
  dataMode: string,
  daemonSession: number,
  provider: ProviderModelsTarget,
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

const NO_PROVIDER: ProviderModelsTarget = { name: "", base_url: "" };

/**
 * The shared check query for `provider`, read without ever running it: its
 * data is what the last **Check models** of this endpoint found.
 */
export function useCachedProviderModels(provider: ProviderModelsTarget | null | undefined) {
  const dataMode = useUiStore((state) => state.dataMode);
  const daemonSession = useUiStore((state) => state.daemonSession);
  return useQuery(
    providerModelsQueryOptions(dataMode, daemonSession, provider ?? NO_PROVIDER),
  );
}

/** The models from {@link useCachedProviderModels}; null until checked. */
export function useProviderModelsSnapshot(
  provider: ProviderModelsTarget | null | undefined,
): AiModelsListData | null {
  const query = useCachedProviderModels(provider);
  if (!provider) return null;
  return query.data?.kind === "ai.list_models" ? (query.data.data ?? null) : null;
}
