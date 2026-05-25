import type {
  RankedSearchResult,
  SearchMatchExactness,
  SearchPrivacyTier,
  SearchRankReason,
  SearchResult,
  SearchResultCategory,
} from "./types";
import { isTransactionLookupQuery } from "@/lib/transactionLookup";

type QueryProfile = {
  raw: string;
  normalized: string;
  terms: string[];
  looksLikeTransactionId: boolean;
};

type TextMatch = {
  exactness: SearchMatchExactness;
  reason: SearchRankReason;
  score: number;
  matchedText: string;
};

export type RankSearchOptions = {
  limit?: number;
  maxPrivacyTier?: SearchPrivacyTier;
};

const PRIVACY_ORDER: Record<SearchPrivacyTier, number> = {
  public: 0,
  local_metadata: 1,
  book_private: 2,
  secret: 3,
};

const EXACTNESS_SCORE: Record<SearchMatchExactness, number> = {
  exact: 800,
  prefix: 500,
  token: 320,
  contains: 160,
};

const CATEGORY_SCORE: Record<SearchResultCategory, number> = {
  action: 90,
  page: 85,
  setting: 75,
  report: 70,
  wallet: 65,
  review_item: 60,
  transaction: 55,
};

export function normalizeSearchQuery(query: string): QueryProfile {
  const normalized = query.trim().toLowerCase();
  return {
    raw: query,
    normalized,
    terms: normalized.split(/\s+/).filter(Boolean),
    looksLikeTransactionId: isLikelyTransactionLookupQuery(query),
  };
}

export function isLikelyTransactionLookupQuery(query: string) {
  return isTransactionLookupQuery(query);
}

export function rankSearchResult(
  result: SearchResult,
  query: string,
): RankedSearchResult | null {
  const profile = normalizeSearchQuery(query);
  if (!profile.terms.length) return null;

  const exactTxidMatch = exactTransactionIdentifier(result, profile.normalized);
  if (exactTxidMatch) {
    return withMatch(result, {
      score: 20_000 + priorityScore(result),
      exactness: "exact",
      reason: "exact_txid",
      matchedText: exactTxidMatch,
    });
  }

  const transactionCandidateMatch = transactionCandidateIdentifier(
    result,
    profile,
  );
  if (transactionCandidateMatch) {
    return withMatch(result, {
      score: 15_000 + priorityScore(result),
      exactness: "prefix",
      reason: "transaction_candidate",
      matchedText: transactionCandidateMatch,
    });
  }

  const textMatch = bestTextMatch(result, profile);
  if (!textMatch) return null;

  const score =
    CATEGORY_SCORE[result.category] +
    textMatch.score +
    priorityScore(result);

  return withMatch(result, {
    ...textMatch,
    score,
  });
}

export function rankSearchResults(
  results: readonly SearchResult[],
  query: string,
  options: RankSearchOptions = {},
) {
  const maxPrivacyTier = options.maxPrivacyTier ?? "book_private";
  const ranked = results
    .filter((result) => privacyAllowed(result, maxPrivacyTier))
    .map((result) => rankSearchResult(result, query))
    .filter((result): result is RankedSearchResult => result !== null)
    .sort(compareRankedResults);

  return typeof options.limit === "number"
    ? ranked.slice(0, Math.max(0, options.limit))
    : ranked;
}

export function searchResultMatches(result: SearchResult, query: string) {
  return rankSearchResult(result, query) !== null;
}

function compareRankedResults(a: RankedSearchResult, b: RankedSearchResult) {
  if (b.match.score !== a.match.score) return b.match.score - a.match.score;
  const titleOrder = a.title.localeCompare(b.title);
  if (titleOrder !== 0) return titleOrder;
  return a.id.localeCompare(b.id);
}

function privacyAllowed(result: SearchResult, maxPrivacyTier: SearchPrivacyTier) {
  const tier = result.privacyTier ?? "book_private";
  return PRIVACY_ORDER[tier] <= PRIVACY_ORDER[maxPrivacyTier];
}

function priorityScore(result: SearchResult) {
  const priority = result.ranking?.priority ?? 0;
  if (!Number.isFinite(priority)) return 0;
  return Math.max(-100, Math.min(100, priority));
}

function exactTransactionIdentifier(result: SearchResult, normalizedQuery: string) {
  if (result.category !== "transaction") return null;
  return transactionIdentifiers(result).find(
    (value) => value.trim().toLowerCase() === normalizedQuery,
  );
}

function transactionIdentifiers(result: SearchResult) {
  return [
    result.metadata?.transactionId,
    result.metadata?.externalId,
    result.metadata?.explorerId,
    ...identifierLikeKeywords(result.keywords ?? []),
    ...identifierLikeKeywords(result.metadata?.searchTokens ?? []),
  ].filter((value): value is string => Boolean(value));
}

function identifierLikeKeywords(keywords: readonly string[]) {
  return keywords.filter((keyword) => isLikelyTransactionLookupQuery(keyword));
}

function transactionCandidateIdentifier(
  result: SearchResult,
  profile: QueryProfile,
) {
  if (result.category !== "transaction" || !profile.looksLikeTransactionId) {
    return null;
  }
  return transactionIdentifiers(result).find((value) => {
    const normalized = value.trim().toLowerCase();
    return (
      normalized.startsWith(profile.normalized) ||
      profile.normalized.startsWith(normalized)
    );
  });
}

function bestTextMatch(result: SearchResult, profile: QueryProfile) {
  const candidates: TextMatch[] = [];
  for (const field of searchableFields(result)) {
    const match = matchTextField(field.value, field.reason, profile);
    if (match) candidates.push(match);
  }
  if (!candidates.length) return null;
  return candidates.sort((a, b) => b.score - a.score)[0];
}

function searchableFields(result: SearchResult) {
  return [
    { value: result.title, reason: "title" as const },
    { value: result.subtitle ?? "", reason: "subtitle" as const },
    ...(result.keywords ?? []).map((value) => ({
      value,
      reason: "keyword" as const,
    })),
    ...(result.metadata?.searchTokens ?? []).map((value) => ({
      value,
      reason: "metadata" as const,
    })),
    ...(result.action?.label
      ? [{ value: result.action.label, reason: "metadata" as const }]
      : []),
  ];
}

function matchTextField(
  value: string,
  reason: SearchRankReason,
  profile: QueryProfile,
): TextMatch | null {
  const normalized = value.trim().toLowerCase();
  if (!normalized) return null;
  if (normalized === profile.normalized) {
    return textMatch("exact", reason, value);
  }
  if (normalized.startsWith(profile.normalized)) {
    return textMatch("prefix", reason, value);
  }

  const tokens = normalized.split(/[^a-z0-9:_-]+/).filter(Boolean);
  if (
    profile.terms.every((term) =>
      tokens.some((token) => token === term || token.startsWith(term)),
    )
  ) {
    return textMatch("token", reason, value);
  }

  if (profile.terms.every((term) => normalized.includes(term))) {
    return textMatch("contains", reason, value);
  }
  return null;
}

function textMatch(
  exactness: SearchMatchExactness,
  reason: SearchRankReason,
  matchedText: string,
) {
  return {
    exactness,
    reason,
    score: EXACTNESS_SCORE[exactness],
    matchedText,
  };
}

function withMatch(result: SearchResult, match: RankedSearchResult["match"]) {
  return {
    ...result,
    match,
  };
}
