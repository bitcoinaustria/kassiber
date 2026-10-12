/**
 * Explicit provider checks for Settings → AI providers.
 *
 * Privacy contract (docs/reference/ai.md, docs/reference/privacy-and-security.md):
 * - Opening Settings contacts no provider. For CLI providers the page reads
 *   the daemon's last check with `cached_only`, which never starts a probe;
 *   HTTP providers show whatever **Check models** already found for their
 *   current endpoint; after a URL edit they read "not checked" again.
 * - A check runs only from a click, for that one provider: CLI providers via
 *   `ai.provider_runtime.status` scoped to that runtime, HTTP providers via
 *   the picker's `ai.list_models` query, so a check here also fills the picker.
 * - Off-device providers first need Kassiber's acknowledgement, and offline
 *   mode refuses them before anything is asked.
 */

import * as React from "react";
import { useQueries, useQueryClient } from "@tanstack/react-query";

import {
  DaemonRequestError,
  daemonQueryKey,
  useDaemonMutation,
} from "@/daemon/client";
import { getTransport, type DaemonEnvelope } from "@/daemon/transport";
import {
  isNativeAiProviderLocator,
  providerEnabled,
  type AiModelRow,
  type AiModelsListData,
  type AiProviderRow,
  type AiProviderRuntimeStatus,
  type AiProviderRuntimeStatusData,
} from "@/lib/aiCapabilities";
import {
  providerModelsQueryKey,
  providerModelsQueryOptions,
} from "@/components/ai/providerModelQuery";
import { useUiStore } from "@/store/ui";

export interface ProviderCheckState {
  /** CLI runtime snapshot from the last check, if any. */
  runtime: AiProviderRuntimeStatus | null;
  /** Models known from the last check (CLI status or HTTP discovery). */
  models: AiModelRow[];
  /** HTTP providers: the last discovery answered. */
  reachable: boolean;
  /**
   * CLI providers: the newest result is a model list (Check models in the
   * picker) rather than a runtime check, so readiness and version are unknown.
   */
  modelsListed: boolean;
  checkedAt: string | null;
  /** The last refresh failed and an older result is shown. */
  stale: boolean;
  pending: boolean;
  error: string | null;
}

const EMPTY_STATE: ProviderCheckState = {
  runtime: null,
  models: [],
  reachable: false,
  modelsListed: false,
  checkedAt: null,
  stale: false,
  pending: false,
  error: null,
};

type RefusalReason = "offline" | "declined";

export interface ProviderChecks {
  stateFor: (provider: AiProviderRow) => ProviderCheckState;
  check: (provider: AiProviderRow) => Promise<void>;
  checkAll: (providers: AiProviderRow[]) => Promise<void>;
  /** Why the provider's last check was refused before contacting it, if it was. */
  refusalFor: (provider: AiProviderRow) => RefusalReason | undefined;
  anyPending: boolean;
}

// Keyed by the locator as well as the name: an edited row (say a Codex row
// pointed at `claude-cli://default`) reads its new runtime's snapshot instead
// of keeping the old one, and that read is `cached_only`, never a probe.
function runtimeStatusKey(
  dataMode: string,
  daemonSession: number,
  provider: Pick<AiProviderRow, "name" | "base_url">,
) {
  return daemonQueryKey(dataMode, daemonSession, "ai.provider_runtime.status", {
    provider: provider.name,
    base_url: provider.base_url,
    cached_only: true,
  });
}

async function invokeRuntimeStatus(
  provider: string,
  mode: "cached" | "refresh",
): Promise<DaemonEnvelope<AiProviderRuntimeStatusData>> {
  const envelope = await getTransport().invoke<AiProviderRuntimeStatusData>({
    kind: "ai.provider_runtime.status",
    args:
      mode === "cached"
        ? { provider, cached_only: true }
        : { provider, refresh: true },
  });
  if (envelope.kind === "error" || envelope.error) {
    throw new DaemonRequestError("ai.provider_runtime.status", envelope);
  }
  return envelope;
}

/**
 * What a transient check outcome (pending, error, refusal) belongs to: the
 * same book session and endpoint the runtime and model queries are keyed by,
 * so a row pointed elsewhere, or a same-named row in another book, never
 * shows an outcome from a check it did not have.
 */
function checkTargetKey(
  dataMode: string,
  daemonSession: number,
  provider: Pick<AiProviderRow, "name" | "base_url">,
): string {
  return JSON.stringify([dataMode, daemonSession, provider.name, provider.base_url]);
}

function checkedTime(value: string | null | undefined): number | null {
  if (!value) return null;
  const at = Date.parse(value);
  return Number.isNaN(at) ? null : at;
}

