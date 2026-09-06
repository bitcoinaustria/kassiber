import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { PsbtResult } from "./PsbtPanel";
import { FeatureDetails } from "./FeatureDetails";
import { CaseComparison } from "./CaseComparison";
import { JobProgress } from "./JobProgress";
import { EntropyPanel } from "./EntropyPanel";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { DEFAULT_ANALYSIS_QUERY, type AnalysisNode } from "@/lib/chainAnalysis";
import type { PsbtAnalysis } from "@/lib/chainAnalysisWorkbench";

describe("structured evidence panels", () => {
  it("opens the unfiltered entropy form on the known subject's network and leaves ambiguous domains unselected", () => {
    const txid = "2".repeat(64);
    const observed: AnalysisNode = {
      id: `tx:bitcoin:regtest:${txid}`, txid, kind: "transaction", chain: "bitcoin",
      network: "regtest", label: "TX", wallet_ids: [], amount_msat: null,
      asset: "BTC", evidence: []
    };
    const render = (nodes: AnalysisNode[]) => renderToStaticMarkup(
      <QueryClientProvider client={new QueryClient()}>
        <EntropyPanel subject={txid} query={DEFAULT_ANALYSIS_QUERY} observedNodes={nodes} onError={() => {}} />
      </QueryClientProvider>,
    );
    expect(render([observed])).toContain('<option selected=""');
    expect(render([observed])).toMatch(/<option[^>]*selected=""[^>]*>regtest<\/option>/);
    expect(render([observed])).not.toContain("does not identify one observed");
    const ambiguous = render([observed, { ...observed, id: `tx:bitcoin:main:${txid}`, network: "main" }]);
    expect(ambiguous).toContain("does not identify one observed");
    expect(ambiguous).toMatch(/<option value="" selected="">/);
  });
  it("shows unknown PSBT totals and unverified supplied UTXO evidence explicitly", () => {
    const result: PsbtAnalysis={
      network:"regtest",
      psbt_version:2,
      subject_id:"psbt:transaction",
      transaction_facts:{
        complete:false,
        inputs:[{
          input_index:0,
          output_id:"out",
          amount_msat:null,
          utxo_evidence:"missing"
        }],
        outputs:[]
      },
      features:{features:[]},
      findings:[],
      totals:{
        input_msat:null,
        output_msat:"1000",
        fee_msat:null,
        final_vsize:null,
        final_fee_rate_sat_vb:null
      },
      coverage:{
        known_input_amounts:0,
        input_count:1,
        missing_input_indices:[0],
        previous_transaction_hashes_verified:0
      },
      validation:{
        status:"structurally_valid",
        limitations:["supplied_utxo_amounts_are_not_chain_verified"],
        input_metadata:[]
      }
    };
    const html=renderToStaticMarkup(<PsbtResult result={result} />);
    expect(html).toContain("Unknown");
    expect(html).toContain("0/1");
    expect(html).toContain("supplied_utxo_amounts_are_not_chain_verified");
    expect(html).not.toContain("<pre");
  });
  it("retains partial feature availability rather than an all-clear score", () => {
    const html=renderToStaticMarkup(<FeatureDetails snapshot={{
      extractor_version:"v1",
      source:"psbt_supplied",
      features:[{
        code:"signature_encodings",
        availability:"partial",
        value:{
          count:0,
          verified:false
        },
        assumptions:["encoding_is_not_signature_verification"]
      }]
    }} />);
    expect(html).toContain("partial");
    expect(html).toContain("encoding_is_not_signature_verification");
    expect(html).toContain("No");
  });
  it("explains changed source exposure and corrected features even when topology is unchanged", () => {
    const html=renderToStaticMarkup(<CaseComparison comparison={{
      changed:true,
      removed_exposure:[{
        id:"exposure",
        claim:{label:"revoked_claim"}
      }],
      changed_transaction_features:[{
        id:"tx",
        before:{feature:"unknown"},
        after:{feature:"observed"}
      }]
    }} />);
    expect(html).toContain("Label exposure");
    expect(html).toContain("Transaction characteristics");
    expect(html).toContain("revoked_claim");
    expect(html).toContain("observed");
  });
  it("shows real dataset cleanup progress with cancellation, without fabricated percentages", () => {
    const html=renderToStaticMarkup(<JobProgress
    job={{
      job_id:"job",
      status:"running",
      request:{},
      result:null,
      cancel_requested:false,
      elapsed_ms:12,
      progress:{
        phase:"discarding",
        rows_deleted:2000
      }
    }}
    pollError={false}
    onResume={()=>{ }}
    onCancel={()=>{ }} />);
    expect(html).toContain("2000 rows removed");
    expect(html).toContain("Cancel computation");
    expect(html).not.toContain("%");
  });
});
