/**
 * Ranked model search for the provider/model picker.
 *
 * Adapted from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
 * `packages/shared/src/searchRanking.ts` and
 * `apps/web/src/components/chat/modelPickerSearch.ts`. Kassiber indexes its own
 * model metadata (id, display name, owner, OpenCode source namespace) and the
 * provider's name, and searches only what is already known locally — typing a
 * query never asks a provider for its models.
 */

export function normalizeSearchQuery(input: string): string {
  return input.trim().toLowerCase();
}

/** Lower is better. Null when `query` is not a subsequence of `value`. */
export function scoreSubsequenceMatch(value: string, query: string): number | null {
  if (!query) return 0;

  let queryIndex = 0;
  let firstMatchIndex = -1;
  let previousMatchIndex = -1;
  let gapPenalty = 0;

  for (let valueIndex = 0; valueIndex < value.length; valueIndex += 1) {
    if (value[valueIndex] !== query[queryIndex]) continue;

    if (firstMatchIndex === -1) firstMatchIndex = valueIndex;
    if (previousMatchIndex !== -1) {
      gapPenalty += valueIndex - previousMatchIndex - 1;
    }

    previousMatchIndex = valueIndex;
    queryIndex += 1;
    if (queryIndex === query.length) {
      const spanPenalty = valueIndex - firstMatchIndex + 1 - query.length;
      const lengthPenalty = Math.min(64, value.length - query.length);
      return firstMatchIndex * 2 + gapPenalty * 3 + spanPenalty + lengthPenalty;
    }
  }

  return null;
}

function lengthPenalty(value: string, query: string): number {
  return Math.min(64, Math.max(0, value.length - query.length));
}

const BOUNDARY_MARKERS = [" ", "-", "_", "/", ":", "."] as const;

function findBoundaryMatchIndex(value: string, query: string): number | null {
  let bestIndex: number | null = null;
  for (const marker of BOUNDARY_MARKERS) {
    const index = value.indexOf(`${marker}${query}`);
    if (index === -1) continue;
    const matchIndex = index + marker.length;
    if (bestIndex === null || matchIndex < bestIndex) bestIndex = matchIndex;
  }
  return bestIndex;
}

/**
 * Tiered match: exact, then prefix, then word boundary, then substring, then
 * (for tokens of three or more characters) an in-order fuzzy match. Inputs must
 * already be normalized.
 */
export function scoreQueryMatch(input: {
  value: string;
  query: string;
  exactBase: number;
  prefixBase: number;
  boundaryBase: number;
  includesBase: number;
  fuzzyBase?: number;
}): number | null {
  const { value, query } = input;
  if (!value || !query) return null;
  if (value === query) return input.exactBase;
  if (value.startsWith(query)) {
    return input.prefixBase + lengthPenalty(value, query);
  }
  const boundaryIndex = findBoundaryMatchIndex(value, query);
  if (boundaryIndex !== null) {
    return input.boundaryBase + boundaryIndex * 2 + lengthPenalty(value, query);
  }
  const includesIndex = value.indexOf(query);
  if (includesIndex !== -1) {
    return input.includesBase + includesIndex * 2 + lengthPenalty(value, query);
  }
  if (input.fuzzyBase !== undefined) {
    const fuzzyScore = scoreSubsequenceMatch(value, query);
    if (fuzzyScore !== null) return input.fuzzyBase + fuzzyScore;
  }
  return null;
}

export interface ModelPickerSearchable {
  /** Model id as sent to the provider. */
  id: string;
  displayName?: string | null;
  ownedBy?: string | null;
  /** OpenCode-style source namespace (`omlx`, `ollama`, ...). */
  sourceProvider?: string | null;
  /** Provider row name (routing key) and its display name. */
  providerName: string;
  providerDisplayName: string;
  isFavorite?: boolean;
}

const FAVORITE_SCORE_BOOST = 24;

export function buildModelPickerSearchText(model: ModelPickerSearchable): string {
  return normalizeSearchQuery(
    [
      model.displayName,
      model.id,
      model.ownedBy,
      model.sourceProvider,
      model.providerDisplayName,
      model.providerName,
    ]
      .filter((value): value is string => typeof value === "string" && value.length > 0)
      .join(" "),
  );
}

function searchFields(model: ModelPickerSearchable): string[] {
  // Field order is significant: earlier fields win ties, so a hit on the
  // model's own name outranks a hit on the provider that serves it.
  return [
    model.displayName,
    model.id,
    model.sourceProvider,
    model.ownedBy,
    model.providerDisplayName,
    model.providerName,
  ]
    .filter((value): value is string => typeof value === "string" && value.length > 0)
    .map(normalizeSearchQuery)
    .concat(buildModelPickerSearchText(model));
}

/**
 * Lower is better; null means at least one token matched nothing. Every token
 * must match some field, so "claude opus" narrows rather than widens.
 */
export function scoreModelPickerSearch(
  model: ModelPickerSearchable,
  query: string,
): number | null {
  const tokens = normalizeSearchQuery(query)
    .split(/\s+/u)
    .filter((token) => token.length > 0);
  if (tokens.length === 0) return 0;

  const fields = searchFields(model);
  let score = 0;
  for (const token of tokens) {
    let best: number | null = null;
    for (let index = 0; index < fields.length; index += 1) {
      const base = index * 10;
      const fieldScore = scoreQueryMatch({
        value: fields[index],
        query: token,
        exactBase: base,
        prefixBase: base + 2,
        boundaryBase: base + 4,
        includesBase: base + 6,
        ...(token.length >= 3 ? { fuzzyBase: base + 100 } : {}),
      });
      if (fieldScore !== null && (best === null || fieldScore < best)) {
        best = fieldScore;
      }
    }
    if (best === null) return null;
    score += best;
  }
  return model.isFavorite ? score - FAVORITE_SCORE_BOOST : score;
}

/**
 * Rank `items` against `query`, dropping non-matches. Ties keep favorites
 * first, then the caller's order (which is privacy-first), so equal matches
 * never reorder an on-device model below an off-device one.
 */
export function rankModelPickerItems<T>(
  items: readonly T[],
  query: string,
  toSearchable: (item: T) => ModelPickerSearchable,
): T[] {
  return items
    .map((item, index) => {
      const searchable = toSearchable(item);
      return {
        item,
        index,
        isFavorite: searchable.isFavorite === true,
        score: scoreModelPickerSearch(searchable, query),
      };
    })
    .filter(
      (entry): entry is typeof entry & { score: number } => entry.score !== null,
    )
    .sort((a, b) => {
      if (a.score !== b.score) return a.score - b.score;
      if (a.isFavorite !== b.isFavorite) return a.isFavorite ? -1 : 1;
      return a.index - b.index;
    })
    .map((entry) => entry.item);
}
