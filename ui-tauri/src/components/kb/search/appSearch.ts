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
  /**
   * Translator that can resolve `nav:`, `search:`, and `settings:` prefixed
   * keys. Localizes result titles/subtitles/alias arrays so a German user's
   * visible-term query (e.g. "Transaktionen") matches the localized title.
   *
   * Typed structurally rather than as a namespace-branded `TFunction` so any
   * `useTranslation([...])` / `getFixedT` instance can be passed; result keys
   * are built dynamically and resolved with a `never` cast at each call site.
   */
  t: AppTranslate;
};

/** Minimal translator shape used by the localized builders. */
type AppTranslate = (
  key: string,
  options?: Record<string, unknown>,
) => unknown;

const SEARCH_LIMIT = 8;
const DEFAULT_PAGE_ALIAS_TEMPLATES = [
  "{{title}} page",
  "{{title}} screen",
  "{{title}} view",
  "open {{title}}",
  "go to {{title}}",
];
const DEFAULT_CONNECTION_LABEL_ALIAS_TEMPLATES = [
  "{{label}} wallet",
  "{{label}} connection",
  "{{label}} detail",
  "{{label}} wallet detail",
  "open {{label}}",
  "open {{label}} wallet",
  "open {{label}} detail",
];
const DEFAULT_CONNECTION_KIND_ALIAS_TEMPLATES = [
  "{{kind}} wallet",
  "{{kind}} connection",
];
const DEFAULT_CONNECTION_STATUS_ALIAS_TEMPLATES = ["{{status}} wallet"];
const DEFAULT_CONNECTION_NETWORK_ALIAS_TEMPLATES = ["{{network}} wallet"];

/**
 * Page id → existing `nav:book.*` title key, reused where the side-nav already
 * names the surface. Pages with no nav key fall back to `search:page.<id>.title`.
 */
const PAGE_NAV_TITLE_KEYS: Record<string, string> = {
  "page:overview": "nav:book.overview",
  "page:transactions": "nav:book.transactions",
  "page:connections": "nav:book.wallets",
  "page:journals": "nav:book.ledger",
  "page:reports": "nav:book.reports",
  "page:quarantine": "nav:book.quarantine",
  "page:egress": "nav:book.egress",
  "page:source-of-funds": "nav:book.sourceFunds",
  "page:swaps-transfers": "nav:book.swaps",
  "page:logs": "nav:book.logs",
  "page:assistant": "nav:book.assistant",
  "page:exit-tax": "nav:book.exitTax",
  "page:settings": "nav:book.settings",
};

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
    subtitle: "Books, book-set overview, and tax settings",
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
    id: "page:exit-tax",
    category: "page",
    title: "Exit Tax",
    subtitle: "Wegzugsbesteuerung deemed-disposal estimate",
    keywords: [
      "exit",
      "exit-tax",
      "wegzug",
      "wegzugsbesteuerung",
      "departure",
      "leaving",
      "emigration",
      "relocation",
      "deemed disposal",
    ],
    iconKey: "report",
    route: { to: "/exit-tax" },
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
    id: "page:egress",
    category: "page",
    title: "Egress",
    subtitle: "Outbound connection ledger",
    keywords: ["egress", "network", "privacy", "telemetry", "hosts"],
    iconKey: "shield",
    route: { to: "/egress" },
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
  t,
}: BuildAppSearchOptions): RankedSearchResult[] {
  if (!query.trim()) return [];

  const safeResults = [
    ...resolvedTransactionResults(resolvedTransaction, query),
    ...PAGE_RESULTS.filter((result) => {
      if (!aiFeaturesEnabled && result.route?.to === "/assistant") return false;
      if (!developerToolsEnabled && result.route?.to === "/logs") return false;
      return true;
    }).map((result) => localizePageResult(result, t)),
    ...ACTION_RESULTS.filter((result) => {
      if (!developerToolsEnabled && result.id === "action:open-logs") return false;
      return true;
    }).map((result) => localizeActionResult(result, t)),
    ...settingsResults(t),
    ...snapshotResults(snapshot, query, t),
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

/** Strip the `page:` / `action:` prefix to get the bare id used in i18n keys. */
function bareId(resultId: string): string {
  const sep = resultId.indexOf(":");
  return sep === -1 ? resultId : resultId.slice(sep + 1);
}

/**
 * Append an extra keyword without dropping the existing (English) keywords —
 * English queries must keep matching even when the UI is German.
 */
function withLocalizedKeyword(
  keywords: readonly string[] | undefined,
  localizedTitle: string,
): string[] {
  return uniqueKeywords(keywords ?? [], [localizedTitle]);
}

function uniqueKeywords(
  ...groups: Array<readonly string[] | undefined>
): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  for (const group of groups) {
    for (const keyword of group ?? []) {
      const normalized = keyword.trim().toLowerCase();
      if (!normalized || seen.has(normalized)) continue;
      out.push(keyword);
      seen.add(normalized);
    }
  }
  return out;
}

function translatedString(t: AppTranslate, key: string): string {
  const value = t(key);
  return typeof value === "string" ? value : "";
}

function translatedStringList(
  t: AppTranslate,
  key: string,
  variables: Record<string, string>,
): string[] {
  const value = t(key, { ...variables, returnObjects: true });
  if (Array.isArray(value)) {
    return value.filter((item): item is string => typeof item === "string");
  }
  if (typeof value === "string") {
    return value
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean);
  }
  return [];
}

function renderSearchPhraseTemplates(
  templates: readonly string[],
  variables: Record<string, string>,
): string[] {
  return templates.map((template) =>
    Object.entries(variables).reduce(
      (phrase, [key, value]) => phrase.replaceAll(`{{${key}}}`, value),
      template,
    ),
  );
}

