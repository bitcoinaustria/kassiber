import type { OverviewSnapshot, Tx } from "@/mocks/seed";
import { SETTINGS_SECTIONS } from "@/components/kb/settings/SettingsNavigation";
import type { SettingsMenuSection } from "@/components/kb/menuIntent";

import {
  isLikelyTransactionLookupQuery,
  rankSearchResults,
} from "./ranking";
import {
  transactionLookupStateForQuery,
  type ResolvedTransactionLookup,
} from "./transactions";
import type {
  RankedSearchResult,
  SearchResult,
} from "./types";

type BuildAppSearchOptions = {
  snapshot?: OverviewSnapshot;
  query: string;
  aiFeaturesEnabled: boolean;
  developerToolsEnabled: boolean;
  resolvedTransaction?: ResolvedTransactionLookup | null;
  isResolvingTransaction?: boolean;
  limit?: number;
};

const SEARCH_LIMIT = 8;

const PAGE_RESULTS: SearchResult[] = [
  {
    id: "page:overview",
    category: "page",
    title: "Overview",
    subtitle: "Portfolio, balance, activity",
    keywords: ["dashboard", "home", "balance", "portfolio"],
    iconKey: "activity",
    route: { to: "/overview" },
    privacyTier: "public",
  },
  {
    id: "page:transactions",
    category: "page",
    title: "Transactions",
    subtitle: "Transaction rows and filters",
    keywords: ["tx", "counterparty", "account", "amount", "import"],
    iconKey: "transaction",
    route: { to: "/transactions" },
    privacyTier: "public",
  },
  {
    id: "page:connections",
    category: "page",
    title: "Wallets",
    subtitle: "Wallets, imports, backends, and sync",
    keywords: ["connections", "wallets", "xpub", "backend", "sync"],
    iconKey: "wallet",
    route: { to: "/connections" },
    privacyTier: "public",
  },
  {
    id: "page:books",
    category: "page",
    title: "Books",
    subtitle: "Books, book sets, Bird's Eye, and tax settings",
    keywords: ["book", "books", "book set", "bird's eye", "overview", "tax", "country"],
    iconKey: "book",
    route: { to: "/books" },
    privacyTier: "public",
  },
  {
    id: "page:source-of-funds",
    category: "page",
    title: "Source of Funds",
    subtitle: "Wallet sources and local provenance summaries",
    keywords: ["source", "funds", "wallet", "balance", "provenance"],
    iconKey: "shield",
    route: { to: "/source-of-funds" },
    privacyTier: "public",
  },
  {
    id: "page:journals",
    category: "page",
    title: "Ledger",
    subtitle: "Processed tax ledger",
    keywords: ["journal", "process", "entries", "fees", "basis", "ledger"],
    iconKey: "ledger",
    route: { to: "/journals" },
    privacyTier: "public",
  },
  {
    id: "page:reports",
    category: "page",
    title: "Reports",
    subtitle: "Capital gains and exports",
    keywords: ["csv", "pdf", "xlsx", "tax", "austria", "e1kv"],
    iconKey: "report",
    route: { to: "/reports" },
    privacyTier: "public",
  },
  {
    id: "page:quarantine",
    category: "page",
    title: "Quarantine",
    subtitle: "Review ambiguous rows",
    keywords: ["review", "issues", "missing", "price"],
    iconKey: "shield",
    route: { to: "/quarantine" },
    privacyTier: "public",
  },
  {
    id: "page:swaps-transfers",
    category: "page",
    title: "Swaps & Transfers",
    subtitle: "Review candidate swap and transfer pairings",
    keywords: ["swap", "swaps", "transfer", "transfers", "review", "pair"],
    iconKey: "transaction",
    route: { to: "/swaps" },
    privacyTier: "public",
  },
  {
    id: "page:settings",
    category: "page",
    title: "Settings",
    subtitle: "Preferences, integrations, local data",
    keywords: ["preferences", "backends", "providers", "privacy", "lock"],
    iconKey: "settings",
    route: { to: "/settings" },
    privacyTier: "public",
  },
  {
    id: "page:logs",
    category: "page",
    title: "Logs",
    subtitle: "Typed local log stream and redacted troubleshooting export",
    keywords: ["log", "logs", "error", "daemon", "download"],
    iconKey: "logs",
    route: { to: "/logs" },
    privacyTier: "public",
  },
  {
    id: "page:assistant",
    category: "page",
    title: "Assistant",
    subtitle: "Ask Kassiber",
    keywords: ["chat", "ai", "tools"],
    iconKey: "assistant",
    route: { to: "/assistant" },
    privacyTier: "public",
  },
];

