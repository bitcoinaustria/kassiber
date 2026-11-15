import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import "@/i18n";

// Missing-wallet suggestions are advisory; only canonical custody issues
// hold back tax basis. The verdict must say which one the owner is looking at.
const snapshot = vi.hoisted(() => ({ summary: {} as Record<string, unknown>, gaps: [] as unknown[] }));
vi.mock("@/daemon/client", () => ({
  useDaemon: () => ({}),
  useDaemonInfinite: () => ({ data: { pages: [{ data: snapshot }] } }),
  useDaemonMutation: () => ({}),
}));

import { CustodyGapsContent } from "./CustodyGaps";

const gap = {
  gap_id: "gap-1",
  status: "needs_review",
  confidence: "strong",
  promotion_eligible: false,
  source_wallet_label: "Cold storage",
  destination_wallet_labels: ["Mining rig"],
  started_at: "2022-08-30T00:00:00Z",
  ended_at: "2022-08-30T00:00:00Z",
  asset: "BTC",
  source_total_msat: "5905070000",
  source_fee_msat: "583000",
  source_debit_msat: "5905653000",
  return_total_msat: "5916780000",
  residual_msat: "0",
};

const base = { total: 1, needs_review: 1, conflicting: 0, resolved: 0, dismissed: 0, unresolved_msat: "0", derived_state_current: true, search_complete: true };

describe("missing-wallet verdict", () => {
  beforeEach(() => {
    snapshot.gaps = [gap];
  });

  it("calls advisory suggestions non-blocking and shows no held amount", () => {
    snapshot.summary = { ...base, canonical_issue_count: 0, canonical_unresolved_by_asset: [] };
    const html = renderToStaticMarkup(<CustodyGapsContent focusGapId={null} />);
    expect(html).toContain("1 send may have come back through a wallet you didn&#x27;t connect");
    expect(html).toContain("they don&#x27;t block reports");
    expect(html).not.toContain("holds back tax basis");
    expect(html).toContain("Strong match");
    expect(html).not.toContain("Nonblocking search hint</");
  });

  it("names the blocking custody issue and its exact amount when there is one", () => {
    snapshot.summary = { ...base, canonical_issue_count: 1, canonical_unresolved_by_asset: [{ asset: "BTC", amount_msat: "600000000" }] };
    const html = renderToStaticMarkup(<CustodyGapsContent focusGapId={null} />);
    expect(html).toContain("1 custody issue holds back tax basis");
    expect(html).toContain("0.006 BTC");
    expect(html).not.toContain("don&#x27;t block reports");
  });
});
