import {
  MOCK_OVERVIEW,
  type Connection,
  type MarketRateSnapshot,
  type PortfolioPoint,
  type Tx,
} from "@/mocks/seed";
import { MOCK_PROFILES, type Workspace } from "@/mocks/profiles";

export interface WorkspaceBookBoundary {
  id: string;
  label: string;
}

export type WorkspaceConnection = Connection & {
  workspaceId: string;
  workspaceLabel: string;
  profileId: string;
  profileLabel: string;
  book: WorkspaceBookBoundary;
};

export type WorkspaceTx = Tx & {
  workspaceId: string;
  workspaceLabel: string;
  profileId: string;
  profileLabel: string;
  book: WorkspaceBookBoundary;
};

export interface WorkspaceFiatBookRow {
  profileId: string;
  profileLabel: string;
  fiatCurrency: string;
  balance: number;
  costBasis: number;
  unrealized: number;
  realizedYTD: number;
}

export interface WorkspaceFiatRollup {
  mode: "empty" | "single" | "mixed";
  fiatCurrency: string | null;
  currencies: string[];
  mixed: boolean;
  partial: boolean;
  eurBalance: number | null;
  eurCostBasis: number | null;
  eurUnrealized: number | null;
  eurRealizedYTD: number | null;
  btcBalance: number;
  books: WorkspaceFiatBookRow[];
  label?: string;
}

export interface WorkspaceBookOverview {
  profile: {
    id: string;
    label: string;
    fiatCurrency: string;
    taxCountry?: string;
    taxLongTermDays?: number;
    gainsAlgorithm?: string;
  };
  workspace: {
    id: string;
    label: string;
  };
  connections: Connection[];
  txs: Tx[];
  activityTxs: Tx[];
  balanceSeries: number[];
  portfolioSeries: PortfolioPoint[];
  fiat: {
    fiatCurrency?: string | null;
    eurBalance: number;
    eurCostBasis: number;
    eurUnrealized: number;
    eurRealizedYTD: number;
  };
  marketRate?: MarketRateSnapshot;
  status: {
    workspace: string | null;
    profile: string | null;
    workspaceId: string;
    profileId: string;
    transactionCount?: number;
    needsJournals: boolean;
    quarantines: number;
    journalEntryCount?: number;
    freshnessStatus?: string;
    freshnessReason?: string;
  };
  journals: {
    status: string;
    needs_processing: boolean;
    last_processed_at: string | null;
    last_processed_tx_count: number;
    journal_input_version: number;
    last_processed_input_version: number;
    active_transaction_count: number;
    journal_entry_count: number;
    quarantine_count: number;
    reason: string;
  };
  readiness: {
    ready: boolean;
    hints: string[];
  };
}

export interface WorkspaceOverviewSnapshot {
  workspace: { id: string; label: string } | null;
  scope: { kind: "workspace"; label: "Book set" };
  books: WorkspaceBookOverview[];
  connections: WorkspaceConnection[];
  txs: WorkspaceTx[];
  activityTxs: WorkspaceTx[];
  balanceSeries: number[];
  portfolioSeries: Array<
    Partial<PortfolioPoint> & {
      date: string;
      label: string;
      balanceBtc: number;
      books?: Array<{
        profileId: string;
        profileLabel: string;
        fiatCurrency: string;
        balanceBtc: number;
        value: number;
        costBasis: number;
      }>;
    }
  >;
  fiat: WorkspaceFiatRollup;
  status: {
    workspace: string | null;
    workspaceId: string | null;
    bookCount: number;
    transactionCount: number;
    needsJournals: boolean;
    quarantines: number;
    ready: boolean;
    readyBooks?: number;
    blockedBooks?: number;
    mixedFiat: boolean;
  };
}

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function workspaceById(workspaceId: string): Workspace {
  return (
    MOCK_PROFILES.workspaces.find((workspace) => workspace.id === workspaceId) ??
    MOCK_PROFILES.workspaces[0]
  );
}

