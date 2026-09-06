import { DEFAULT_ANALYSIS_QUERY, type AnalysisQuery } from "./chainAnalysis";

export type AnalysisWorkspace = "graph" | "psbt" | "datasets";
export type AnalysisTab = "findings" | "frontier" | "clusters" | "paths" | "coverage" | "entropy" | "labels" | "patterns" | "exposure";
export type AnalysisSearch = Partial<AnalysisQuery> & { workspace?: AnalysisWorkspace; tab?: AnalysisTab };

const oneOf = <T extends string>(value: unknown, choices: readonly T[]): T | undefined =>
  typeof value === "string" && choices.includes(value as T) ? value as T : undefined;
const boundedText = (value: unknown, limit: number): string | undefined =>
  typeof value === "string" && value.length > 0 && value.length <= limit ? value : undefined;
const boundedInteger = (value: unknown, min: number, max: number): number | undefined =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= min && value <= max ? value : undefined;

/** Navigation selects a local view; it never schedules acquisition or a query. */
export function parseAnalysisSearch(search: Record<string, unknown>): AnalysisSearch {
  const values: AnalysisSearch = {
    mode: oneOf(search.mode, ["overview", "trace", "path"]),
    subject: boundedText(search.subject, 1024),
    target: boundedText(search.target, 1024),
    observer: oneOf(search.observer, ["owner", "public", "disclosed"]),
    chain: oneOf(search.chain, ["bitcoin", "liquid"]),
    network: oneOf(({ liquidv1: "main", liquidtestnet: "test", elementsregtest: "regtest" } as Record<string, string>)[String(search.network)] ?? search.network, ["main", "test", "signet", "regtest"]),
    direction: oneOf(search.direction, ["backward", "forward", "both"]),
    depth: boundedInteger(search.depth, 1, 50),
    node_limit: boundedInteger(search.node_limit, 25, 2000),
    edge_limit: boundedInteger(search.edge_limit, 50, 6000),
    include_relations: typeof search.include_relations === "boolean" ? search.include_relations : undefined,
    include_hypotheses: typeof search.include_hypotheses === "boolean" ? search.include_hypotheses : undefined,
    min_amount_msat: typeof search.min_amount_msat === "string" && /^\d{1,30}$/.test(search.min_amount_msat) ? search.min_amount_msat : undefined,
    start: boundedText(search.start, 40),
    end: boundedText(search.end, 40),
    workspace: oneOf(search.workspace, ["graph", "psbt", "datasets"]),
    tab: oneOf(search.tab, ["findings", "frontier", "clusters", "paths", "coverage", "entropy", "labels", "patterns", "exposure"]),
  };
  return Object.fromEntries(Object.entries(values).filter(([, value]) => value !== undefined));
}

export function analysisSearchQuery(search: AnalysisSearch): AnalysisQuery {
  const query = { ...search };
  delete query.workspace;
  delete query.tab;
  return {
    ...DEFAULT_ANALYSIS_QUERY,
    ...(query.subject && !query.mode ? { mode: "trace" } : {}),
    ...query,
  };
}

export function analysisInvestigationSearch(query: AnalysisQuery, workspace: AnalysisWorkspace = "graph", tab: AnalysisTab = "findings"): AnalysisSearch {
  return parseAnalysisSearch({ ...query, workspace, tab });
}

/** Public views accept physical identities only; private record aliases never cross the observer boundary. */
export function transactionAnalysisSearch(record: { explorerId?: string; txnId?: string; chain?: string | null; network?: string | null }): AnalysisSearch {
  const domain = parseAnalysisSearch({ chain: record.chain, network: record.network });
  const txid = [record.explorerId, record.txnId].find(value => typeof value === "string" && /^[a-fA-F0-9]{64}$/.test(value));
  return parseAnalysisSearch({
    ...domain,
    ...(txid && domain.chain && domain.network ? { subject: txid.toLowerCase(), mode: "trace" } : { mode: "overview" }),
    observer: "public", include_relations: false, include_hypotheses: false,
  });
}
