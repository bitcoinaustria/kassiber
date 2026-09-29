import { create } from "zustand";

import type { ConnectionHealthStatus } from "@/lib/connectionHealth";

export type ConnectionHealthRecord = {
  status: ConnectionHealthStatus;
  fingerprint?: string;
  message?: string;
  checkedAt?: string;
};

/** How often the opt-in automatic check probes the book's connections. */
export const AUTOMATIC_CONNECTION_CHECK_INTERVAL_MS = 5 * 60 * 1000;

interface ConnectionHealthState {
  records: Record<string, ConnectionHealthRecord>;
  checking: boolean;
  /** When the last automatic round started, so a remount does not re-probe. */
  lastAutomaticCheckAt: number | null;
  setChecking: (checking: boolean) => void;
  recordResults: (results: Array<[string, ConnectionHealthRecord]>) => void;
  markAutomaticCheck: (at: number) => void;
}

// Deliberately not persisted: results carry endpoint-derived messages, and a
// status from a previous launch says nothing about the connection now. The
// store only outlives the title-bar panel, which remounts with the shell.
export const useConnectionHealthStore = create<ConnectionHealthState>()(
  (set) => ({
    records: {},
    checking: false,
    lastAutomaticCheckAt: null,
    setChecking: (checking) => set({ checking }),
    recordResults: (results) =>
      set((state) => {
        const records = { ...state.records };
        for (const [id, record] of results) {
          records[id] = record;
        }
        return { records };
      }),
    markAutomaticCheck: (at) => set({ lastAutomaticCheckAt: at }),
  }),
);

/** A record describes the row only while the row's endpoint is unchanged. */
export function currentHealthRecord(
  records: Record<string, ConnectionHealthRecord>,
  row: { id: string; fingerprint: string },
): ConnectionHealthRecord | undefined {
  const record = records[row.id];
  if (!record || record.fingerprint !== row.fingerprint) return undefined;
  return record;
}

export function automaticCheckDelay(
  lastCheckAt: number | null,
  now: number,
  interval = AUTOMATIC_CONNECTION_CHECK_INTERVAL_MS,
): number {
  if (lastCheckAt === null) return 0;
  return Math.max(0, lastCheckAt + interval - now);
}