const ACTION_RESULTS: SearchResult[] = [
  {
    id: "action:process-journals",
    category: "action",
    title: "Process journals",
    subtitle: "Refresh local report-ready journal state",
    keywords: ["ledger", "journal", "journals", "process", "rebuild", "reports"],
    iconKey: "ledger",
    action: { id: "process-journals", label: "Process journals" },
    privacyTier: "public",
    ranking: { priority: 20 },
  },
  {
    id: "action:sync-wallets",
    category: "action",
    title: "Sync wallets",
    subtitle: "Open Connections to refresh watch-only sources",
    keywords: ["wallet", "wallets", "sync", "refresh", "connections"],
    iconKey: "sync",
    route: { to: "/connections" },
    privacyTier: "public",
  },
  {
    id: "action:add-wallet",
    category: "action",
    title: "Add wallet",
    subtitle: "Open the wallet connection dialog",
    keywords: ["wallet", "connection", "descriptor", "xpub", "add", "import"],
    iconKey: "wallet",
    action: { id: "add-wallet", label: "Add wallet" },
    privacyTier: "public",
  },
  {
    id: "action:import-btcpay",
    category: "action",
    title: "Import BTCPay",
    subtitle: "Open BTCPay wallet-source setup",
    keywords: ["btcpay", "merchant", "store", "invoice", "payment", "import"],
    iconKey: "wallet",
    action: { id: "import-btcpay", label: "Import BTCPay" },
    privacyTier: "public",
  },
  {
    id: "action:export-report",
    category: "action",
    title: "Export report",
    subtitle: "Open report export tools",
    keywords: ["report", "reports", "export", "pdf", "csv", "xlsx", "tax"],
    iconKey: "report",
    route: { to: "/reports" },
    privacyTier: "public",
  },
  {
    id: "action:open-logs",
    category: "action",
    title: "Open logs",
    subtitle: "Open redacted local troubleshooting logs",
    keywords: ["logs", "log", "daemon", "debug", "troubleshoot", "support"],
    iconKey: "logs",
    route: { to: "/logs" },
    privacyTier: "public",
  },
  {
    id: "action:change-passphrase",
    category: "action",
    title: "Change passphrase",
    subtitle: "Open lock and encryption settings",
    keywords: ["password", "passphrase", "security", "lock", "encryption"],
    iconKey: "lock",
    route: { to: "/settings", hash: "security" },
    metadata: { settingSection: "security" },
    privacyTier: "public",
  },
];

export function buildAppSearchResults({
  snapshot,
  query,
  aiFeaturesEnabled,
  developerToolsEnabled,
  resolvedTransaction,
  isResolvingTransaction,
  limit = SEARCH_LIMIT,
}: BuildAppSearchOptions): RankedSearchResult[] {
  if (!query.trim()) return [];

  const safeResults = [
    ...resolvedTransactionResults(resolvedTransaction, query),
    ...PAGE_RESULTS.filter((result) => {
      if (!aiFeaturesEnabled && result.route?.to === "/assistant") return false;
      if (!developerToolsEnabled && result.route?.to === "/logs") return false;
      return true;
    }),
    ...ACTION_RESULTS.filter((result) => {
      if (!developerToolsEnabled && result.id === "action:open-logs") return false;
      return true;
    }),
    ...settingsResults(),
    ...snapshotResults(snapshot, query),
  ];

  const resolvedTransactionId = resolvedTransaction?.transaction?.id ?? null;
  const ranked = rankSearchResults(safeResults, query)
    .filter(
      (result) =>
        !(
          resolvedTransactionId &&
          result.category === "transaction" &&
          result.id !== `tx:resolved:${resolvedTransactionId}` &&
          result.metadata?.transactionId === resolvedTransactionId
        ),
    )
    .slice(0, limit);
  const lookupState = transactionLookupStateForQuery(query, {
    isFetching: isResolvingTransaction,
    resolved: resolvedTransaction,
  });
  const transactionRanked = ranked.filter(
    (result) => result.category === "transaction",
  );
  const statusResult = transactionLookupStatusResult({
    query,
    lookupState,
    transactionMatchCount: transactionRanked.length,
  });
  if (!statusResult) return ranked;

  const rankedStatus = rankSearchResults([statusResult], query, { limit: 1 })[0];
  if (!rankedStatus) return ranked;
  return [rankedStatus, ...ranked].slice(0, limit);
}

