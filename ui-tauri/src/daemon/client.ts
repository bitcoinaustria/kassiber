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

interface DaemonErrorFormatOptions {
  includeDetails?: boolean;
}

export class DaemonRequestError extends Error {
  envelope: DaemonEnvelope;

  constructor(kind: string, envelope: DaemonEnvelope) {
    super(formatDaemonEnvelopeError(envelope) ?? `daemon ${kind} failed`);
    this.name = "DaemonRequestError";
    this.envelope = envelope;
  }
}

export function formatDaemonEnvelopeError(
  envelope: DaemonEnvelope,
  options: DaemonErrorFormatOptions = {},
): string | null {
  const error = envelope.error;
  if (!error) return null;

  const parts = [error.message || error.code].filter(Boolean);
  if (error.hint) {
    parts.push(error.hint);
  }
  const detailText = options.includeDetails
    ? formatErrorDetails(error.details)
    : null;
  if (detailText) {
    parts.push(detailText);
  }
  return parts.join("\n\n");
}

function formatErrorDetails(details: unknown): string | null {
  if (details === null || details === undefined) return null;
  if (typeof details === "string") return details;
  try {
    return JSON.stringify(details, null, 2);
  } catch {
    return String(details);
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
  session: number,
  kind: string,
  args?: Record<string, unknown>,
) {
  return args
    ? (["daemon", mode, session, kind, args] as const)
    : (["daemon", mode, session, kind] as const);
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
  const daemonSession = useUiStore((state) => state.daemonSession);
  return useQuery<DaemonEnvelope<T>>({
    queryKey: daemonQueryKey(dataMode, daemonSession, kind, args),
    queryFn: async () => {
      const envelope = await getTransport(dataMode).invoke<T>({ kind, args });
      if (envelope.kind === "auth_required") {
        handleAuthRequired(envelope);
      }
      if (envelope.kind === "error" || envelope.error) {
        throw new DaemonRequestError(kind, envelope);
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
        throw new DaemonRequestError(kind, envelope);
      }
      return envelope;
    },
    onSuccess: () =>
      queryClient.invalidateQueries({
        queryKey: ["daemon", dataMode],
      }),
  });
}
