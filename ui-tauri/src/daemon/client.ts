/**
 * TanStack Query helpers over the active daemon transport.
 *
 * `useDaemon` is the read hook every screen will call — typed wrappers
 * per `kind` are added once the Pydantic→JSON Schema→TS pipeline lands.
 */

import {
  type QueryClient,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
  type UseQueryOptions,
  type UseQueryResult,
  type UseInfiniteQueryOptions,
  type UseInfiniteQueryResult,
  type InfiniteData,
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

function isAuthRequiredEnvelope(value: unknown): value is DaemonEnvelope {
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
  if (isAuthRequiredEnvelope(detail)) {
    return { envelope: detail };
  }
  if (!detail || typeof detail !== "object") return null;

  const candidate = detail as Partial<DaemonAuthRequiredEventDetail>;
  if (!isAuthRequiredEnvelope(candidate.envelope)) return null;
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
  if (parsed.daemonSession === undefined) {
    return currentDaemonSession === 0;
  }
  return (
    parsed.daemonSession === currentDaemonSession
  );
}

export function dispatchDaemonAuthRequired(
  envelope: DaemonEnvelope,
  daemonSession: number,
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
  daemonSession: number,
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

export function useDaemonInfinite<T = unknown>(
  kind: string,
  args: Record<string, unknown> | undefined,
  getNextPageParam: (lastPage: DaemonEnvelope<T>) => unknown,
  options?: Omit<
    UseInfiniteQueryOptions<
      DaemonEnvelope<T>,
      Error,
      InfiniteData<DaemonEnvelope<T>>,
      readonly unknown[],
      unknown
    >,
    "queryKey" | "queryFn" | "initialPageParam" | "getNextPageParam"
  >,
): UseInfiniteQueryResult<InfiniteData<DaemonEnvelope<T>>, Error> {
  const dataMode = useUiStore((state) => state.dataMode);
  const daemonSession = useUiStore((state) => state.daemonSession);
  return useInfiniteQuery<
    DaemonEnvelope<T>,
    Error,
    InfiniteData<DaemonEnvelope<T>>,
    readonly unknown[],
    unknown
  >({
    queryKey: daemonQueryKey(dataMode, daemonSession, kind, args),
    initialPageParam: null,
    queryFn: async ({ pageParam }) => {
      const envelope = await getTransport(dataMode).invoke<T>({
        kind,
        args: {
          ...(args ?? {}),
          ...(typeof pageParam === "string" ? { cursor: pageParam } : {}),
        },
      });
      if (envelope.kind === "auth_required") {
        handleAuthRequired(envelope, daemonSession);
      }
      if (envelope.kind === "error" || envelope.error) {
        throw new DaemonRequestError(kind, envelope);
      }
      return envelope;
    },
    getNextPageParam,
    staleTime: 5 * 60 * 1000,
    retry: (failureCount, error) =>
      error instanceof DaemonAuthRequiredError ? false : failureCount < 3,
    ...options,
  });
}

export function daemonMutationKey(dataMode: DataMode, kind: string) {
  return ["daemon-mutation", dataMode, kind] as const;
}

const TARGETED_DAEMON_QUERY_INVALIDATIONS: Record<string, readonly string[]> = {
  "ui.freshness.run": [
    "ui.activity.history",
    "ui.activity.stale",
    "ui.connections.node.snapshot",
    "ui.journals.events.list",
    "ui.journals.quarantine",
    "ui.journals.snapshot",
    "ui.journals.transfers.list",
    "ui.next_actions",
    "ui.overview.snapshot",
    "ui.rates.coverage",
    "ui.rates.summary",
    "ui.report.blockers",
    "ui.reports.balance_history",
    "ui.reports.balance_sheet",
    "ui.reports.capital_gains",
    "ui.reports.lightning_profitability",
    "ui.reports.portfolio_summary",
    "ui.reports.summary",
    "ui.reports.tax_summary",
    "ui.transactions.extremes",
    "ui.transactions.list",
    "ui.transactions.resolve",
    "ui.wallets.list",
    "ui.wallets.utxos",
    "ui.workspace.health",
  ],
  "ui.wallets.sync": [
    "ui.activity.history",
    "ui.activity.stale",
    "ui.connections.node.snapshot",
    "ui.journals.events.list",
    "ui.journals.quarantine",
    "ui.journals.snapshot",
    "ui.next_actions",
    "ui.overview.snapshot",
    "ui.report.blockers",
    "ui.reports.balance_history",
    "ui.reports.balance_sheet",
    "ui.reports.capital_gains",
    "ui.reports.lightning_profitability",
    "ui.reports.portfolio_summary",
    "ui.reports.summary",
    "ui.reports.tax_summary",
    "ui.transactions.extremes",
    "ui.transactions.list",
    "ui.transactions.resolve",
    "ui.wallets.list",
    "ui.wallets.utxos",
    "ui.workspace.health",
  ],
  "ui.journals.process": [
    "ui.activity.history",
    "ui.activity.stale",
    "ui.journals.events.list",
    "ui.journals.quarantine",
    "ui.journals.snapshot",
    "ui.journals.transfers.list",
    "ui.next_actions",
    "ui.overview.snapshot",
    "ui.report.blockers",
    "ui.reports.balance_history",
    "ui.reports.balance_sheet",
    "ui.reports.capital_gains",
    "ui.reports.portfolio_summary",
    "ui.reports.summary",
    "ui.reports.tax_summary",
    "ui.transactions.extremes",
    "ui.transactions.list",
    "ui.transactions.resolve",
    "ui.workspace.health",
  ],
  "ui.attachments.add": [
    "ui.attachments.list",
    "ui.audit.evidence.summary",
    "ui.report.blockers",
    "ui.source_funds.coverage",
    "ui.source_funds.evidence.list",
    "ui.source_funds.preview",
  ],
  "ui.attachments.copy": [
    "ui.attachments.list",
    "ui.audit.evidence.summary",
    "ui.report.blockers",
    "ui.source_funds.coverage",
    "ui.source_funds.evidence.list",
    "ui.source_funds.preview",
  ],
  "ui.attachments.remove": [
    "ui.attachments.list",
    "ui.audit.evidence.summary",
    "ui.report.blockers",
    "ui.source_funds.coverage",
    "ui.source_funds.evidence.list",
    "ui.source_funds.preview",
  ],
  "ui.attachments.rename": [
    "ui.attachments.list",
    "ui.audit.evidence.summary",
    "ui.source_funds.evidence.list",
    "ui.source_funds.preview",
  ],
};

export function invalidatedDaemonQueryKindsForMutation(kind: string) {
  return TARGETED_DAEMON_QUERY_INVALIDATIONS[kind] ?? null;
}

function invalidateDaemonQueriesForMutation(
  queryClient: QueryClient,
  dataMode: DataMode,
  kind: string,
) {
  const queryKinds = invalidatedDaemonQueryKindsForMutation(kind);
  if (!queryKinds) {
    void queryClient.invalidateQueries({
      queryKey: ["daemon", dataMode],
    });
    return;
  }
  const affectedKinds = new Set(queryKinds);
  void queryClient.invalidateQueries({
    queryKey: ["daemon", dataMode],
    predicate: (query) =>
      query.queryKey.some(
        (part) => typeof part === "string" && affectedKinds.has(part),
      ),
  });
}

export function mutationAdvancesDaemonSession(kind: string) {
  return kind === "ui.profiles.switch";
}

export function useDaemonMutation<T = unknown>(
  kind: string,
  options?: { dataMode?: DataMode },
) {
  const selectedDataMode = useUiStore((state) => state.dataMode);
  const dataMode = options?.dataMode ?? selectedDataMode;
  const queryClient = useQueryClient();
  return useMutation({
    // Sharing the mutation key across all `useDaemonMutation(kind, ...)`
    // instances lets callers ask the QueryClient for in-flight counts
    // (`isMutating`) and gate duplicate jobs across the menu, route screens,
    // and shared hooks.
    mutationKey: daemonMutationKey(dataMode, kind),
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
    onSuccess: () => {
      if (mutationAdvancesDaemonSession(kind)) {
        useUiStore.getState().bumpDaemonSession();
      }
      invalidateDaemonQueriesForMutation(queryClient, dataMode, kind);
    },
  });
}

export interface DaemonStreamMutationOptions<R> {
  dataMode?: DataMode;
  onProgress?: (record: R) => void;
}

/**
 * Mutation hook for daemon kinds that emit interleaved progress envelopes
 * before the terminal envelope. Wraps `transport.stream` so callers can
 * surface per-record progress (e.g. "Imported 1,200 / 5,000 rows") without
 * dropping into the lower-level transport.
 */
export function useDaemonStreamMutation<T = unknown, R = unknown>(
  kind: string,
  options?: DaemonStreamMutationOptions<R>,
) {
  const selectedDataMode = useUiStore((state) => state.dataMode);
  const dataMode = options?.dataMode ?? selectedDataMode;
  const queryClient = useQueryClient();
  return useMutation({
    mutationKey: daemonMutationKey(dataMode, kind),
    mutationFn: async (args?: Record<string, unknown>) => {
      const daemonSession = useUiStore.getState().daemonSession;
      const envelope = await getTransport(dataMode).stream<T, R>(
        { kind, args },
        {
          onRecord: (record) => {
            if (record.data !== undefined) {
              options?.onProgress?.(record.data as R);
            }
          },
        },
      );
      if (envelope.kind === "auth_required") {
        handleAuthRequired(envelope, daemonSession);
      }
      if (envelope.kind === "error" || envelope.error) {
        throw new DaemonRequestError(kind, envelope);
      }
      return envelope;
    },
    onSuccess: () =>
      invalidateDaemonQueriesForMutation(queryClient, dataMode, kind),
  });
}
