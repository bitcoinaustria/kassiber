/**
 * Explicit provider checks for Settings → AI providers.
 *
 * Privacy contract (docs/reference/ai.md, docs/reference/privacy-and-security.md):
 * - Opening Settings contacts no provider. The page reads the daemon's last
 *   check with `cached_only`, which never starts a probe: the runtime status
 *   for CLI providers, the model listing for HTTP ones. The daemon keeps the
 *   last check across restarts, for the endpoint it was made at; after a URL
 *   edit a provider reads "not checked" again.
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

// Names the locator this row had when the read or check began: if the row
// was pointed elsewhere meanwhile, the daemon refuses rather than starting
// another runtime, or filing its result under this locator.
function savedModelsKey(
  dataMode: string,
  daemonSession: number,
  provider: Pick<AiProviderRow, "name" | "base_url">,
) {
  return daemonQueryKey(dataMode, daemonSession, "ai.list_models", {
    provider: provider.name,
    base_url: provider.base_url,
    cached_only: true,
  });
}

/** An HTTP provider's last listing, as the daemon kept it; never a probe. */
async function invokeSavedModels(
  provider: Pick<AiProviderRow, "name" | "base_url">,
): Promise<DaemonEnvelope<AiModelsListData>> {
  const envelope = await getTransport().invoke<AiModelsListData>({
    kind: "ai.list_models",
    args: { provider: provider.name, base_url: provider.base_url, cached_only: true },
  });
  if (envelope.kind === "error" || envelope.error) {
    throw new DaemonRequestError("ai.list_models", envelope);
  }
  return envelope;
}

async function invokeRuntimeStatus(
  provider: Pick<AiProviderRow, "name" | "base_url">,
  mode: "cached" | "refresh",
): Promise<DaemonEnvelope<AiProviderRuntimeStatusData>> {
  const target = { provider: provider.name, base_url: provider.base_url };
  const envelope = await getTransport().invoke<AiProviderRuntimeStatusData>({
    kind: "ai.provider_runtime.status",
    args:
      mode === "cached"
        ? { ...target, cached_only: true }
        : { ...target, refresh: true },
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
  onChecked,
}: {
  providers: AiProviderRow[];
  /** AI features on: the daemon refuses runtime kinds otherwise. */
  enabled: boolean;
  offline: boolean;
  /** Ask before contacting off-device providers that were never acknowledged. */
  confirm: (providers: AiProviderRow[]) => Promise<boolean>;
  onAcknowledged: () => Promise<unknown>;
  /** After a check ran: the provider list reports when it was last checked. */
  onChecked?: () => unknown;
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
      queryFn: () => invokeRuntimeStatus(row, "cached"),
      enabled,
      retry: false,
      staleTime: Infinity,
      refetchOnWindowFocus: false,
      meta: { shellProgress: false },
    })),
  });
  const httpProviders = React.useMemo(
    () => providers.filter((row) => !isNativeAiProviderLocator(row.base_url)),
    [providers],
  );
  // The same display read for HTTP providers: the listing the last check
  // found, kept by the daemon across restarts.
  const savedModelQueries = useQueries({
    queries: httpProviders.map((row) => ({
      queryKey: savedModelsKey(dataMode, daemonSession, row),
      queryFn: () => invokeSavedModels(row),
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
      // This session's check, else the last one the daemon kept.
      const saved =
        savedModelQueries[httpProviders.findIndex((row) => row.name === provider.name)]?.data
          ?.data ?? null;
      const data = listed ?? (saved?.checked_at ? saved : null);
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
      httpProviders,
      modelQueryFor,
      pending,
      runtimeQueries,
      savedModelQueries,
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
          const envelope = await invokeRuntimeStatus(provider, "refresh");
          queryClient.setQueryData(
            runtimeStatusKey(dataMode, daemonSession, provider),
            envelope,
          );
          const status = envelope.data?.providers?.[0];
          const modelsKey = providerModelsQueryKey(dataMode, daemonSession, provider);
          if (status?.state === "ready") {
            // The probe already listed this runtime's models; hand them to the
            // picker so it does not have to ask again. A failed refresh comes
            // back as the last good snapshot marked stale with its error; the
            // picker's copy keeps that marking, as for a stale HTTP list,
            // instead of presenting old models as freshly checked.
            const models: DaemonEnvelope<AiModelsListData> = {
              ...envelope,
              kind: "ai.list_models",
              data: {
                provider: provider.name,
                models: status.models ?? [],
                checked_at: envelope.data?.checked_at ?? null,
                stale: envelope.data?.stale === true,
                error: envelope.data?.error ?? null,
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
          // The query for this row as the user clicked it, not whichever row
          // now carries the name: a check waits for the acknowledgement and a
          // list refresh first, and its request names the endpoint it was
          // asked for, so the daemon refuses if the row moved meanwhile.
          await queryClient.fetchQuery({
            ...providerModelsQueryOptions(dataMode, daemonSession, provider),
            staleTime: 0,
          });
        }
      } catch (error) {
        setErrors((prev) => ({ ...prev, [target]: errorMessage(error) }));
      } finally {
        setPending((prev) => ({ ...prev, [target]: false }));
        onChecked?.();
      }
    },
    [daemonSession, dataMode, onChecked, queryClient],
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
      // Errors for providers that could not be acknowledged; they are not
      // checked. Nothing here may reject: the click handlers drop the promise.
      const failed: Record<string, string | null> = {};
      if (unacknowledged.length > 0) {
        let failure: unknown = null;
        let approved = false;
        try {
          approved = await confirm(unacknowledged);
        } catch (error) {
          failure = error;
        }
        if (approved) {
          try {
            for (const row of unacknowledged) {
              await acknowledge.mutateAsync({ name: row.name });
            }
          } catch (error) {
            failure = error;
          }
          // Refresh the rows either way: some acknowledgements may have
          // landed before one failed.
          try {
            await onAcknowledged();
          } catch (error) {
            failure ??= error;
          }
        } else if (failure === null) {
          for (const row of unacknowledged) nextRefusals[targetOf(row)] = "declined";
        }
        if (!approved || failure !== null) {
          allowed = reachable.filter((row) => !unacknowledged.includes(row));
        }
        if (failure !== null) {
          for (const row of unacknowledged) failed[targetOf(row)] = errorMessage(failure);
        }
      }
      setRefusals((prev) => {
        const next = { ...prev };
        for (const row of requested) delete next[targetOf(row)];
        return { ...next, ...nextRefusals };
      });
      if (Object.keys(failed).length > 0) {
        setErrors((prev) => ({ ...prev, ...failed }));
      }
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
