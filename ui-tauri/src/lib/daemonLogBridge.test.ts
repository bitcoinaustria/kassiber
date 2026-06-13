import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { clearAppLogRecords, getAppLogRecords } from "./appLogs";
import type { DaemonEnvelope, DaemonTransport } from "@/daemon/transport";
import {
  DAEMON_LOG_BRIDGE_BACKOFF_MS,
  DAEMON_LOG_BRIDGE_INTERVAL_MS,
  DAEMON_LOG_BRIDGE_MAX_PAGES_PER_TICK,
  DAEMON_LOG_BRIDGE_PAGE_LIMIT,
  startDaemonLogBridge,
  stopDaemonLogBridge,
} from "./daemonLogBridge";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

interface RingSnapshot {
  records: Array<Record<string, unknown>>;
  last_id: number;
  gap: boolean;
  started_at: string;
  buffer_bytes: number;
  max_bytes: number;
}

function snapshot(partial: Partial<RingSnapshot>): DaemonEnvelope<RingSnapshot> {
  return {
    kind: "ui.logs.snapshot",
    schema_version: 1,
    data: {
      records: [],
      last_id: 0,
      gap: false,
      started_at: "S1",
      buffer_bytes: 0,
      max_bytes: 4194304,
      ...partial,
    },
  };
}

function ringRecord(id: number, startedAt = "S1"): Record<string, unknown> {
  return {
    id,
    ts: `2026-06-12T19:00:0${id % 10}.000Z`,
    level: "info",
    module: "kassiber.daemon",
    file: "kassiber/daemon.py",
    line: 100 + id,
    msg: `record ${id} under ${startedAt}`,
    fields: {},
  };
}

// A transport whose invoke() pops the next queued envelope (last one repeats),
// recording every request so tests can assert cursor advancement.
function queuedTransport(
  queue: DaemonEnvelope<RingSnapshot>[],
): DaemonTransport & { calls: Array<Record<string, unknown>> } {
  const calls: Array<Record<string, unknown>> = [];
  return {
    calls,
    invoke: (async (req: { kind: string; args?: Record<string, unknown> }) => {
      calls.push(req.args ?? {});
      const next = queue.length > 1 ? queue.shift()! : queue[0];
      return next;
    }) as DaemonTransport["invoke"],
    stream: (async () => {
      throw new Error("stream is not used by the bridge");
    }) as DaemonTransport["stream"],
  };
}

const baseOptions = (transport: DaemonTransport) => ({
  isEnabled: () => true,
  transport: () => transport,
  documentVisible: () => true,
  visibilityTarget: undefined,
});

