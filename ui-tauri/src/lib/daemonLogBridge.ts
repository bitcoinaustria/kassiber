/**
 * Daemon/supervisor log bridge.
 *
 * Folds the Python daemon's RAM log ring (`ui.logs.snapshot`) and the Rust
 * supervisor's lifecycle ring (`daemon_lifecycle_snapshot`) into the webview
 * log ring so the Logs screen shows all three layers in one place. RAM-only
 * on every side; the transport wrapper and the daemon both skip logging the
 * poll itself so the bridge cannot feed back into the rings it reads.
 */

import { getTransport, type DaemonTransport } from "@/daemon/transport";
import {
  emitAppLog,
  type AppLogField,
  type AppLogLevel,
} from "@/lib/appLogs";

export const DAEMON_LOG_BRIDGE_INTERVAL_MS = 4_000;
export const DAEMON_LOG_BRIDGE_BACKOFF_MS = 30_000;
export const DAEMON_LOG_BRIDGE_PAGE_LIMIT = 500;
export const DAEMON_LOG_BRIDGE_MAX_PAGES_PER_TICK = 4;

interface DaemonRingRecord {
  id: number;
  ts: string;
  level: AppLogLevel;
  module: string;
  file: string;
  line: number;
  msg: string;
  fields: Record<string, AppLogField>;
}

interface DaemonRingSnapshot {
  records: DaemonRingRecord[];
  last_id: number;
  gap: boolean;
  started_at: string;
  buffer_bytes: number;
  max_bytes: number;
}

interface SupervisorLifecycleRecord {
  id: number;
  tsMs: number;
  event: string;
  detail: string;
  stderrTail: string;
  source: string;
}

interface SupervisorLifecycleSnapshot {
  records: SupervisorLifecycleRecord[];
  lastId: number;
}

export interface DaemonLogBridgeVisibilityTarget {
  addEventListener(type: "visibilitychange", listener: () => void): void;
  removeEventListener(type: "visibilitychange", listener: () => void): void;
}

export interface DaemonLogBridgeOptions {
  /** Re-checked every tick; polling is skipped while it returns false. */
  isEnabled: () => boolean;
  transport?: () => DaemonTransport;
  documentVisible?: () => boolean;
  visibilityTarget?: DaemonLogBridgeVisibilityTarget;
}

interface BridgeState {
  isEnabled: () => boolean;
  transport: () => DaemonTransport;
  documentVisible: () => boolean;
  cursor: number;
  startedAt: string | null;
  lifecycleCursor: number;
  failing: boolean;
  stopped: boolean;
  ticking: boolean;
  timer: ReturnType<typeof setTimeout> | null;
  removeVisibilityListener: (() => void) | null;
}

let activeBridge: BridgeState | null = null;

function defaultDocumentVisible(): boolean {
  return (
    typeof document === "undefined" || document.visibilityState === "visible"
  );
}

function defaultVisibilityTarget(): DaemonLogBridgeVisibilityTarget | undefined {
  return typeof document === "undefined" ? undefined : document;
}

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function startDaemonLogBridge(options: DaemonLogBridgeOptions): void {
  if (activeBridge) return;
  const state: BridgeState = {
    isEnabled: options.isEnabled,
    transport: options.transport ?? (() => getTransport()),
    documentVisible: options.documentVisible ?? defaultDocumentVisible,
    cursor: 0,
    startedAt: null,
    lifecycleCursor: 0,
    failing: false,
    stopped: false,
    ticking: false,
    timer: null,
    removeVisibilityListener: null,
  };
  activeBridge = state;

  const target = options.visibilityTarget ?? defaultVisibilityTarget();
  if (target) {
    const onVisibilityChange = () => {
      if (state.stopped || state.ticking || state.timer !== null) return;
      schedule(state, 0);
    };
    target.addEventListener("visibilitychange", onVisibilityChange);
    state.removeVisibilityListener = () =>
      target.removeEventListener("visibilitychange", onVisibilityChange);
  }

  schedule(state, 0);
}

export function stopDaemonLogBridge(): void {
  const state = activeBridge;
  if (!state) return;
  activeBridge = null;
  state.stopped = true;
  if (state.timer !== null) {
    clearTimeout(state.timer);
    state.timer = null;
  }
  state.removeVisibilityListener?.();
  state.removeVisibilityListener = null;
}

function schedule(state: BridgeState, delayMs: number): void {
  if (state.stopped || state.timer !== null) return;
  state.timer = setTimeout(() => {
    state.timer = null;
    void tick(state);
  }, delayMs);
}

