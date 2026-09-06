import { layoutEvidenceGraph } from "@/components/evidence-graph/layout";

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

/**
 * Primary graph caption. Meaningful labels (wallets, records, custom names) pass
 * through; labels that merely repeat or prefix the physical identity are replaced
 * by a head+tail abbreviation so same-prefix identities stay distinguishable.
 * An outpoint keeps its index. The full identity remains in the accessible name.
 */
export function analysisNodeCaption(
  node: Pick<AnalysisNode, "label" | "txid" | "outpoint" | "id">,
  maximum = 26,
): string {
  const identity = node.outpoint || node.txid || node.id;
  const label = (node.label || "").trim();
  const stem = label.replace(/[….]+$/, "");
  const colon = node.outpoint ? node.outpoint.lastIndexOf(":") : -1;
  const [core, index] =
    node.outpoint && colon > 0
      ? [node.outpoint.slice(0, colon), node.outpoint.slice(colon + 1)]
      : [identity, undefined];
  // Backend output labels are `${txid prefix}:${vout}`; the prefix alone is not readable.
  const abbreviatedOutpoint =
    index !== undefined &&
    /^[0-9a-f]+:\d+$/i.test(stem) &&
    stem.endsWith(`:${index}`) &&
    core.startsWith(stem.slice(0, stem.lastIndexOf(":")));
  const derived =
    !stem ||
    abbreviatedOutpoint ||
    [node.id, node.txid, node.outpoint].some(
      (value) => value && (value === label || value.startsWith(stem)),
    );
  if (!derived) return shortAnalysisId(label, maximum);
  const abbreviated =
    core.length > 20 ? `${core.slice(0, 8)}…${core.slice(-8)}` : core;
  return index === undefined ? abbreviated : `${abbreviated}:${index}`;
}

/**
 * Layers the engine actually applied. The public observer never sees private
 * custody relations, whatever the request said; hypotheses stay as requested.
 */
export function analysisEffectiveLayers(
  query: Pick<AnalysisQuery, "observer" | "include_relations" | "include_hypotheses">,
): { relations: boolean; hypotheses: boolean } {
  return {
    relations: query.include_relations && query.observer !== "public",
    hypotheses: query.include_hypotheses,
  };
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

/** The node the executed subject names exactly; ambiguity or a fuzzy match yields nothing. */
export function analysisSubjectNodeId(
  result: Pick<AnalysisResult, "query" | "nodes">,
): string | undefined {
  const subject = result.query.subject?.trim();
  if (!subject) return undefined;
  // Outputs carry their creating txid, so a bare txid marks the transaction only.
  const matches = result.nodes.filter(
    (node) =>
      node.id === subject ||
      node.outpoint === subject ||
      (node.kind === "transaction" && node.txid === subject),
  );
  return matches.length === 1 ? matches[0].id : undefined;
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

export type { GraphPosition, GraphCamera, EvidenceGraphLayout as AnalysisLayout } from "@/components/evidence-graph/types";
export { zoomGraphCamera as zoomAnalysisCamera, panGraphCamera as panAnalysisCamera } from "@/components/evidence-graph/layout";

/** Analysis controls which edge authorities affect the shared presentation layout. */
export function layoutAnalysisGraph(nodes: AnalysisNode[], edges: AnalysisEdge[]) {
  return layoutEvidenceGraph(nodes, edges.map((edge) => ({
    source: edge.source, target: edge.target, participatesInLayout: edge.kind !== "hypothesis",
  })));
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
