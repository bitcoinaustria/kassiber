/**
 * The one query behind **Check models**, shared by the model picker and
 * Settings → Assistant so a check in either place fills both.
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

import * as React from "react";
import { useQueries, useQuery, useQueryClient } from "@tanstack/react-query";

import { DaemonRequestError, daemonQueryKey } from "@/daemon/client";
import { getTransport, type DaemonEnvelope } from "@/daemon/transport";
import {
  isNativeAiProviderLocator,
  type AiModelsListData,
  type AiProviderRow,
  type AiProviderRuntimeStatusData,
} from "@/lib/aiCapabilities";
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
    // The last checked inventory stays available for the entire app session.
    gcTime: Infinity,
    meta: { shellProgress: false },
  } as const;
}

/**
 * Restore the daemon's saved checks after launch. These are local display
 * reads, never discovery: only fetchProviderModels may contact a provider.
 * Both kinds matter because Settings' CLI check saves a runtime snapshot,
 * while Check models saves a model listing. The newer check wins.
 */
export function useRestoreProviderModelChecks(
  providers: readonly (ProviderModelsTarget & Pick<AiProviderRow, "last_checked_at">)[],
  enabled: boolean,
): void {
  const client = useQueryClient();
  const dataMode = useUiStore((state) => state.dataMode);
  const daemonSession = useUiStore((state) => state.daemonSession);
  const savedQuery = <T,>(kind: string, provider: ProviderModelsTarget, active: boolean) => {
    const args = { provider: provider.name, base_url: provider.base_url, cached_only: true };
    return {
      queryKey: daemonQueryKey(dataMode, daemonSession, kind, args),
      queryFn: async () => {
        const envelope = await getTransport().invoke<T>({ kind, args });
        if (envelope.kind !== kind || envelope.error) throw new DaemonRequestError(kind, envelope);
        return envelope;
      },
      enabled: enabled && active,
      retry: false,
      staleTime: Infinity,
      gcTime: Infinity,
      refetchOnWindowFocus: false,
      meta: { shellProgress: false },
    } as const;
  };
  const listings = useQueries({ queries: providers.map((provider) =>
    savedQuery<AiModelsListData>("ai.list_models", provider, Boolean(provider.last_checked_at))) });
  const runtimes = useQueries({ queries: providers.map((provider) =>
    savedQuery<AiProviderRuntimeStatusData>("ai.provider_runtime.status", provider,
      Boolean(provider.last_checked_at) && isNativeAiProviderLocator(provider.base_url))) });

  React.useEffect(() => {
    if (!enabled) return;
    providers.forEach((provider, index) => {
      if (!provider.last_checked_at) return;
      // Resolve both saved sources before seeding a CLI inventory. Otherwise
      // the first response could mask a runtime result with the same timestamp.
      if (listings[index]?.isPending ||
        (isNativeAiProviderLocator(provider.base_url) && runtimes[index]?.isPending)) return;
      const listing = listings[index]?.data?.data;
      const runtime = runtimes[index]?.data?.data;
      const at = (value: string | null | undefined) => value ? Date.parse(value) || 0 : 0;
      let snapshot: AiModelsListData | undefined = listing;
      if (runtime && at(runtime.checked_at) >= at(listing?.checked_at)) {
        const status = runtime.providers?.[0];
        snapshot = {
          provider: provider.name,
          models: status?.state === "ready" ? status.models ?? [] : [],
          checked_at: runtime.checked_at,
          stale: runtime.stale,
          error: runtime.error,
        };
      }
      // An unchecked endpoint contributes nothing, and a delayed display
      // read must not overwrite a newer explicit check (or its error).
      if (!snapshot || !at(snapshot.checked_at)) return;
      const key = providerModelsQueryKey(dataMode, daemonSession, provider);
      const current = client.getQueryData<DaemonEnvelope<AiModelsListData>>(key);
      if (current && (!at(current.data?.checked_at) || at(current.data?.checked_at) >= at(snapshot.checked_at))) return;
      client.setQueryData<DaemonEnvelope<AiModelsListData>>(key, { kind: "ai.list_models", schema_version: 1, data: snapshot });
    });
  }, [client, daemonSession, dataMode, enabled, listings, providers, runtimes]);
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
