import type { AiChatToolCall } from "@/daemon/stream";

/**
 * Typed view of a `ui.visuals.render` tool result. The daemon builds every
 * figure from local reads; the model only chose the source, measure, filters
 * and title. A payload that does not match these shapes is not drawn, so a
 * malformed or unexpected envelope falls back to the ordinary tool card
 * instead of a misleading visual.
 */

export const CHAT_VISUAL_TOOL = "ui.visuals.render";
// A balance history can hold 240 buckets for each of several assets.
const MAX_ROWS = 5000;

export type ChatVisualSource =
  | "overview"
  | "balance_history"
  | "portfolio"
  | "tax_summary"
  | "cash_flow"
  | "transactions"
  | "transaction_flow"
  | "readiness";

export type ChatVisualUnit =
  | { kind: "fiat"; code: string }
  | { kind: "asset"; code: null }
  | { kind: "sats"; code: "sat" }
  | { kind: "count"; code: null };

export interface ChatVisualCaveat {
  code: string;
  count?: number;
}

interface VisualBase {
  callId: string;
  measure: string;
  measures: string[];
  title: string | null;
  unit: ChatVisualUnit;
  fiatCurrency: string;
  asOf: string | null;
  caveats: ChatVisualCaveat[];
  filters: Record<string, string>;
}

export interface OverviewStats {
  portfolioValue: number | null;
  btcBalance: number | null;
  costBasis: number | null;
  unrealized: number | null;
  realizedYtd: number | null;
  btcPrice: number | null;
  priceAt: string | null;
  basisComplete: boolean;
  transactions: number | null;
}

export interface OverviewPoint {
  date: string;
  balanceBtc: number | null;
  marketValue: number | null;
  costBasis: number | null;
}

export interface OverviewWallet {
  label: string | null;
  otherCount: number;
  balanceBtc: number;
  share: number;
}

export interface OverviewVisual extends VisualBase {
  source: "overview";
  stats: OverviewStats;
  series: OverviewPoint[];
  wallets: OverviewWallet[];
}

export interface BalanceRow {
  period: string;
  asset: string;
  quantity: number | null;
  marketValue: number | null;
  costBasis: number | null;
}

export interface BalanceVisual extends VisualBase {
  source: "balance_history";
  interval: string;
  rows: BalanceRow[];
}

export interface HoldingRow {
  wallet: string;
  asset: string;
  quantity: number | null;
  marketValue: number | null;
  costBasis: number | null;
  unrealized: number | null;
}

export interface AllocationVisual extends VisualBase {
  source: "portfolio";
  groupBy: "wallet" | "asset";
  rows: HoldingRow[];
}

export interface TaxYear {
  year: number;
  proceeds: number | null;
  costBasis: number | null;
  gainLoss: number | null;
}

export interface TaxDetail extends TaxYear {
  asset: string;
  transactionType: string;
  capitalGainsType: string;
  quantity: number | null;
}

export interface TaxVisual extends VisualBase {
  source: "tax_summary";
  years: TaxYear[];
  details: TaxDetail[];
}

export interface FlowStats {
  count: number;
  btc: number;
  fiat: number;
  missingPrices: number;
  largest: { label: string; btc: number | null; fiat: number | null } | null;
}

export interface CashFlowRow {
  period: string;
  bucket: string;
  inBtc: number;
  outBtc: number;
  inFiat: number;
  outFiat: number;
  inCount: number;
  outCount: number;
  transferCount: number;
  swapCount: number;
  missingPrices: number;
}

export interface CashFlowVisual extends VisualBase {
  source: "cash_flow";
  period: string;
  bucket: "day" | "week" | "month" | "quarter";
  rows: CashFlowRow[];
  totals: {
    incoming: FlowStats;
    outgoing: FlowStats;
    transfers: FlowStats;
    swaps: FlowStats;
    review: number;
    pending: number;
  };
}

export interface TransactionRow {
  id: string;
  date: string | null;
  direction: "in" | "out" | "none";
  type: string;
  asset: string;
  amountSat: number;
  feeSat: number;
  fiatValue: number | null;
  wallet: string;
  counterparty: string;
  note: string;
  status: string;
  tags: string[];
}

export interface TransactionsVisual extends VisualBase {
  source: "transactions";
  sort: string;
  rows: TransactionRow[];
  /** In and out totals, only when every row is a BTC-denominated asset. */
  summary: { shown: number; total: number; inSat: number | null; outSat: number | null };
}