async function tick(state: BridgeState): Promise<void> {
  if (state.stopped) return;
  state.ticking = true;
  try {
    if (!state.documentVisible()) {
      // Paused while hidden; the visibilitychange listener re-arms the loop.
      return;
    }
    if (!state.isEnabled()) {
      schedule(state, DAEMON_LOG_BRIDGE_INTERVAL_MS);
      return;
    }
    await pollDaemonRing(state);
    await pollSupervisorRing(state);
    if (state.stopped) return;
    state.failing = false;
    schedule(state, DAEMON_LOG_BRIDGE_INTERVAL_MS);
  } catch (error) {
    if (state.stopped) return;
    if (!state.failing) {
      state.failing = true;
      emitBridgeLog(
        "warning",
        "daemon:bridge-poll",
        "Daemon log poll failed; backing off",
        {
          error_message: {
            type: "text",
            value: error instanceof Error ? error.message : String(error),
          },
        },
      );
    }
    schedule(state, DAEMON_LOG_BRIDGE_BACKOFF_MS);
  } finally {
    state.ticking = false;
  }
}

function emitBridgeLog(
  level: AppLogLevel,
  module: string,
  msg: string,
  fields: Record<string, AppLogField>,
): void {
  emitAppLog({
    level,
    module,
    file: "lib/daemonLogBridge.ts",
    line: 0,
    msg,
    fields,
  });
}

async function pollDaemonRing(state: BridgeState): Promise<void> {
  const transport = state.transport();
  for (let page = 0; page < DAEMON_LOG_BRIDGE_MAX_PAGES_PER_TICK; page += 1) {
    const envelope = await transport.invoke<DaemonRingSnapshot>({
      kind: "ui.logs.snapshot",
      args: { after_id: state.cursor, limit: DAEMON_LOG_BRIDGE_PAGE_LIMIT },
    });
    if (state.stopped) return;
    if (envelope.error || !envelope.data) {
      throw new Error(
        envelope.error?.message ?? "ui.logs.snapshot returned no data",
      );
    }
    const data = envelope.data;
    if (state.startedAt !== data.started_at) {
      const firstSnapshot = state.startedAt === null;
      state.startedAt = data.started_at;
      if (!firstSnapshot) {
        // Daemon restart: ring ids start over, so the old cursor is stale.
        state.cursor = 0;
        continue;
      }
    }
    if (data.gap) {
      emitBridgeLog(
        "info",
        "daemon:bridge",
        "Daemon log records were evicted before the bridge caught up",
        {
          after_id: { type: "number", value: state.cursor },
          last_id: { type: "number", value: data.last_id },
        },
      );
    }
    for (const record of data.records) {
      emitAppLog({
        id: `daemon-${data.started_at}-${record.id}`,
        ts: record.ts,
        level: record.level,
        module: record.module,
        file: record.file,
        line: record.line,
        msg: record.msg,
        fields: record.fields,
      });
    }
    // A truncated page leaves records between the last returned id and
    // last_id; advance only as far as what was actually folded.
    const lastRecord = data.records[data.records.length - 1];
    state.cursor =
      data.records.length === DAEMON_LOG_BRIDGE_PAGE_LIMIT && lastRecord
        ? lastRecord.id
        : data.last_id;
    if (data.records.length < DAEMON_LOG_BRIDGE_PAGE_LIMIT) break;
  }
}

async function pollSupervisorRing(state: BridgeState): Promise<void> {
  if (!isTauriRuntime()) return;
  const { invoke } = await import("@tauri-apps/api/core");
  const snapshot = await invoke<SupervisorLifecycleSnapshot>(
    "daemon_lifecycle_snapshot",
    { afterId: state.lifecycleCursor },
  );
  if (state.stopped) return;
  for (const record of snapshot.records) {
    const fields: Record<string, AppLogField> = {
      source: { type: "text", value: record.source },
    };
    if (record.stderrTail) {
      fields.stderr_tail = { type: "text", value: record.stderrTail };
    }
    emitAppLog({
      id: `super-${record.id}`,
      ts: new Date(record.tsMs).toISOString(),
      level: lifecycleLevel(record),
      module: "supervisor",
      file: "supervisor.rs",
      line: 0,
      msg: record.detail ? `${record.event}: ${record.detail}` : record.event,
      fields,
    });
  }
  state.lifecycleCursor = snapshot.lastId;
}

function lifecycleLevel(record: SupervisorLifecycleRecord): AppLogLevel {
  if (record.event === "spawned") return "info";
  if (record.event === "exited" && /\bstatus:?\s*0\b/i.test(record.detail)) {
    return "info";
  }
  return "error";
}