/**
 * Whether a CLI provider's model list (from the picker's Check models) is
 * newer than its runtime check. Ties and unreadable times go to the runtime
 * check, which says more (version, sign-in).
 */
function modelListIsNewer(
  runtimeCheckedAt: string | null | undefined,
  listedAt: string | null | undefined,
): boolean {
  const listed = checkedTime(listedAt);
  if (listed === null) return false;
  const runtime = checkedTime(runtimeCheckedAt);
  return runtime === null || listed > runtime;
}

function errorMessage(error: unknown): string | null {
  if (!error) return null;
  return error instanceof Error ? error.message : String(error);
}

export function useAiProviderChecks({
  providers,
  enabled,
  offline,
  confirm,
  onAcknowledged,
}: {
  providers: AiProviderRow[];
  /** AI features on: the daemon refuses runtime kinds otherwise. */
  enabled: boolean;
  offline: boolean;
  /** Ask before contacting off-device providers that were never acknowledged. */
  confirm: (providers: AiProviderRow[]) => Promise<boolean>;
  onAcknowledged: () => Promise<unknown>;
}): ProviderChecks {
  const queryClient = useQueryClient();
  const dataMode = useUiStore((state) => state.dataMode);
  const daemonSession = useUiStore((state) => state.daemonSession);
  const acknowledge = useDaemonMutation("ai.providers.acknowledge", {
    invalidateQueries: false,
  });
  const [pending, setPending] = React.useState<Record<string, boolean>>({});
  const [errors, setErrors] = React.useState<Record<string, string | null>>({});
  const [refusals, setRefusals] = React.useState<Record<string, RefusalReason>>({});

  const cliProviders = React.useMemo(
    () => providers.filter((row) => isNativeAiProviderLocator(row.base_url)),
    [providers],
  );

  // Display reads of an earlier check. `cached_only` never probes, so these
  // are safe on mount; they still need AI features on to reach the daemon.
  const runtimeQueries = useQueries({
    queries: cliProviders.map((row) => ({
      queryKey: runtimeStatusKey(dataMode, daemonSession, row),
      queryFn: () => invokeRuntimeStatus(row.name, "cached"),
      enabled,
      retry: false,
      staleTime: Infinity,
      refetchOnWindowFocus: false,
      meta: { shellProgress: false },
    })),
  });
  // Never enabled: only an explicit refetch() runs discovery. Read for CLI
  // providers too, so a Check models from the picker shows here as well.
  const modelQueries = useQueries({
    queries: providers.map((row) =>
      providerModelsQueryOptions(dataMode, daemonSession, row),
    ),
  });
  const modelQueryFor = React.useCallback(
    (provider: AiProviderRow) =>
      modelQueries[providers.findIndex((row) => row.name === provider.name)],
    [modelQueries, providers],
  );

  const stateFor = React.useCallback(
    (provider: AiProviderRow): ProviderCheckState => {
      const target = checkTargetKey(dataMode, daemonSession, provider);
      const isPending = pending[target] === true;
      const ownError = errors[target] ?? null;
      const cliIndex = cliProviders.findIndex((row) => row.name === provider.name);
      const query = modelQueryFor(provider);
      const listed =
        query?.data?.kind === "ai.list_models" ? (query.data.data ?? null) : null;
      if (cliIndex >= 0) {
        const data = runtimeQueries[cliIndex]?.data?.data;
        const runtime = data?.providers?.[0] ?? null;
        const runtimeState = {
          ...EMPTY_STATE,
          pending: isPending,
          error: ownError ?? (data?.stale ? (data.error?.message ?? null) : null),
        };
        if (listed && modelListIsNewer(runtime ? data?.checked_at : null, listed.checked_at)) {
          // The picker listed models after the last runtime check (or there
          // was none). A ready runtime keeps its version and sign-in line; an
          // older not-ready one is superseded, since listing models needed a
          // working CLI.
          const ready = runtime?.state === "ready" ? runtime : null;
          return {
            ...runtimeState,
            runtime: ready,
            models: listed.models,
            modelsListed: ready === null,
            checkedAt: listed.checked_at ?? null,
            stale: listed.stale === true,
          };
        }
        return {
          ...runtimeState,
          runtime,
          // A runtime check that is not ready offers no models, whatever an
          // older model list said.
          models: runtime?.state === "ready" ? runtime.models : [],
          checkedAt: data?.checked_at ?? null,
          stale: data?.stale === true,
        };
      }
      const data = listed;
      return {
        ...EMPTY_STATE,
        models: data?.models ?? [],
        reachable: Boolean(data) && data?.stale !== true,
        checkedAt: data?.checked_at ?? null,
        stale: data?.stale === true,
        pending: isPending || query?.isFetching === true,
        error:
          ownError ??
          errorMessage(query?.error) ??
          (data?.stale ? (data.error?.message ?? null) : null),
      };
    },
    [
      cliProviders,
      daemonSession,
      dataMode,
      errors,
      modelQueryFor,
      pending,
      runtimeQueries,
    ],
  );

  const refusalFor = React.useCallback(
    (provider: AiProviderRow) =>
      refusals[checkTargetKey(dataMode, daemonSession, provider)],
    [daemonSession, dataMode, refusals],
  );

  const runCheck = React.useCallback(
    async (provider: AiProviderRow) => {
      const target = checkTargetKey(dataMode, daemonSession, provider);
      setPending((prev) => ({ ...prev, [target]: true }));
      setErrors((prev) => ({ ...prev, [target]: null }));
      try {
        if (isNativeAiProviderLocator(provider.base_url)) {
          const envelope = await invokeRuntimeStatus(provider.name, "refresh");
          queryClient.setQueryData(
            runtimeStatusKey(dataMode, daemonSession, provider),
            envelope,
          );
          const status = envelope.data?.providers?.[0];
          const modelsKey = providerModelsQueryKey(dataMode, daemonSession, provider);
          if (status?.state === "ready") {
            // The probe already listed this runtime's models; hand them to the
            // picker so it does not have to ask again.
            const models: DaemonEnvelope<AiModelsListData> = {
              ...envelope,
              kind: "ai.list_models",
              data: {
                provider: provider.name,
                models: status.models ?? [],
                checked_at: envelope.data?.checked_at ?? null,
                stale: false,
                error: null,
              },
            };
            queryClient.setQueryData(modelsKey, models);
          } else {
            // Not installed, signed out, or failing: whatever an earlier check
            // listed is no longer on offer, so the picker must stop offering
            // those models and their capabilities (fast mode included).
            // Resetting a disabled query never refetches it.
            await queryClient.resetQueries({ queryKey: modelsKey, exact: true });
          }
        } else {
          await modelQueryFor(provider)?.refetch();
        }
      } catch (error) {
        setErrors((prev) => ({ ...prev, [target]: errorMessage(error) }));
      } finally {
        setPending((prev) => ({ ...prev, [target]: false }));
      }
    },
    [daemonSession, dataMode, modelQueryFor, queryClient],
  );

  const checkMany = React.useCallback(
    async (requested: AiProviderRow[]) => {
      if (!enabled) return;
      // A switched-off provider is never contacted; the daemon refuses it too.
      const candidates = requested.filter(providerEnabled);
      if (candidates.length === 0) return;
      const nextRefusals: Record<string, RefusalReason> = {};
      const targetOf = (row: AiProviderRow) =>
        checkTargetKey(dataMode, daemonSession, row);
      // Offline mode keeps only providers marked local; refuse the rest here,
      // before any acknowledgement prompt or daemon call.
      const reachable = candidates.filter((row) => {
        if (offline && row.kind !== "local") {
          nextRefusals[targetOf(row)] = "offline";
          return false;
        }
        return true;
      });
      const unacknowledged = reachable.filter(
        (row) => row.kind !== "local" && !row.acknowledged_at,
      );
      let allowed = reachable;
      if (unacknowledged.length > 0) {
        if (await confirm(unacknowledged)) {
          try {
            for (const row of unacknowledged) {
              await acknowledge.mutateAsync({ name: row.name });
            }
          } finally {
            await onAcknowledged();
          }
        } else {
          for (const row of unacknowledged) nextRefusals[targetOf(row)] = "declined";
          allowed = reachable.filter((row) => !unacknowledged.includes(row));
        }
      }
      setRefusals((prev) => {
        const next = { ...prev };
        for (const row of requested) delete next[targetOf(row)];
        return { ...next, ...nextRefusals };
      });
      await Promise.allSettled(allowed.map((row) => runCheck(row)));
    },
    [
      acknowledge,
      confirm,
      daemonSession,
      dataMode,
      enabled,
      offline,
      onAcknowledged,
      runCheck,
    ],
  );

  const check = React.useCallback(
    (provider: AiProviderRow) => checkMany([provider]),
    [checkMany],
  );

  return {
    stateFor,
    check,
    checkAll: checkMany,
    refusalFor,
    anyPending:
      acknowledge.isPending || Object.values(pending).some(Boolean),
  };
}