function localizedAliasKeywords(
  t: AppTranslate,
  key: string,
  variables: Record<string, string>,
  fallbackTemplates: readonly string[],
): string[] {
  return uniqueKeywords(
    renderSearchPhraseTemplates(fallbackTemplates, variables),
    translatedStringList(t, key, variables),
  );
}

function localizePageResult(
  result: SearchResult,
  t: AppTranslate,
): SearchResult {
  const id = bareId(result.id);
  const titleKey = PAGE_NAV_TITLE_KEYS[result.id] ?? `search:page.${id}.title`;
  const title = translatedString(t, titleKey);
  const subtitle = translatedString(t, `search:page.${id}.subtitle`);
  return {
    ...result,
    title,
    subtitle,
    keywords: uniqueKeywords(
      result.keywords,
      [title],
      localizedAliasKeywords(
        t,
        "search:aliases.pagePhrases",
        { title },
        DEFAULT_PAGE_ALIAS_TEMPLATES,
      ),
    ),
  };
}

function localizeActionResult(
  result: SearchResult,
  t: AppTranslate,
): SearchResult {
  const id = bareId(result.id);
  const title = translatedString(t, `search:action.${id}.title`);
  const subtitle = translatedString(t, `search:action.${id}.subtitle`);
  return {
    ...result,
    title,
    subtitle,
    keywords: withLocalizedKeyword(result.keywords, title),
  };
}

function settingsResults(t: AppTranslate): SearchResult[] {
  const groupPrefix = translatedString(t, "search:settings.groupPrefix");
  return SETTINGS_SECTIONS.map((section) => {
    const title = translatedString(t, `settings:${section.labelKey}`);
    const descKey = section.labelKey.replace(/\.label$/, ".description");
    const description = translatedString(t, `settings:${descKey}`);
    return {
      id: `setting:${section.id}`,
      category: "setting" as const,
      title,
      subtitle: `${groupPrefix} · ${description}`,
      keywords: withLocalizedKeyword(
        [
          "settings",
          "preferences",
          section.slug,
          section.group,
          section.description,
          section.label,
        ],
        title,
      ),
      iconKey: "settings",
      route: { to: "/settings", hash: section.slug },
      metadata: {
        settingSection: section.slug as SettingsMenuSection,
        searchTokens: [section.id, section.slug],
      },
      privacyTier: "public" as const,
    };
  });
}

function snapshotResults(
  snapshot: OverviewSnapshot | undefined,
  query: string,
  t: AppTranslate,
): SearchResult[] {
  return [
    ...(snapshot?.connections.map((connection) =>
      connectionResult(t, connection),
    ) ?? []),
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

function connectionResult(
  t: AppTranslate,
  connection: OverviewSnapshot["connections"][number],
): SearchResult {
  const chainNetwork = [connection.chain, connection.network]
    .filter(Boolean)
    .join(" ");
  const searchTokens = uniqueKeywords(
    [connection.label, connection.kind, connection.status, chainNetwork],
    localizedAliasKeywords(
      t,
      "search:aliases.connectionLabelPhrases",
      { label: connection.label },
      DEFAULT_CONNECTION_LABEL_ALIAS_TEMPLATES,
    ),
    localizedAliasKeywords(
      t,
      "search:aliases.connectionKindPhrases",
      { kind: connection.kind },
      DEFAULT_CONNECTION_KIND_ALIAS_TEMPLATES,
    ),
    localizedAliasKeywords(
      t,
      "search:aliases.connectionStatusPhrases",
      { status: connection.status },
      DEFAULT_CONNECTION_STATUS_ALIAS_TEMPLATES,
    ),
    chainNetwork
      ? localizedAliasKeywords(
          t,
          "search:aliases.connectionNetworkPhrases",
          { network: chainNetwork },
          DEFAULT_CONNECTION_NETWORK_ALIAS_TEMPLATES,
        )
      : [],
  );
  return {
    id: `wallet:${connection.id}`,
    category: "wallet",
    title: connection.label,
    subtitle: [
      connection.kind.toUpperCase(),
      connection.chain,
      connection.network,
      connection.status,
    ]
      .filter(Boolean)
      .join(" · "),
    keywords: uniqueKeywords([
      "connection",
      "connections",
      "wallet",
      "wallets",
      "source",
      "sync",
      connection.kind,
      connection.status,
      connection.chain ?? "",
      connection.network ?? "",
    ]),
    iconKey: "wallet",
    route: {
      to: "/connections/$connectionId",
      params: { connectionId: connection.id },
    },
    metadata: {
      walletId: connection.id,
      walletKind: connection.kind,
      searchTokens,
    },
    privacyTier: "local_metadata",
    ranking: { priority: 10 },
  };
}

function transactionResult(tx: Tx, query: string): SearchResult {
  const partialTxidMatch = isPartialTransactionQuery(tx, query);
  return {
    id: `tx:recent:${tx.id}`,
    category: "transaction",
    title: partialTxidMatch
      ? "Open partial transaction match"
      : tx.counter
        ? `${tx.id} · ${tx.counter}`
        : tx.id,
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
    const transactionRef = query.trim();
    return {
      id: "lookup:transaction:loading",
      category: "transaction",
      title: "Looking up transaction",
      subtitle: "Open matching details when found",
      keywords: [query, "transaction", "txid", "lookup"],
      iconKey: "search",
      route: { to: "/transactions", search: { tx: transactionRef } },
      metadata: {
        transactionId: transactionRef,
      },
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
