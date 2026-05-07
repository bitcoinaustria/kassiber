/**
 * Mock data for the Transactions screen.
 *
 * Until the Pydantic→JSON Schema pipeline lands, this fixture extends
 * MOCK_OVERVIEW.txs with a few extra rows to exercise the secondary
 * "More" filter dropdown (Mint, Melt, Rebalance, Fee).
 */

import { MOCK_OVERVIEW, type Tx } from "@/mocks/seed";

const EXTRA_TXS: Tx[] = [
  {
    id: "tx13",
    date: "2026-04-02 18:40",
    type: "Mint",
    account: "NWC · Alby → Cashu · minibits",
    counter: "Mint · ecash from LN",
    amountSat: 250_000,
    eur: 178.55,
    rate: 71_420.0,
    tag: "Swap",
    note: "Review as ecash mint before year-end close.",
    conf: 1,
  },
  {
    id: "tx14",
    date: "2026-03-30 12:11",
    type: "Melt",
    account: "Cashu · minibits → NWC · Alby",
    counter: "Melt · ecash to LN",
    amountSat: -120_000,
    feeSat: 21,
    eur: -85.7,
    rate: 71_417.0,
    tag: "Swap",
    conf: 1,
  },
  {
    id: "tx15",
    date: "2026-03-28 09:55",
    type: "Rebalance",
    account: "Home Node (CLN)",
    counter: "Channel rebalance · LN circular",
    amountSat: -450,
    eur: -0.32,
    rate: 71_420.0,
    tag: "Bank fees",
    note: "Channel maintenance, no external counterparty.",
    conf: 1,
  },
  {
    id: "tx16",
    date: "2026-03-25 21:03",
    type: "Fee",
    account: "Multisig Vault",
    counter: "Miner fee · UTXO send",
    amountSat: -3_410,
    feeSat: 3_410,
    eur: -2.44,
    rate: 71_440.5,
    tag: "Bank fees",
    conf: 480,
  },
  {
    id: "tx17",
    date: "2025-12-18 17:34",
    type: "Income",
    account: "Cold Storage",
    counter: "Invoice · Umbrella Studio",
    amountSat: 1_320_000,
    eur: 942.75,
    rate: 71_420.0,
    tag: "Revenue",
    note: "Prior-year revenue kept so 1 Year differs from YTD in demos.",
    conf: 2_940,
  },
  {
    id: "tx18",
    date: "2025-08-21 08:44",
    type: "Expense",
    account: "Multisig Vault",
    counter: "Hardware wallet order",
    amountSat: -360_000,
    feeSat: 410,
    eur: -257.11,
    rate: 71_420.0,
    tag: "Capex",
    conf: 18_402,
  },
  {
    id: "tx19",
    date: "2024-11-05 10:12",
    type: "Transfer",
    account: "Cold Storage → Multisig Vault",
    counter: "Vault migration",
    amountSat: -15_000_000,
    eur: -10_713.03,
    rate: 71_420.18,
    tag: "Transfer",
    conf: 72_100,
    internal: true,
  },
  {
    id: "tx20",
    date: "2023-06-12 15:25",
    type: "Expense",
    account: "Home Node (CLN)",
    counter: "Node hardware refresh",
    amountSat: -1_200_000,
    feeSat: 880,
    eur: -856.8,
    rate: 71_400.0,
    tag: "Capex",
    conf: 141_220,
  },
];

export interface TransactionsList {
  txs: Tx[];
  year: number;
}

export const MOCK_TRANSACTIONS: TransactionsList = {
  txs: [...MOCK_OVERVIEW.txs, ...EXTRA_TXS],
  year: 2026,
};
