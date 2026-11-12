import * as React from "react";

import {
  normalizeTableSort,
  type TableSort,
  type TableSortId,
} from "./tableSort";
import { useUiStore } from "./ui";

/**
 * A table's column sort, remembered across navigation and restarts. Returns
 * the stored sort when it names a key in `keys`, an explicitly stored `null`
 * (unsorted), or `fallback` when nothing usable is stored.
 */
export function useRememberedTableSort<K extends string>(
  table: TableSortId,
  keys: readonly K[],
  fallback: TableSort<K> | null,
): [TableSort<K> | null, (sort: TableSort<K> | null) => void] {
  const stored = useUiStore((state) => state.tableSorts[table]);
  const setTableSort = useUiStore((state) => state.setTableSort);
  const sort = React.useMemo(() => {
    if (stored === null) return null;
    return normalizeTableSort(stored, keys) ?? fallback;
    // Pass `keys` and `fallback` as module constants so this stays stable.
  }, [stored, keys, fallback]);
  const setSort = React.useCallback(
    (next: TableSort<K> | null) => setTableSort(table, next),
    [setTableSort, table],
  );
  return [sort, setSort];
}
