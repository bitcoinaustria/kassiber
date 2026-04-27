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
  "ai.providers.list": {
    providers: [
      {
        name: "ollama",
        base_url: "http://localhost:11434/v1",
        kind: "local",
        default_model: "qwen3.6:35b",
        notes: "Local Ollama (mock).",
        acknowledged_at: "2026-04-27T08:00:00Z",
        created_at: "2026-04-27T08:00:00Z",
        updated_at: "2026-04-27T08:00:00Z",
        has_api_key: false,
        is_default: true,
      },
    ],
    default: "ollama",
  },
  "ai.list_models": {
    provider: "ollama",
    models: [
      { id: "qwen3.6:35b", owned_by: "library" },
      { id: "llama3.3:70b", owned_by: "library" },
    ],
  },
  "ai.test_connection": {
    base_url: "http://localhost:11434/v1",
    model_count: 2,
    models: [
      { id: "qwen3.6:35b", owned_by: "library" },
      { id: "llama3.3:70b", owned_by: "library" },
    ],
  },
};

/**
 * Pre-canned AI chat stream for mock mode. Mimics what Ollama's
 * OpenAI-compat shim sends for current reasoning builds (Qwen3, Gemma
 * reasoning) and what OpenAI o1/o3 endpoints send: structured
 * `reasoning` deltas first, then visible `content` deltas — never both
 * in one stream. The inline `<think>...</think>` tag style used by
 * DeepSeek-R1 / QwQ is covered by `lib/thinkParser.test.ts` rather than
 * mixed into this fixture, so the mock matches one realistic provider
 * shape instead of conflating two.
 */
export const MOCK_AI_CHAT_STREAM: {
  content?: string;
  reasoning?: string;
  delayMs?: number;
}[] = [
  { reasoning: "Looking at the workspace. ", delayMs: 20 },
  { reasoning: "Counting recent imports and checking journal freshness.", delayMs: 60 },
  { content: "Here is what I see:\n\n", delayMs: 30 },
  { content: "- 4 transactions imported in the last 7 days\n", delayMs: 40 },
  { content: "- 0 quarantined entries\n", delayMs: 40 },
  { content: "- Journals are up to date.\n\n", delayMs: 40 },
  { content: "You're good to run reports.", delayMs: 80 },
];