export interface FlowNode {
  side: "input" | "output" | "fee";
  label: string;
  labelKind: "wallet" | "generic" | "fee" | "overflow";
  position: number;
  ownership: string;
  valueSats: number | null;
  overflowCount?: number;
}

export interface TransactionFlowVisual extends VisualBase {
  source: "transaction_flow";
  nodes: FlowNode[];
  transaction: TransactionRow | null;
  localTransaction: string | null;
}

export type CheckStatus = "ok" | "warning" | "blocked";
export const CHECK_NAMES = ["transactions", "journals", "quarantine", "prices", "custody", "review_queue", "blocker"] as const;
export type CheckName = (typeof CHECK_NAMES)[number];
export const CHECK_ACTIONS = ["process_journals", "review_quarantine", "fill_prices", "review_custody", "open_review", "import"] as const;
export type CheckAction = (typeof CHECK_ACTIONS)[number];

export interface ReadinessCheck {
  check: CheckName;
  /** A blocker's own title, from the daemon (English, like its other messages). */
  title: string | null;
  status: CheckStatus;
  count: number | null;
  detail: string | null;
  action: CheckAction | null;
}

export interface ReadinessVisual extends VisualBase {
  source: "readiness";
  checks: ReadinessCheck[];
  summary: { ready: boolean; passed: number; total: number };
}

export type ChatVisual =
  | OverviewVisual
  | BalanceVisual
  | AllocationVisual
  | TaxVisual
  | CashFlowVisual
  | TransactionsVisual
  | TransactionFlowVisual
  | ReadinessVisual;

type Json = Record<string, unknown>;

/** Thrown inside the parser for a contract break; caught at the top. */
class Malformed extends Error {}

function record(value: unknown): Json | null {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as Json) : null;
}

function need<T>(value: T | null | undefined): T {
  if (value === null || value === undefined) throw new Malformed();
  return value;
}

