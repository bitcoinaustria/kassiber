import type { ToolFormatId } from "@/components/transactions/dashboard/exportFormats";

import type { CrossCheckFile } from "./crossCheckFile";

export type CrossCheckView = "attention" | "matched" | "all";

/**
 * The comparison being worked through, kept while the user opens one of its
 * transactions and comes back. Memory only, for the same book session and
 * data mode; the result itself returns from the query cache.
 */
export interface CrossCheckMemory {
  session: string;
  file: CrossCheckFile;
  tool: ToolFormatId | null;
  year: number | null;
  pairKey: string;
  view: CrossCheckView;
  query: string;
}

let memory: CrossCheckMemory | null = null;

export function keptCrossCheck(session: string): CrossCheckMemory | null {
  return memory?.session === session ? memory : null;
}

export function keepCrossCheck(next: CrossCheckMemory | null) {
  memory = next;
}

/** Drop the kept comparison (tests start from an empty picker). */
export function forgetCrossCheck() {
  memory = null;
}
