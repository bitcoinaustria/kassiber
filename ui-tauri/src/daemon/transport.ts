/**
 * Daemon transport selector.
 *
 * Three runtime modes per docs/plan/04-desktop-ui.md §2.6:
 *   - "mock"   — fixture responses, no Python required (default dev mode)
 *   - "bridge" — authenticated localhost WebSocket, dev-only (later)
 *   - "tauri"  — JSONL over stdin/stdout via Rust supervisor (production)
 *
 * The Tauri mode calls the Rust shell command boundary, which forwards
 * whitelisted requests to the local Python daemon.
 */

import { mockDaemon, mockStream } from "./mock";
import { useUiStore, type DataMode } from "@/store/ui";

export type DaemonMode = "mock" | "bridge" | "tauri";

function defaultDaemonMode(): DaemonMode {
  if (typeof window !== "undefined" && "__TAURI_INTERNALS__" in window) {
    return "tauri";
  }
  if (import.meta.env.DEV) {
    return "bridge";
  }
  return "mock";
}

const RAW_MODE = (import.meta.env.VITE_DAEMON ?? defaultDaemonMode()) as string;

if (!["mock", "bridge", "tauri"].includes(RAW_MODE)) {
  throw new Error(
    `VITE_DAEMON must be one of mock|bridge|tauri (got ${RAW_MODE})`,
  );
}

export const DAEMON_MODE = RAW_MODE as DaemonMode;

export interface DaemonRequest {
  kind: string;
  args?: Record<string, unknown>;
}

export interface DaemonEnvelope<T = unknown> {
  kind: string;
  schema_version: number;
  request_id?: string | number | null;
  data?: T;
  error?: {
    code: string;
    message: string;
    hint?: string | null;
    details?: unknown;
    retryable?: boolean;
  };
}

/**
 * Stream record forwarded by the Rust supervisor while a streaming kind
 * (e.g. `ai.chat`) is in flight. Mid-stream records have a kind shaped
 * like `<request_kind>.delta`; the terminal record matches the request
 * kind exactly and is delivered as the resolved value of `stream()`.
 */
export interface DaemonStreamRecord<T = unknown> {
  kind: string;
  schema_version: number;
  request_id?: string | number | null;
  data?: T;
}

export interface DaemonStreamOptions<T = unknown> {
  /** Receive each mid-stream record (kind = `<request_kind>.delta` etc.). */
  onRecord?: (record: DaemonStreamRecord<T>) => void;
  /** Optional abort signal; transports that support cancellation watch this. */
  signal?: AbortSignal;
}

export interface DaemonTransport {
  invoke<T = unknown>(req: DaemonRequest): Promise<DaemonEnvelope<T>>;
  /** Streaming variant; resolves with the terminal envelope. */
  stream<T = unknown, R = unknown>(
    req: DaemonRequest,
    options?: DaemonStreamOptions<R>,
  ): Promise<DaemonEnvelope<T>>;
}

const tauriDaemon: DaemonTransport = {
  async invoke<T = unknown>(
    req: DaemonRequest,
  ): Promise<DaemonEnvelope<T>> {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<DaemonEnvelope<T>>("daemon_invoke", { request: req });
  },
  async stream<T = unknown, R = unknown>(
    req: DaemonRequest,
    options?: DaemonStreamOptions<R>,
  ): Promise<DaemonEnvelope<T>> {
    const { invoke } = await import("@tauri-apps/api/core");
    const { listen } = await import("@tauri-apps/api/event");

    // Subscribe to the shared stream channel BEFORE invoking; the
    // supervisor allocates request_id internally and emits records with
    // that id, so we filter by matching the terminal envelope's id once
    // it returns.
    const buffered: DaemonStreamRecord<R>[] = [];
    let terminalRequestId: string | number | null | undefined;
    const flush = (record: DaemonStreamRecord<R>) => {
      if (terminalRequestId !== undefined) {
        if (record.request_id === terminalRequestId) {
          options?.onRecord?.(record);
        }
        return;
      }
      buffered.push(record);
    };

    const unlisten = await listen<DaemonStreamRecord<R>>(
      "daemon://stream",
      (event) => {
        flush(event.payload);
      },
    );

    try {
      const envelope = await invoke<DaemonEnvelope<T>>("daemon_invoke", {
        request: req,
      });
      terminalRequestId = envelope.request_id;
      // Drain anything buffered before the terminal envelope arrived.
      for (const record of buffered) {
        if (record.request_id === terminalRequestId) {
          options?.onRecord?.(record);
        }
      }
      return envelope;
    } finally {
      unlisten();
    }
  },
};

const bridgeDaemon: DaemonTransport = {
  async invoke<T = unknown>(
    req: DaemonRequest,
  ): Promise<DaemonEnvelope<T>> {
    const response = await fetch("/__kassiber__/daemon", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(req),
    });
    return response.json() as Promise<DaemonEnvelope<T>>;
  },
  async stream<T = unknown>(req: DaemonRequest): Promise<DaemonEnvelope<T>> {
    return {
      kind: "error",
      schema_version: 1,
      error: {
        code: "stream_not_supported",
        message: `bridge mode does not support streaming kinds (${req.kind}); use VITE_DAEMON=mock or run inside the Tauri shell.`,
        retryable: false,
      },
    };
  },
};

export function getTransport(dataMode?: DataMode): DaemonTransport {
  if ((dataMode ?? useUiStore.getState().dataMode) === "mock") {
    return { invoke: mockDaemon.invoke, stream: mockStream };
  }

  switch (DAEMON_MODE) {
    case "mock":
      return { invoke: mockDaemon.invoke, stream: mockStream };
    case "bridge":
      return bridgeDaemon;
    case "tauri":
      return tauriDaemon;
  }
}
