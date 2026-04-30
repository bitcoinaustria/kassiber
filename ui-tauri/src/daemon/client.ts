/**
 * TanStack Query helpers over the active daemon transport.
 *
 * `useDaemon` is the read hook every screen will call — typed wrappers
 * per `kind` are added once the Pydantic→JSON Schema→TS pipeline lands.
 */

import {
  useMutation,
  useQuery,
  useQueryClient,
  type UseQueryOptions,
  type UseQueryResult,
} from "@tanstack/react-query";
import { getTransport, type DaemonEnvelope } from "./transport";
import { useUiStore, type DataMode } from "@/store/ui";

export const DAEMON_AUTH_REQUIRED_EVENT = "kassiber:auth-required";

export class DaemonAuthRequiredError extends Error {
  envelope: DaemonEnvelope;

  constructor(envelope: DaemonEnvelope) {
    super("Daemon authentication required");
    this.name = "DaemonAuthRequiredError";
    this.envelope = envelope;
  }
}

function handleAuthRequired(envelope: DaemonEnvelope): never {
  if (typeof window !== "undefined") {
    const event = () =>
      window.dispatchEvent(
        new CustomEvent(DAEMON_AUTH_REQUIRED_EVENT, { detail: envelope }),
      );
    event();
    window.setTimeout(event, 0);
  }
  throw new DaemonAuthRequiredError(envelope);
}

export function daemonQueryKey(
  mode: string,
  kind: string,
  args?: Record<string, unknown>,
) {
  return args
    ? (["daemon", mode, kind, args] as const)
    : (["daemon", mode, kind] as const);
}

export function useDaemon<T = unknown>(
  kind: string,
  args?: Record<string, unknown>,
  options?: Omit<
    UseQueryOptions<DaemonEnvelope<T>>,
    "queryKey" | "queryFn"
  >,
): UseQueryResult<DaemonEnvelope<T>> {
  const dataMode = useUiStore((state) => state.dataMode);
  return useQuery<DaemonEnvelope<T>>({
    queryKey: daemonQueryKey(dataMode, kind, args),
    queryFn: async () => {
      const envelope = await getTransport(dataMode).invoke<T>({ kind, args });
      if (envelope.kind === "auth_required") {
        handleAuthRequired(envelope);
      }
      return envelope;
    },
    staleTime: 5 * 60 * 1000,
    retry: (failureCount, error) =>
      error instanceof DaemonAuthRequiredError ? false : failureCount < 3,
    ...options,
  });
}

export function useDaemonMutation<T = unknown>(
  kind: string,
  options?: { dataMode?: DataMode },
) {
  const selectedDataMode = useUiStore((state) => state.dataMode);
  const dataMode = options?.dataMode ?? selectedDataMode;
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (args?: Record<string, unknown>) => {
      const envelope = await getTransport(dataMode).invoke<T>({ kind, args });
      if (envelope.kind === "auth_required") {
        handleAuthRequired(envelope);
      }
      if (envelope.kind === "error" || envelope.error) {
        throw new Error(envelope.error?.message ?? `daemon ${kind} failed`);
      }
      return envelope;
    },
    onSuccess: () =>
      queryClient.invalidateQueries({
        queryKey: ["daemon", dataMode],
      }),
  });
}
