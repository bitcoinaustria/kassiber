export interface AnalysisQuery {
  mode: "overview" | "trace" | "path";
  subject?: string;
  target?: string;
  chain?: "bitcoin" | "liquid";
  network?: string;
  direction: "backward" | "forward" | "both";
  depth: number;
  node_limit: number;
  edge_limit: number;
  include_relations: boolean;
  include_hypotheses: boolean;
  observer: "owner" | "public" | "disclosed";
  min_amount_msat?: string;
  start?: string;
  end?: string;
}

export interface AnalysisNode {
  id: string;
  kind: "transaction" | "output" | "record";
  chain: string;
  network: string;
  label: string;
  txid?: string;
  transaction_id?: string;
  outpoint?: string;
  wallet_ids: string[];
  amount_msat: string | null;
  asset: string | null;
  address?: string;
  occurred_at?: string;
  status?: string;
  evidence: unknown[];
}
export interface AnalysisEdge {
  id: string;
  source: string;
  target: string;
  kind: "creates" | "spends" | "custody" | "hypothesis";
  evidence_level: string;
  amount_msat: string | null;
  asset?: string;
  label: string;
  status?: string;
  evidence: unknown[];
}

export interface AnalysisEntropyRequest {
  subject: string;
  chain?: AnalysisQuery["chain"];
  network?: string;
  max_states: number;
  max_duration_ms?: number;
  observer?: "owner" | "public" | "disclosed";
  scenario?: import("./chainAnalysisWorkbench").EntropyScenario;
}

export interface AnalysisEntropyOutcome {
  request: AnalysisEntropyRequest;
  result: Record<string, unknown>;
}

/** A late response remains bound to the inputs actually submitted. */
export function currentAnalysisEntropyOutcome(
  request: AnalysisEntropyRequest,
  outcome: AnalysisEntropyOutcome | null,
  pending: boolean,
): AnalysisEntropyOutcome | null {
  if (pending || !outcome) return null;
  const submitted = outcome.request;
  return request.subject.trim() === submitted.subject.trim() &&
    request.chain === submitted.chain &&
    request.network === submitted.network &&
    request.max_states === submitted.max_states &&
    request.max_duration_ms === submitted.max_duration_ms &&
    request.observer === submitted.observer &&
    JSON.stringify(request.scenario) === JSON.stringify(submitted.scenario)
    ? outcome
    : null;
}
export interface AnalysisFinding {
  id: string;
  code: string;
  severity: string;
  title: string;
  detail: string;
  node_ids: string[];
  edge_ids: string[];
  evidence: unknown[];
}
export interface AnalysisFrontier {
  node_id?: string;
  reason: string;
  [key: string]: unknown;
}
export interface AnalysisCluster {
  id: string;
  node_ids?: string[];
  label?: string;
  evidence_level?: string;
  [key: string]: unknown;
}
export interface AnalysisResult {
  transaction_features?: Array<{ subject: string; features: import("./chainAnalysisWorkbench").FeatureSnapshot }>;
  schema_version: number;
  snapshot_id: string;
  query: AnalysisQuery;
  summary: Record<string, unknown>;
  nodes: AnalysisNode[];
  edges: AnalysisEdge[];
  findings: AnalysisFinding[];
  clusters: AnalysisCluster[];
  paths: Array<
    | { node_ids?: string[]; edge_ids?: string[]; [key: string]: unknown }
    | string[]
  >;
  frontier: AnalysisFrontier[];
  coverage: Record<string, unknown>;
  capabilities: Record<string, unknown>;
  patterns?: AnalysisFinding[];
  exposure?: Array<{
    id: string;
    kind: string;
    node_ids: string[];
    edge_ids: string[];
    claim?: Record<string, unknown>;
    [key: string]: unknown;
  }>;
}

export function analysisNetworkInput(network?: string): string {
  return (
    (
      {
        liquidv1: "main",
        liquidtestnet: "test",
        elementsregtest: "regtest",
      } as Record<string, string>
    )[network || ""] ||
    network ||
    "main"
  );
}

export function analysisUtcInput(value: string): string | undefined {
  if (!value) return undefined;
  return `${value.length === 16 ? `${value}:00` : value}Z`;
}

export const DEFAULT_ANALYSIS_QUERY: AnalysisQuery = {
  mode: "overview",
  direction: "both",
  depth: 4,
  node_limit: 400,
  edge_limit: 1200,
  include_relations: true,
  include_hypotheses: false,
  observer: "owner",
};

export function analysisQueryKey(query: AnalysisQuery): string {
  return JSON.stringify([
    query.mode,
    query.subject?.trim() || null,
    query.mode === "path" ? query.target?.trim() || null : null,
    query.chain || null,
    query.network || null,
    query.direction,
    query.depth,
    query.node_limit,
    query.edge_limit,
    query.include_relations,
    query.include_hypotheses,
    query.observer,
    query.min_amount_msat || null,
    query.start || null,
    query.end || null,
  ]);
}

/** Format backend decimal integer text without ever passing amounts through Number. */
export function formatAnalysisAmount(
  amount: string | null | undefined,
  asset?: string | null,
): string {
  if (amount == null || !/^-?\d+$/.test(amount)) return "—";
  const value = BigInt(amount);
  const sign = value < 0n ? "−" : "";
  const magnitude = value < 0n ? -value : value;
  if (asset && !["BTC", "LBTC", "L-BTC"].includes(asset.toUpperCase()))
    return `${sign}${magnitude.toString()} msat · ${asset}`;
  const whole = magnitude / 100_000_000_000n;
  const fraction = (magnitude % 100_000_000_000n)
    .toString()
    .padStart(11, "0")
    .replace(/0+$/, "");
  return `${sign}${whole}${fraction ? `.${fraction}` : ""} ${asset || "BTC"}`;
}

