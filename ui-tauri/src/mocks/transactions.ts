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
    conf: 1,
  },
  {
    id: "tx14",
    date: "2026-03-30 12:11",
    type: "Melt",
    account: "Cashu · minibits → NWC · Alby",
    counter: "Melt · ecash to LN",
    amountSat: -120_000,
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
    conf: 1,
  },
  {
    id: "tx16",
    date: "2026-03-25 21:03",
    type: "Fee",
    account: "Multisig Vault",
    counter: "Miner fee · UTXO send",
    amountSat: -3_410,
    eur: -2.44,
    rate: 71_440.5,
    tag: "Bank fees",
    conf: 480,
  },
];

export interface TransactionsLedger {
  txs: Tx[];
  year: number;
}

export const MOCK_TRANSACTIONS: TransactionsLedger = {
  txs: [...MOCK_OVERVIEW.txs, ...EXTRA_TXS],
  year: 2026,
};
