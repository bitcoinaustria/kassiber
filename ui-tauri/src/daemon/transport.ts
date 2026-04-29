/**
 * Daemon transport selector.
 *
 * Three runtime modes per docs/plan/04-desktop-ui.md §2.6:
 *   - "mock"   — fixture responses, no Python required (default dev mode)
 *   - "bridge" — Vite dev-server bridge to the Python daemon, dev-only
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
  /**
   * Optional client-allocated request_id. Streaming transports allocate
   * one automatically so they can filter `daemon://stream` records as
   * they arrive instead of buffering them until the terminal envelope.
   */
  request_id?: string;
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

function isTerminalEnvelopeKind(kind: string, requestKind: string): boolean {
  return kind === requestKind || kind === "error" || kind === "auth_required";
}

export async function readBridgeNdjsonStream<T = unknown, R = unknown>(
  response: Response,
  requestKind: string,
  requestId: string,
  options?: DaemonStreamOptions<R>,
): Promise<DaemonEnvelope<T>> {
  if (!response.ok) {
    throw new Error(`bridge stream failed with HTTP ${response.status}`);
  }
  if (!response.body) {
    throw new Error("bridge stream response did not include a body");
  }

  const decoder = new TextDecoder();
  const reader = response.body.getReader();
  let buffer = "";
  let terminal: DaemonEnvelope<T> | null = null;

  const handleLine = (line: string) => {
    const trimmed = line.trim();
    if (!trimmed) return;
    const record = JSON.parse(trimmed) as DaemonStreamRecord<R>;
    if (record.request_id !== requestId) {
      return;
    }
    if (isTerminalEnvelopeKind(record.kind, requestKind)) {
      terminal = record as DaemonEnvelope<T>;
      return;
    }
    if (!options?.signal?.aborted) {
      options?.onRecord?.(record);
    }
  };

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    let lineEnd = buffer.indexOf("\n");
    while (lineEnd >= 0) {
      handleLine(buffer.slice(0, lineEnd));
      buffer = buffer.slice(lineEnd + 1);
      lineEnd = buffer.indexOf("\n");
    }
  }

  buffer += decoder.decode();
  handleLine(buffer);

  if (!terminal) {
    throw new Error("bridge stream ended without a terminal daemon envelope");
  }
  return terminal;
}

export function makeDaemonRequestId(): string {
  if (
    typeof globalThis.crypto !== "undefined" &&
    typeof globalThis.crypto.randomUUID === "function"
  ) {
    return globalThis.crypto.randomUUID();
  }
  return `req-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
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

    // Allocate the request_id client-side and pass it through; the
    // supervisor honors a String request_id when supplied, so we can
    // filter `daemon://stream` records as they arrive instead of
    // buffering them until the terminal envelope returns.
    const requestId = req.request_id ?? makeDaemonRequestId();

    const unlisten = await listen<DaemonStreamRecord<R>>(
      "daemon://stream",
      (event) => {
        if (options?.signal?.aborted) return;
        if (event.payload.request_id === requestId) {
          options?.onRecord?.(event.payload);
        }
      },
    );

    try {
      return await invoke<DaemonEnvelope<T>>("daemon_invoke", {
        request: { ...req, request_id: requestId },
      });
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
  async stream<T = unknown, R = unknown>(
    req: DaemonRequest,
    options?: DaemonStreamOptions<R>,
  ): Promise<DaemonEnvelope<T>> {
    const requestId = req.request_id ?? makeDaemonRequestId();
    const response = await fetch("/__kassiber__/daemon/stream", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ ...req, request_id: requestId }),
    });
    return readBridgeNdjsonStream<T, R>(
      response,
      req.kind,
      requestId,
      options,
    );
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
