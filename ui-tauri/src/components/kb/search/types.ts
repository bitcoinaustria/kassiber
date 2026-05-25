import type {
  AppRoutePath,
  SettingsMenuSection,
} from "@/components/kb/menuIntent";

export const SEARCH_RESULT_CATEGORIES = [
  "page",
  "action",
  "transaction",
  "wallet",
  "report",
  "review_item",
  "setting",
] as const;

export type SearchResultCategory = (typeof SEARCH_RESULT_CATEGORIES)[number];

export type SearchPrivacyTier =
  | "public"
  | "local_metadata"
  | "book_private"
  | "secret";

export type SearchIconKey =
  | "activity"
  | "assistant"
  | "book"
  | "database"
  | "file_search"
  | "ledger"
  | "lock"
  | "logs"
  | "report"
  | "search"
  | "settings"
  | "shield"
  | "sync"
  | "transaction"
  | "wallet";

export type SearchMatchExactness =
  | "exact"
  | "prefix"
  | "token"
  | "contains";

export type SearchRankReason =
  | "exact_txid"
  | "transaction_candidate"
  | "title"
  | "subtitle"
  | "keyword"
  | "metadata";

export type SearchRouteTarget = {
  to: AppRoutePath | "/connections/$connectionId";
  params?: Record<string, string | number | boolean | null | undefined>;
  search?: Record<string, string | number | boolean | null | undefined>;
  hash?: string;
};

export type SearchActionId =
  | "add-wallet"
  | "import-btcpay"
  | "process-journals";

export type SearchActionTarget = {
  id: SearchActionId;
  label?: string;
  requiresConsent?: boolean;
  args?: Record<string, unknown>;
};

export type SearchResultMetadata = {
  transactionId?: string | null;
  externalId?: string | null;
  explorerId?: string | null;
  walletId?: string | null;
  walletKind?: string | null;
  reportId?: string | null;
  reviewItemId?: string | null;
  settingSection?: SettingsMenuSection | null;
  sourceKind?: string | null;
  searchTokens?: readonly string[];
};

export type SearchRankingHints = {
  priority?: number;
  exactness?: SearchMatchExactness;
  reason?: SearchRankReason;
};

export type SearchResult = {
  id: string;
  category: SearchResultCategory;
  title: string;
  subtitle?: string;
  keywords?: readonly string[];
  iconKey?: SearchIconKey | string;
  route?: SearchRouteTarget;
  action?: SearchActionTarget;
  metadata?: SearchResultMetadata;
  privacyTier?: SearchPrivacyTier;
  ranking?: SearchRankingHints;
};

export type RankedSearchResult = SearchResult & {
  match: {
    score: number;
    exactness: SearchMatchExactness;
    reason: SearchRankReason;
    matchedText: string;
  };
};
