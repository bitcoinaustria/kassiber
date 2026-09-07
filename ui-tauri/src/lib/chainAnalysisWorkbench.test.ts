import { describe, expect, it } from "vitest";
import { currentAnalysisEntropyOutcome, type AnalysisEntropyRequest, type AnalysisNode } from "./chainAnalysis";
import { datasetPreviewKey, datasetReplacementManifest, entropyLinkChanges, entropyScenario, entropySubjectDomain, exactSatoshiMsat, isCurrentDatasetPreview, payjoinOptions, psbtAssistantContext, type Dataset, type DatasetManifest, type DatasetPreview } from "./chainAnalysisWorkbench";

describe("entropy subject domains", () => {
    const txid = "2".repeat(64);
    const node = (network: string, chain = "bitcoin"): AnalysisNode => ({
        id: `tx:${chain}:${network}:${txid}`, txid, network, chain,
        kind: "transaction", label: "TX", wallet_ids: [],
        amount_msat: null, asset: "BTC", evidence: []
    });
    it("binds an unfiltered raw txid to the uniquely observed regtest transaction", () => {
        const observed = node("regtest");
        expect(entropySubjectDomain(txid, [observed])).toEqual({
            status: "resolved", subject: observed.id, chain: "bitcoin", network: "regtest"
        });
        expect(entropySubjectDomain(observed.id, [observed, node("main")])).toHaveProperty("network", "regtest");
    });
    it("does not choose a network or chain for ambiguous hashes", () => {
        const nodes = [node("main"), node("regtest"), node("liquidv1", "liquid")];
        expect(entropySubjectDomain(txid, nodes)).toEqual({ status: "ambiguous" });
        expect(entropySubjectDomain(txid, nodes, { network: "main" })).toEqual({ status: "ambiguous" });
        expect(entropySubjectDomain(txid, nodes, { chain: "bitcoin", network: "regtest" })).toHaveProperty("subject", node("regtest").id);
        expect(entropySubjectDomain(txid, nodes, { chain: "liquid", network: "main" })).toEqual({
            status: "resolved", subject: node("liquidv1", "liquid").id, chain: "liquid", network: "main"
        });
    });
    it("derives a new domain for a changed canonical subject and leaves unknown/invalid domains unknown", () => {
        const nodes = [node("main"), node("regtest")];
        expect(entropySubjectDomain(nodes[0].id, nodes)).toHaveProperty("network", "main");
        expect(entropySubjectDomain(nodes[1].id, nodes)).toHaveProperty("network", "regtest");
        expect(entropySubjectDomain("missing", nodes)).toEqual({ status: "unknown" });
        expect(entropySubjectDomain(txid, [node("")])).toEqual({ status: "unknown" });
        expect(entropySubjectDomain(txid, [node("invented")])).toEqual({ status: "unknown" });
        expect(entropySubjectDomain(nodes[1].id, nodes, { network: "main" })).toEqual({ status: "unknown" });
    });
});

describe("exact fee forms", () => {
    it("hands off both current PSBT grants and constraints without stale comparison authority", () => {
        const constraints = payjoinOptions({
            payment: "0",
            feeOutput: "1",
            maximum: "1234000",
            minimumRate: "1.25",
            substitute: true
        });
        const input = {
            beforeToken: "opaque-before",
            afterToken: "opaque-after",
            network: "regtest",
            payjoinEnabled: true,
            constraints,
            currentInputKey: "new-selection",
            comparisonInputKey: "old-selection",
            entropyJob: {
                job_id: "job",
                source_token: "opaque-after",
                network: "regtest"
            }
        };
        const context = psbtAssistantContext(input);
        expect(context).toEqual({
            before_token: "opaque-before",
            after_token: "opaque-after",
            network: "regtest",
            payjoin: constraints,
            current_comparison_available: false,
            entropy_job_id: "job",
            entropy_source_token: "opaque-after"
        });
        expect(JSON.stringify(context)).not.toMatch(/filename|file_path|raw_psbt/);
        expect(psbtAssistantContext({
            ...input,
            afterToken: "replacement"
        })).not.toHaveProperty("entropy_job_id");
        expect(psbtAssistantContext({
            ...input,
            network: "main"
        })).not.toHaveProperty("entropy_job_id");
        expect(psbtAssistantContext({
            ...input,
            constraints: null
        })).toBeNull();
        expect(psbtAssistantContext({
            ...input,
            payjoinEnabled: false
        })).not.toHaveProperty("payjoin");
    });
    it("preserves whole-satoshi msat beyond Number precision", () => {
        const amount = "2100000000000000000";
        expect(entropyScenario("coinjoin_intrafees", "joinmarket", amount, "1000")).toEqual({
            kind: "coinjoin_intrafees",
            protocol: "joinmarket",
            max_received_fee_msat: amount,
            max_paid_fee_msat: "1000"
        });
        for (const invalid of ["-1000", "0.1", "1", "1e6", "1,000", "2100000000000001000"])
            expect(exactSatoshiMsat(invalid)).toBe(false);
        expect(entropyScenario("independent", "wabisabi", "bad", "bad")).toEqual({
            kind: "independent",
            protocol: "wabisabi"
        });
    });
    it("validates Payjoin output choices and preserves fee-rate decimals", () => {
        const form = {
            payment: "0",
            feeOutput: "1",
            maximum: "9007199254741000",
            minimumRate: "1.000000000001",
            substitute: false
        };
        expect(payjoinOptions(form)).toEqual({
            payment_output_index: 0,
            additional_fee_output_index: 1,
            max_additional_fee_contribution_msat: form.maximum,
            minimum_fee_rate_sat_vb: form.minimumRate,
            allow_output_substitution: false
        });
        expect(payjoinOptions({
            ...form,
            feeOutput: "0"
        })).toBeNull();
        expect(payjoinOptions({
            ...form,
            payment: "999999999999999"
        })).toBeNull();
        expect(payjoinOptions({
            ...form,
            maximum: "1001"
        })).toBeNull();
        expect(payjoinOptions({
            ...form,
            minimumRate: "Infinity"
        })).toBeNull();
    });
});

