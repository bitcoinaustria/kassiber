/**
 * Remembered column sorts, one per table, persisted with the UI store
 * (`tableSorts`). Pure types and validation; `useRememberedTableSort` binds
 * them to the store. Stored values are untrusted: a table only accepts a sort
 * whose key it still offers.
 */

export const TABLE_SORT_IDS = ["wallets", "review", "transactions"] as const;
export type TableSortId = (typeof TABLE_SORT_IDS)[number];

export type TableSortDirection = "asc" | "desc";

export interface TableSort<K extends string = string> {
  key: K;
  direction: TableSortDirection;
}

/** `null` records a deliberately unsorted table (the transactions default). */
export type TableSorts = Partial<Record<TableSortId, TableSort | null>>;

function isTableSortId(value: string): value is TableSortId {
  return (TABLE_SORT_IDS as readonly string[]).includes(value);
}

export function normalizeTableSort<K extends string>(
  value: unknown,
  keys: readonly K[],
): TableSort<K> | null {
  if (!value || typeof value !== "object") return null;
  const { key, direction } = value as { key?: unknown; direction?: unknown };
  if (typeof key !== "string" || !(keys as readonly string[]).includes(key)) {
    return null;
  }
  if (direction !== "asc" && direction !== "desc") return null;
  return { key: key as K, direction };
}

/** Keeps known tables with a well-formed sort (or an explicit `null`). */
export function normalizeTableSorts(value: unknown): TableSorts {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const out: TableSorts = {};
  for (const [table, sort] of Object.entries(value as Record<string, unknown>)) {
    if (!isTableSortId(table)) continue;
    if (sort === null) {
      out[table] = null;
      continue;
    }
    if (!sort || typeof sort !== "object") continue;
    const { key, direction } = sort as { key?: unknown; direction?: unknown };
    if (typeof key !== "string" || key === "") continue;
    if (direction !== "asc" && direction !== "desc") continue;
    out[table] = { key, direction };
  }
  return out;
}
