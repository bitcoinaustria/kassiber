import { describe, expect, it } from "vitest";
import { privacyHeadline, type PrivacyFinding, type PrivacyMirrorPayload } from "./privacyMirror";

const summary = (overrides: Partial<PrivacyMirrorPayload["summary"]> = {}): PrivacyMirrorPayload["summary"] => ({
  status: "no_observed_exposure", finding_count: 0, attention_count: 0, owned_output_count: 5,
  analyzed_transaction_count: 13, local_transaction_count: 13, domain_count: 1, ...overrides,
});
const relevance = (...values: PrivacyFinding["relevance"][]) => values.map(value => ({ relevance: value }));

describe("Privacy Mirror headline", () => {
  it("reports personal findings from relevance, not from the warning-severity attention count", () => {
    // Actual backend shape: an ordinary owned RBF-signalling spend is an info finding.
    expect(privacyHeadline(summary({ finding_count: 1, owned_output_count: 1 }), relevance("own_spend"))).toBe("personal_findings");
    expect(privacyHeadline(summary({ status: "findings", finding_count: 2, attention_count: 1 }), relevance("owned_output", "nearby_context"))).toBe("personal_findings");
    expect(privacyHeadline(summary({ status: "findings", finding_count: 1, attention_count: 1, owned_output_count: 0 }), relevance("own_spend"))).toBe("personal_findings");
    // A warning-severity surrounding finding is not personal exposure.
    expect(privacyHeadline(summary({ status: "findings", finding_count: 1, attention_count: 1 }), relevance("received_context"))).toBe("context_only");
  });
  it("separates surrounding-activity findings from personal exposure only in an assessed snapshot", () => {
    const context = relevance("received_context", "nearby_context", "received_context");
    expect(privacyHeadline(summary({ status: "findings", finding_count: 3 }), context)).toBe("context_only");
    // QA book: 13 local transactions, no owned outputs, backend unavailable, 3 context findings.
    expect(privacyHeadline(summary({ status: "unavailable", finding_count: 3, owned_output_count: 0 }), context)).toBe("no_owned_outputs");
    expect(privacyHeadline(summary({ status: "unavailable", finding_count: 3 }), context)).toBe("unavailable");
    expect(privacyHeadline(summary({ status: "findings", finding_count: 3, owned_output_count: 0 }), context)).toBe("no_owned_outputs");
    expect(privacyHeadline(summary({ status: "findings", finding_count: 3, local_transaction_count: 0, analyzed_transaction_count: 0 }), context)).toBe("no_local_evidence");
  });
  it("distinguishes no local transactions from no owned outputs in a fully examined set", () => {
    expect(privacyHeadline(summary({ status: "unavailable", owned_output_count: 0, analyzed_transaction_count: 0, local_transaction_count: 0, domain_count: 0 }), [])).toBe("no_local_evidence");
    expect(privacyHeadline(summary({ owned_output_count: 0 }), [])).toBe("no_owned_outputs");
    expect(privacyHeadline(summary({ status: "unavailable", owned_output_count: 0 }), [])).toBe("no_owned_outputs");
  });
  it("never upgrades an unavailable assessment with owned outputs into a clean result", () => {
    expect(privacyHeadline(summary({ status: "unavailable" }), [])).toBe("unavailable");
    expect(privacyHeadline(summary(), [])).toBe("nothing_found");
    // Partial examination keeps the same headline; the coverage badge, not the headline, states the gap.
    expect(privacyHeadline(summary({ analyzed_transaction_count: 2 }), [])).toBe("nothing_found");
  });
});