export function isSearchResultActivatable(
  result: RankedSearchResult | SearchResult | undefined,
) {
  return Boolean(result?.action || result?.route);
}

export function searchResultForActivation(
  results: readonly RankedSearchResult[],
  activeIndex: number,
) {
  const activeResult = results[activeIndex];
  if (isSearchResultActivatable(activeResult)) return activeResult;
  return (
    results.slice(activeIndex + 1).find(isSearchResultActivatable) ??
    results.slice(0, activeIndex).find(isSearchResultActivatable) ??
    null
  );
}

function resolvedTransactionResults(
  resolved: ResolvedTransactionLookup | null | undefined,
  query: string,
): SearchResult[] {
  const transaction = resolved?.transaction;
  if (!transaction) return [];
  if (resolved.query?.trim().toLowerCase() !== query.trim().toLowerCase()) {
    return [];
  }
  return [
    {
      id: `tx:resolved:${transaction.id}`,
      category: "transaction",
      title: "Open exact transaction",
      subtitle: [
        "Exact txid match",
        transaction.account,
        transaction.type,
        transaction.date,
      ]
        .filter(Boolean)
        .join(" · "),
      keywords: [
        "transaction",
        "tx",
        "txid",
        transaction.id,
        transaction.externalId ?? "",
        transaction.explorerId ?? "",
        transaction.counter ?? "",
      ],
      iconKey: "transaction",
      route: { to: "/transactions", search: { tx: transaction.id } },
      metadata: {
        transactionId: transaction.id,
        externalId: transaction.externalId,
        explorerId: transaction.explorerId,
        searchTokens: [transaction.counter ?? ""].filter(Boolean),
      },
      privacyTier: "book_private",
      ranking: { priority: 80 },
    },
  ];
}

function settingsResults(): SearchResult[] {
  return SETTINGS_SECTIONS.map((section) => ({
    id: `setting:${section.id}`,
    category: "setting" as const,
    title: section.label,
    subtitle: `${section.group} settings · ${section.description}`,
    keywords: [
      "settings",
      "preferences",
      section.slug,
      section.group,
      section.description,
      section.label,
    ],
    iconKey: "settings",
    route: { to: "/settings", hash: section.slug },
    metadata: {
      settingSection: section.slug as SettingsMenuSection,
      searchTokens: [section.id, section.slug],
    },
    privacyTier: "public" as const,
  }));
}