function bookOverview(workspace: Workspace, profileIndex: number): WorkspaceBookOverview {
  const profile = workspace.profiles[profileIndex] ?? workspace.profiles[0];
  const overview = clone(MOCK_OVERVIEW);
  const scale = profileIndex + 1;
  const fiatCurrency = profile.fiatCurrency ?? workspace.currency;
  overview.fiat = {
    ...overview.fiat,
    fiatCurrency,
    eurBalance: overview.fiat.eurBalance / scale,
    eurCostBasis: overview.fiat.eurCostBasis / scale,
    eurUnrealized: overview.fiat.eurUnrealized / scale,
    eurRealizedYTD: overview.fiat.eurRealizedYTD / scale,
  };
  overview.connections = overview.connections.slice(0, profileIndex === 0 ? 3 : 2);
  overview.txs = overview.txs.slice(0, 5);
  overview.activityTxs = overview.activityTxs?.slice(0, 5);
  return {
    profile: {
      id: profile.id,
      label: profile.name,
      fiatCurrency,
      taxCountry: profile.taxCountry,
      taxLongTermDays: profile.taxLongTermDays,
      gainsAlgorithm: profile.gainsAlgorithm,
    },
    workspace: { id: workspace.id, label: workspace.name },
    connections: overview.connections,
    txs: overview.txs,
    activityTxs: overview.activityTxs ?? overview.txs,
    balanceSeries: overview.balanceSeries.map((value) => value / scale),
    portfolioSeries: (overview.portfolioSeries ?? []).map((point) => ({
      ...point,
      balanceBtc: point.balanceBtc / scale,
      valueEur: point.valueEur / scale,
      costBasisEur: point.costBasisEur / scale,
    })),
    fiat: overview.fiat,
    marketRate: overview.marketRate,
    status: {
      workspace: workspace.name,
      profile: profile.name,
      workspaceId: workspace.id,
      profileId: profile.id,
      transactionCount: overview.status?.transactionCount ?? overview.txs.length,
      needsJournals: profileIndex > 0,
      quarantines: profileIndex > 0 ? 1 : 0,
      journalEntryCount: profileIndex > 0 ? 12 : 40,
      freshnessStatus: profileIndex > 0 ? "stale" : "current",
      freshnessReason:
        profileIndex > 0
          ? "active transaction count changed since last processing"
          : "journals match the active transaction count and input version",
    },
    journals: {
      status: profileIndex > 0 ? "stale" : "current",
      needs_processing: profileIndex > 0,
      last_processed_at: profileIndex > 0 ? "2026-04-17T08:00:00Z" : "2026-04-18T08:00:00Z",
      last_processed_tx_count: profileIndex > 0 ? 4 : 5,
      journal_input_version: profileIndex > 0 ? 3 : 2,
      last_processed_input_version: profileIndex > 0 ? 2 : 2,
      active_transaction_count: overview.status?.transactionCount ?? overview.txs.length,
      journal_entry_count: profileIndex > 0 ? 12 : 40,
      quarantine_count: profileIndex > 0 ? 1 : 0,
      reason:
        profileIndex > 0
          ? "active transaction count changed since last processing"
          : "journals match the active transaction count and input version",
    },
    readiness: {
      ready: profileIndex === 0,
      hints:
        profileIndex === 0
          ? ["Reports are ready from the current processed journal state."]
          : [
              "Run journal processing before trusting reports.",
              "Review quarantined transactions before tax export.",
            ],
    },
  };
}

