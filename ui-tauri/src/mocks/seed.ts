/**
 * Mock data seeded from claude-design's MOCK constant in components/strings.jsx.
 *
 * These values exist only to drive the UI translation against realistic
 * shapes until the Pydantic→JSON Schema pipeline (Phase 1.2 §2.2) generates
 * fixtures from real `kassiber.core.api.contracts` models. At that point the
 * shapes here become test cases for the schema and these literals get
 * replaced with schema-driven factories.
 */

export type ConnectionStatus = "synced" | "syncing" | "idle" | "error";

export type ConnectionKind =
  | "xpub"
  | "address"
  | "descriptor"
  | "core-ln"
  | "lnd"
  | "nwc"
  | "cashu"
  | "btcpay"
  | "kraken"
  | "bitstamp"
  | "coinbase"
  | "bitpanda"
  | "river"
  | "strike"
  | "phoenix"
  | "custom"
  | "csv"
  | "bip329";

export interface Connection {
  id: string;
  kind: ConnectionKind;
  label: string;
  last: string;
  /** balance in BTC (float) */
  balance: number;
  status: ConnectionStatus;
  addresses?: number;
  gap?: number;
  channels?: number;
}

export type TxType =
  | "Income"
  | "Expense"
  | "Transfer"
  | "Fee"
  | "Swap"
  | "Mint"
  | "Melt"
  | "Consolidation"
  | "Rebalance";

export interface Tx {
  id: string;
  date: string;
  type: TxType;
  account: string;
  counter: string;
  amountSat: number;
  eur: number;
  rate: number;
  tag: string;
  conf: number;
  internal?: boolean;
}

export interface FiatSnapshot {
  eurBalance: number;
  eurCostBasis: number;
  eurUnrealized: number;
  eurRealizedYTD: number;
}

export interface PortfolioPoint {
  date: string;
  label: string;
  balanceBtc: number;
  valueEur: number;
  costBasisEur: number;
}

export interface OverviewSnapshot {
  priceEur: number;
  priceUsd: number;
  connections: Connection[];
  txs: Tx[];
  /** monthly-ish BTC totals across the span */
  balanceSeries: number[];
  /** dated portfolio points from the daemon, using real source dates/rates */
  portfolioSeries?: PortfolioPoint[];
  fiat: FiatSnapshot;
  status?: {
    workspace: string | null;
    profile: string | null;
    transactionCount?: number;
    needsJournals: boolean;
    quarantines: number;
  };
}

