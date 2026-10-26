import type { ToolFormatId } from "@/components/transactions/dashboard/exportFormats";

/** Tools a book can be checked against; each is also an export format. */
export const CROSS_CHECK_TOOLS: ToolFormatId[] = ["cointracking", "blockpit"];

/** A tool export chosen for a cross-check: its name and bytes, never a path. */
export interface CrossCheckFile {
  name: string;
  base64: string;
}

/** Columns only one tool's export has. */
const SIGNATURES: [ToolFormatId, string[]][] = [
  ["blockpit", ["integration name", "outgoing asset", "incoming asset"]],
  ["cointracking", ["buy currency", "sell currency"]],
];

/**
 * Which tool wrote this CSV, from its header row; null when unsure. Only the
 * first few kilobytes are decoded, so a large export costs nothing here.
 */
export function detectCrossCheckTool(base64: string): ToolFormatId | null {
  let head: string;
  try {
    const chunk = base64.slice(0, 8192 - (8192 % 4)).replace(/=+$/, "");
    head = atob(chunk.slice(0, chunk.length - (chunk.length % 4)));
  } catch {
    return null;
  }
  const firstLine =
    head
      .replace(/^\uFEFF|^\u00EF\u00BB\u00BF/, "")
      .split(/\r?\n/)
      .find((line) => line.trim()) ?? "";
  const header = firstLine.toLowerCase();
  for (const [tool, columns] of SIGNATURES) {
    if (columns.every((column) => header.includes(column))) return tool;
  }
  return null;
}