function snapshotResults(
  snapshot: OverviewSnapshot | undefined,
  query: string,
): SearchResult[] {
  return [
    ...(snapshot?.connections.map((connection) => ({
      id: `wallet:${connection.id}`,
      category: "wallet" as const,
      title: connection.label,
      subtitle: `${connection.kind.toUpperCase()} · ${connection.status}`,
      keywords: [
        "connection",
        "wallet",
        "sync",
        connection.kind,
        connection.status,
      ],
      iconKey: "wallet",
      route: {
        to: "/connections/$connectionId" as const,
        params: { connectionId: connection.id },
      },
      metadata: {
        walletId: connection.id,
        walletKind: connection.kind,
      },
      privacyTier: "local_metadata" as const,
    })) ?? []),
    ...(snapshot?.txs.map((tx) => transactionResult(tx, query)) ?? []),
    ...(snapshot?.status?.needsJournals
      ? [
          {
            id: "review:journals",
            category: "review_item" as const,
            title: "Ledger needs processing",
            subtitle: "Reports are stale until journal processing runs",
            keywords: ["journal", "reports", "stale", "process"],
            iconKey: "ledger",
            action: {
              id: "process-journals" as const,
              label: "Process journals",
            },
            privacyTier: "local_metadata" as const,
            ranking: { priority: 15 },
          },
        ]
      : []),
    ...((snapshot?.status?.quarantines ?? 0) > 0
      ? [
          {
            id: "review:quarantine",
            category: "review_item" as const,
            title: "Transactions quarantined",
            subtitle: `${snapshot?.status?.quarantines ?? 0} rows need review`,
            keywords: ["quarantine", "review", "missing", "price"],
            iconKey: "shield",
            route: { to: "/quarantine" as const },
            privacyTier: "local_metadata" as const,
          },
        ]
      : []),
  ];
}

function transactionResult(tx: Tx, query: string): SearchResult {
  const partialTxidMatch = isPartialTransactionQuery(tx, query);
  return {
    id: `tx:recent:${tx.id}`,
    category: "transaction",
    title: partialTxidMatch ? "Open partial transaction match" : `${tx.id} · ${tx.counter}`,
    subtitle: [
      partialTxidMatch ? "Partial txid match" : null,
      tx.account,
      tx.type,
      tx.tag,
    ]
      .filter(Boolean)
      .join(" · "),
    keywords: [
      "transaction",
      "transactions",
      "tx",
      tx.id,
      tx.externalId ?? "",
      tx.explorerId ?? "",
      tx.account,
      tx.counter,
      tx.type,
      tx.tag,
    ],
    iconKey: "transaction",
    route: { to: "/transactions", search: { tx: tx.id } },
    metadata: {
      transactionId: tx.id,
      externalId: tx.externalId,
      explorerId: tx.explorerId,
      searchTokens: [tx.account, tx.counter, tx.type, tx.tag],
    },
    privacyTier: "book_private",
    ranking: { priority: partialTxidMatch ? 40 : 0 },
  };
}

function transactionLookupStatusResult({
  query,
  lookupState,
  transactionMatchCount,
}: {
  query: string;
  lookupState: ReturnType<typeof transactionLookupStateForQuery>;
  transactionMatchCount: number;
}): SearchResult | null {
  if (lookupState === "idle" || lookupState === "matched") return null;
  if (transactionMatchCount > 1) {
    return {
      id: "lookup:transaction:multiple",
      category: "review_item",
      title: "Multiple transaction matches",
      subtitle: "Choose the matching transaction below",
      keywords: [query, "transaction", "txid", "multiple"],
      iconKey: "transaction",
      privacyTier: "local_metadata",
      ranking: { priority: 70 },
    };
  }
  if (transactionMatchCount === 1) return null;
  if (lookupState === "looking_up") {
    return {
      id: "lookup:transaction:loading",
      category: "review_item",
      title: "Looking up transaction",
      subtitle: "Checking local transaction rows",
      keywords: [query, "transaction", "txid", "lookup"],
      iconKey: "search",
      privacyTier: "local_metadata",
      ranking: { priority: 70 },
    };
  }
  return {
    id: "lookup:transaction:not-found",
    category: "review_item",
    title: "No local transaction match",
    subtitle: "This txid is not in the active book",
    keywords: [query, "transaction", "txid", "not found"],
    iconKey: "search",
    privacyTier: "local_metadata",
    ranking: { priority: 70 },
  };
}

function isPartialTransactionQuery(tx: Tx, query: string) {
  if (!isLikelyTransactionLookupQuery(query)) return false;
  const normalizedQuery = query.trim().toLowerCase();
  return [tx.id, tx.externalId, tx.explorerId].some((value) => {
    const normalized = value?.trim().toLowerCase();
    if (!normalized || normalized === normalizedQuery) return false;
    return (
      normalized.startsWith(normalizedQuery) ||
      normalizedQuery.startsWith(normalized)
    );
  });
}
