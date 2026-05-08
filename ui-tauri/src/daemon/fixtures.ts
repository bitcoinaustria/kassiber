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
  "ui.transactions.extremes": {
    largest: MOCK_TRANSACTIONS.txs.slice(0, 3),
    smallest: MOCK_TRANSACTIONS.txs.slice(-3),
    filters: { limit: 3, sort: "amount", scope: "all_time_before_limit" },
  },
  "ui.transactions.search": {
    txs: MOCK_TRANSACTIONS.txs.slice(0, 4),
    filters: {
      query: "invoice",
      limit: 25,
      sort: "occurred-at",
      order: "desc",
    },
  },
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
  "ui.reports.summary": {
    workspace: "My Books",
    profile: "local books",
    wallet: null,
    fiat_currency: "EUR",
    metrics: {
      wallets_in_scope: 2,
      assets_in_scope: 1,
      active_transactions: 12,
      excluded_transactions: 0,
      inbound_transactions: 7,
      outbound_transactions: 5,
      journal_entries: 18,
      quarantines: 0,
    },
    asset_flow: [
      {
        asset: "BTC",
        inbound_amount: 0.42,
        inbound_amount_sat: 42_000_000,
        inbound_amount_msat: 42_000_000_000,
        outbound_amount: 0.15,
        outbound_amount_sat: 15_000_000,
        outbound_amount_msat: 15_000_000_000,
        fee_amount: 0.0001,
        fee_amount_sat: 10_000,
        fee_amount_msat: 10_000_000,
      },
    ],
    wallet_flow: [],
  },
  "ui.reports.balance_sheet": {
    rows: [
      {
        account: "treasury",
        account_label: "Treasury",
        asset: "BTC",
        quantity: 0.27,
        quantity_sat: 27_000_000,
        quantity_msat: 27_000_000_000,
        cost_basis: 19_275.0,
        market_value: 19_283.45,
        unrealized_pnl: 8.45,
      },
    ],
    totals_by_asset: [
      {
        asset: "BTC",
        quantity: 0.27,
        quantity_sat: 27_000_000,
        quantity_msat: 27_000_000_000,
        cost_basis: 19_275.0,
        market_value: 19_283.45,
        unrealized_pnl: 8.45,
      },
    ],
    summary: { row_count: 1, asset_count: 1 },
  },
  "ui.reports.portfolio_summary": {
    rows: [
      {
        wallet: "Multisig Vault",
        asset: "BTC",
        quantity: 0.27,
        quantity_sat: 27_000_000,
        quantity_msat: 27_000_000_000,
        average_cost: 71_388.89,
        cost_basis: 19_275.0,
        market_value: 19_283.45,
        unrealized_pnl: 8.45,
      },
    ],
    totals_by_asset: [
      {
        asset: "BTC",
        quantity: 0.27,
        quantity_sat: 27_000_000,
        quantity_msat: 27_000_000_000,
        cost_basis: 19_275.0,
        market_value: 19_283.45,
        unrealized_pnl: 8.45,
      },
    ],
    summary: { row_count: 1, asset_count: 1 },
  },
  "ui.reports.tax_summary": {
    rows: [
      {
        row_type: "year_total",
        year: 2025,
        asset: "BTC",
        quantity: 0.253,
        quantity_msat: 25_300_000_000,
        proceeds: 17_991.95,
        cost_basis: 10_078.35,
        gain_loss: 7_913.6,
      },
    ],
    available_years: [2025, 2026],
    filters: { year: 2025 },
    summary: { row_count: 1, available_year_count: 2 },
  },
  "ui.reports.balance_history": {
    rows: [
      { bucket: "2026-01-01T00:00:00Z", asset: "BTC", quantity: 0.12 },
      { bucket: "2026-02-01T00:00:00Z", asset: "BTC", quantity: 0.18 },
      { bucket: "2026-03-01T00:00:00Z", asset: "BTC", quantity: 0.27 },
    ],
    filters: { interval: "month", limit: 120 },
    summary: { row_count: 3, total_row_count: 3, truncated: false },
  },
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
      workspace: "My Books",
      profile: "local books",
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
  "ui.journals.process": {
    profile: "local books",
    entries_created: 42,
    quarantined: 0,
    transfers_detected: 3,
    cross_asset_pairs: 0,
    auto_priced: 0,
    processed_transactions: MOCK_OVERVIEW.txs.length,
    processed_at: "2026-04-26T12:00:00Z",
  },
  "ui.rates.coverage": {
    workspace: "My Books",
    profile: "local books",
    summary: {
      active_transactions: 12,
      priced_transactions: 11,
      missing_price_transactions: 1,
      cache_coverable_missing: 1,
      cache_uncovered_missing: 0,
    },
    items: [
      {
        id: "mock-missing-price",
        externalId: "mock-txid",
        date: "2026-04-22T10:20:00Z",
        wallet: "Multisig Vault",
        direction: "inbound",
        asset: "BTC",
        amountSat: 50_000,
        amountMsat: 50_000_000,
        fiatCurrency: "EUR",
        missingFiatRate: true,
        missingFiatValue: true,
        cachePair: "BTC-EUR",
        cacheHasRate: true,
        cacheRateAt: "2026-04-22T00:00:00Z",
      },
    ],
    filters: { limit: 25 },
  },
  "ui.report.blockers": {
    ready: false,
    blockers: [
      {
        id: "missing_prices",
        severity: "review",
        title: "Missing transaction prices",
        detail: "1 transaction(s) are missing fiat price fields.",
        daemon_kind: "ui.rates.coverage",
      },
    ],
    health: {
      counts: {
        wallets: 2,
        transactions: 12,
        active_transactions: 12,
        journal_entries: 42,
        quarantines: 0,
        rate_pairs: 1,
      },
      journals: {
        status: "current",
        needs_processing: false,
        quarantine_count: 0,
        last_processed_at: "2026-04-26T12:00:00Z",
      },
      reports: { ready: true, hints: ["Reports are ready."] },
    },
    rates_coverage: {
      summary: {
        active_transactions: 12,
        missing_price_transactions: 1,
        cache_coverable_missing: 1,
      },
      items: [],
    },
  },
  "ui.audit.changes_since_last_answer": {
    changed: false,
    baseline: { since: "2026-04-26T12:00:00Z" },
    workspace: "My Books",
    profile: "local books",
    counts_since: {
      transactions: 0,
      journal_entries: 0,
      journal_quarantines: 0,
      wallets: 0,
      rates: 0,
    },
    latest: { journals_processed_at: "2026-04-26T12:00:00Z" },
    current: {
      generated_at: "2026-04-26T12:01:00Z",
      active_transactions: 12,
      journals_processed_at: "2026-04-26T12:00:00Z",
      quarantines: 0,
    },
  },
  "ui.maintenance.settings": {
    workspace: "My Books",
    profile: { id: "mock-profile", label: "local books" },
    settings: {
      auto_sync_before_report_reads: false,
      setting_key: "ai.auto_sync_before_report_reads.profile.mock-profile",
    },
  },
  "ui.maintenance.configure": {
    workspace: "My Books",
    profile: { id: "mock-profile", label: "local books" },
    settings: {
      auto_sync_before_report_reads: true,
      setting_key: "ai.auto_sync_before_report_reads.profile.mock-profile",
    },
  },
  "ui.maintenance.run": {
    ready: true,
    sync_mode: "if_enabled",
    maintenance: {
      journals: {
        kind: "ui.journals.process",
        schema_version: 1,
        data: {
          processed_transactions: MOCK_OVERVIEW.txs.length,
          quarantined: 0,
          processed_at: "2026-04-26T12:00:00Z",
        },
      },
    },
    blockers: [],
    health: {
      counts: { active_transactions: 12, quarantines: 0 },
      journals: { status: "current", needs_processing: false },
      reports: { ready: true },
    },
    settings: { auto_sync_before_report_reads: false },
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
