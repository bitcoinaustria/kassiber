import { describe, expect, it } from "vitest";

import i18n from "@/i18n";
import {
  buildAppSearchResults,
  isLikelyTransactionLookupQuery,
  rankSearchResults,
  searchResultForActivation,
  transactionLookupLabelForState,
  transactionLookupStateForQuery,
  type SearchResult,
} from ".";
import type { OverviewSnapshot } from "@/mocks/seed";

// vitest.setup initializes i18n to English, so EN titles resolve and the
// existing English assertions below still hold. Dynamic, prefixed keys fall
// outside the typed-key union, so adapt the fixed-T to the search builder's
// small structural translator shape.
const fixedT = i18n.getFixedT("en", null);
const t = (key: string, options?: Record<string, unknown>) =>
  fixedT(key as never, options as never) as unknown;
const fixedDeT = i18n.getFixedT("de", null);
const deT = (key: string, options?: Record<string, unknown>) =>
  fixedDeT(key as never, options as never) as unknown;

const results: SearchResult[] = [
  {
    id: "page:transactions",
    category: "page",
    title: "Transactions",
    subtitle: "Transaction rows and filters",
    keywords: [
      "tx",
      "counterparty",
      "account",
      "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    ],
    iconKey: "transaction",
    route: { to: "/transactions" },
    privacyTier: "public",
  },
  {
    id: "action:sync",
    category: "action",
    title: "Sync wallets",
    subtitle: "Refresh local wallet rows",
    keywords: ["scan", "refresh", "connection"],
    iconKey: "sync",
    action: { id: "process-journals", requiresConsent: true },
    privacyTier: "local_metadata",
  },
  {
    id: "tx:exact",
    category: "transaction",
    title: "Deposit from customer",
    subtitle: "Cold Storage - Income",
    keywords: ["transaction"],
    iconKey: "transaction",
    route: { to: "/transactions", search: { tx: "tx-exact" } },
    metadata: {
      transactionId: "tx-exact",
      explorerId:
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      searchTokens: ["customer deposit"],
    },
    privacyTier: "book_private",
  },
  {
    id: "tx:candidate",
    category: "transaction",
    title: "0123456789ab candidate",
    subtitle: "Multisig Vault - Transfer",
    keywords: ["transaction"],
    iconKey: "transaction",
    route: { to: "/transactions", search: { tx: "tx-candidate" } },
    metadata: {
      transactionId: "tx-candidate",
      searchTokens: ["0123456789ab"],
    },
    privacyTier: "book_private",
  },
  {
    id: "setting:secret",
    category: "setting",
    title: "Reveal descriptor",
    subtitle: "Sensitive wallet material",
    keywords: ["descriptor"],
    privacyTier: "secret",
  },
];

describe("search result ranking", () => {
  it("orders exact txid matches ahead of transaction candidates and page matches", () => {
    const ranked = rankSearchResults(
      results,
      "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    );

    expect(ranked.map((result) => result.id).slice(0, 3)).toEqual([
      "tx:exact",
      "tx:candidate",
      "page:transactions",
    ]);
    expect(ranked[0].match.reason).toBe("exact_txid");
    expect(ranked[1].match.reason).toBe("transaction_candidate");
  });

  it("matches page and action results with multi-term queries", () => {
    const ranked = rankSearchResults(results, "sync wallet");

    expect(ranked[0]).toMatchObject({
      id: "action:sync",
      category: "action",
    });
  });

  it("keeps secret-tier results out of normal local search", () => {
    expect(rankSearchResults(results, "descriptor")).toHaveLength(0);
    expect(
      rankSearchResults(results, "descriptor", { maxPrivacyTier: "secret" }),
    ).toHaveLength(1);
  });

  it("recognizes the transaction lookup queries AppShell already issues", () => {
    expect(isLikelyTransactionLookupQuery("0123456789ab")).toBe(true);
    expect(isLikelyTransactionLookupQuery("tx:local-123")).toBe(true);
    expect(
      isLikelyTransactionLookupQuery("550e8400-e29b-41d4-a716-446655440000"),
    ).toBe(true);
    expect(isLikelyTransactionLookupQuery("transactions")).toBe(false);
  });
});

describe("transaction lookup labels", () => {
  it("labels lookup states without persisting query text", () => {
    const query = "tx:abc123";

    expect(
      transactionLookupLabelForState(
        transactionLookupStateForQuery(query, { isFetching: true }),
      ),
    ).toEqual({ state: "looking_up", label: "Looking up transaction" });
    expect(
      transactionLookupLabelForState(
        transactionLookupStateForQuery(query, {
          resolved: { query, transaction: { id: query } },
        }),
      ),
    ).toEqual({ state: "matched", label: "Transaction found" });
    expect(
      transactionLookupLabelForState(transactionLookupStateForQuery(query)),
    ).toEqual({ state: "not_found", label: "No local transaction match" });
    expect(
      transactionLookupLabelForState(
        transactionLookupStateForQuery("reports"),
      ),
    ).toEqual({
      state: "idle",
      label: "Search pages, actions, and local data",
    });
  });
});

describe("app search results", () => {
  const txid =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
  const snapshot = {
    priceEur: 0,
    priceUsd: 0,
    connections: [
      {
        id: "wallet-1",
        label: "Cold Storage",
        kind: "descriptor",
        last: "2026-04-18",
        balance: 1,
        status: "synced",
      },
    ],
    txs: [
      {
        id: "tx1",
        explorerId: txid,
        date: "2026-04-18 14:22",
        type: "Income",
        account: "Cold Storage",
        counter: "ACME GmbH",
        amountSat: 1,
        eur: 1,
        rate: 1,
        tag: "Revenue",
        conf: 6,
      },
      {
        id: "tx12",
        explorerId: `${txid.slice(0, 20)}ffffffffffffffffffffffffffffffffffffffffffff`,
        date: "2026-04-19 14:22",
        type: "Income",
        account: "Cold Storage",
        counter: "Shop",
        amountSat: 1,
        eur: 1,
        rate: 1,
        tag: "Revenue",
        conf: 6,
      },
    ],
    balanceSeries: [],
    fiat: {
      eurBalance: 0,
      eurCostBasis: 0,
      eurUnrealized: 0,
      eurRealizedYTD: 0,
    },
  } as OverviewSnapshot;

  it("puts resolved exact txid results first and opens the transaction route", () => {
    const ranked = buildAppSearchResults({
      snapshot,
      query: txid.toUpperCase(),
      aiFeaturesEnabled: true,
      developerToolsEnabled: true,
      resolvedTransaction: {
        query: txid.toUpperCase(),
        transaction: {
          id: "tx1",
          explorerId: txid,
          account: "Cold Storage",
          type: "Income",
          date: "2026-04-18 14:22",
        },
      },
      t,
    });

    expect(ranked[0]).toMatchObject({
      id: "tx:resolved:tx1",
      title: "Open exact transaction",
      route: { to: "/transactions", search: { tx: "tx1" } },
    });
  });

  it("lets in-flight txid lookups open the transaction detail deep link", () => {
    const unknownTxid =
      "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd";
    const ranked = buildAppSearchResults({
      snapshot: { ...snapshot, txs: [] },
      query: unknownTxid,
      aiFeaturesEnabled: true,
      developerToolsEnabled: true,
      isResolvingTransaction: true,
      t,
    });

    expect(ranked[0]).toMatchObject({
      id: "lookup:transaction:loading",
      category: "transaction",
      route: { to: "/transactions", search: { tx: unknownTxid } },
    });
    expect(searchResultForActivation(ranked, 0)).toMatchObject({
      id: "lookup:transaction:loading",
      route: { to: "/transactions", search: { tx: unknownTxid } },
    });
  });

  it("surfaces multiple partial transaction matches before the candidate rows", () => {
    const ranked = buildAppSearchResults({
      snapshot,
      query: txid.slice(0, 12),
      aiFeaturesEnabled: true,
      developerToolsEnabled: true,
      resolvedTransaction: { query: txid.slice(0, 12), transaction: null },
      t,
    });

    expect(ranked[0]).toMatchObject({
      id: "lookup:transaction:multiple",
      title: "Multiple transaction matches",
    });
    expect(ranked.filter((result) => result.category === "transaction")).toHaveLength(2);
    expect(searchResultForActivation(ranked, 0)).toMatchObject({
      id: "tx:recent:tx1",
      route: { to: "/transactions", search: { tx: "tx1" } },
    });
  });

  it("finds wallet connection suggestions and opens the wallet detail route", () => {
    const ranked = buildAppSearchResults({
      snapshot,
      query: "cold wallet",
      aiFeaturesEnabled: true,
      developerToolsEnabled: true,
      t,
    });

    expect(ranked[0]).toMatchObject({
      id: "wallet:wallet-1",
      category: "wallet",
      title: "Cold Storage",
      route: {
        to: "/connections/$connectionId",
        params: { connectionId: "wallet-1" },
      },
    });
    expect(searchResultForActivation(ranked, 0)).toMatchObject({
      id: "wallet:wallet-1",
      route: {
        to: "/connections/$connectionId",
        params: { connectionId: "wallet-1" },
      },
    });
  });

  it("finds screens with page and screen wording", () => {
    const ranked = buildAppSearchResults({
      snapshot,
      query: "reports screen",
      aiFeaturesEnabled: true,
      developerToolsEnabled: true,
      t,
    });

    expect(ranked[0]).toMatchObject({
      id: "page:reports",
      category: "page",
      route: { to: "/reports" },
    });
  });

  it("finds localized screen aliases", () => {
    const ranked = buildAppSearchResults({
      snapshot,
      query: "Berichte Ansicht",
      aiFeaturesEnabled: true,
      developerToolsEnabled: true,
      t: deT,
    });

    expect(ranked[0]).toMatchObject({
      id: "page:reports",
      category: "page",
      route: { to: "/reports" },
    });
  });

  it("keeps English page-title aliases under a localized UI", () => {
    const ranked = buildAppSearchResults({
      snapshot,
      query: "reports screen",
      aiFeaturesEnabled: true,
      developerToolsEnabled: true,
      t: deT,
    });

    expect(ranked[0]).toMatchObject({
      id: "page:reports",
      category: "page",
      route: { to: "/reports" },
    });
  });

  it("finds localized wallet connection aliases", () => {
    const ranked = buildAppSearchResults({
      snapshot,
      query: "Cold Storage Verbindung",
      aiFeaturesEnabled: true,
      developerToolsEnabled: true,
      t: deT,
    });

    expect(ranked[0]).toMatchObject({
      id: "wallet:wallet-1",
      category: "wallet",
      route: {
        to: "/connections/$connectionId",
        params: { connectionId: "wallet-1" },
      },
    });
  });

  it("shows a no-local-match state for txid-looking queries", () => {
    const ranked = buildAppSearchResults({
      snapshot,
      query: "abcdefabcdefabcdef",
      aiFeaturesEnabled: true,
      developerToolsEnabled: true,
      resolvedTransaction: { query: "abcdefabcdefabcdef", transaction: null },
      t,
    });

    expect(ranked[0]).toMatchObject({
      id: "lookup:transaction:not-found",
      title: "No local transaction match",
    });
    expect(searchResultForActivation(ranked, 0)).toBeNull();
  });

  it("finds local actions and settings sections", () => {
    const ranked = buildAppSearchResults({
      snapshot,
      query: "change passphrase",
      aiFeaturesEnabled: true,
      developerToolsEnabled: true,
      t,
    });

    expect(ranked[0]).toMatchObject({
      id: "action:change-passphrase",
      category: "action",
      route: { to: "/settings", hash: "security" },
    });
  });

  it("only shows the logs action when developer tools are enabled", () => {
    const baseOptions = {
      snapshot,
      query: "open logs",
      aiFeaturesEnabled: true,
      t,
    };

    expect(
      buildAppSearchResults({
        ...baseOptions,
        developerToolsEnabled: false,
      }).some((result) => result.id === "action:open-logs"),
    ).toBe(false);
    expect(
      buildAppSearchResults({
        ...baseOptions,
        developerToolsEnabled: true,
      })[0],
    ).toMatchObject({
      id: "action:open-logs",
      route: { to: "/logs" },
    });
  });
});
