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

import { mockDaemon } from "./mock";
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

export interface DaemonTransport {
  invoke<T = unknown>(req: DaemonRequest): Promise<DaemonEnvelope<T>>;
}

const tauriDaemon: DaemonTransport = {
  async invoke<T = unknown>(
    req: DaemonRequest,
  ): Promise<DaemonEnvelope<T>> {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<DaemonEnvelope<T>>("daemon_invoke", { request: req });
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
};

export function getTransport(dataMode?: DataMode): DaemonTransport {
  if ((dataMode ?? useUiStore.getState().dataMode) === "mock") {
    return mockDaemon;
  }

  switch (DAEMON_MODE) {
    case "mock":
      return mockDaemon;
    case "bridge":
      return bridgeDaemon;
    case "tauri":
      return tauriDaemon;
  }
}
