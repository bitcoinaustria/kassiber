import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import "@/i18n";
import { router } from "@/routeTree";
import type { PrivacyMirrorPayload } from "@/lib/privacyMirror";

import { PrivacyMirrorPayloadView } from "./PrivacyMirror";

const PRIVACY_MIRROR_TEST_PAYLOAD: PrivacyMirrorPayload = {
  local_only: true,
  advisory_only: true,
  summary: {
    evidence_level: "exact",
    privacy_score: {
      value: 53,
      base: 100,
      coverage_ratio: 0.5,
      factors: [
        { key: "wallet_linkage", linked: 1, total: 2, points: -28 },
        { key: "transaction_leaks", leaking: 2, total: 3, points: -19 },
      ],
    },
    worst_risk: {
      kind: "common_input",
      severity: "warning",
      title: "Common-input ownership heuristic links clusters",
      answer: "Common-input evidence linked two local clusters.",
      evidence_level: "exact",
    },
  },
  adversary_cards: [
    {
      tier: "passive_chain_watcher",
      label: "Passive chain watcher",
      evidence_level: "derived",
      summary: { exposed_cluster_count: 2, wallet_count: 1 },
      model_assumptions: [
        {
          code: "bitcoin_graph_facts_only",
          statement: "Uses local Bitcoin transaction graph facts.",
          evidence_level: "derived",
        },
      ],
    },
  ],
  wallet_view: [
    {
      wallet_id: "wallet:multisig-vault",
      coin_count: 2,
      amount_msat: 18_400_000_000,
      linkage_edge_count: 2,
      evidence_level: "exact",
    },
  ],
  transaction_view: [
    {
      txid: "tx-common-input-demo",
      tell_count: 2,
      tell_kinds: ["sender_common_input", "fee_fingerprint"],
      wallet_penalty_count: 2,
      evidence_level: "exact",
    },
  ],
  utxo_view: [
    {
      coin_id: "coin:1",
      wallet_id: "wallet:multisig-vault",
      amount_msat: 12_500_000_000,
      branch_role: "receive",
      source_proximity: "known_source_proximity",
      evidence_level: "exact",
    },
  ],
  timeline: [
    {
      id: "edge:1",
      kind: "common_input",
      category: "linkage",
      txid: "tx-common-input-demo",
      evidence_level: "exact",
      new_linkage: true,
    },
  ],
  coverage: {
    evidence_level: "exact",
    source_proximity_known_coin_count: 1,
    source_proximity_unknown_coin_count: 0,
    degraded: false,
  },
  evidence_drilldowns: [
    {
      section: "findings",
      id: "common_input_linkage",
      kind: "common_input",
      evidence_level: "exact",
    },
  ],
};

// The elevated PSBT panel uses a daemon mutation, so the view needs a query
// client in context; the mutation is idle at render so no transport is needed.
function renderMirror() {
  return renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      <PrivacyMirrorPayloadView
        payload={PRIVACY_MIRROR_TEST_PAYLOAD}
      />
    </QueryClientProvider>,
  );
}

describe("PrivacyMirror route", () => {
  it("registers the dedicated Privacy Mirror page route", () => {
    expect(router.routesByPath["/privacy-mirror"]).toBeTruthy();
  });

  it("renders the score hero, primary recommendation, findings, and detail sections", () => {
    const html = renderMirror();

    expect(html).toContain('data-testid="privacy-mirror-page"');
    expect(html).toContain('data-testid="privacy-score-grade"');
    expect(html).toContain('data-testid="privacy-mirror-worst-risk"');
    expect(html).toContain('data-testid="privacy-mirror-findings"');
    // Score hero: with weighted tells the mock payload lands at grade C.
    expect(html).toContain("notable exposure");
    expect(html).toContain("What to fix first");
    // The worst risk is shown in plain language, not the engine's phrasing.
    expect(html).toContain("Wallets linked by a shared-input spend");
    // Machine tell tokens are humanized into readable finding titles.
    expect(html).toContain("Common input");
    // Detail sections remain as collapsible triggers.
    expect(html).toContain("Who can infer it");
    expect(html).toContain("The evidence");
    expect(html).toContain("All details");
    // Grounded score: the waterfall shows real factor counts, not a made-up base.
    expect(html).toContain("Linked wallets");
    expect(html).toContain("Origin coverage");
    // Pre-broadcast check is elevated to its own visible section, not buried.
    expect(html).toContain('data-testid="privacy-mirror-psbt"');
    // The old tab shell and its duplicate mobile stack are gone.
    expect(html).not.toContain('data-testid="privacy-mirror-mobile-stack"');
    expect(html).not.toContain('role="tablist"');
  });

  it("keeps AI/export-redacted material out of the rendered mirror", () => {
    const html = renderMirror();

    expect(html).not.toContain("xpub");
    expect(html).not.toContain("descriptor");
    expect(html).not.toContain("bc1q");
    expect(html).not.toContain("script_pubkey");
    expect(html).not.toContain("raw_json");
  });
});
