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
  "ui.reports.export_pdf": {
    file: "~/.kassiber/exports/reports/kassiber-report-mock.pdf",
    filename: "kassiber-report-mock.pdf",
    format: "pdf",
    scope: "report",
    pages: 4,
    bytes: 24576,
    title: "Kassiber Report - Mock",
  },
  "ui.reports.export_capital_gains_csv": {
    file: "~/.kassiber/exports/reports/kassiber-capital-gains-mock.csv",
    filename: "kassiber-capital-gains-mock.csv",
    format: "csv",
    scope: "capital_gains",
    rows: MOCK_CAPITAL_GAINS.lots.length,
    bytes: 8192,
  },
  "ui.reports.export_austrian_e1kv_pdf": {
    file: "~/.kassiber/exports/reports/kassiber-austrian-e1kv-2025-mock.pdf",
    filename: "kassiber-austrian-e1kv-2025-mock.pdf",
    format: "pdf",
    scope: "austrian_e1kv",
    tax_year: 2025,
    pages: 4,
    bytes: 32768,
  },
  "ui.reports.export_austrian_e1kv_xlsx": {
    file: "~/.kassiber/exports/reports/kassiber-austrian-e1kv-2025-mock.xlsx",
    filename: "kassiber-austrian-e1kv-2025-mock.xlsx",
    format: "xlsx",
    scope: "austrian_e1kv",
    tax_year: 2025,
    rows: MOCK_CAPITAL_GAINS.lots.length,
    summary_rows: 3,
    bytes: 24576,
  },
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
  { reasoning: "Preparing a formatting demo. ", delayMs: 20 },
  { reasoning: "Including headings, table, list, quote, and code.", delayMs: 60 },
  { content: "## Review snapshot\n\n", delayMs: 30 },
  {
    content:
      "This is a formatting demo for the assistant transcript. It should show relaxed paragraphs, headings, a CLI-style table, and command output without squeezing everything into a tiny bubble.\n\n",
    delayMs: 40,
  },
  { content: "### CLI-style table\n\n", delayMs: 30 },
  { content: "| Item | Status | Action |\n", delayMs: 30 },
  { content: "| --- | --- | --- |\n", delayMs: 20 },
  { content: "| Journals | Current | Run report exports |\n", delayMs: 30 },
  { content: "| Quarantine | Clear | Keep watching imports |\n", delayMs: 30 },
  { content: "| Rates | Needs review | Sync BTC-EUR cache |\n\n", delayMs: 30 },
  { content: "### Notes\n\n", delayMs: 30 },
  { content: "- Use program output for calculations.\n", delayMs: 30 },
  { content: "- Keep wallet data local.\n", delayMs: 30 },
  { content: "- Reprocess journals after imports.\n\n", delayMs: 30 },
  {
    content:
      "> Tables should be readable and scroll horizontally when needed.\n\n",
    delayMs: 40,
  },
  {
    content: "```bash\nkassiber reports tax-summary --machine\n```\n",
    delayMs: 50,
  },
];