describe("conditional scenario comparisons", () => {
    const result = {
        status: "exact",
        snapshot_id: "snapshot",
        subject: "tx",
        observer: "owner",
        interpretation_count: "9007199254740993",
        link_counts: [{
                input_id: "in",
                output_id: "out",
                interpretation_count: "9007199254740992"
            }]
    };
    it("detects a one-count difference above IEEE integer precision", () => {
        const after = {
            ...result,
            link_counts: [{
                    input_id: "in",
                    output_id: "out",
                    interpretation_count: "9007199254740993"
                }]
        };
        expect(entropyLinkChanges(result, after)).toEqual([{
                input_id: "in",
                output_id: "out",
                before: "9007199254740992/9007199254740993",
                after: "9007199254740993/9007199254740993",
                direction: "increased",
                was_deterministic: false,
                is_deterministic: true
            }]);
    });
    it("rejects partial results, mismatched snapshots/observers/subjects and changed link populations", () => {
        for (const change of [{ status: "timeout" }, { snapshot_id: "changed" }, { observer: "public" }, { subject: "other" }, { link_counts: [] }])
            expect(entropyLinkChanges(result, {
                ...result,
                ...change
            })).toBeNull();
        expect(entropyLinkChanges(result, result)).toEqual([]);
    });
    it("invalidates legacy outcome association when scenario, observer or time changes", () => {
        const request: AnalysisEntropyRequest = {
            subject: "tx",
            max_states: 100,
            observer: "owner",
            max_duration_ms: 1000,
            scenario: {
                kind: "independent",
                protocol: "generic"
            }
        };
        const outcome = {
            request,
            result
        };
        for (const change of [{ observer: "public" as const }, { max_duration_ms: 2000 }, { scenario: {
                    kind: "independent" as const,
                    protocol: "joinmarket" as const
                } }])
            expect(currentAnalysisEntropyOutcome({
                ...request,
                ...change
            }, outcome, false)).toBeNull();
    });
});

describe("dataset preview and replacement boundaries", () => {
    const manifest: DatasetManifest = {
        dataset_key: "test",
        name: "Data",
        version: "1",
        chain: "bitcoin",
        network: "main",
        source: "local",
        license: "MIT",
        attribution_method: "source claim",
        visibility: "private"
    };
    const preview: DatasetPreview = {
        manifest,
        sha256: "a".repeat(64),
        byte_count: 100,
        row_count: 10,
        validation: "complete",
        sample_claims: []
    };
    it("requires the same source grant, manifest, adapter and format for confirmation", () => {
        const key = datasetPreviewKey("grant-a", manifest, "csv", "generic"), submitted = {
            key,
            preview
        };
        expect(isCurrentDatasetPreview(key, submitted)).toBe(true);
        for (const changed of [datasetPreviewKey("grant-b", manifest, "csv", "generic"), datasetPreviewKey("grant-a", {
                ...manifest,
                visibility: "public"
            }, "csv", "generic"), datasetPreviewKey("grant-a", manifest, "jsonl", "generic"), datasetPreviewKey("grant-a", manifest, "csv", "maru92")])
            expect(isCurrentDatasetPreview(changed, submitted)).toBe(false);
        expect(isCurrentDatasetPreview(key, {
            key,
            preview: {
                ...preview,
                validation: "partial"
            }
        })).toBe(false);
        expect(isCurrentDatasetPreview(key, {
            key,
            preview: {
                ...preview,
                sha256: ""
            }
        })).toBe(false);
    });
    it("copies only request fields from stored normalized metadata and pins replacement revision ID", () => {
        const pack: Dataset = {
            id: "dataset-id",
            version: "1",
            status: "active",
            revision: 3,
            row_count: 10,
            content_sha256: "hash",
            visibility: "private",
            manifest: {
                ...manifest,
                chain: "liquid",
                network: "liquidv1",
                ...{
                    format: "csv",
                    adapter: "generic",
                    source_url: null
                }
            }
        };
        expect(datasetReplacementManifest(pack)).toEqual({
            ...manifest,
            chain: "liquid",
            network: "main",
            version: "",
            expected_active_id: "dataset-id"
        });
        expect(datasetReplacementManifest(pack)).not.toHaveProperty("format");
    });
});