function text(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function str(value: unknown): string {
  return typeof value === "string" ? value : "";
}

/** A finite number, `null` for an explicit null; anything else breaks the contract. */
function num(value: unknown): number | null {
  if (value === null || value === undefined) return null;
  if (typeof value === "number" && Number.isFinite(value)) return value;
  throw new Malformed();
}

function int(value: unknown): number {
  const parsed = num(value);
  return parsed === null ? 0 : Math.trunc(parsed);
}

function list(value: unknown, max = MAX_ROWS): Json[] {
  if (!Array.isArray(value) || value.length > max) throw new Malformed();
  return value.map((item) => need(record(item)));
}

function parseUnit(value: unknown): ChatVisualUnit {
  const unit = need(record(value));
  if (unit.kind === "fiat" && typeof unit.code === "string" && /^[A-Z]{3}$/.test(unit.code)) {
    return { kind: "fiat", code: unit.code };
  }
  if (unit.kind === "asset") return { kind: "asset", code: null };
  if (unit.kind === "sats") return { kind: "sats", code: "sat" };
  if (unit.kind === "count") return { kind: "count", code: null };
  throw new Malformed();
}

function parseCaveats(value: unknown): ChatVisualCaveat[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((item) => {
    const caveat = record(item);
    const code = text(caveat?.code);
    if (!code) return [];
    return typeof caveat?.count === "number" && Number.isFinite(caveat.count)
      ? [{ code, count: caveat.count }]
      : [{ code }];
  });
}

function parseFilters(value: unknown): Record<string, string> {
  const filters = record(value) ?? {};
  return Object.fromEntries(
    Object.entries(filters).flatMap(([key, entry]) =>
      typeof entry === "string" && entry ? [[key, entry]] : [],
    ),
  );
}

function parseTransaction(value: Json): TransactionRow {
  const direction = value.direction;
  return {
    id: need(text(value.id)),
    date: text(value.date),
    direction: direction === "in" || direction === "out" ? direction : "none",
    type: str(value.type),
    asset: str(value.asset),
    // Lightning amounts can be fractions of a sat.
    amountSat: num(value.amount_sat) ?? 0,
    feeSat: num(value.fee_sat) ?? 0,
    fiatValue: num(value.fiat_value),
    wallet: str(value.wallet),
    counterparty: str(value.counterparty),
    note: str(value.note),
    status: str(value.status),
    tags: Array.isArray(value.tags) ? value.tags.filter((tag): tag is string => typeof tag === "string") : [],
  };
}

function parseFlowStats(value: unknown): FlowStats {
  const stats = record(value) ?? {};
  const largest = record(stats.largest);
  return {
    count: int(stats.count),
    btc: num(stats.btc) ?? 0,
    fiat: num(stats.fiat) ?? 0,
    missingPrices: int(stats.missing_prices),
    largest: largest
      ? { label: str(largest.label), btc: num(largest.btc), fiat: num(largest.fiat) }
      : null,
  };
}

const FLOW_SIDES = new Set(["input", "output", "fee"]);
const FLOW_LABEL_KINDS = new Set(["wallet", "generic", "fee", "overflow"]);
const CHECK_STATUSES = new Set(["ok", "warning", "blocked"]);
const BUCKETS = new Set(["day", "week", "month", "quarter"]);

function parseSource(source: string, data: Json, visual: Json, base: VisualBase): ChatVisual {
  switch (source) {
    case "overview": {
      const stats = need(record(data.stats));
      return {
        ...base,
        source,
        stats: {
          portfolioValue: num(stats.portfolio_value),
          btcBalance: num(stats.btc_balance),
          costBasis: num(stats.cost_basis),
          unrealized: num(stats.unrealized),
          realizedYtd: num(stats.realized_ytd),
          btcPrice: num(stats.btc_price),
          priceAt: text(stats.price_at),
          basisComplete: stats.basis_complete === true,
          transactions: num(stats.transactions),
        },
        series: list(data.series).map((point) => ({
          date: need(text(point.date)),
          balanceBtc: num(point.balance_btc),
          marketValue: num(point.market_value),
          costBasis: num(point.cost_basis),
        })),
        wallets: list(data.wallets, 50).map((wallet) => ({
          label: text(wallet.label),
          otherCount: int(wallet.other_count),
          balanceBtc: num(wallet.balance_btc) ?? 0,
          share: num(wallet.share) ?? 0,
        })),
      };
    }
    case "balance_history":
      return {
        ...base,
        source,
        interval: text(visual.interval) ?? "month",
        rows: list(data.rows).map((row) => ({
          period: need(text(row.period)),
          asset: str(row.asset),
          quantity: num(row.quantity),
          marketValue: num(row.market_value),
          costBasis: num(row.cost_basis),
        })),
      };
    case "portfolio":
      return {
        ...base,
        source,
        groupBy: visual.group_by === "asset" ? "asset" : "wallet",
        rows: list(data.rows).map((row) => ({
          wallet: str(row.wallet),
          asset: str(row.asset),
          quantity: num(row.quantity),
          marketValue: num(row.market_value),
          costBasis: num(row.cost_basis),
          unrealized: num(row.unrealized_pnl),
        })),
      };
    case "tax_summary": {
      const year = (row: Json): TaxYear => ({
        year: need(num(row.year)),
        proceeds: num(row.proceeds),
        costBasis: num(row.cost_basis),
        gainLoss: num(row.gain_loss),
      });
      return {
        ...base,
        source,
        years: list(data.rows).map(year),
        details: list(data.details ?? []).map((row) => ({
          ...year(row),
          asset: str(row.asset),
          transactionType: str(row.transaction_type),
          capitalGainsType: str(row.capital_gains_type),
          quantity: num(row.quantity),
        })),
      };
    }
    case "cash_flow": {
      const totals = need(record(data.totals));
      const bucket = str(visual.bucket);
      return {
        ...base,
        source,
        period: str(visual.period),
        bucket: (BUCKETS.has(bucket) ? bucket : "month") as CashFlowVisual["bucket"],
        rows: list(data.rows).map((row) => ({
          period: need(text(row.period)),
          bucket: str(row.bucket),
          inBtc: num(row.in_btc) ?? 0,
          outBtc: num(row.out_btc) ?? 0,
          inFiat: num(row.in_fiat) ?? 0,
          outFiat: num(row.out_fiat) ?? 0,
          inCount: int(row.in_count),
          outCount: int(row.out_count),
          transferCount: int(row.transfer_count),
          swapCount: int(row.swap_count),
          missingPrices: int(row.missing_prices),
        })),
        totals: {
          incoming: parseFlowStats(totals.incoming),
          outgoing: parseFlowStats(totals.outgoing),
          transfers: parseFlowStats(totals.transfers),
          swaps: parseFlowStats(totals.swaps),
          review: int(totals.review),
          pending: int(totals.pending),
        },
      };
    }
    case "transactions": {
      const summary = need(record(data.summary));
      return {
        ...base,
        source,
        sort: text(visual.sort) ?? "recent",
        rows: list(data.rows, 50).map(parseTransaction),
        summary: {
          shown: int(summary.shown),
          total: int(summary.total),
          inSat: num(summary.in_sat),
          outSat: num(summary.out_sat),
        },
      };
    }
    case "transaction_flow": {
      const transaction = record(data.transaction);
      const reference = record(data.local_reference);
      return {
        ...base,
        source,
        nodes: list(data.rows).map((row) => {
          const side = str(row.side);
          const labelKind = str(row.label_kind);
          if (!FLOW_SIDES.has(side) || !FLOW_LABEL_KINDS.has(labelKind)) throw new Malformed();
          return {
            side: side as FlowNode["side"],
            label: need(text(row.label)),
            labelKind: labelKind as FlowNode["labelKind"],
            position: int(row.position),
            ownership: text(row.ownership) ?? "unknown",
            valueSats: num(row.value_sats),
            ...(typeof row.overflow_count === "number" ? { overflowCount: row.overflow_count } : {}),
          };
        }),
        transaction: transaction ? parseTransaction(transaction) : null,
        localTransaction: reference?.route === "/transactions" ? text(reference.transaction) : null,
      };
    }
    case "readiness": {
      const summary = need(record(data.summary));
      return {
        ...base,
        source,
        checks: list(data.rows, 50).map((row) => {
          const status = str(row.status);
          const check = str(row.check);
          const action = text(row.action);
          if (
            !CHECK_STATUSES.has(status) ||
            !(CHECK_NAMES as readonly string[]).includes(check) ||
            (action !== null && !(CHECK_ACTIONS as readonly string[]).includes(action))
          ) {
            throw new Malformed();
          }
          return {
            check: check as CheckName,
            title: text(row.title),
            status: status as CheckStatus,
            count: num(row.count),
            detail: text(row.detail),
            action: action as CheckAction | null,
          };
        }),
        summary: {
          ready: summary.ready === true,
          passed: int(summary.passed),
          total: int(summary.total),
        },
      };
    }
    default:
      throw new Malformed();
  }
}

export function chatVisualFromToolCall(call: AiChatToolCall): ChatVisual | null {
  if (call.name !== CHAT_VISUAL_TOOL || call.status !== "done") return null;
  const envelope = record(call.result);
  if (envelope?.kind !== CHAT_VISUAL_TOOL) return null;
  const data = record(envelope.data);
  const visual = record(data?.visual);
  if (!data || !visual || visual.version !== 1) return null;
  try {
    const source = need(text(visual.source));
    const measure = need(text(visual.measure));
    const base: VisualBase = {
      callId: call.callId,
      measure,
      measures: Array.isArray(visual.measures)
        ? visual.measures.filter((item): item is string => typeof item === "string")
        : [measure],
      title: text(visual.title),
      unit: parseUnit(visual.unit),
      fiatCurrency:
        typeof visual.fiat_currency === "string" && /^[A-Z]{3}$/.test(visual.fiat_currency)
          ? visual.fiat_currency
          : "EUR",
      asOf: text(visual.as_of),
      caveats: parseCaveats(data.caveats),
      filters: parseFilters(visual.filters),
    };
    return parseSource(source, data, visual, base);
  } catch (error) {
    if (error instanceof Malformed) return null;
    throw error;
  }
}

/**
 * The visuals in one assistant message, in the order their tools ran. The
 * pre-read router can draw a visual the model then requests again; identical
 * data is shown once, as the latest call, which carries the model's title.
 */
export function chatVisualsForMessage(toolCalls: AiChatToolCall[] | undefined): ChatVisual[] {
  const visuals = (toolCalls ?? []).flatMap((call) => {
    const visual = chatVisualFromToolCall(call);
    return visual ? [visual] : [];
  });
  const seen = new Set<string>();
  const latestFirst: ChatVisual[] = [];
  for (const visual of [...visuals].reverse()) {
    // asOf differs between two reads of the same data a moment apart.
    const signature = JSON.stringify({ ...visual, callId: null, title: null, asOf: null });
    if (seen.has(signature)) continue;
    seen.add(signature);
    latestFirst.push(visual);
  }
  return latestFirst.reverse();
}

/** A visual still being drawn: the source the model asked for, if known. */
export function pendingVisualSources(toolCalls: AiChatToolCall[] | undefined): Array<{
  callId: string;
  source: string | null;
}> {
  return (toolCalls ?? [])
    .filter(
      (call) =>
        call.name === CHAT_VISUAL_TOOL &&
        (call.status === "pending" || call.status === "running"),
    )
    .map((call) => ({
      callId: call.callId,
      source: typeof call.arguments.source === "string" ? call.arguments.source : null,
    }));
}
