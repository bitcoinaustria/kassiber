import { analysisNetworkInput, type AnalysisEntropyRequest, type AnalysisNode, type AnalysisQuery } from "./chainAnalysis";

/** Resolve observed physical identity without choosing among identical cross-network hashes. */
export function entropySubjectDomain(
  subject: string,
  nodes: AnalysisNode[],
  scope: Pick<AnalysisQuery, "chain" | "network"> = {},
): { status: "resolved"; subject: string; chain: "bitcoin" | "liquid"; network: string } | { status: "unknown" | "ambiguous" } {
  const value = subject.trim().toLowerCase();
  if (!value) return { status: "unknown" };
  const candidates = new Map<string, { subject: string; chain: "bitcoin" | "liquid"; network: string }>();
  for (const node of nodes) {
    if (node.kind !== "transaction" || ![node.id, node.txid, node.transaction_id].some(id => id?.toLowerCase() === value)) continue;
    if (node.chain !== "bitcoin" && node.chain !== "liquid") continue;
    if (!node.network) continue;
    const network = analysisNetworkInput(node.network);
    if (!["main", "test", "signet", "regtest"].includes(network) || (node.chain === "liquid" && network === "signet")) continue;
    if ((scope.chain && scope.chain !== node.chain) || (scope.network && analysisNetworkInput(scope.network) !== network)) continue;
    const candidate: { subject: string; chain: "bitcoin" | "liquid"; network: string } = { subject: node.id, chain: node.chain, network };
    candidates.set(JSON.stringify([node.id, node.chain, network]), candidate);
  }
  if (candidates.size !== 1) return { status: candidates.size ? "ambiguous" : "unknown" };
  return { status: "resolved", ...candidates.values().next().value! };
}

export interface EntropyScenario {
  kind: "independent" | "coinjoin_intrafees";
  protocol: "generic" | "whirlpool" | "joinmarket" | "wabisabi";
  max_received_fee_msat?: string;
  max_paid_fee_msat?: string;
}
export function exactSatoshiMsat(value: string): boolean {
    return /^\d{1,19}$/.test(value) && BigInt(value) % 1000n === 0n && BigInt(value) <= 2100000000000000000n;
}
export function entropyScenario(kind: EntropyScenario["kind"], protocol: EntropyScenario["protocol"], received: string, paid: string): EntropyScenario | null {
    if (kind === "independent")
        return {
            kind,
            protocol
        };
    return exactSatoshiMsat(received) && exactSatoshiMsat(paid)
        ? {
            kind,
            protocol,
            max_received_fee_msat: BigInt(received).toString(),
            max_paid_fee_msat: BigInt(paid).toString()
        }
        : null;
}
export interface EntropyJob {
  job_id: string;
  status: "running" | "completed" | "cancelled" | "failed";
  cancel_requested: boolean;
  request: AnalysisEntropyRequest & { snapshot_id?: string; psbt_token?: string };
  progress: { phase: string; states_explored: number; elapsed_ms?: number; interpretation_count_lower_bound?: string };
  result: Record<string, unknown> | null;
  error_code?: string | null;
  elapsed_ms: number;
}
interface LinkCount { input_id: string; output_id: string; interpretation_count: string }
export function entropyLinks(value: unknown): LinkCount[] {
    return Array.isArray(value) ? value.filter((r): r is LinkCount => !!r && typeof r.input_id === "string" && typeof r.output_id === "string" && typeof r.interpretation_count === "string" && /^\d+$/.test(r.interpretation_count)) : [];
}
/** Compare the same submitted subject/observer/snapshot, with exact rational arithmetic. */
export function entropyLinkChanges(before: Record<string, unknown>, after: Record<string, unknown>) {
    if (before.status !== "exact" || after.status !== "exact" || !before.snapshot_id || before.snapshot_id !== after.snapshot_id || before.subject !== after.subject || before.observer !== after.observer)
        return null;
    const a = before.interpretation_count, b = after.interpretation_count;
    if (typeof a !== "string" || typeof b !== "string" || !/^\d+$/.test(a) || !/^\d+$/.test(b) || BigInt(a) === 0n || BigInt(b) === 0n)
        return null;
    const previous = new Map(entropyLinks(before.link_counts).map(r => [JSON.stringify([r.input_id, r.output_id]), r]));
    const next = entropyLinks(after.link_counts);
    if (previous.size !== next.length || next.some(r => !previous.has(JSON.stringify([r.input_id, r.output_id]))))
        return null;
    return next.flatMap(r => {
        const old = previous.get(JSON.stringify([r.input_id, r.output_id]))!;
        const delta = BigInt(r.interpretation_count) * BigInt(a) - BigInt(old.interpretation_count) * BigInt(b);
        return delta === 0n ? [] : [{
                input_id: r.input_id,
                output_id: r.output_id,
                before: `${old.interpretation_count}/${a}`,
                after: `${r.interpretation_count}/${b}`,
                direction: delta > 0n ? "increased" as const : "decreased" as const,
                was_deterministic: old.interpretation_count === a,
                is_deterministic: r.interpretation_count === b
            }];
    });
}
export interface FeatureSnapshot { extractor_version?: string; source?: string; features: Array<{ code: string; value: unknown; availability: string; assumptions?: string[]; evidence?: unknown }>; limitations?: string[] }
export interface PsbtAnalysis {
  network: string; psbt_version: number; subject_id: string;
  transaction_facts: { complete: boolean; inputs: Array<{ input_index: number; output_id: string; amount_msat: string | null; utxo_evidence: string }>; outputs: Array<{ output_index: number; output_id: string; amount_msat: string | null; script_type: string }> };
  features: FeatureSnapshot; findings: Array<{ code: string; severity?: string; detail?: string; assumptions?: string[] }>;
  totals: { input_msat: string | null; output_msat: string; fee_msat: string | null; final_vsize: number | null; final_fee_rate_sat_vb: string | null };
  coverage: { known_input_amounts: number; input_count: number; missing_input_indices: number[]; previous_transaction_hashes_verified: number };
  validation: { status: string; limitations: string[]; input_metadata: Array<{ input_index: number; finalized: boolean; partial_signature_count: number; derivation_metadata_present: boolean; sighash_declared: number | null }> };
}
export interface PsbtComparison { before: PsbtAnalysis; after: PsbtAnalysis; delta: { inputs_added: string[]; inputs_removed: string[]; output_scripts_added: number; output_scripts_removed: number; fee_msat: string | null; features_changed: Array<{ code: string; before: unknown; after: unknown }>; findings_added: string[]; findings_removed: string[] }; payjoin?: { status: string; checks: Array<{ code: string; status: string; details: Record<string, unknown> }>; limitations?: string[] } }
export interface DatasetManifest { dataset_key: string; name: string; version: string; chain: "bitcoin" | "liquid"; network: string; source: string; license: string; attribution_method: string; visibility: "public" | "private"; expected_active_id?: string }
export interface DatasetClaim { id?: string; subject: string; label: string; category: string; source_record?: unknown; revision?: number; confidence?: string; source?: string; valid_from?: string | null; valid_until?: string | null; dataset_status?: string; dataset_version?: string; license?: string }
export interface Dataset { id: string; revision: number; status: string; version: string; row_count: number; content_sha256: string | null; visibility: string; manifest: DatasetManifest }
export interface DatasetPreview { manifest: DatasetManifest; sha256: string; byte_count: number; row_count: number; validation: string; sample_claims: DatasetClaim[] }
export function datasetPreviewKey(sourceToken: string | undefined, manifest: DatasetManifest, format: string, adapter: string): string {
    return JSON.stringify([sourceToken || null, ...Object.keys(manifest).sort().map(k => [k, manifest[k as keyof DatasetManifest]]), format, adapter]);
}
export function isCurrentDatasetPreview(currentKey: string, submitted: { key: string; preview: DatasetPreview } | null): boolean {
    return !!submitted && submitted.key === currentKey && submitted.preview.validation === "complete" && /^[a-f0-9]{64}$/.test(submitted.preview.sha256);
}
export function datasetReplacementManifest(dataset: Dataset): DatasetManifest {
    const old = dataset.manifest;
    // Stored manifests also contain parser and normalized optional fields, which
    // cannot be blindly spread into a new strict import request.
    return {
        dataset_key: old.dataset_key,
        name: old.name,
        version: "",
        chain: old.chain,
        network: ({
            liquidv1: "main",
            liquidtestnet: "test",
            elementsregtest: "regtest"
        } as Record<string, string>)[old.network] || old.network,
        source: old.source,
        license: old.license,
        attribution_method: old.attribution_method,
        visibility: old.visibility,
        expected_active_id: dataset.id
    };
}

