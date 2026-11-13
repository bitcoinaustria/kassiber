import type { BookChange } from "@/daemon/bookRefresh";

export type BookFreshnessState =
  | { kind: "updating"; reason: "syncing" | "processing" | "loading" }
  | { kind: "failed" }
  | { kind: "needs-journals" }
  | { kind: "current"; change: BookChange | null };

export interface BookFreshnessInputs {
  syncing: boolean;
  processingJournals: boolean;
  loadingChanges: boolean;
  refreshFailed: boolean;
  needsJournals: boolean;
  lastChange: BookChange | null;
}

/** One state for the whole book, most urgent first. */
export function bookFreshnessState(inputs: BookFreshnessInputs): BookFreshnessState {
  if (inputs.syncing) return { kind: "updating", reason: "syncing" };
  if (inputs.processingJournals) return { kind: "updating", reason: "processing" };
  if (inputs.loadingChanges) return { kind: "updating", reason: "loading" };
  if (inputs.refreshFailed) return { kind: "failed" };
  if (inputs.needsJournals) return { kind: "needs-journals" };
  return { kind: "current", change: inputs.lastChange };
}