describe("daemon log bridge", () => {
  beforeEach(() => {
    clearAppLogRecords();
    vi.useFakeTimers();
    vi.mocked(tauriInvoke).mockReset();
  });

  afterEach(() => {
    stopDaemonLogBridge();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("folds daemon records with daemon-<started_at>-<id> ids and advances the cursor", async () => {
    const transport = queuedTransport([
      snapshot({ records: [ringRecord(4), ringRecord(5)], last_id: 5 }),
      snapshot({ records: [], last_id: 5 }),
    ]);
    startDaemonLogBridge(baseOptions(transport));

    await vi.advanceTimersByTimeAsync(1);
    const ids = getAppLogRecords().map((record) => record.id);
    expect(ids).toEqual(["daemon-S1-4", "daemon-S1-5"]);

    await vi.advanceTimersByTimeAsync(DAEMON_LOG_BRIDGE_INTERVAL_MS);
    // First tick advanced the cursor to last_id (5); the second poll uses it.
    expect(transport.calls.at(-1)).toMatchObject({ after_id: 5 });
  });

  it("resets the cursor when started_at changes (daemon restart)", async () => {
    const transport = queuedTransport([
      snapshot({ records: [ringRecord(7)], last_id: 7, started_at: "S1" }),
      // The bridge polls at the stale cursor, detects the new started_at, and
      // discards this page before re-polling from 0.
      snapshot({ records: [], last_id: 1, started_at: "S2" }),
      snapshot({ records: [ringRecord(1, "S2")], last_id: 1, started_at: "S2" }),
    ]);
    startDaemonLogBridge(baseOptions(transport));

    await vi.advanceTimersByTimeAsync(1);
    expect(transport.calls[0]).toMatchObject({ after_id: 0 });

    await vi.advanceTimersByTimeAsync(DAEMON_LOG_BRIDGE_INTERVAL_MS);
    // The restart tick polls at the stale cursor (7), detects the new
    // started_at, resets to 0, and re-polls — so a post-first-tick poll uses
    // after_id 0 and the folded record carries the new started_at in its id.
    expect(transport.calls.slice(1).some((call) => call.after_id === 0)).toBe(
      true,
    );
    expect(getAppLogRecords().map((record) => record.id)).toContain(
      "daemon-S2-1",
    );
  });

  it("emits a single warning and backs off when polling fails", async () => {
    const transport: DaemonTransport = {
      invoke: (async () => {
        throw new Error("daemon offline");
      }) as DaemonTransport["invoke"],
      stream: (async () => {
        throw new Error("unused");
      }) as DaemonTransport["stream"],
    };
    startDaemonLogBridge(baseOptions(transport));

    await vi.advanceTimersByTimeAsync(1);
    await vi.advanceTimersByTimeAsync(DAEMON_LOG_BRIDGE_BACKOFF_MS);

    const warnings = getAppLogRecords().filter(
      (record) => record.module === "daemon:bridge-poll",
    );
    expect(warnings).toHaveLength(1);
    expect(warnings[0]?.level).toBe("warning");
  });

  it("notes a gap exactly once when daemon records were evicted", async () => {
    const transport = queuedTransport([
      snapshot({ records: [ringRecord(9)], last_id: 9, gap: true }),
      snapshot({ records: [], last_id: 9, gap: false }),
    ]);
    startDaemonLogBridge(baseOptions(transport));

    await vi.advanceTimersByTimeAsync(1);
    const notices = getAppLogRecords().filter(
      (record) => record.module === "daemon:bridge",
    );
    expect(notices).toHaveLength(1);
    expect(notices[0]?.level).toBe("info");
  });

  it("bounds catch-up to the page cap within one tick", async () => {
    const fullPage = snapshot({
      records: Array.from({ length: DAEMON_LOG_BRIDGE_PAGE_LIMIT }, (_v, i) =>
        ringRecord(i + 1),
      ),
      last_id: 10_000,
    });
    const transport = queuedTransport([fullPage]);
    startDaemonLogBridge(baseOptions(transport));

    await vi.advanceTimersByTimeAsync(1);
    expect(transport.calls).toHaveLength(DAEMON_LOG_BRIDGE_MAX_PAGES_PER_TICK);
  });

  it("skips polling while disabled", async () => {
    const transport = queuedTransport([snapshot({})]);
    startDaemonLogBridge({ ...baseOptions(transport), isEnabled: () => false });

    await vi.advanceTimersByTimeAsync(1);
    await vi.advanceTimersByTimeAsync(DAEMON_LOG_BRIDGE_INTERVAL_MS);
    expect(transport.calls).toHaveLength(0);
  });

  it("folds supervisor lifecycle records with mapped levels in the Tauri runtime", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
    vi.mocked(tauriInvoke).mockResolvedValue({
      records: [
        { id: 1, tsMs: 1_760_000_000_000, event: "spawned", detail: "", stderrTail: "", source: "bundled_sidecar" },
        { id: 2, tsMs: 1_760_000_001_000, event: "exited", detail: "status: 0", stderrTail: "", source: "bundled_sidecar" },
        { id: 3, tsMs: 1_760_000_002_000, event: "killed", detail: "daemon_timeout", stderrTail: "boom", source: "bundled_sidecar" },
      ],
      lastId: 3,
    });
    const transport = queuedTransport([snapshot({})]);
    startDaemonLogBridge(baseOptions(transport));

    // The supervisor poll resolves through a dynamic import() whose microtask
    // chain settles unpredictably under fake timers; flush until the records
    // are actually folded (the true end-state) rather than a fixed count, all
    // well within the 4s reschedule so no second tick runs.
    const supervisorRecords = () =>
      getAppLogRecords().filter((record) => record.module === "supervisor");
    for (let i = 0; i < 50 && supervisorRecords().length === 0; i += 1) {
      await vi.advanceTimersByTimeAsync(1);
    }
    const supervisor = supervisorRecords();
    expect(supervisor.map((record) => [record.id, record.level])).toEqual([
      ["super-1", "info"],
      ["super-2", "info"],
      ["super-3", "error"],
    ]);
    const killed = supervisor.find((record) => record.id === "super-3");
    expect(killed?.fields.stderr_tail).toEqual({ type: "text", value: "boom" });
  });
});