export function mockWorkspaceOverviewSnapshot(
  workspaceId = MOCK_PROFILES.activeWorkspaceId ?? "w1",
): WorkspaceOverviewSnapshot {
  const workspace = workspaceById(workspaceId);
  const books = workspace.profiles.map((_, index) => bookOverview(workspace, index));
  const currencies = Array.from(
    new Set(books.map((book) => book.profile.fiatCurrency).filter(Boolean)),
  ).sort();
  const sameFiat = currencies.length <= 1;
  const connections = books.flatMap((book) =>
    book.connections.map((connection) => ({
      ...connection,
      workspaceId: workspace.id,
      workspaceLabel: workspace.name,
      profileId: book.profile.id,
      profileLabel: book.profile.label,
      book: { id: book.profile.id, label: book.profile.label },
    })),
  );
  const txs = books.flatMap((book) =>
    book.txs.map((tx) => ({
      ...tx,
      workspaceId: workspace.id,
      workspaceLabel: workspace.name,
      profileId: book.profile.id,
      profileLabel: book.profile.label,
      book: { id: book.profile.id, label: book.profile.label },
    })),
  );
  const btcBalance = connections.reduce(
    (total, connection) => total + connection.balance,
    0,
  );
  const fiatBooks = books.map((book) => ({
    profileId: book.profile.id,
    profileLabel: book.profile.label,
    fiatCurrency: book.profile.fiatCurrency,
    balance: book.fiat.eurBalance,
    costBasis: book.fiat.eurCostBasis,
    unrealized: book.fiat.eurUnrealized,
    realizedYTD: book.fiat.eurRealizedYTD,
  }));
  return {
    workspace: { id: workspace.id, label: workspace.name },
    scope: { kind: "workspace", label: "Book set" },
    books,
    connections,
    txs,
    activityTxs: txs,
    balanceSeries: books[0]?.balanceSeries.map((_, index) =>
      books.reduce((total, book) => total + (book.balanceSeries[index] ?? 0), 0),
    ) ?? [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    portfolioSeries: books[0]?.portfolioSeries.map((point, index) => ({
      date: point.date,
      label: point.label,
      balanceBtc: books.reduce(
        (total, book) => total + (book.portfolioSeries[index]?.balanceBtc ?? 0),
        0,
      ),
      valueEur: sameFiat
        ? books.reduce(
            (total, book) => total + (book.portfolioSeries[index]?.valueEur ?? 0),
            0,
          )
        : undefined,
      costBasisEur: sameFiat
        ? books.reduce(
            (total, book) =>
              total + (book.portfolioSeries[index]?.costBasisEur ?? 0),
            0,
          )
        : undefined,
    })) ?? [],
    fiat: sameFiat
      ? {
          mode: "single",
          fiatCurrency: currencies[0] ?? null,
          currencies,
          mixed: false,
          partial: false,
          eurBalance: fiatBooks.reduce((total, row) => total + row.balance, 0),
          eurCostBasis: fiatBooks.reduce((total, row) => total + row.costBasis, 0),
          eurUnrealized: fiatBooks.reduce((total, row) => total + row.unrealized, 0),
          eurRealizedYTD: fiatBooks.reduce((total, row) => total + row.realizedYTD, 0),
          btcBalance,
          books: fiatBooks,
        }
      : {
          mode: "mixed",
          fiatCurrency: null,
          currencies,
          mixed: true,
          partial: true,
          eurBalance: null,
          eurCostBasis: null,
          eurUnrealized: null,
          eurRealizedYTD: null,
          btcBalance,
          books: fiatBooks,
          label: "Mixed fiat currencies; per-book fiat rows are shown without conversion.",
        },
    status: {
      workspace: workspace.name,
      workspaceId: workspace.id,
      bookCount: books.length,
      transactionCount: books.reduce(
        (total, book) => total + (book.status.transactionCount ?? 0),
        0,
      ),
      needsJournals: books.some((book) => book.journals.needs_processing),
      quarantines: books.reduce(
        (total, book) => total + book.journals.quarantine_count,
        0,
      ),
      ready: books.every((book) => book.readiness.ready),
      readyBooks: books.filter((book) => book.readiness.ready).length,
      blockedBooks: books.filter((book) => !book.readiness.ready).length,
      mixedFiat: !sameFiat,
    },
  };
}

export const MOCK_WORKSPACE_OVERVIEW = mockWorkspaceOverviewSnapshot();
