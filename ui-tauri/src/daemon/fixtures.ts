/**
 * Hand-rolled mock fixtures, keyed by daemon `kind`.
 *
 * Each entry is the `data` body of a successful envelope. Kept minimal
 * until Phase 1.2 §2.2 generates real fixtures from the Pydantic schema.
 * Add entries as screens get translated.
 */

import { MOCK_OVERVIEW } from "@/mocks/seed";
import { MOCK_TRANSACTIONS } from "@/mocks/transactions";
import { MOCK_PROFILES } from "@/mocks/profiles";
import { MOCK_CAPITAL_GAINS } from "@/mocks/reports";

export const fixtures: Record<string, unknown> = {
  status: {
    version: "0.0.0-ui-scaffold",
    data_root: "~/.kassiber",
    workspace: null,
    profile: null,
  },
  "ui.overview.snapshot": MOCK_OVERVIEW,
  "ui.transactions.list": MOCK_TRANSACTIONS,
  "ui.wallets.sync": {
    results: MOCK_OVERVIEW.connections.map((connection) => ({
      wallet: connection.label,
      status: "synced",
      inserted: 0,
      updated: 0,
    })),
  },
  "ui.profiles.snapshot": MOCK_PROFILES,
  "ui.reports.capital_gains": MOCK_CAPITAL_GAINS,
  "ui.journals.snapshot": {
    status: {
      workspace: "Demo Workspace",
      profile: "local profile",
      transactionCount: MOCK_OVERVIEW.txs.length,
      journalEntryCount: 42,
      needsJournals: false,
      quarantines: 0,
      lastProcessedAt: "2026-04-26T12:00:00Z",
    },
    entryTypes: [
      { type: "acquisition", count: 28, gainLossEur: 0 },
      { type: "disposal", count: 8, gainLossEur: 1240.5 },
      { type: "transfer_in", count: 3, gainLossEur: 0 },
      { type: "transfer_out", count: 3, gainLossEur: 0 },
    ],
    recent: [],
  },
};
