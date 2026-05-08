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

export interface DaemonAuthRequiredEventDetail {
  envelope: DaemonEnvelope;
  daemonSession?: number;
}

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

function isDaemonEnvelope(value: unknown): value is DaemonEnvelope {
  if (!value || typeof value !== "object") return false;
  const candidate = value as Partial<DaemonEnvelope>;
  return (
    candidate.kind === "auth_required" &&
    typeof candidate.schema_version === "number"
  );
}

export function parseDaemonAuthRequiredEventDetail(
  detail: unknown,
): DaemonAuthRequiredEventDetail | null {
  if (isDaemonEnvelope(detail)) {
    return { envelope: detail };
  }
  if (!detail || typeof detail !== "object") return null;

  const candidate = detail as Partial<DaemonAuthRequiredEventDetail>;
  if (!isDaemonEnvelope(candidate.envelope)) return null;
  return {
    envelope: candidate.envelope,
    daemonSession:
      typeof candidate.daemonSession === "number"
        ? candidate.daemonSession
        : undefined,
  };
}

export function shouldHandleDaemonAuthRequiredEvent(
  detail: unknown,
  currentDaemonSession: number,
): boolean {
  const parsed = parseDaemonAuthRequiredEventDetail(detail);
  if (!parsed) return false;
  return (
    parsed.daemonSession === undefined ||
    parsed.daemonSession === currentDaemonSession
  );
}

export function dispatchDaemonAuthRequired(
  envelope: DaemonEnvelope,
  daemonSession?: number,
): void {
  if (typeof window !== "undefined") {
    const detail: DaemonAuthRequiredEventDetail = {
      envelope,
      daemonSession,
    };
    const event = () =>
      window.dispatchEvent(
        new CustomEvent(DAEMON_AUTH_REQUIRED_EVENT, { detail }),
      );
    event();
    window.setTimeout(event, 0);
  }
}

function handleAuthRequired(
  envelope: DaemonEnvelope,
  daemonSession?: number,
): never {
  dispatchDaemonAuthRequired(envelope, daemonSession);
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
        handleAuthRequired(envelope, daemonSession);
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
      const daemonSession = useUiStore.getState().daemonSession;
      const envelope = await getTransport(dataMode).invoke<T>({ kind, args });
      if (envelope.kind === "auth_required") {
        handleAuthRequired(envelope, daemonSession);
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