export const MOCK_OVERVIEW: OverviewSnapshot = {
  priceEur: 71_420.18,
  priceUsd: 76_597.49,
  connections: [
    {
      id: "c1",
      kind: "xpub",
      label: "Cold Storage",
      last: "2m ago",
      balance: 1.24810472,
      status: "synced",
      addresses: 142,
      gap: 10,
    },
    {
      id: "c2",
      kind: "descriptor",
      label: "Multisig 2/3 Vault",
      last: "2m ago",
      balance: 3.0814290,
      status: "synced",
      addresses: 86,
      gap: 10,
    },
    {
      id: "c3",
      kind: "core-ln",
      label: "Home Node (CLN)",
      last: "18s ago",
      balance: 0.04821309,
      status: "synced",
      channels: 12,
    },
    {
      id: "c4",
      kind: "nwc",
      label: "Alby Hub",
      last: "1h ago",
      balance: 0.00213500,
      status: "idle",
    },
    {
      id: "c5",
      kind: "cashu",
      label: "minibits.cash",
      last: "3h ago",
      balance: 0.00019823,
      status: "synced",
    },
  ],
  txs: [
    { id: "tx1", date: "2026-04-18 14:22", type: "Income", account: "Cold Storage", counter: "Invoice · ACME GmbH", amountSat: 2_450_000, eur: 1749.79, rate: 71420.18, tag: "Revenue", conf: 41 },
    { id: "tx2", date: "2026-04-17 09:08", type: "Expense", account: "Home Node (CLN)", counter: "Server rental · Hetzner", amountSat: -120_431, eur: -86.0, rate: 71432.10, tag: "Hosting", conf: 140 },
    { id: "tx3", date: "2026-04-16 16:51", type: "Transfer", account: "Cold Storage → Vault", counter: "Internal transfer", amountSat: -50_000_000, eur: -35710.09, rate: 71420.18, tag: "Transfer", conf: 220, internal: true },
    { id: "tx4", date: "2026-04-15 11:14", type: "Income", account: "NWC · Alby", counter: "Client payment · LN", amountSat: 92_808, eur: 66.27, rate: 71398.42, tag: "Revenue", conf: 1 },
    { id: "tx5", date: "2026-04-14 22:02", type: "Expense", account: "Multisig Vault", counter: "Equipment · BitcoinStore", amountSat: -890_210, eur: -635.71, rate: 71412.0, tag: "Capex", conf: 420 },
    { id: "tx6", date: "2026-04-12 08:30", type: "Income", account: "Cold Storage", counter: "Sale · Consulting", amountSat: 3_800_000, eur: 2713.97, rate: 71420.18, tag: "Revenue", conf: 612 },
    { id: "tx7", date: "2026-04-11 19:45", type: "Expense", account: "Cashu · minibits", counter: "Coffee", amountSat: -8_400, eur: -6.0, rate: 71428.57, tag: "Meals", conf: 1 },
    { id: "tx8", date: "2026-04-09 10:00", type: "Fee", account: "Home Node (CLN)", counter: "Channel open", amountSat: -18_210, eur: -13.01, rate: 71445.91, tag: "Bank fees", conf: 380 },
    { id: "tx9", date: "2026-04-07 13:12", type: "Income", account: "Multisig Vault", counter: "Invoice · Globex AG", amountSat: 1_210_000, eur: 864.18, rate: 71420.0, tag: "Revenue", conf: 820 },
    { id: "tx10", date: "2026-04-06 15:30", type: "Swap", account: "NWC · Alby → Cashu · minibits", counter: "LN → ecash swap", amountSat: 500_000, eur: 357.10, rate: 71420.0, tag: "Swap", conf: 1 },
    { id: "tx11", date: "2026-04-05 11:08", type: "Swap", account: "Multisig Vault → Home Node (CLN)", counter: "Submarine swap · on-chain → LN", amountSat: 2_000_000, eur: 1428.40, rate: 71420.0, tag: "Swap", conf: 12 },
    { id: "tx12", date: "2026-04-03 09:22", type: "Consolidation", account: "Cold Storage", counter: "12 UTXOs → 1", amountSat: -42_180, eur: -30.13, rate: 71432.0, tag: "Consolidation", conf: 210 },
  ],
  balanceSeries: [0.8, 1.1, 1.6, 1.55, 2.2, 2.4, 2.8, 3.1, 3.6, 4.0, 4.3, 4.38],
  portfolioSeries: [
    { date: "2025-05-31", label: "2025-05-31", balanceBtc: 0.8, valueEur: 57_136.14, costBasisEur: 42_880 },
    { date: "2025-06-30", label: "2025-06-30", balanceBtc: 1.1, valueEur: 78_562.20, costBasisEur: 58_920 },
    { date: "2025-07-31", label: "2025-07-31", balanceBtc: 1.6, valueEur: 114_272.29, costBasisEur: 86_120 },
    { date: "2025-08-31", label: "2025-08-31", balanceBtc: 1.55, valueEur: 110_701.28, costBasisEur: 84_450 },
    { date: "2025-09-30", label: "2025-09-30", balanceBtc: 2.2, valueEur: 157_124.40, costBasisEur: 106_700 },
    { date: "2025-10-31", label: "2025-10-31", balanceBtc: 2.4, valueEur: 171_408.43, costBasisEur: 118_240 },
    { date: "2025-11-30", label: "2025-11-30", balanceBtc: 2.8, valueEur: 199_976.50, costBasisEur: 137_980 },
    { date: "2025-12-31", label: "2025-12-31", balanceBtc: 3.1, valueEur: 221_402.56, costBasisEur: 150_220 },
    { date: "2026-01-31", label: "2026-01-31", balanceBtc: 3.6, valueEur: 257_112.65, costBasisEur: 167_900 },
    { date: "2026-02-28", label: "2026-02-28", balanceBtc: 4.0, valueEur: 285_680.72, costBasisEur: 181_430 },
    { date: "2026-03-31", label: "2026-03-31", balanceBtc: 4.3, valueEur: 307_106.77, costBasisEur: 193_100 },
    { date: "2026-04-30", label: "2026-04-30", balanceBtc: 4.38, valueEur: 312_842.77, costBasisEur: 198_502.40 },
  ],
  fiat: {
    eurBalance: 312_842.77,
    eurCostBasis: 198_502.40,
    eurUnrealized: 114_340.37,
    eurRealizedYTD: 42_118.92,
  },
};
