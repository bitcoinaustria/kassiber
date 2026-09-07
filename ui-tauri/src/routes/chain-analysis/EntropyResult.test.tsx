import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import "@/i18n";
import { EntropyResult } from "./InvestigationPanels";
import {
  currentAnalysisEntropyOutcome,
  type AnalysisEntropyOutcome,
  type AnalysisEntropyRequest,
} from "@/lib/chainAnalysis";
import fixture from "./__fixtures__/entropy.json";

describe("real engine entropy contract", () => {
  const request: AnalysisEntropyRequest = {
    subject: "tx-a",
    chain: "bitcoin",
    network: "regtest",
    max_states: 50_000,
  };
  it("keeps an in-flight result for A hidden after the subject is edited to B", () => {
    let outcome: AnalysisEntropyOutcome | null = null;
    let currentRequest = request;
    const submitted = currentRequest;
    expect(
      currentAnalysisEntropyOutcome(currentRequest, outcome, true),
    ).toBeNull();
    currentRequest = { ...request, subject: "tx-b" };
    outcome = { request: submitted, result: fixture.exact };
    expect(
      currentAnalysisEntropyOutcome(currentRequest, outcome, false),
    ).toBeNull();
    expect(currentAnalysisEntropyOutcome(request, outcome, false)).toBe(
      outcome,
    );
    expect(currentAnalysisEntropyOutcome(request, outcome, true)).toBeNull();
  });
  it("does not reuse an old count after changing the search budget or domain", () => {
    const outcome = { request, result: fixture.exact };
    expect(
      currentAnalysisEntropyOutcome(
        { ...request, max_states: 100 },
        outcome,
        false,
      ),
    ).toBeNull();
    expect(
      currentAnalysisEntropyOutcome(
        { ...request, network: "main" },
        outcome,
        false,
      ),
    ).toBeNull();
    expect(
      currentAnalysisEntropyOutcome(
        { ...request, chain: "liquid" },
        outcome,
        false,
      ),
    ).toBeNull();
  });
  it("shows the resolved subject beside the submitted domain and state budget", () => {
    const html = renderToStaticMarkup(
      <EntropyResult
        result={{
          ...fixture.exact,
          subject: "transaction:bitcoin:regtest:tx-a",
        }}
        request={request}
      />,
    );
    expect(html).toContain("Analyzed subject");
    expect(html).toContain("transaction:bitcoin:regtest:tx-a");
    expect(html).toContain("bitcoin / regtest");
    expect(html).toContain(">50000</dd>");
  });
  it("shows exact interpretation count and conditional entropy using the engine field names", () => {
    const html = renderToStaticMarkup(<EntropyResult result={fixture.exact} />);
    expect(fixture.exact.interpretation_count).toBe("3");
    expect(html).toContain("Feasible partitions");
    expect(html).toContain(">3</dd>");
    expect(html).toContain("1.584962500721156");
    expect(html).toContain("no_intergroup_payments_or_intrafees");
  });
  it("shows a lower bound without publishing an entropy or exact interpretation total for a bounded search", () => {
    const html = renderToStaticMarkup(
      <EntropyResult result={fixture.bounded} />,
    );
    expect(html).toContain("Interpretations found (lower bound)");
    expect(html).not.toContain("Entropy (bits)");
    expect(html).not.toContain("Feasible partitions");
    expect(html).toContain(
      `>${fixture.bounded.interpretation_count_lower_bound}</dd>`,
    );
  });
  it("explains unsupported Payjoin without presenting a privacy number", () => {
    const html = renderToStaticMarkup(
      <EntropyResult result={fixture.unsupported} />,
    );
    expect(html).toContain("joint_payment_or_unknown_collaboration");
    expect(html).toContain("unsupported");
    expect(html).not.toContain("Interpretations found (lower bound)");
    expect(html).not.toContain("Entropy (bits)");
  });
});