export function payjoinOptions(form: { payment: string; feeOutput: string; maximum: string; minimumRate: string; substitute: boolean }) {
    const index = (value: string) => /^\d{1,4}$/.test(value) && Number(value) <= 4095;
    if (!index(form.payment) || (form.feeOutput && (!index(form.feeOutput) || Number(form.feeOutput) === Number(form.payment))) || !exactSatoshiMsat(form.maximum) || (form.minimumRate && !/^\d{1,12}(\.\d{1,12})?$/.test(form.minimumRate)))
        return null;
    return {
        payment_output_index: Number(form.payment),
        ...(form.feeOutput ? { additional_fee_output_index: Number(form.feeOutput) } : {}),
        max_additional_fee_contribution_msat: BigInt(form.maximum).toString(),
        allow_output_substitution: form.substitute,
        ...(form.minimumRate ? { minimum_fee_rate_sat_vb: form.minimumRate } : {})
    };
}

export function psbtAssistantContext(input: {
  beforeToken: string; afterToken?: string; network: string;
  payjoinEnabled: boolean; constraints: ReturnType<typeof payjoinOptions>;
  currentInputKey: string; comparisonInputKey?: string;
  entropyJob?: { job_id: string; source_token: string; network: string } | null;
}) {
    if (!input.beforeToken || (input.payjoinEnabled && !input.constraints))
        return null;
    const job = input.entropyJob;
    return {
        before_token: input.beforeToken,
        ...(input.afterToken ? { after_token: input.afterToken } : {}),
        network: input.network,
        ...(input.payjoinEnabled ? { payjoin: input.constraints } : {}),
        current_comparison_available: input.currentInputKey === input.comparisonInputKey,
        ...(job && job.network === input.network && [input.beforeToken, input.afterToken].includes(job.source_token)
            ? {
                entropy_job_id: job.job_id,
                entropy_source_token: job.source_token
            } : {}),
    };
}
