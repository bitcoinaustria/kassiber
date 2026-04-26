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
import { useUiStore } from "@/store/ui";

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
    queryFn: () => getTransport(dataMode).invoke<T>({ kind, args }),
    staleTime: 5 * 60 * 1000,
    ...options,
  });
}

export function useDaemonMutation<T = unknown>(kind: string) {
  const dataMode = useUiStore((state) => state.dataMode);
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (args?: Record<string, unknown>) => {
      const envelope = await getTransport(dataMode).invoke<T>({ kind, args });
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
