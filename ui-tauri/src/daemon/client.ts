/**
 * TanStack Query helpers over the active daemon transport.
 *
 * `useDaemon` is the read hook every screen will call — typed wrappers
 * per `kind` are added once the Pydantic→JSON Schema→TS pipeline lands.
 */

import {
  useQuery,
  type UseQueryOptions,
  type UseQueryResult,
} from "@tanstack/react-query";
import { getTransport, type DaemonEnvelope } from "./transport";

const transport = getTransport();

export function daemonQueryKey(kind: string, args?: Record<string, unknown>) {
  return args ? (["daemon", kind, args] as const) : (["daemon", kind] as const);
}

export function useDaemon<T = unknown>(
  kind: string,
  args?: Record<string, unknown>,
  options?: Omit<
    UseQueryOptions<DaemonEnvelope<T>>,
    "queryKey" | "queryFn"
  >,
): UseQueryResult<DaemonEnvelope<T>> {
  return useQuery<DaemonEnvelope<T>>({
    queryKey: daemonQueryKey(kind, args),
    queryFn: () => transport.invoke<T>({ kind, args }),
    staleTime: 5 * 60 * 1000,
    ...options,
  });
}