export function shortAnalysisId(value: string, maximum = 24): string {
  return value.length > maximum
    ? `${value.slice(0, Math.floor(maximum / 2))}…${value.slice(-8)}`
    : value;
}

export function analysisNodeSubject(node: AnalysisNode): string {
  // The canonical ID retains chain/network identity for otherwise identical txids.
  return node.id;
}

export interface AnalysisCase {
  id: string;
  title: string;
  created_at: string;
  snapshot_id: string;
  query: AnalysisQuery;
  summary?: Record<string, unknown>;
  result?: AnalysisResult;
}

export function analysisCsv(result: AnalysisResult): string {
  const rows: unknown[][] = [
    [
      "snapshot_id",
      "row_type",
      "id",
      "kind",
      "chain",
      "network",
      "source",
      "target",
      "amount_msat",
      "asset",
      "label",
      "evidence_level",
      "evidence_json",
    ],
  ];
  result.nodes.forEach((node) =>
    rows.push([
      result.snapshot_id,
      "node",
      node.id,
      node.kind,
      node.chain,
      node.network,
      "",
      "",
      node.amount_msat,
      node.asset,
      node.label,
      "",
      JSON.stringify(node.evidence),
    ]),
  );
  result.edges.forEach((edge) =>
    rows.push([
      result.snapshot_id,
      "edge",
      edge.id,
      edge.kind,
      "",
      "",
      edge.source,
      edge.target,
      edge.amount_msat,
      edge.asset,
      edge.label,
      edge.evidence_level,
      JSON.stringify(edge.evidence),
    ]),
  );
  return (
    rows
      .map((row) =>
        row
          .map((cell) => {
            let value = cell == null ? "" : String(cell);
            // Human/provider labels are untrusted spreadsheet input. Preserve exact amounts.
            if (!/^-?\d+$/.test(value) && /^\s*[=+@-]/.test(value))
              value = `'${value}`;
            return `"${value.replace(/"/g, '""')}"`;
          })
          .join(","),
      )
      .join("\r\n") + "\r\n"
  );
}

export interface GraphPosition {
  x: number;
  y: number;
}
export interface GraphCamera extends GraphPosition {
  scale: number;
}

export function zoomAnalysisCamera(
  previous: GraphCamera,
  factor: number,
  point: GraphPosition,
): GraphCamera {
  const scale = Math.max(0.001, Math.min(4, previous.scale * factor));
  const ratio = scale / previous.scale;
  return {
    scale,
    x: point.x - (point.x - previous.x) * ratio,
    y: point.y - (point.y - previous.y) * ratio,
  };
}

export function panAnalysisCamera(
  previous: GraphCamera,
  delta: GraphPosition,
): GraphCamera {
  return { ...previous, x: previous.x + delta.x, y: previous.y + delta.y };
}
export interface AnalysisLayout {
  positions: Map<string, GraphPosition>;
  width: number;
  height: number;
}

/** Stable layered layout of a bounded result; hypotheses never distort the physical layout. */
export function layoutAnalysisGraph(
  nodes: AnalysisNode[],
  edges: AnalysisEdge[],
): AnalysisLayout {
  const ids = new Set(nodes.map((node) => node.id));
  const adjacency = new Map<string, string[]>();
  const indegree = new Map(nodes.map((node) => [node.id, 0]));
  for (const edge of edges) {
    if (
      edge.kind === "hypothesis" ||
      !ids.has(edge.source) ||
      !ids.has(edge.target) ||
      edge.source === edge.target
    )
      continue;
    adjacency.set(edge.source, [
      ...(adjacency.get(edge.source) || []),
      edge.target,
    ]);
    indegree.set(edge.target, (indegree.get(edge.target) || 0) + 1);
  }
  const roots = nodes
    .filter((node) => indegree.get(node.id) === 0)
    .map((node) => node.id);
  const levels = new Map<string, number>();
  const queue = [...roots];
  roots.forEach((id) => levels.set(id, 0));
  // Breadth-first levels avoid unbounded loops in reviewed/cyclic relations.
  for (let i = 0; i < queue.length; i++) {
    const id = queue[i];
    for (const next of adjacency.get(id) || []) {
      if (!levels.has(next)) {
        levels.set(next, Math.min(50, (levels.get(id) || 0) + 1));
        queue.push(next);
      }
    }
  }
  for (const node of nodes) if (!levels.has(node.id)) levels.set(node.id, 0);
  const columns = new Map<number, AnalysisNode[]>();
  for (const node of nodes) {
    const level = levels.get(node.id) || 0;
    columns.set(level, [...(columns.get(level) || []), node]);
  }
  const positions = new Map<string, GraphPosition>();
  const maxRows = Math.max(
    1,
    ...[...columns.values()].map((items) => items.length),
  );
  const height = Math.max(380, maxRows * 90 + 100);
  for (const [level, entries] of columns) {
    entries.forEach((node, index) =>
      positions.set(node.id, {
        x: 145 + level * 265,
        y: (height - entries.length * 90) / 2 + 45 + index * 90,
      }),
    );
  }
  return {
    positions,
    width: Math.max(700, (Math.max(0, ...columns.keys()) + 1) * 265 + 40),
    height,
  };
}

export function pathNodeIds(path: AnalysisResult["paths"][number]): string[] {
  return Array.isArray(path) ? path : path.node_ids || [];
}

export function readableAnalysisValue(value: unknown): string {
  if (value == null) return "—";
  if (typeof value === "string") return value;
  if (typeof value === "boolean") return value ? "true" : "false";
  if (typeof value === "number" || typeof value === "bigint")
    return String(value);
  return JSON.stringify(value, null, 2);
}
