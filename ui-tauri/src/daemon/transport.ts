/**
 * Daemon transport selector.
 *
 * Three runtime modes per docs/plan/04-desktop-ui.md §2.6:
 *   - "mock"   — fixture responses, no Python required (default dev mode)
 *   - "bridge" — authenticated localhost WebSocket, dev-only (later)
 *   - "tauri"  — JSONL over stdin/stdout via Rust supervisor (production)
 *
 * Today only "mock" is implemented; the other two land alongside the
 * daemon (Phase 1.1) and the Rust supervisor (Phase 1.2) respectively.
 */

import { mockDaemon } from "./mock";

export type DaemonMode = "mock" | "bridge" | "tauri";

const RAW_MODE = (import.meta.env.VITE_DAEMON ?? "mock") as string;

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

export function getTransport(): DaemonTransport {
  switch (DAEMON_MODE) {
    case "mock":
      return mockDaemon;
    case "bridge":
      throw new Error("bridge transport not yet implemented (Phase 1.1)");
    case "tauri":
      throw new Error("tauri transport not yet implemented (Phase 1.2)");
  }
}
